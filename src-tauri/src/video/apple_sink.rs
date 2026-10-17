// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! macOS hardware video decode sink (MOBILE_RTSP.md P3) — the AVFoundation counterpart of
//! `win_sink` / `android_sink`: H.264/HEVC access units from the RTSP client are wrapped as
//! compressed `CMSampleBuffer`s and enqueued into an `AVSampleBufferDisplayLayer`, which decodes
//! them through VideoToolbox and renders straight into the hole-punch layer below the WebView
//! (`apple_host.rs`). One decode thread owns the conversion; the layers live in the host.
//!
//! **One layer per surface** (VIDEO_MULTISINK_WINDOW.md §4.2): the video widget and the floating
//! window at once, or a surface in the detached video window. AVFoundation decodes inside the
//! layer, so a second surface means a second VideoToolbox session — accepted (D9); a shared
//! `VTDecompressionSession` feeding both would be a large block of code for a few percent of GPU.
//! Everything that belongs to a session is therefore per output: the resume-on-keyframe wait, the
//! control timebase and its pacing anchor. The format description is immutable and shared.
//!
//! An output that leaves the published list is DROPPED, not hidden: unlike the Windows sink (one
//! decoder, N cheap presents) a kept layer here would decode the whole stream for a picture nobody
//! sees. The price is that a surface coming back waits for the next keyframe.
//!
//! The real work is the framing conversion: the depacketizer delivers Annex-B (start codes,
//! in-band parameter sets), CoreMedia wants AVCC/HVCC (4-byte big-endian length prefixes, the
//! parameter sets in a `CMVideoFormatDescription`). Per AU the NAL units are split, VPS/SPS/PPS
//! are captured (the description is rebuilt only when their bytes change — encoders resend
//! identical sets on every keyframe), and the slice/SEI units are re-emitted length-prefixed.
//!
//! Presentation: with the smoothing buffer at 0 (the latency-first default) every sample is
//! tagged `DisplayImmediately`. Depths 1–3 schedule each frame `depth × frame-interval` behind
//! the media timeline on the layer's control timebase — the Android `Pacing` logic (EMA
//! interval, anchor, re-anchor on jumps or depth changes).
//!
//! Resume on keyframe: after a flush (decode error, `requiresFlushToResumeDecoding`) AUs are
//! dropped until one carries an IDR/IRAP, so the decoder never restarts mid-GOP.

// API generation: the direct AVSampleBufferDisplayLayer enqueue/flush/status methods are what
// macOS 13 (our minimum) offers; macOS 14 moved them to `sampleBufferRenderer` and marked these
// deprecated. One generation, targeted cleanly — revisit when the minimum OS moves to 14.
// (`CMTimebase::create_with_master_clock` and `CFDictionarySetValue` are renamed-only.)
#![allow(deprecated)]

use std::ptr::{null, null_mut, NonNull};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use objc2::rc::Retained;
use objc2_av_foundation::{
    AVLayerVideoGravityResizeAspect, AVQueuedSampleBufferRenderingStatus, AVSampleBufferDisplayLayer,
};
use objc2_core_foundation::{kCFBooleanTrue, CFArray, CFDictionarySetValue, CFMutableDictionary, CFRetained};
use objc2_core_media::{
    kCMBlockBufferAssureMemoryNowFlag, kCMSampleAttachmentKey_DisplayImmediately, kCMTimeInvalid,
    CMBlockBuffer, CMClock, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimebase,
    CMVideoFormatDescriptionCreateFromH264ParameterSets, CMVideoFormatDescriptionCreateFromHEVCParameterSets,
    CMVideoFormatDescriptionGetPresentationDimensions,
};

use super::apple_host as host;
use super::rtsp::VideoCodec;
use super::surface::SinkSurface;

/// How long a layer attach may take (one main-thread hop away) before the output is given up on.
const LAYER_ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
/// RTP video clock — the timestamps `push` receives are 90 kHz ticks.
const TIMESCALE: i32 = 90_000;
/// Surfaces served at once (D1: the widget tile plus one large surface). Each costs a decode here,
/// so the cap is a real budget, not just politeness.
const MAX_OUTPUTS: usize = 2;

enum Cmd {
    /// One access unit (Annex-B, in-band parameter sets) + its unwrapped 90 kHz timestamp.
    Frame(Vec<u8>, u64),
    Orient { mirror: bool, rotate180: bool },
    Stop,
}

#[derive(Default)]
struct Shared {
    error: Mutex<Option<String>>,
    presented: AtomicU64,
    /// Frames skipped while no surface was on screen, so the stall detector sees them as progress.
    dropped_hidden: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    /// Smoothing-buffer depth in frames (0 = display immediately).
    buffer_frames: AtomicU32,
    stopping: AtomicBool,
    /// What the router last published, latest-wins, plus a revision the decode thread compares
    /// against its own. Deliberately NOT a channel message: geometry arrives once per animation
    /// frame during a drag, and anything queued behind the frames would make the layer trail the
    /// hole by the whole backlog (the lesson `win_sink` learned in PR #128).
    surfaces: Mutex<Vec<SinkSurface>>,
    rev: AtomicU64,
}

impl Shared {
    fn fail(&self, msg: String) {
        log::warn!("[video] apple sink: {msg}");
        self.error.lock().unwrap().get_or_insert(msg);
    }
}

/// The display layer, shared between the main thread (tree, geometry) and the decode thread
/// (enqueue / flush / status — AVFoundation's queued-rendering entry points are thread-safe).
struct Layer(Retained<AVSampleBufferDisplayLayer>);
// SAFETY: the layer's tree membership and geometry are only touched on the main thread (host);
// the decode thread uses the enqueue/flush/status calls, which AVFoundation documents as
// callable from any thread.
unsafe impl Send for Layer {}
unsafe impl Sync for Layer {}

pub struct AppleVideoSink {
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl AppleVideoSink {
    /// Bring the sink up for `codec`: create the display layer (on the main thread, inside the
    /// host's container) and start the decode thread. Decoder problems surface later through
    /// [`Self::error`] — the same contract as the other sinks.
    pub fn start(codec: VideoCodec) -> Result<Self, String> {
        let hevc = matches!(codec, VideoCodec::H265);
        let shared = Arc::new(Shared::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                decode_loop(hevc, &rx, &shared);
                host::remove_all();
            })
        };
        Ok(Self { tx, thread: Some(thread), shared })
    }

    /// Queue one access unit (Annex-B) with its unwrapped 90 kHz timestamp.
    pub fn push(&self, au: Vec<u8>, ts90k: u64) {
        let _ = self.tx.send(Cmd::Frame(au, ts90k));
    }

    /// First fatal sink error, if any — the stream ends on it (see rtsp_native).
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// The surfaces to present into (VIDEO_MULTISINK_WINDOW.md §4.1), highest priority first and
    /// capped at [`MAX_OUTPUTS`]. Each is served by its own layer in its own window; an empty list
    /// means nothing is on screen and every output goes away. Cheap: the decode thread picks the
    /// list up on its next pass (see `Shared::surfaces`).
    pub fn set_surfaces(&self, surfaces: &[SinkSurface]) {
        let mut want: Vec<SinkSurface> = surfaces.iter().take(MAX_OUTPUTS).cloned().collect();
        if surfaces.len() > MAX_OUTPUTS {
            log::debug!(
                "[video] apple sink: {} surfaces published, {MAX_OUTPUTS} served",
                surfaces.len()
            );
        }
        let mut slot = self.shared.surfaces.lock().unwrap();
        if *slot != want {
            std::mem::swap(&mut *slot, &mut want);
            self.shared.rev.fetch_add(1, Ordering::Release);
        }
    }

    /// Smoothing-buffer depth in frames (0 = display immediately — the latency-first default).
    pub fn set_buffer(&self, frames: u32) {
        self.shared.buffer_frames.store(frames.min(3), Ordering::Relaxed);
    }

    /// Mirror / 180° rotation — a layer transform, applied live.
    pub fn set_orient(&self, mirror: bool, rotate180: bool) {
        let _ = self.tx.send(Cmd::Orient { mirror, rotate180 });
    }

    pub fn frames_presented(&self) -> u64 {
        self.shared.presented.load(Ordering::Relaxed)
    }

    /// Frames skipped while no surface was on screen.
    pub fn frames_dropped_hidden(&self) -> u64 {
        self.shared.dropped_hidden.load(Ordering::Relaxed)
    }

    /// Display picture size (clean aperture, not the coded size), once the parameter sets arrived.
    pub fn picture_size(&self) -> Option<(u32, u32)> {
        let w = self.shared.width.load(Ordering::Relaxed);
        let h = self.shared.height.load(Ordering::Relaxed);
        (w > 0 && h > 0).then_some((w, h))
    }
}

impl Drop for AppleVideoSink {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One live output: the surface it serves plus everything that belongs to its VideoToolbox
/// session, which a second layer has a second of.
struct Out {
    key: String,
    layer: Arc<Layer>,
    /// This session starts on an intra frame; a flush or new parameter sets restart the wait.
    wait_for_idr: bool,
    timebase: Option<CFRetained<CMTimebase>>,
    pacing: Pacing,
    /// Last rect handed to the host — placing is a main-thread hop, so it happens on change only.
    rect: [i32; 8],
}

/// Create a display layer for surface `key` in `window`, on the main thread (inside the host's
/// attach, which builds the container view there if this is the surface's first appearance), and
/// return the shared handle the decode thread enqueues through.
fn create_layer(key: String, window: String) -> Result<Arc<Layer>, String> {
    let slot: Arc<Mutex<Option<Arc<Layer>>>> = Arc::new(Mutex::new(None));
    let fill = slot.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let res = host::attach(key, window, move || {
            // SAFETY: plain AVFoundation object creation + property setup on the main thread.
            let layer = unsafe { AVSampleBufferDisplayLayer::new() };
            unsafe {
                if let Some(g) = AVLayerVideoGravityResizeAspect {
                    layer.setVideoGravity(g);
                }
            }
            *fill.lock().unwrap() = Some(Arc::new(Layer(layer.clone())));
            Retained::into_super(layer)
        });
        let _ = tx.send(res);
    });
    rx.recv_timeout(LAYER_ATTACH_TIMEOUT)
        .map_err(|_| "display layer attach timed out".to_string())??;
    let layer = slot.lock().unwrap().take();
    layer.ok_or_else(|| "display layer was not created".into())
}

/// Bring `outs` in line with what the router published: a surface that appeared gets a container,
/// a layer and its rect; one that moved gets the new rect; one that left is dropped entirely (see
/// the module docs — a kept layer would keep decoding).
///
/// A surface whose window has no NSWindow (a detached window that is already closing) simply gets
/// no output and is retried on the next push; nothing here can fail the stream.
fn sync_outputs(outs: &mut Vec<Out>, want: &[SinkSurface], order: &mut Vec<String>) {
    outs.retain(|out| {
        if want.iter().any(|s| s.key == out.key) {
            return true;
        }
        // SAFETY: thread-safe queued-rendering call.
        unsafe { out.layer.0.flushAndRemoveImage() };
        host::remove(out.key.clone());
        log::info!("[video] apple sink: output {} gone", out.key);
        false
    });
    for s in want {
        let rect = [s.full.0, s.full.1, s.full.2, s.full.3, s.clip.0, s.clip.1, s.clip.2, s.clip.3];
        if let Some(out) = outs.iter_mut().find(|o| o.key == s.key) {
            if out.rect != rect {
                out.rect = rect;
                host::place(s.key.clone(), rect);
            }
            continue;
        }
        match create_layer(s.key.clone(), s.window.clone()) {
            Ok(layer) => {
                host::place(s.key.clone(), rect);
                log::info!("[video] apple sink: output for surface {} created", s.key);
                outs.push(Out {
                    key: s.key.clone(),
                    layer,
                    wait_for_idr: true,
                    timebase: None,
                    pacing: Pacing::default(),
                    rect,
                });
            }
            Err(e) => log::warn!("[video] apple sink: no output for surface {}: {e}", s.key),
        }
    }
    // Stack them the way the DOM stacks their surfaces — matters wherever two overlap, e.g. the
    // floating window dragged over the video widget, where the widget has to stay on top.
    let want_order: Vec<String> = want
        .iter()
        .filter(|s| outs.iter().any(|o| o.key == s.key))
        .map(|s| s.key.clone())
        .collect();
    if *order != want_order {
        host::restack(want_order.clone());
        *order = want_order;
    }
}

/// The decode thread: follow the published surfaces, then convert + enqueue AUs until stop.
/// Layers come and go with the surfaces — there is none until the DOM asks for one.
fn decode_loop(hevc: bool, rx: &Receiver<Cmd>, shared: &Shared) {
    log::info!("[video] apple sink: decode thread up ({})", if hevc { "HEVC" } else { "H.264" });

    let mut sets = ParameterSets::default();
    let mut format: Option<CFRetained<CMFormatDescription>> = None;
    let mut outs: Vec<Out> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut applied_rev = u64::MAX; // force one sync before the first frame

    loop {
        // Surfaces first, so a frame always lands in the layout the DOM published last.
        let rev = shared.rev.load(Ordering::Acquire);
        if rev != applied_rev {
            applied_rev = rev;
            let want = shared.surfaces.lock().unwrap().clone();
            sync_outputs(&mut outs, &want, &mut order);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => {
                for out in &outs {
                    unsafe { out.layer.0.flushAndRemoveImage() };
                }
                return;
            }
            Ok(Cmd::Orient { mirror, rotate180 }) => host::set_orient(mirror, rotate180),
            Ok(Cmd::Frame(au, ts90k)) => {
                // Parameter sets first — a rebuilt description is what makes the next IDR decodable.
                let changed = sets.absorb(hevc, &au);
                if changed || format.is_none() {
                    match sets.format_description(hevc) {
                        Ok(Some(desc)) => {
                            // SAFETY: valid description from the create call above.
                            let dims = unsafe {
                                CMVideoFormatDescriptionGetPresentationDimensions(&desc, false, true)
                            };
                            if dims.width > 0.0 && dims.height > 0.0 {
                                shared.width.store(dims.width as u32, Ordering::Relaxed);
                                shared.height.store(dims.height as u32, Ordering::Relaxed);
                                log::info!("[video] apple sink: picture {}x{}", dims.width as u32, dims.height as u32);
                            }
                            format = Some(desc);
                            if changed {
                                // New sets mid-stream: every session must restart on the next IDR.
                                for out in &mut outs {
                                    out.wait_for_idr = true;
                                }
                            }
                        }
                        Ok(None) => {} // sets incomplete — keep waiting
                        Err(e) => {
                            shared.fail(e);
                            return;
                        }
                    }
                }
                let Some(desc) = format.as_ref() else { continue };
                if outs.is_empty() {
                    // Nothing on screen: no session to feed, so nothing to decode.
                    shared.dropped_hidden.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let depth = shared.buffer_frames.load(Ordering::Relaxed);
                let pts = ts90k as i64;
                // Computed at most once per AU, and only while some session is waiting for one.
                let mut intra: Option<bool> = None;
                // ONE sample per frame, enqueued into every live layer: CMSampleBuffer is
                // immutable and reference-counted, and each layer decodes it for itself (D9).
                let mut sample = None;

                for out in &mut outs {
                    // Flush check first: a revoked decoder (app backgrounded) also reports Failed.
                    // SAFETY: thread-safe queued-rendering calls.
                    unsafe {
                        if out.layer.0.requiresFlushToResumeDecoding() {
                            log::warn!(
                                "[video] apple sink: {} requires a flush — resuming on the next keyframe",
                                out.key
                            );
                            out.layer.0.flush();
                            out.wait_for_idr = true;
                            out.pacing.anchor = None;
                            continue;
                        }
                        if out.layer.0.status() == AVQueuedSampleBufferRenderingStatus::Failed {
                            let msg = out
                                .layer
                                .0
                                .error()
                                .map(|e| e.localizedDescription().to_string())
                                .unwrap_or_else(|| "display layer failed".into());
                            shared.fail(format!("decode failed ({}): {msg}", out.key));
                            return;
                        }
                    }
                    if out.wait_for_idr {
                        if !*intra.get_or_insert_with(|| has_intra_frame(hevc, &au)) {
                            continue;
                        }
                        out.wait_for_idr = false;
                    }

                    // Depth > 0 paces on this session's control timebase; depth 0 displays
                    // immediately. Switching depth re-anchors the timeline.
                    if depth == 0 {
                        if out.timebase.take().is_some() {
                            unsafe { out.layer.0.setControlTimebase(None) };
                        }
                    } else {
                        let tb = match out.timebase.as_ref() {
                            Some(tb) => tb.clone(),
                            None => match new_timebase() {
                                Ok(tb) => {
                                    unsafe { out.layer.0.setControlTimebase(Some(&tb)) };
                                    out.timebase = Some(tb.clone());
                                    tb
                                }
                                Err(e) => {
                                    shared.fail(e);
                                    return;
                                }
                            },
                        };
                        out.pacing.schedule(&tb, pts, depth);
                    }

                    if sample.is_none() {
                        match make_sample(hevc, desc, &au, pts, depth == 0) {
                            Ok(s) => sample = Some(s),
                            Err(e) => {
                                shared.fail(e);
                                return;
                            }
                        }
                    }
                    let Some(buf) = sample.as_ref() else { continue };
                    // SAFETY: valid sample buffer; enqueue is thread-safe.
                    unsafe { out.layer.0.enqueueSampleBuffer(buf) };
                }
                // Per FRAME, not per surface — it is what the panel shows as fps.
                if sample.is_some() {
                    shared.presented.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// A control timebase on the host clock, running at rate 1 — the layer displays a sample when
/// the timebase reaches its PTS.
fn new_timebase() -> Result<CFRetained<CMTimebase>, String> {
    // SAFETY: CoreMedia create call with an out-pointer; the retained result is taken over below.
    unsafe {
        let clock = CMClock::host_time_clock();
        let mut out: *mut CMTimebase = null_mut();
        let st = CMTimebase::create_with_master_clock(None, &clock, NonNull::new_unchecked(&mut out));
        if st != 0 || out.is_null() {
            return Err(format!("CMTimebaseCreate failed ({st})"));
        }
        let tb = CFRetained::from_raw(NonNull::new_unchecked(out));
        tb.set_rate(1.0);
        Ok(tb)
    }
}

/// Presentation pacing for smoothing-buffer depths > 0 (the Android sink's logic): the
/// timebase is anchored so that a frame's PTS falls `depth × frame-interval` after now; the EMA
/// of the media frame interval sizes that cushion; a timeline jump or a changed depth re-anchors.
#[derive(Default)]
struct Pacing {
    /// (media pts at anchor, depth) — once anchored, the timebase runs on its own.
    anchor: Option<(i64, u32)>,
    /// EMA of the media-time frame interval (90 kHz ticks); seeds at 60 fps.
    interval: i64,
    last_pts: Option<i64>,
}

impl Pacing {
    fn schedule(&mut self, tb: &CMTimebase, pts: i64, depth: u32) {
        if self.interval == 0 {
            self.interval = 1500; // 60 fps in 90 kHz ticks
        }
        if let Some(last) = self.last_pts {
            let delta = pts - last;
            if (450..=9000).contains(&delta) {
                self.interval += (delta - self.interval) / 8;
            }
        }
        self.last_pts = Some(pts);
        let lead = depth as i64 * self.interval;
        // SAFETY: plain timebase reads/writes.
        unsafe {
            let now = tb.time();
            let now_ticks = if now.timescale == TIMESCALE { now.value } else { now.value * TIMESCALE as i64 / now.timescale.max(1) as i64 };
            let re_anchor = match self.anchor {
                Some((_, d)) if d == depth => {
                    // Drifted past the cushion or behind the clock: re-anchor (no runaway schedule).
                    let due_in = pts - now_ticks;
                    !(0..=lead + 9000).contains(&due_in)
                }
                _ => true,
            };
            if re_anchor {
                tb.set_time(CMTime::new(pts - lead, TIMESCALE));
                self.anchor = Some((pts, depth));
            }
        }
    }
}

/// VPS/SPS/PPS bytes as last seen (emulation prevention kept — CoreMedia wants raw NAL bytes).
#[derive(Default)]
struct ParameterSets {
    vps: Vec<Vec<u8>>,
    sps: Vec<Vec<u8>>,
    pps: Vec<Vec<u8>>,
}

impl ParameterSets {
    /// Capture the parameter sets in `au`. Returns true when the set changed byte-wise.
    fn absorb(&mut self, hevc: bool, au: &[u8]) -> bool {
        let mut changed = false;
        for nal in nal_units(au) {
            let Some(&b0) = nal.first() else { continue };
            let slot = if hevc {
                match (b0 >> 1) & 0x3f {
                    32 => &mut self.vps,
                    33 => &mut self.sps,
                    34 => &mut self.pps,
                    _ => continue,
                }
            } else {
                match b0 & 0x1f {
                    7 => &mut self.sps,
                    8 => &mut self.pps,
                    _ => continue,
                }
            };
            if !slot.iter().any(|s| s == nal) {
                // A re-sent set with the same id but different content replaces; a genuinely new
                // one (different id) is added. Ids are not parsed: for the streams a GCS meets,
                // the newest instance per type is what the encoder uses.
                slot.clear();
                slot.push(nal.to_vec());
                changed = true;
            }
        }
        changed
    }

    /// Build the format description once all required sets are present.
    fn format_description(&self, hevc: bool) -> Result<Option<CFRetained<CMFormatDescription>>, String> {
        let mut all: Vec<&[u8]> = Vec::new();
        if hevc {
            if self.vps.is_empty() || self.sps.is_empty() || self.pps.is_empty() {
                return Ok(None);
            }
            all.extend(self.vps.iter().map(Vec::as_slice));
        } else if self.sps.is_empty() || self.pps.is_empty() {
            return Ok(None);
        }
        all.extend(self.sps.iter().map(Vec::as_slice));
        all.extend(self.pps.iter().map(Vec::as_slice));
        let mut ptrs: Vec<NonNull<u8>> = all.iter().map(|s| NonNull::from(&s[0])).collect();
        let mut sizes: Vec<usize> = all.iter().map(|s| s.len()).collect();
        let mut out: *const CMFormatDescription = null();
        // SAFETY: the pointer/size arrays outlive the call; CoreMedia copies the sets.
        let st = unsafe {
            if hevc {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    None,
                    all.len(),
                    NonNull::new_unchecked(ptrs.as_mut_ptr()),
                    NonNull::new_unchecked(sizes.as_mut_ptr()),
                    4,
                    None,
                    NonNull::new_unchecked(&mut out),
                )
            } else {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    None,
                    all.len(),
                    NonNull::new_unchecked(ptrs.as_mut_ptr()),
                    NonNull::new_unchecked(sizes.as_mut_ptr()),
                    4,
                    NonNull::new_unchecked(&mut out),
                )
            }
        };
        if st != 0 || out.is_null() {
            return Err(format!("format description from parameter sets failed ({st})"));
        }
        // SAFETY: a +1 retained object from a Create call.
        Ok(Some(unsafe { CFRetained::from_raw(NonNull::new_unchecked(out as *mut CMFormatDescription)) }))
    }
}

/// One compressed sample: the AU's slice/SEI units with 4-byte length prefixes (parameter sets
/// and access-unit delimiters dropped — they live in the description / carry nothing).
fn make_sample(
    hevc: bool,
    desc: &CMFormatDescription,
    au: &[u8],
    pts: i64,
    display_immediately: bool,
) -> Result<CFRetained<CMSampleBuffer>, String> {
    let mut data: Vec<u8> = Vec::with_capacity(au.len() + 16);
    for nal in nal_units(au) {
        let Some(&b0) = nal.first() else { continue };
        let skip = if hevc {
            matches!((b0 >> 1) & 0x3f, 32 | 33 | 34 | 35)
        } else {
            matches!(b0 & 0x1f, 7 | 8 | 9)
        };
        if skip {
            continue;
        }
        data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        data.extend_from_slice(nal);
    }
    if data.is_empty() {
        return Err("access unit without slices".into());
    }

    // SAFETY: CoreMedia create calls with out-pointers; every retained result is taken over.
    unsafe {
        let mut bb: *mut CMBlockBuffer = null_mut();
        let st = CMBlockBuffer::create_with_memory_block(
            None,
            null_mut(),
            data.len(),
            None,
            null(),
            0,
            data.len(),
            kCMBlockBufferAssureMemoryNowFlag,
            NonNull::new_unchecked(&mut bb),
        );
        if st != 0 || bb.is_null() {
            return Err(format!("CMBlockBufferCreate failed ({st})"));
        }
        let bb = CFRetained::from_raw(NonNull::new_unchecked(bb));
        let st = CMBlockBuffer::replace_data_bytes(
            NonNull::new_unchecked(data.as_ptr() as *mut std::ffi::c_void),
            &bb,
            0,
            data.len(),
        );
        if st != 0 {
            return Err(format!("CMBlockBufferReplaceDataBytes failed ({st})"));
        }

        let timing = CMSampleTimingInfo {
            duration: kCMTimeInvalid,
            presentationTimeStamp: CMTime::new(pts, TIMESCALE),
            decodeTimeStamp: kCMTimeInvalid,
        };
        let size = data.len();
        let mut sb: *mut CMSampleBuffer = null_mut();
        let st = CMSampleBuffer::create_ready(
            None,
            Some(&bb),
            Some(desc),
            1,
            1,
            &timing,
            1,
            &size,
            NonNull::new_unchecked(&mut sb),
        );
        if st != 0 || sb.is_null() {
            return Err(format!("CMSampleBufferCreateReady failed ({st})"));
        }
        let sb = CFRetained::from_raw(NonNull::new_unchecked(sb));

        if display_immediately {
            if let Some(atts) = sb.sample_attachments_array(true) {
                let atts: &CFArray = &atts;
                let dict = atts.value_at_index(0) as *const CFMutableDictionary;
                if let (Some(dict), Some(yes)) = (dict.as_ref(), kCFBooleanTrue) {
                    CFDictionarySetValue(
                        Some(dict),
                        (kCMSampleAttachmentKey_DisplayImmediately as *const _) as *const std::ffi::c_void,
                        (yes as *const _) as *const std::ffi::c_void,
                    );
                }
            }
        }
        Ok(sb)
    }
}

/// Split an Annex-B byte stream into NAL unit payloads (start codes of 3 or 4 bytes).
fn nal_units(au: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i + 3 <= au.len() {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { au.len() };
        // A 4-byte start code leaves a trailing zero on the previous unit.
        while e > s && au[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&au[s..e]);
        }
    }
    out
}

/// Does this AU contain an intra frame (H.264 IDR / HEVC IRAP)?
fn has_intra_frame(hevc: bool, au: &[u8]) -> bool {
    nal_units(au).iter().any(|nal| {
        let Some(&b0) = nal.first() else { return false };
        if hevc {
            (16..=21).contains(&((b0 >> 1) & 0x3f))
        } else {
            b0 & 0x1f == 5
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_annex_b_and_finds_idr() {
        // SPS (7), PPS (8), IDR (5) with 4- and 3-byte start codes.
        let au = [
            0, 0, 0, 1, 0x67, 1, 2, // SPS
            0, 0, 1, 0x68, 3, // PPS
            0, 0, 0, 1, 0x65, 9, 9, 9, // IDR
        ];
        let units = nal_units(&au);
        assert_eq!(units.len(), 3);
        assert_eq!(units[0], &[0x67, 1, 2]);
        assert_eq!(units[1], &[0x68, 3]);
        assert_eq!(units[2], &[0x65, 9, 9, 9]);
        assert!(has_intra_frame(false, &au));
        assert!(!has_intra_frame(true, &au));

        let mut sets = ParameterSets::default();
        assert!(sets.absorb(false, &au));
        assert!(!sets.absorb(false, &au), "identical sets must not count as a change");
        assert_eq!(sets.sps.len(), 1);
    }
}
