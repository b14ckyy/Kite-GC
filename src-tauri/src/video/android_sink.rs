// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Android hardware video decode sink (MOBILE_RTSP.md P2.2 stage 3) — the MediaCodec
//! counterpart of `win_sink`: H.264/HEVC access units from the RTSP client go into the
//! platform decoder (`AMediaCodec`, NDK C API — no Kotlin in the decode path) and render
//! straight onto the hole-punch SurfaceView below the WebView (`android/native_video.rs`,
//! `NativeVideo.kt`). One decode thread owns codec + surface.
//!
//! Presentation: with the smoothing buffer at 0 (the latency-first default) every decoded
//! frame renders on release. Depths 1–3 use `releaseOutputBufferAtTime` instead — the frame
//! is scheduled `depth × frame-interval` behind its media timeline and SurfaceFlinger does
//! the pacing (no queue of our own, unlike the Windows sink; the held frames sit in the
//! surface's buffer queue, which is why the depth stays capped small).
//!
//! Surface lifecycle: the codec's lifetime is keyed to the view host's surface GENERATION
//! (bumped on surfaceCreated/surfaceDestroyed), NOT to activity pause — PiP keeps a paused
//! activity visible and playing. When the surface dies (app background, view destroyed) the
//! decoder tears down, waits for the next surface, rebuilds, and drops frames until the next
//! IDR/IRAP so the fresh decoder never starts mid-GOP.
//!
//! Orientation: 180° rotation maps to the decoder format's `rotation-degrees` — a toggle
//! rebuilds the decoder (short blackout, resumes on the next IDR/IRAP). Mirroring is NOT
//! available on this path, and that is measured, not assumed: view transforms do not reach
//! a SurfaceView's surface content, and an `ANativeWindow_setBuffersTransform` on the
//! decode window gets overwritten by MediaCodec's own per-buffer transform state (both
//! visually disproven on the M11). The remaining route — a GL intermediary — is out of all
//! proportion for a niche toggle; the DOM-rendered MJPEG route mirrors fine on Android.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;
use ndk::native_window::NativeWindow;

use super::rtsp::VideoCodec;
use crate::android::native_video as view_host;

/// How long the initial wait for a surface may take before the sink declares failure —
/// generous, the view creation is one UI-thread hop away. Later losses (app background)
/// wait indefinitely: the stream keeps running and rejoins on the next surface.
const FIRST_SURFACE_TIMEOUT: Duration = Duration::from_secs(10);

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
    width: AtomicU32,
    height: AtomicU32,
    /// Smoothing-buffer depth in frames (0 = render on release). Written by the panel
    /// stepper, read by the decode loop per frame.
    buffer_frames: AtomicU32,
    stopping: AtomicBool,
}

impl Shared {
    fn fail(&self, msg: String) {
        log::warn!("[video] android sink: {msg}");
        self.error.lock().unwrap().get_or_insert(msg);
    }
}

pub struct AndroidVideoSink {
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl AndroidVideoSink {
    /// Bring the sink up for `codec`: create the (1×1, invisible-until-arranged) surface
    /// layer and start the decode thread. Decoder problems surface later through
    /// [`Self::error`] — mirroring the Windows sink's contract with `rtsp_native`.
    pub fn start(codec: VideoCodec) -> Result<Self, String> {
        let mime: &'static str = match codec {
            VideoCodec::H265 => "video/hevc",
            _ => "video/avc",
        };
        view_host::show(0, 0, 1, 1)?;
        let shared = Arc::new(Shared::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                decode_loop(mime, &rx, &shared);
                if let Err(e) = view_host::destroy() {
                    log::debug!("[video] android sink: view teardown: {e}");
                }
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

    /// On-screen video rect (PHYSICAL px, window coords): the FULL box `x/y/w/h` for the
    /// aspect-fit layout plus the VISIBLE part `cx/cy/cw/ch` — the view host clips the
    /// video at that edge (scrolled panels), it never shrinks into the remainder.
    #[allow(clippy::too_many_arguments)]
    pub fn set_rect(&self, x: i32, y: i32, w: i32, h: i32, cx: i32, cy: i32, cw: i32, ch: i32) {
        if let Err(e) = view_host::set_rect(x, y, w, h, cx, cy, cw, ch) {
            log::debug!("[video] android sink: set_rect: {e}");
        }
    }

    pub fn set_visible(&self, visible: bool) {
        if let Err(e) = view_host::set_visible(visible) {
            log::debug!("[video] android sink: set_visible: {e}");
        }
    }

    /// Smoothing-buffer depth in frames (0 = render on release — the latency-first
    /// default). Capped small: scheduled frames occupy the surface's finite buffer queue.
    pub fn set_buffer(&self, frames: u32) {
        self.shared.buffer_frames.store(frames.min(3), Ordering::Relaxed);
    }

    /// Mirror / 180° rotation — applied live as a buffer transform on the decode window.
    pub fn set_orient(&self, mirror: bool, rotate180: bool) {
        let _ = self.tx.send(Cmd::Orient { mirror, rotate180 });
    }

    pub fn frames_presented(&self) -> u64 {
        self.shared.presented.load(Ordering::Relaxed)
    }

    /// Decoded picture size (display area), once the decoder reported its output format.
    pub fn picture_size(&self) -> Option<(u32, u32)> {
        let w = self.shared.width.load(Ordering::Relaxed);
        let h = self.shared.height.load(Ordering::Relaxed);
        (w > 0 && h > 0).then_some((w, h))
    }
}

impl Drop for AndroidVideoSink {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The decode thread: (re)acquire surface → build decoder → pump AUs until stop or surface
/// loss. One iteration of the outer loop = one decoder session on one surface.
fn decode_loop(mime: &'static str, rx: &Receiver<Cmd>, shared: &Shared) {
    // A resumed session starts clean on the next IDR/IRAP; the very first session takes the
    // stream as it comes (the depacketizer prepends parameter sets and the client starts
    // delivery on an intra frame where the source provides one).
    let mut wait_for_idr = false;
    // Current orientation, kept across decoder sessions (the frontend syncs it right after
    // the sink starts, so it can arrive while we still wait for the first surface).
    let mut orient = (false, false);

    'session: loop {
        let window = match wait_for_surface(rx, shared, &mut orient) {
            Some(w) => w,
            None => return, // stopped (or initial timeout — error already recorded)
        };
        let generation = view_host::generation().unwrap_or(0);

        let Some(codec) = MediaCodec::from_decoder_type(mime) else {
            shared.fail(format!("no {mime} hardware decoder on this device"));
            return;
        };
        let mut format = MediaFormat::new();
        format.set_str("mime", mime);
        // Nominal size — the decoder re-negotiates from the in-band SPS/VPS and reports the
        // real dimensions through OUTPUT_FORMAT_CHANGED below.
        format.set_i32("width", 1280);
        format.set_i32("height", 720);
        // Best effort (API 30+ decoders; unknown keys are ignored elsewhere).
        format.set_i32("low-latency", 1);
        if orient.1 {
            format.set_i32("rotation-degrees", 180);
        }
        if let Err(e) = codec.configure(&format, Some(&window), MediaCodecDirection::Decoder) {
            shared.fail(format!("decoder configure failed: {e:?}"));
            return;
        }
        if let Err(e) = codec.start() {
            shared.fail(format!("decoder start failed: {e:?}"));
            return;
        }
        log::info!(
            "[video] android sink: {} decoder up ({})",
            mime,
            codec.name().unwrap_or_else(|_| "?".into())
        );
        let mut pacing = Pacing::default();

        loop {
            // Surface gone (app background, view destroyed)? End this decoder session and
            // wait for the next surface; resume clean on an intra frame.
            if view_host::generation().unwrap_or(generation) != generation {
                log::warn!("[video] android sink: surface lost — rebuilding the decoder on the next one");
                let _ = codec.stop();
                wait_for_idr = true;
                continue 'session;
            }

            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = codec.stop();
                    return;
                }
                Ok(Cmd::Orient { mirror, rotate180 }) => {
                    if mirror && !orient.0 {
                        log::debug!("[video] android sink: mirror is not available on the hardware decode path");
                    }
                    orient.0 = mirror;
                    if rotate180 != orient.1 {
                        orient.1 = rotate180;
                        log::info!("[video] android sink: rotation changed — rebuilding the decoder");
                        let _ = codec.stop();
                        wait_for_idr = true;
                        continue 'session;
                    }
                }
                Ok(Cmd::Frame(au, ts90k)) => {
                    if wait_for_idr && !has_intra_frame(mime, &au) {
                        continue;
                    }
                    wait_for_idr = false;
                    if let Err(e) = feed(&codec, &au, ts90k) {
                        shared.fail(e);
                        let _ = codec.stop();
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }

            if let Err(e) = drain(&codec, shared, &mut pacing) {
                shared.fail(e);
                let _ = codec.stop();
                return;
            }
        }
    }
}

/// Presentation pacing for smoothing-buffer depths > 0: frames are scheduled on the
/// compositor clock `depth × frame-interval` behind their media timeline, absorbing that
/// much arrival jitter. `anchor` maps media time onto CLOCK_MONOTONIC; a timeline jump
/// (loss gap, seek-like discontinuity) or a changed depth re-anchors.
#[derive(Default)]
struct Pacing {
    /// (monotonic ns, media µs) of the anchor frame, plus the depth it was built for.
    anchor: Option<(i64, i64, u32)>,
    /// EMA of the media-time frame interval (µs); seeds at 60 fps until measured.
    interval_us: i64,
    last_pts_us: Option<i64>,
}

fn monotonic_ns() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: plain out-parameter syscall; CLOCK_MONOTONIC always exists on Android.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64
}

/// Wait for a live surface, draining (and dropping) queued frames meanwhile so a stop is
/// never missed and the channel never balloons. Returns `None` on stop or initial timeout.
fn wait_for_surface(
    rx: &Receiver<Cmd>,
    shared: &Shared,
    orient: &mut (bool, bool),
) -> Option<NativeWindow> {
    let started = Instant::now();
    let mut reported_first_timeout = false;
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            return None;
        }
        match view_host::acquire_native_window() {
            Ok(Some(w)) => return Some(w),
            Ok(None) => {}
            Err(e) => {
                shared.fail(format!("surface lookup failed: {e}"));
                return None;
            }
        }
        // The INITIAL surface must arrive quickly (the view is one UI hop away); report a
        // clear error if it never does. Presentation counter > 0 means we ran before — then
        // this is a background/rebuild wait, which is open-ended by design.
        let ran_before = shared.presented.load(Ordering::Relaxed) > 0;
        if !ran_before && started.elapsed() > FIRST_SURFACE_TIMEOUT && !reported_first_timeout {
            reported_first_timeout = true;
            shared.fail("no output surface appeared".to_string());
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return None,
            Ok(Cmd::Orient { mirror, rotate180 }) => *orient = (mirror, rotate180),
            Ok(Cmd::Frame(..)) => {} // dropped — nothing to decode onto yet
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Copy one AU into a decoder input buffer. A stuffed decoder (no input buffer in 100 ms)
/// drops the AU — the client's frame counters make that visible.
fn feed(codec: &MediaCodec, au: &[u8], ts90k: u64) -> Result<(), String> {
    match codec.dequeue_input_buffer(Duration::from_millis(100)) {
        Ok(DequeuedInputBufferResult::Buffer(mut ib)) => {
            let dst = ib.buffer_mut();
            if dst.len() < au.len() {
                log::debug!(
                    "[video] android sink: AU of {} bytes exceeds the input buffer ({}) — dropped",
                    au.len(),
                    dst.len()
                );
                // The buffer must go back to the codec: queue it empty.
                return codec
                    .queue_input_buffer(ib, 0, 0, 0, 0)
                    .map_err(|e| format!("queueInputBuffer (empty): {e:?}"));
            }
            // SAFETY: `dst` is at least `au.len()` bytes (checked above); MaybeUninit<u8>
            // has u8's layout.
            unsafe {
                std::ptr::copy_nonoverlapping(au.as_ptr(), dst.as_mut_ptr().cast::<u8>(), au.len());
            }
            let pts_us = ts90k * 1000 / 90;
            codec
                .queue_input_buffer(ib, 0, au.len(), pts_us, 0)
                .map_err(|e| format!("queueInputBuffer: {e:?}"))
        }
        Ok(DequeuedInputBufferResult::TryAgainLater) => Ok(()),
        Err(e) => Err(format!("dequeueInputBuffer: {e:?}")),
    }
}

/// Render every decoded frame that is ready (immediately at depth 0, scheduled on the
/// compositor clock otherwise — see [`Pacing`]) and track output-format changes.
fn drain(codec: &MediaCodec, shared: &Shared, pacing: &mut Pacing) -> Result<(), String> {
    loop {
        match codec.dequeue_output_buffer(Duration::ZERO) {
            Ok(DequeuedOutputBufferInfoResult::Buffer(ob)) => {
                let depth = shared.buffer_frames.load(Ordering::Relaxed);
                if depth == 0 {
                    codec
                        .release_output_buffer(ob, true)
                        .map_err(|e| format!("releaseOutputBuffer: {e:?}"))?;
                } else {
                    let pts_us = ob.info().presentation_time_us();
                    // Track the media frame interval (EMA, clamped to sane frame times).
                    if pacing.interval_us == 0 {
                        pacing.interval_us = 16_667; // seed: 60 fps
                    }
                    if let Some(last) = pacing.last_pts_us {
                        let delta = pts_us - last;
                        if (5_000..=100_000).contains(&delta) {
                            pacing.interval_us += (delta - pacing.interval_us) / 8;
                        }
                    }
                    pacing.last_pts_us = Some(pts_us);

                    let now = monotonic_ns();
                    let lead_ns = depth as i64 * pacing.interval_us * 1_000;
                    let target = match pacing.anchor {
                        Some((a_ns, a_us, a_depth)) if a_depth == depth => {
                            a_ns + (pts_us - a_us) * 1_000
                        }
                        _ => {
                            pacing.anchor = Some((now + lead_ns, pts_us, depth));
                            now + lead_ns
                        }
                    };
                    // A timeline jump (loss gap) or drift beyond the cushion: re-anchor so
                    // the schedule never runs away — same 100 ms-style cap as the Windows
                    // sink's pacing after gaps.
                    let target = if target < now || target > now + lead_ns + 100_000_000 {
                        pacing.anchor = Some((now + lead_ns, pts_us, depth));
                        now + lead_ns
                    } else {
                        target
                    };
                    codec
                        .release_output_buffer_at_time(ob, target)
                        .map_err(|e| format!("releaseOutputBufferAtTime: {e:?}"))?;
                }
                shared.presented.fetch_add(1, Ordering::Relaxed);
            }
            Ok(DequeuedOutputBufferInfoResult::OutputFormatChanged) => {
                let format = codec.output_format();
                let mut w = format.i32("width").unwrap_or(0);
                let mut h = format.i32("height").unwrap_or(0);
                // Display area over coded size (CTU/macroblock padding — the 1280×736 trap
                // the Windows sink hit): the crop rect is inclusive on both ends.
                if let (Some(l), Some(r), Some(t), Some(b)) = (
                    format.i32("crop-left"),
                    format.i32("crop-right"),
                    format.i32("crop-top"),
                    format.i32("crop-bottom"),
                ) {
                    w = r - l + 1;
                    h = b - t + 1;
                }
                if w > 0 && h > 0 {
                    shared.width.store(w as u32, Ordering::Relaxed);
                    shared.height.store(h as u32, Ordering::Relaxed);
                    if let Err(e) = view_host::set_video_size(w, h) {
                        log::debug!("[video] android sink: set_video_size: {e}");
                    }
                    log::info!("[video] android sink: output {w}x{h}");
                }
            }
            Ok(DequeuedOutputBufferInfoResult::OutputBuffersChanged) => {}
            Ok(DequeuedOutputBufferInfoResult::TryAgainLater) => return Ok(()),
            Err(e) => return Err(format!("dequeueOutputBuffer: {e:?}")),
        }
    }
}

/// Does this Annex-B AU contain an intra frame (H.264 IDR / HEVC IRAP)? Used to start a
/// rebuilt decoder clean instead of mid-GOP.
fn has_intra_frame(mime: &str, au: &[u8]) -> bool {
    let hevc = mime == "video/hevc";
    let mut i = 0usize;
    while i + 3 < au.len() {
        // 00 00 01 start code (with or without a leading zero byte).
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            let b = match au.get(i + 3) {
                Some(b) => *b,
                None => return false,
            };
            if hevc {
                let t = (b >> 1) & 0x3F;
                if (16..=21).contains(&t) {
                    return true; // BLA/IDR/CRA
                }
            } else if b & 0x1F == 5 {
                return true; // IDR slice
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}
