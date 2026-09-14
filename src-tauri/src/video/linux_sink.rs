// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Linux GStreamer decode sink (MOBILE_RTSP.md P2.3) — the third `PlatformSink` next to
//! `win_sink` (Media Foundation) and `android_sink` (MediaCodec). H.264/HEVC access units
//! from the RTSP client go into
//!
//!   appsrc → h264parse/h265parse → decodebin3 → glupload → glcolorconvert → tee
//!                            ├→ queue → valve → glvideoflip → gtkglsink                  (slot 0)
//!                            └→ queue → valve → glvideoflip → gldownload → videoconvert → gtksink
//!
//! The tee sits BEHIND the GL upload on purpose. In front of it, one GL branch and one CPU branch
//! have to agree on a buffer type, and the only thing both accept is system memory — the VA decoder
//! then stops handing out DMABuf and every frame is copied through RAM and uploaded again. Measured
//! on the Debian 13 laptop: the decoder's caps were plain `video/x-raw`/NV12, the video engine idled
//! at 16 % while Render/3D sat at 80 % and the memory bus moved 8.8 GB/s (2026-09-08). Behind the
//! upload the decoder keeps its zero-copy path, the tee shares GL buffers, and only the SECOND
//! branch pays a readback — and only while it actually has a surface, because its valve drops
//! everything otherwise.
//!
//! and each gtkglsink's GtkGLArea widget is what `linux_host` places below the WebView, one per
//! slot (VIDEO_MULTISINK_WINDOW.md §4.2 — the video widget and the floating window at once).
//!
//! One such pipeline PER WINDOW. The main window's has both branches; the detached video window
//! gets a pipeline of its own, fed the same access units from the same RTSP connection — never a
//! second network stream, which over an LTE or Wi-Fi link to the aircraft would be the opposite of
//! an improvement. It is a second DECODE (the macOS sink's D9 trade, for the same reason: a
//! `gtkglsink` widget cannot move between windows, and two of them inside ONE pipeline cannot share
//! the single GL context a pipeline distributes — across two pipelines they can, so the detached
//! window keeps hardware-accelerated rendering instead of falling back to the CPU leg).
//! Docked surfaces stay on ONE decode with a tee, deliberately: the Pi 5 decodes H.264 in software,
//! where a second decode would cost far more than the CPU convert of the small second surface.
//!
//! EXCEPT on the V4L2 stateless decoder (Pi 5 HEVC), which does not survive a second consumer of
//! its frames at all: with the readback branch running next to the GL sink, its capture buffers go
//! back from a second thread and are re-armed by the decode thread microseconds later, the driver
//! refuses the next `VIDIOC_S_EXT_CTRLS`, and the plugin walks on into a SIGSEGV instead of ending
//! the stream (Marc, 2026-09-08 — gdb inside `libgstv4l2codecs`, confirmed frame by frame with
//! `GST_DEBUG`; VA-API on the laptop never showed it). There the main window is served by ONE
//! PIPELINE PER SEAT, the shape the detached window has been using all along — see
//! `LinuxVideoSink::per_slot`. Two pipelines never share a buffer.
//!
//! BOTH branches are built up front and stay for the sink's life: the maximum is known (two), and
//! adding a branch to a running pipeline means pad blocking and re-negotiation, which on the Pi's
//! stateless decoder is exactly the kind of interruption it cannot conceal. An unused branch is
//! idled by its `valve`, which drops buffers before the colour convert and the render — the decode
//! is upstream of the tee, so a second surface costs a convert and a render, never a second decode.
//! decodebin3 picks the decoder the machine has — VA-API (`vah264dec`/`vah265dec`, Intel/
//! AMD), V4L2 (Pi 4 H264 stateful, Pi 5 HEVC stateless `v4l2slh265dec`) or software
//! (`avdec_*`) — and the GL leg imports DMABuf output without a copy where the decoder
//! offers it. That import needs a GLES context (`lib.rs` asks GDK for one: a desktop-GL
//! context cannot take the Pi 5's tiled `NV12_128C8` and gst-gl aborts on it). When the GL
//! sink can't come up (no usable GL context) the cairo path `videoconvert → videoflip →
//! gtksink` takes over for the rest of the process.
//!
//! Conformance windows (`rtsp::h265_sps`): the V4L2 stateless HEVC decoder (Pi 5) COPIES
//! every frame when the SPS crops the coded picture (NVENC 720p = coded 736, every 1080p),
//! into a system-memory buffer that still carries DMABuf caps — gst-gl aborts on it. So for
//! that decoder the sink strips the window from every SPS it pushes (the decoder then hands
//! the full coded frame out zero-copy) and lays the widget out for the CODED picture such
//! that the display part fills the aspect-fitted rect and the padding rows lie outside the
//! visible box, hidden under the WebView. Measured on the Pi 5: 60 fps at ~23 % of one core
//! either way; the copy path was 45–57 fps at a full core. Only an SPS that does not parse
//! still goes cairo (the one route that survives a copied frame).
//!
//! Presentation: smoothing buffer 0 (the latency-first default) = `sync=false`, every
//! decoded frame renders as soon as it exists. Depths 1–3 = `sync=true` with the frames'
//! PTS scheduled `depth × frame-interval` behind their arrival timeline (same pacing model
//! as the Android sink: one anchor maps media time onto the pipeline clock, a timeline jump
//! or drift beyond the cushion re-anchors), so each step adds ONE frame time of cushion.
//! The cushion is measured at the SINK — after decode and GL upload — so those get a fixed
//! allowance on top (`PROCESSING_ALLOWANCE_NS`), a frame may reach the sink a little late
//! and still show (`LATE_TOLERANCE_NS`), and the sink never sends QoS upstream: a decoder
//! that skips frames on QoS breaks the reference chain, and artefacts are worse than a
//! repeated picture.
//!
//! Threads: `push` runs on the RTSP thread (appsrc is thread-safe), decode and render on
//! GStreamer's own streaming threads, GL drawing on the GTK main thread through the
//! widget; a small bus thread turns pipeline errors into the sink's error slot.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gst::prelude::*;

use super::linux_host;
use super::rtsp::{strip_window, SpsProbe, VideoCodec, Window};
use super::surface::SinkSurface;

/// The host places the widget one main-thread hop away; the GL sink needs it there before
/// it starts (its GL context comes from the realized GtkGLArea).
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);

/// Sticky: once the GL sink failed to start, every later sink in this process goes cairo.
static GL_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// Paced mode: what decode + GL upload may take on top of the user's `depth × frame-interval`
/// cushion, since the cushion is judged at the sink. Measured on a Pi 5: software H.264 720p
/// decodes in ~10 ms plus the upload. Without it a one-frame cushion at 60 fps (16 ms) was
/// spent before the frame reached the sink — every frame late and dropped, and the sink's QoS
/// events made the decoder skip on top: a 1 fps slideshow at 30–40 % CPU in the field.
const PROCESSING_ALLOWANCE_NS: u64 = 20_000_000;

/// Paced mode: how late a frame may reach the sink and still be shown. gtkglsink's default is
/// 5 ms — tighter than one display refresh. A frame 15 ms late is still the newest picture
/// there is; dropping it only lengthens the previous one.
const LATE_TOLERANCE_NS: i64 = 20_000_000;

/// The GL displays of every sink this process ran, kept alive on purpose. gtkglsink's
/// widget wraps GDK's EGLDisplay in a GstGLDisplayEGL it considers its OWN
/// (`gst_gl_display_egl_from_gl_display` never marks it foreign), so finalizing it calls
/// `eglTerminate` on the display WebKitGTK and GDK render with — measured: every later
/// `eglMakeCurrent` of the WebView fails (EGL_NOT_INITIALIZED, 60 warnings/s) and the UI
/// freezes — and every later GL sink fails to start, silently demoting the app to the
/// cairo path. Holding the `gst.gl.GLDisplay` context the sink posts at start (it
/// references the display) keeps the finalizer from ever running. Cost: a few small
/// wrapper objects per stream start. Verified by `stop_start_cycles_under_a_webview`.
static GL_DISPLAYS: Mutex<Vec<gst::Context>> = Mutex::new(Vec::new());

#[derive(Default)]
struct Shared {
    error: Mutex<Option<String>>,
    presented: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    /// Smoothing-buffer depth in frames (0 = render on decode).
    buffer_frames: AtomicU32,
    stopping: AtomicBool,
}

impl Shared {
    fn fail(&self, msg: String) {
        log::warn!("[video] linux sink: {msg}");
        self.error.lock().unwrap().get_or_insert(msg);
    }
}

/// Media-time → pipeline-running-time mapping for the paced (depth > 0) mode; see the
/// module docs. All times in ns.
#[derive(Default)]
struct Pacing {
    /// (running time, media time, depth) of the anchor frame.
    anchor: Option<(u64, u64, u32)>,
    /// EMA of the media frame interval; seeds at 60 fps until measured.
    interval: u64,
    last_media: Option<u64>,
}

pub struct LinuxVideoSink {
    /// One per window; `main` is built at start and lives as long as the sink, the detached video
    /// window's comes and goes with its surfaces.
    pipes: Mutex<Vec<Pipe>>,
    /// What every new pipeline is built from.
    codec: VideoCodec,
    gl: bool,
    shared: Arc<Shared>,
    pacing: Mutex<Pacing>,
    /// Conformance window stripped from this stream's SPS (module docs), if any.
    window: Option<Window>,
    geom: Mutex<Geom>,
}

/// The machine decodes HEVC on a stateless V4L2 decoder (Pi 5): it cannot conceal a
/// missing reference — a picture after lost data stalls it, and the stop then hangs the
/// kernel driver. The RTSP client holds pictures until the next IRAP for such a decoder.
pub fn hevc_decoder_is_stateless() -> bool {
    gst::init().is_ok() && gst::ElementFactory::find("v4l2slh265dec").is_some()
}

/// Last rect and orientation from the frontend — re-laid out when either changes.
#[derive(Default)]
struct Geom {
    mirror: bool,
    rotate180: bool,
}

/// What a slot is showing right now.
#[derive(Clone)]
struct Slot {
    /// The surface's `window/id` — kept so a surface keeps its slot across pushes and its
    /// widget never has to move.
    key: String,
    /// The surface's own rect (physical px), before the conformance-window correction.
    rect: [i32; 8],
}

/// One window's pipeline: its own decode, its own branches, its own bus thread.
struct Pipe {
    /// Tauri window label whose host tree this pipeline renders into.
    label: String,
    /// First host seat this pipeline renders into; it owns `slot_base .. slot_base + branches`.
    /// Several pipelines of one window sit side by side in per-seat mode (see `per_slot`).
    slot_base: usize,
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    branches: Vec<Branch>,
    bus_thread: Option<JoinHandle<()>>,
    /// Ends THIS pipeline's bus loop. The sink-wide `Shared::stopping` only ever ends all of them
    /// at once, so a single window's teardown had nothing to stop its bus thread with and hung in
    /// `join()` forever — with the sink mutex held, which then swallowed the next stop, the next
    /// start ("Starting" for good) and the dock-back (Marc, 2026-09-08).
    stopping: Arc<AtomicBool>,
    /// Per slot: the surface it currently serves. `None` = idle (valve dropping, clip parked).
    slots: Vec<Option<Slot>>,
}

/// One pipeline branch and the elements that belong only to it.
struct Branch {
    /// Drops everything while this slot is idle — cheaper than blocking, and it cannot stall
    /// the tee (which would stall the OTHER branch with it).
    valve: gst::Element,
    flip: gst::Element,
    sink: gst::Element,
}

impl LinuxVideoSink {
    /// Bring the sink up for `codec`: build the pipeline, hand its widget to the host and
    /// start decoding. GL first, cairo fallback. Decode problems after this surface through
    /// [`Self::error`] — the same contract as the other sinks. `sps` is the stream's SPS
    /// verdict (`rtsp::probe_au`); see the module docs for what a window means here.
    pub fn start(codec: VideoCodec, sps: SpsProbe) -> Result<Self, String> {
        gst::init().map_err(|e| format!("gstreamer init: {e}"))?;
        let v4l2_hevc = matches!(codec, VideoCodec::H265)
            && gst::ElementFactory::find("v4l2slh265dec").is_some();
        let mut gl = !GL_UNAVAILABLE.load(Ordering::Relaxed);
        let mut window = None;
        match sps {
            SpsProbe::Window(w) if v4l2_hevc => {
                log::info!(
                    "[video] linux sink: conformance window {}x{} -> {}x{} stripped for the V4L2 decoder; padding hidden under the WebView",
                    w.coded_w,
                    w.coded_h,
                    w.display_w(),
                    w.display_h()
                );
                window = Some(w);
            }
            SpsProbe::Unparsed if v4l2_hevc && gl => {
                log::warn!("[video] linux sink: SPS did not parse — cairo sink for this stream (the V4L2 decoder may copy)");
                gl = false;
            }
            _ => {}
        }
        let shared = Arc::new(Shared::default());
        // What the app is to treat as the picture's size — see the probe below.
        let display = window.map(|w| (w.display_w(), w.display_h()));
        // The decoder decides the shape of the main window's pipeline(s) — see [`build_main`].
        let per_slot = v4l2_hevc;
        if per_slot {
            log::info!(
                "[video] linux sink: one pipeline per seat — the stateless V4L2 decoder does not share its frames"
            );
        }
        let (main, gl) = match build_main(per_slot, codec, gl, display, &shared) {
            Ok(p) => (p, gl),
            Err(e) if gl => {
                log::warn!("[video] linux sink: GL sink unavailable ({e}) — using the cairo sink");
                GL_UNAVAILABLE.store(true, Ordering::Relaxed);
                (build_main(per_slot, codec, false, display, &shared)?, false)
            }
            Err(e) => return Err(e),
        };
        Ok(Self {
            pipes: Mutex::new(main),
            codec,
            gl,
            shared,
            pacing: Mutex::new(Pacing::default()),
            window,
            geom: Mutex::new(Geom { mirror: false, rotate180: false }),
        })
    }
}

/// The main window's pipeline(s): one per seat where the decoder demands it, otherwise a single one
/// with a branch per seat. A half-built set is taken down rather than handed back — the caller
/// retries the whole thing without GL.
///
/// `per_slot` is for the V4L2 stateless decoder (Pi 5 HEVC), which does not survive a second, slower
/// consumer of its frames. With the readback branch running alongside the GL sink its capture
/// buffers go back from a second thread and are re-armed by the decode thread microseconds later;
/// the driver then refuses the next `VIDIOC_S_EXT_CTRLS` ("Driver did not accept the bitstream
/// parameters") and the plugin walks on into a SIGSEGV instead of ending the stream (Marc, Pi 5,
/// 2026-09-08 — gdb backtrace inside `libgstv4l2codecs`, confirmed frame by frame with `GST_DEBUG`).
/// Two pipelines never share a buffer, and two `gtkglsink`s CAN live side by side across pipelines —
/// that is what the detached window has been doing all along.
///
/// The price is a second decode; on that hardware it is a hardware decode, and the machine already
/// runs two whenever the picture is detached. Everywhere else the tee stays: with one decode
/// upstream of it a second surface costs 5-6 % of a core instead of a whole decoder, and VA-API (the
/// laptop) never showed the defect.
fn build_main(
    per_slot: bool,
    codec: VideoCodec,
    gl: bool,
    display: Option<(u32, u32)>,
    shared: &Arc<Shared>,
) -> Result<Vec<Pipe>, String> {
    if !per_slot {
        return Ok(vec![build_pipe("main", 0, linux_host::SLOTS, codec, gl, display, shared)?]);
    }
    let mut pipes = Vec::with_capacity(linux_host::SLOTS);
    for slot in 0..linux_host::SLOTS {
        match build_pipe("main", slot, 1, codec, gl, display, shared) {
            Ok(p) => pipes.push(p),
            Err(e) => {
                for p in pipes {
                    teardown_pipe(p, shared);
                }
                return Err(e);
            }
        }
    }
    Ok(pipes)
}

/// Build one pipeline with `slots` branches and start it. `label` names the host tree its widgets
/// go into — the main window's, or the detached video window's — and `slot_base` the first seat in
/// it that this pipeline serves.
fn build_pipe(
    label: &str,
    slot_base: usize,
    slots: usize,
    codec: VideoCodec,
    gl: bool,
    display: Option<(u32, u32)>,
    shared: &Arc<Shared>,
) -> Result<Pipe, String> {
        let (parser, caps) = match codec {
            VideoCodec::H265 => ("h265parse", "video/x-h265,stream-format=byte-stream,alignment=au"),
            _ => ("h264parse", "video/x-h264,stream-format=byte-stream,alignment=au"),
        };
        let make = |name: &str| {
            gst::ElementFactory::make(name)
                .build()
                .map_err(|_| format!("GStreamer element '{name}' is not installed"))
        };

        let pipeline = gst::Pipeline::new();
        let appsrc = make("appsrc")?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| "appsrc is not an AppSrc".to_string())?;
        appsrc.set_caps(Some(&caps.parse::<gst::Caps>().map_err(|e| format!("caps: {e}"))?));
        appsrc.set_format(gst::Format::Time);
        appsrc.set_is_live(true);
        appsrc.set_do_timestamp(false);
        // Bounded: a decoder that can't keep up drops the OLDEST queued AUs instead of
        // building a latency backlog. The bound must cover the START-UP burst — device open
        // and negotiation take 100–150 ms, at 60 fps that is 6–9 AUs queued before the first
        // decode; with 8 the second frame was dropped and the stateless V4L2 decoder hung on
        // its missing reference (Pi 5, 2026-09-05). A stateless decoder never conceals a
        // dropped AU; 120 AUs (2 s at 60 fps) only trips on a sustained overload.
        appsrc.set_property("max-buffers", 120u64);
        appsrc.set_property_from_str("leaky-type", "downstream");
        let parse = make(parser)?;
        let decode = make("decodebin3")?;
        // Name the decoder decodebin3 settles on — the one line that tells a Pi log whether the
        // hardware block (`v4l2slh265dec`) or software (`avdec_*`) does the work.
        if let Some(bin) = decode.downcast_ref::<gst::Bin>() {
            bin.connect_deep_element_added(|_, _, el| {
                if let Some(f) = el.factory() {
                    let klass = f.metadata(gst::ELEMENT_METADATA_KLASS).unwrap_or("");
                    if klass.contains("Decoder") && klass.contains("Video") {
                        log::info!("[video] linux sink: decoder {}", f.name());
                    }
                }
            });
        }

        // One decode feeds every branch, and the split happens AFTER the upload (module docs): the
        // decoder keeps whatever zero-copy memory it prefers, and the tee shares the uploaded
        // frames.
        let tee = make("tee")?;
        // ONLY the upload goes in front of the tee. The colour convert stays in the branches: in
        // front, it would have to produce a format BOTH branches accept, which lands on RGBA — 3.7 MB
        // per 720p frame against 1.4 MB as NV12, 221 MB/s of graphics memory instead of 83 at 60 fps.
        // That surcharge eats exactly the headroom the window compositing needs, and the picture
        // loses frames as the window grows (Marc, Debian 13, 2026-09-08). Behind the tee, branch 0
        // is byte for byte the chain that shipped.
        let head: Vec<gst::Element> = if gl { vec![make("glupload")?] } else { Vec::new() };
        pipeline
            .add_many([appsrc.upcast_ref::<gst::Element>(), &parse, &decode])
            .map_err(|e| format!("add: {e}"))?;
        pipeline.add_many(head.iter()).map_err(|e| format!("add: {e}"))?;
        pipeline.add(&tee).map_err(|e| format!("add: {e}"))?;
        gst::Element::link_many([appsrc.upcast_ref::<gst::Element>(), &parse, &decode])
            .map_err(|e| format!("link: {e}"))?;
        gst::Element::link_many(head.iter()).map_err(|e| format!("link: {e}"))?;
        // Cairo has no head at all — there the decoder's system memory IS the common denominator,
        // so the tee sits right behind it.
        match head.last() {
            Some(last) => last.link(&tee).map_err(|e| format!("link head->tee: {e}"))?,
            None => {}
        }
        // decodebin3's source pad appears once the decoder negotiated: link it then.
        let head_in = head.first().cloned().unwrap_or_else(|| tee.clone());
        decode.connect_pad_added(move |_, pad| {
            let Some(sinkpad) = head_in.static_pad("sink") else { return };
            if sinkpad.is_linked() {
                return;
            }
            if let Err(e) = pad.link(&sinkpad) {
                log::warn!("[video] linux sink: linking the decoder output failed: {e:?}");
            }
        });

        let mut branches = Vec::with_capacity(slots);
        for slot in 0..slots {
            // A queue per branch so a slow one cannot stall the tee — and with it the other
            // branch. Leaky downstream: a branch that falls behind drops its own OLD frames
            // instead of building latency for everyone.
            let queue = make("queue")?;
            // Measured in TIME, not in buffers, and generously: in paced mode (`set_buffer` > 0) the
            // sink holds every frame until its timestamp, so the cushion's worth of frames waits
            // here. A three-buffer bucket threw the rest away — 60 fps in, ~10 fps on screen, while
            // the counter in front of it still read 60 (Marc, 2026-09-08). One second covers every
            // depth the stepper offers with room to spare; leaky only ever trips on a real overrun,
            // and then it drops this branch's OLDEST frames rather than stalling the tee and with it
            // the other branch.
            queue.set_property("max-size-buffers", 0u32);
            queue.set_property("max-size-bytes", 0u32);
            queue.set_property("max-size-time", 1_000_000_000u64);
            queue.set_property_from_str("leaky", "downstream");
            // Idle slots drop here, before the colour convert and the render. `forward-sticky-events`
            // is not optional: the default `drop-all` swallows the CAPS event too, downstream never
            // negotiates, and the failed caps event travels back up as `not-negotiated (-4)` at the
            // appsrc — the whole stream dies before the first frame (Marc, Debian 13, 2026-09-08).
            // The property is only settable in NULL/READY, hence here and not at first use.
            let valve = make("valve")?;
            valve.set_property_from_str("drop-mode", "forward-sticky-events");
            valve.set_property("drop", true);

            // The branch itself. Only the FIRST one renders through GL: two `gtkglsink`s in one
            // pipeline each bring their own GtkGLArea and therefore their own GL context, while a
            // pipeline distributes a single one — the second then has nothing to negotiate against
            // and the stream dies with `not-negotiated (-4)` at the appsrc, before a frame is shown
            // (proven by elimination on Debian 13, 2026-09-08). The second branch therefore reads
            // the frame back and renders on the CPU. It pays that only while it HAS a surface: its
            // valve drops everything otherwise. Slot 0 goes to the highest-priority — largest —
            // surface, so the readback serves the small widget tile.
            let (leg, flip, sink) = if gl && slot == 0 {
                let convert = make("glcolorconvert")?;
                let flip = make("glvideoflip")?;
                let sink = make("gtkglsink")?;
                (vec![convert, flip.clone(), sink.clone()], flip, sink)
            } else if gl {
                let convert = make("glcolorconvert")?;
                let flip = make("glvideoflip")?;
                let download = make("gldownload")?;
                let to_cpu = make("videoconvert")?;
                to_cpu.set_property("n-threads", 0u32);
                let sink = make("gtksink")?;
                (vec![convert, flip.clone(), download, to_cpu, sink.clone()], flip, sink)
            } else {
                let convert = make("videoconvert")?;
                // All cores: single-threaded the Pi 5 converts a cropped 720p60 HEVC at 45 fps,
                // with threads 57 (then the decoder's own copy is the limit).
                convert.set_property("n-threads", 0u32);
                let flip = make("videoflip")?;
                let sink = make("gtksink")?;
                (vec![convert, flip.clone(), sink.clone()], flip, sink)
            };
            // Latency first: render on decode. `set_buffer` flips this for the paced depths.
            sink.set_property("sync", false);
            // A sink that receives nothing must not hold the pipeline in preroll: an idle branch's
            // valve drops every buffer, and with the default async behaviour the PLAYING transition
            // then never completes — the LIVE branch shows its one preroll frame and freezes
            // (Marc, 2026-09-08).
            sink.set_property("async", false);
            // Paced mode only (no effect while unsynced): see the two constants above.
            sink.set_property("max-lateness", LATE_TOLERANCE_NS);
            sink.set_property("qos", false);

            let mut chain = vec![queue.clone(), valve.clone()];
            chain.extend(leg);
            pipeline.add_many(chain.iter()).map_err(|e| format!("add: {e}"))?;
            gst::Element::link_many(chain.iter()).map_err(|e| format!("link: {e}"))?;
            tee.link(&queue).map_err(|e| format!("link tee->queue: {e}"))?;

            // The widget must exist (and be realized, for GL) before the sink starts.
            let widget_sink = sink.clone();
            let placed = linux_host::attach(label, slot_base + slot, move || {
                Some(widget_sink.property::<gtk::Widget>("widget"))
            });
            match placed.recv_timeout(ATTACH_TIMEOUT) {
                Ok(true) => {}
                Ok(false) => return Err("the host has no place for the video widget".to_string()),
                Err(_) => return Err("the video host did not answer (not installed?)".to_string()),
            }
            branches.push(Branch { valve, flip, sink });
        }

        // Counted at the TEE's input, not at a sink: it is one number for the stream (as on the
        // other platforms), and a probe on one branch would read zero whenever that slot is the
        // idle one. The caps event carries the DISPLAY size — decoders report the cropped picture
        // there, the coded size stays in the meta.
        //
        // Only the MAIN pipeline counts. Every window decodes the same stream, so letting the
        // detached window's pipeline add to the same counter reported 120 fps for a 60 fps source
        // (Marc, 2026-09-08) — the panel shows the stream's rate, not the sum of the renders.
        if let (Some(pad), true) = (tee.static_pad("sink"), label == "main" && slot_base == 0) {
            let s = shared.clone();
            pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                s.presented.fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });
            let s = shared.clone();
            pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                if let Some(gst::PadProbeData::Event(ev)) = &info.data {
                    if let gst::EventView::Caps(c) = ev.view() {
                        if let Some(st) = c.caps().structure(0) {
                            let w = st.get::<i32>("width").unwrap_or(0);
                            let h = st.get::<i32>("height").unwrap_or(0);
                            if w > 0 && h > 0 {
                                // These caps carry what the DECODER hands out, and with a stripped
                                // conformance window that is the CODED picture — padding included.
                                // The app sizes every surface from this number, so a 720p stream
                                // reported as 1280x736 tilts every picture box by 2 %: the frame
                                // then holds a shape the picture cannot fill, and what the picture
                                // does not cover is see-through (Marc, Pi 5, 2026-09-08). Report
                                // what is meant to be SEEN; the padding stays the sink's business.
                                let (rw, rh) = display.unwrap_or((w as u32, h as u32));
                                s.width.store(rw, Ordering::Relaxed);
                                s.height.store(rh, Ordering::Relaxed);
                                // The FULL caps, not just the structure name: the memory feature is
                                // the whole story of whether the decoder hands its frames over
                                // without a copy (`memory:DMABuf` / `memory:VAMemory`) or through
                                // system RAM.
                                log::info!(
                                    "[video] linux sink: output {w}x{h} {}",
                                    c.caps().to_string()
                                );
                            }
                        }
                    }
                }
                gst::PadProbeReturn::Ok
            });
        }

        let bus = pipeline.bus().ok_or("no pipeline bus")?;
        let stopping = Arc::new(AtomicBool::new(false));
        let bus_thread = {
            let shared = Arc::clone(shared);
            let stopping = Arc::clone(&stopping);
            Some(std::thread::spawn(move || bus_loop(&bus, &shared, &stopping)))
        };

        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let _ = pipeline.set_state(gst::State::Null);
            stopping.store(true, Ordering::SeqCst);
            if let Some(t) = bus_thread {
                let _ = t.join();
            }
            let _ =
                linux_host::detach(label, slot_base..slot_base + slots).recv_timeout(ATTACH_TIMEOUT);
            let detail = shared.error.lock().unwrap().clone().unwrap_or_default();
            return Err(format!("pipeline start failed ({e}) {detail}"));
        }
        log::info!(
            "[video] linux sink: {} pipeline up in window {label} ({}, {slots} slot(s) from seat {slot_base})",
            if matches!(codec, VideoCodec::H265) { "HEVC" } else { "H.264" },
            if gl { "GL" } else { "cairo" }
        );
        Ok(Pipe {
            label: label.to_string(),
            slot_base,
            pipeline,
            appsrc,
            branches,
            bus_thread,
            stopping,
            slots: vec![None; slots],
        })
}

/// Stop one window's pipeline and give its widgets back, in the order the GL widget needs (see the
/// sink's `Drop`).
fn teardown_pipe(mut pipe: Pipe, shared: &Shared) {
    // Before anything else: the bus loop is the one thread that must be told, and `join()` below
    // waits for it.
    pipe.stopping.store(true, Ordering::SeqCst);
    let _ = pipe.appsrc.end_of_stream();
    let _ = pipe.pipeline.set_state(gst::State::Null);
    if let Some(t) = pipe.bus_thread.take() {
        let _ = t.join();
    }
    let seats = pipe.slot_base..pipe.slot_base + pipe.branches.len();
    let _ = linux_host::detach(&pipe.label, seats).recv_timeout(ATTACH_TIMEOUT);
    let _ = shared;
    log::info!("[video] linux sink: pipeline for window {} stopped", pipe.label);
}

impl LinuxVideoSink {

    /// Queue one access unit (Annex-B) with its unwrapped 90 kHz timestamp.
    pub fn push(&self, au: Vec<u8>, ts90k: u64) {
        let au = match self.window {
            Some(_) => strip_window(&au).unwrap_or(au),
            None => au,
        };
        let media_ns = ts90k * 100_000 / 9; // 90 kHz → ns
        let pts = self.schedule(media_ns);
        let mut buffer = gst::Buffer::from_mut_slice(au);
        if let Some(b) = buffer.get_mut() {
            b.set_pts(gst::ClockTime::from_nseconds(pts));
        }
        // Every window's pipeline gets the SAME access unit from the one RTSP connection — the
        // split is at the decoder, never at the network. Err = flushing/stopped: that pipeline is
        // ending anyway.
        for pipe in self.pipes.lock().unwrap().iter() {
            let _ = pipe.appsrc.push_buffer(buffer.clone());
        }
    }

    /// Media time → running-time PTS. Anchored at the first frame so the picture starts
    /// immediately; at depth > 0 shifted by the cushion and re-anchored on timeline jumps or
    /// drift (the RTP clock and this machine's clock are not the same clock).
    fn schedule(&self, media_ns: u64) -> u64 {
        let mut p = self.pacing.lock().unwrap();
        let depth = self.shared.buffer_frames.load(Ordering::Relaxed);
        if p.interval == 0 {
            p.interval = 16_666_667; // seed: 60 fps
        }
        if let Some(last) = p.last_media {
            let delta = media_ns.saturating_sub(last);
            if (5_000_000..=100_000_000).contains(&delta) {
                p.interval = (p.interval * 7 + delta) / 8;
            }
        }
        p.last_media = Some(media_ns);

        let now = self.running_time();
        let lead = if depth > 0 { depth as u64 * p.interval + PROCESSING_ALLOWANCE_NS } else { 0 };
        let target = match p.anchor {
            Some((a_run, a_media, a_depth)) if a_depth == depth => {
                (a_run as i64 + (media_ns as i64 - a_media as i64)).max(0) as u64
            }
            _ => {
                p.anchor = Some((now + lead, media_ns, depth));
                now + lead
            }
        };
        // A loss gap (target in the past) or drift past the cushion + 100 ms: re-anchor so
        // the schedule never runs away in either direction.
        if target < now || target > now + lead + 100_000_000 {
            p.anchor = Some((now + lead, media_ns, depth));
            now + lead
        } else {
            target
        }
    }

    /// The MAIN pipeline's running time. With a second window open there are two clocks, but the
    /// pacing cushion is one number for the stream and the difference between two pipelines started
    /// seconds apart is far below the cushion itself.
    fn running_time(&self) -> u64 {
        let pipes = self.pipes.lock().unwrap();
        let Some(pipe) = pipes.first() else { return 0 };
        match (pipe.pipeline.clock(), pipe.pipeline.base_time()) {
            (Some(clock), Some(base)) => clock.time().map(|t| t.saturating_sub(base).nseconds()).unwrap_or(0),
            _ => 0,
        }
    }

    /// First fatal sink error, if any — the stream ends on it (see rtsp_native).
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// The surfaces to present into (VIDEO_MULTISINK_WINDOW.md §4.1), highest priority first.
    ///
    /// Grouped BY WINDOW: each window has its own pipeline (the main one, and the detached video
    /// window's while it is open), and within a window the surfaces are capped at
    /// [`linux_host::SLOTS`] and assigned to seats. A surface keeps the seat it already had — its
    /// branch owns a realized GL widget, and moving a picture between seats would mean moving that
    /// widget, which is precisely what must not happen (see `linux_host`). Seats nobody claims are
    /// idled: valve dropping, clip parked.
    ///
    /// A window that appears gets a pipeline built for it; one whose surfaces are all gone has its
    /// pipeline torn down, so a closed detached window costs nothing.
    pub fn set_surfaces(&self, surfaces: &[SinkSurface]) {
        // Which windows want something, in first-seen (priority) order.
        let mut windows: Vec<&str> = Vec::new();
        for s in surfaces {
            if !windows.iter().any(|w| *w == s.window) {
                windows.push(&s.window);
            }
        }

        let mut pipes = self.pipes.lock().unwrap();
        // Gone: every pipeline except the main one whose window asks for nothing any more.
        let mut i = 0;
        while i < pipes.len() {
            if pipes[i].label != "main" && !windows.iter().any(|w| *w == pipes[i].label) {
                let pipe = pipes.remove(i);
                let shared = Arc::clone(&self.shared);
                // The window's video layer goes back NOW, from this thread: `uninstall` only posts
                // to the GTK loop, so it lands there before anything a pipeline built later can
                // queue — the layer can never be pulled out from under a NEW window of the same
                // name. What is left is GStreamer work that waits for that same loop, so it must
                // not run here: this is called with the sink's lock held, and on the GTK thread
                // itself whenever a Tauri command drives it. Off it goes.
                linux_host::uninstall(&pipe.label);
                std::thread::spawn(move || teardown_pipe(pipe, &shared));
            } else {
                i += 1;
            }
        }
        // New: a window with surfaces and no pipeline yet. ONE slot — a second window shows one
        // picture, and two `gtkglsink`s in one pipeline cannot share its single GL context.
        for w in &windows {
            if pipes.iter().any(|p| p.label == *w) {
                continue;
            }
            let display = self.window.map(|win| (win.display_w(), win.display_h()));
            match build_pipe(w, 0, 1, self.codec, self.gl, display, &self.shared) {
                Ok(p) => pipes.push(p),
                Err(e) => log::warn!("[video] linux sink: no pipeline for window {w}: {e}"),
            }
        }

        // Seats belong to the WINDOW, not to one pipeline: in per-seat mode the main window's are
        // spread over several of them, and a surface has to keep the seat it holds no matter which
        // pipeline owns it.
        let labels: Vec<String> = pipes.iter().fold(Vec::new(), |mut v, p| {
            if !v.iter().any(|l| l == &p.label) {
                v.push(p.label.clone());
            }
            v
        });
        for label in labels {
            // This window's seats as (pipeline, branch, host seat), in seat order.
            let mut seats: Vec<(usize, usize, usize)> = Vec::new();
            for (pi, p) in pipes.iter().enumerate() {
                if p.label != label {
                    continue;
                }
                for b in 0..p.slots.len() {
                    seats.push((pi, b, p.slot_base + b));
                }
            }
            seats.sort_by_key(|(_, _, host_slot)| *host_slot);

            let want: Vec<&SinkSurface> =
                surfaces.iter().filter(|s| s.window == label).take(seats.len()).collect();
            let mut held: Vec<Option<Slot>> =
                seats.iter().map(|(pi, b, _)| pipes[*pi].slots[*b].clone()).collect();
            // Keep every surface that still wants a seat where it is, then fill the freed seats
            // with the newcomers in priority order.
            for seat in held.iter_mut() {
                if let Some(cur) = seat {
                    if !want.iter().any(|s| s.key == cur.key) {
                        *seat = None;
                    }
                }
            }
            for s in &want {
                let rect = [s.full.0, s.full.1, s.full.2, s.full.3, s.clip.0, s.clip.1, s.clip.2, s.clip.3];
                if let Some(cur) = held.iter_mut().flatten().find(|c| c.key == s.key) {
                    cur.rect = rect;
                    continue;
                }
                let Some(free) = held.iter_mut().find(|c| c.is_none()) else { continue };
                *free = Some(Slot { key: s.key.clone(), rect });
            }
            // Which seat each surface ended up in, in the published (priority) order — the host
            // stacks the clips by it, because two surfaces may overlap.
            let order: Vec<usize> = want
                .iter()
                .filter_map(|s| held.iter().position(|c| c.as_ref().is_some_and(|c| c.key == s.key)))
                .map(|i| seats[i].2)
                .collect();

            for (i, (pi, b, host_slot)) in seats.iter().enumerate() {
                let live = held[i].clone();
                if let Some(branch) = pipes[*pi].branches.get(*b) {
                    branch.valve.set_property("drop", live.is_none());
                }
                if let Some(seat) = &live {
                    linux_host::set_rect(&label, *host_slot, self.host_rect(seat.rect));
                }
                linux_host::set_visible(&label, *host_slot, live.is_some());
                pipes[*pi].slots[*b] = live;
            }
            if !order.is_empty() {
                linux_host::restack(&label, order);
            }
        }
    }

    /// Turn a surface's rect into the one the host lays the widget out with. With a stripped
    /// conformance window the DISPLAY picture is aspect-fitted into the full box and the widget
    /// grown to the CODED picture around it, so the padding rows fall outside the visible box; the
    /// visible box is also clipped to the fitted picture (a letterboxed box would otherwise show
    /// the padding in its bars). Without a window it is the surface's own rect.
    fn host_rect(&self, rect: [i32; 8]) -> [i32; 8] {
        let [x, y, w, h, cx, cy, cw, ch] = rect;
        let Some(win) = self.window else { return rect };
        let g = self.geom.lock().unwrap();
        let (dw, dh) = (win.display_w().max(1) as f64, win.display_h().max(1) as f64);
        let scale = (w as f64 / dw).min(h as f64 / dh);
        let (fw, fh) = (dw * scale, dh * scale);
        let (fx, fy) = (x as f64 + (w as f64 - fw) / 2.0, y as f64 + (h as f64 - fh) / 2.0);
        // Flips move the padding: `horiz` and `180` swap left/right, `180` and `vert` swap
        // top/bottom (see set_orient).
        let left = if g.mirror != g.rotate180 { win.right } else { win.left } as f64;
        let top = if g.rotate180 { win.bottom } else { win.top } as f64;
        let (vx, vy) = ((cx as f64).max(fx), (cy as f64).max(fy));
        let (vx2, vy2) = (((cx + cw) as f64).min(fx + fw), ((cy + ch) as f64).min(fy + fh));
        let r = |v: f64| v.round() as i32;
        [
            r(fx - left * scale),
            r(fy - top * scale),
            r(win.coded_w as f64 * scale),
            r(win.coded_h as f64 * scale),
            r(vx),
            r(vy),
            r((vx2 - vx).max(1.0)),
            r((vy2 - vy).max(1.0)),
        ]
    }

    /// Re-apply every live seat's rect — after an orientation change, which moves the padding.
    fn relayout(&self) {
        let pipes = self.pipes.lock().unwrap();
        for pipe in pipes.iter() {
            for (slot, seat) in pipe.slots.iter().enumerate() {
                if let Some(seat) = seat {
                    let host_slot = pipe.slot_base + slot;
                    linux_host::set_rect(&pipe.label, host_slot, self.host_rect(seat.rect));
                }
            }
        }
    }

    /// Smoothing-buffer depth in frames (0 = render on decode, the latency-first default).
    pub fn set_buffer(&self, frames: u32) {
        let frames = frames.min(3);
        let before = self.shared.buffer_frames.swap(frames, Ordering::Relaxed);
        if (before == 0) != (frames == 0) {
            for pipe in self.pipes.lock().unwrap().iter() {
                for b in &pipe.branches {
                    b.sink.set_property("sync", frames > 0);
                }
            }
        }
    }

    /// Mirror / 180° rotation — live, in the flip element (a passthrough at identity).
    pub fn set_orient(&self, mirror: bool, rotate180: bool) {
        let direction = match (mirror, rotate180) {
            (false, false) => "identity",
            (true, false) => "horiz",
            (false, true) => "180",
            (true, true) => "vert", // mirror + 180° = vertical flip
        };
        // Per branch: the flip element belongs to one leg, so every leg of every pipeline needs
        // telling.
        for pipe in self.pipes.lock().unwrap().iter() {
            for b in &pipe.branches {
                b.flip.set_property_from_str("video-direction", direction);
            }
        }
        {
            let mut g = self.geom.lock().unwrap();
            g.mirror = mirror;
            g.rotate180 = rotate180;
        }
        // A flip moves the conformance-window padding, so every live seat is laid out again.
        if self.window.is_some() {
            self.relayout();
        }
    }

    pub fn frames_presented(&self) -> u64 {
        self.shared.presented.load(Ordering::Relaxed)
    }

    /// Decoded picture size (display area), once the pipeline negotiated its output.
    pub fn picture_size(&self) -> Option<(u32, u32)> {
        let w = self.shared.width.load(Ordering::Relaxed);
        let h = self.shared.height.load(Ordering::Relaxed);
        if w == 0 || h == 0 {
            return None;
        }
        // A stripped window: the decoder reports the coded size, the picture is the window.
        Some(self.window.map_or((w, h), |win| (win.display_w(), win.display_h())))
    }
}

impl Drop for LinuxVideoSink {
    /// Teardown order matters for the GL widget: (1) the pipeline stops, (2) the host takes
    /// the widget out of the tree — waited for, so the main thread's reference is gone —
    /// and only then (3) the pipeline (the sink element, holding the widget's LAST
    /// reference) drops on this worker thread. gtkglsink's widget finalizer hops onto the
    /// GTK main loop and blocks until it ran, so the last unref must never come from a
    /// blocked main thread, and the main loop must be free when it happens.
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // Inline, unlike the single-window teardown above: the sink is going away as a whole, so
        // nothing can build a pipeline behind our back, and the caller (a stream stop, a restart)
        // must not find half a sink still holding GL widgets.
        let pipes: Vec<Pipe> = self.pipes.lock().unwrap().drain(..).collect();
        for pipe in pipes {
            let label = pipe.label.clone();
            teardown_pipe(pipe, &self.shared);
            // A window of its own (the detached one) hands its video layer back with the pipeline
            // that fed it. Leaving it on file would make the NEXT window under that label inherit a
            // tree belonging to a destroyed one — see the rebuild check in `linux_host`. The main
            // window keeps its layer: it outlives every stream and is only ever installed once.
            if label != "main" {
                linux_host::uninstall(&label);
            }
            // The pipeline drops at the end of this iteration — step (3).
        }
    }
}

/// Pipeline errors → the sink's error slot (the stream ends on it and the frontend
/// reconnects with a fresh sink); warnings go to the log only.
fn bus_loop(bus: &gst::Bus, shared: &Shared, stopping: &AtomicBool) {
    use gst::MessageView;
    while !shared.stopping.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
        let Some(msg) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(200),
            &[gst::MessageType::Error, gst::MessageType::Warning, gst::MessageType::HaveContext],
        ) else {
            continue;
        };
        let src = msg.src().map(|s| s.name().to_string()).unwrap_or_default();
        match msg.view() {
            MessageView::HaveContext(h) => {
                // gtkglsink announces its display at start — keep it (see GL_DISPLAYS).
                let ctx = h.context();
                if ctx.context_type() == "gst.gl.GLDisplay" {
                    log::debug!("[video] linux sink: GL display from {src} kept alive");
                    GL_DISPLAYS.lock().unwrap().push(ctx);
                }
            }
            MessageView::Error(e) => {
                let debug = e.debug().map(|d| d.to_string()).unwrap_or_default();
                // A pipeline on its way out takes its window's widgets with it, and whatever it
                // shouts on the way ("Output widget was destroyed") is expected — it must not end
                // the STREAM, which is what happened when the detached window closed while its
                // last frames were still in flight (Marc, 2026-09-08).
                if stopping.load(Ordering::SeqCst) {
                    log::debug!("[video] linux sink: {src} while stopping: {}", e.error());
                } else {
                    shared.fail(format!("{src}: {} ({debug})", e.error()));
                }
            }
            MessageView::Warning(w) => {
                log::debug!("[video] linux sink: {src}: {}", w.error());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::video::rtsp_native::{NativeRtsp, Started};

    use gtk::glib;
    use gtk::prelude::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// The one surface these end-to-end tests publish — the router's payload for a 640×360 hole.
    fn test_surface() -> crate::video::surface::SurfaceRect {
        crate::video::surface::SurfaceRect {
            id: "main".into(),
            x: 20,
            y: 20,
            w: 640,
            h: 360,
            cx: 20,
            cy: 20,
            cw: 640,
            ch: 360,
        }
    }

    /// Start/stop cycles of the real sink under a real GtkWindow (needs a display and a
    /// running H264/HEVC source, e.g. tools/simulators/rtsp_test_server.py --codec h264):
    /// `KITE_RTSP_URL=rtsp://127.0.0.1:8600/live cargo test linux_sink -- --ignored --nocapture`
    /// The GTK loop runs on the test thread, the scenario on a worker — exactly the
    /// app's thread split. A watchdog aborts (with a message) if anything deadlocks.
    #[test]
    #[ignore]
    fn stop_start_cycles_do_not_hang() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        gtk::init().expect("gtk init (needs a display)");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Kite linux sink bench");
        window.set_default_size(800, 500);
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&vbox);
        window.show_all();
        linux_host::install_tree_for("main", &window, &vbox, None).expect("host tree");

        let done = Arc::new(AtomicBool::new(false));
        let worker = {
            let done = done.clone();
            std::thread::spawn(move || {
                let server = NativeRtsp::new();
                for cycle in 0..3 {
                    let t = Instant::now();
                    let started = server.start(Arc::new(|| {}), &url, "auto", None).expect("start");
                    assert!(matches!(started, Started::Sink { .. }), "expected the sink route");
                    eprintln!("cycle {cycle}: started in {:?}", t.elapsed());
                    server.sink_surfaces("main", vec![test_surface()]);
                    std::thread::sleep(Duration::from_secs(3));
                    let (presented, size, err) = server.sink_stats().expect("sink stats");
                    eprintln!("cycle {cycle}: presented={presented} size={size:?} err={err:?}");
                    assert!(err.is_none(), "sink error: {err:?}");
                    assert!(presented > 30, "expected >30 presented frames, got {presented}");
                    let t = Instant::now();
                    server.stop();
                    eprintln!("cycle {cycle}: stop took {:?}", t.elapsed());
                    assert!(server.sink_stats().is_none(), "sink must be gone after stop");
                    // The single-client test server needs a moment to come back.
                    std::thread::sleep(Duration::from_secs(1));
                }
                done.store(true, Ordering::SeqCst);
            })
        };
        // Watchdog: a deadlock on either thread must end the run with a verdict.
        {
            let done = done.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(90));
                if !done.load(Ordering::SeqCst) {
                    eprintln!("WATCHDOG: the scenario did not finish in 90 s — deadlock");
                    std::process::abort();
                }
            });
        }
        while !done.load(Ordering::SeqCst) {
            gtk::main_iteration_do(false);
            std::thread::sleep(Duration::from_millis(3));
        }
        worker.join().expect("worker panicked");
    }

    /// The app's real situation: an RGBA (transparent) window, a WebKitWebView repainting
    /// continuously as the overlay child, the sink below — cycling H264 and HEVC sources
    /// with the frontend's stop order (hide, then stop). Needs a display and BOTH test
    /// servers: `KITE_RTSP_URL=rtsp://127.0.0.1:8600/live KITE_RTSP_URL2=rtsp://127.0.0.1:8601/live
    /// cargo test under_a_webview -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn stop_start_cycles_under_a_webview() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        let url2 = std::env::var("KITE_RTSP_URL2").unwrap_or_else(|_| url.clone());
        gtk::init().expect("gtk init (needs a display)");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Kite linux sink bench (webview)");
        window.set_default_size(800, 500);
        if let Some(screen) = gtk::gdk::Screen::default() {
            if let Some(visual) = screen.rgba_visual() {
                window.set_visual(Some(&visual));
            }
        }
        window.set_app_paintable(true);
        window.connect_draw(|_, cr| {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cr.set_operator(gtk::cairo::Operator::Source);
            let _ = cr.paint();
            glib::Propagation::Proceed
        });
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&vbox);
        let webview = webkit2gtk::WebView::new();
        {
            use webkit2gtk::WebViewExt;
            webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
            // A page that repaints every frame (like the map) with a transparent hole.
            webview.load_html(
                "<body style='margin:0;background:#3d3f3e'>\
                 <div style='position:absolute;left:20px;top:20px;width:640px;height:360px;\
                 background:transparent;border:2px solid #37a8db'></div>\
                 <canvas id=c width=200 height=100 style='position:absolute;right:10px;bottom:10px'></canvas>\
                 <script>const x=c.getContext('2d');let i=0;(function f(){x.fillStyle='hsl('+(i++%360)+',80%,50%)';\
                 x.fillRect(0,0,200,100);requestAnimationFrame(f)})();</script></body>",
                None,
            );
        }
        vbox.pack_start(&webview, true, true, 0);
        window.show_all();
        linux_host::install_tree_for("main", &window, &vbox, Some(webview.upcast_ref())).expect("host tree");

        let done = Arc::new(AtomicBool::new(false));
        let worker = {
            let done = done.clone();
            std::thread::spawn(move || {
                let server = NativeRtsp::new();
                for cycle in 0..4 {
                    let u = if cycle % 2 == 0 { &url } else { &url2 };
                    let t = Instant::now();
                    let started = server.start(Arc::new(|| {}), u, "auto", None).expect("start");
                    let Started::Sink { codec } = started else { panic!("expected the sink route") };
                    eprintln!("cycle {cycle}: {codec} started in {:?}", t.elapsed());
                    server.sink_surfaces("main", vec![test_surface()]);
                    std::thread::sleep(Duration::from_secs(3));
                    let (presented, size, err) = server.sink_stats().expect("sink stats");
                    eprintln!("cycle {cycle}: presented={presented} size={size:?} err={err:?}");
                    assert!(err.is_none(), "sink error: {err:?}");
                    assert!(presented > 30, "expected >30 presented frames, got {presented}");
                    // Frontend order on stop: the router drops the surface, then the stream stops.
                    server.sink_surfaces("main", vec![]);
                    std::thread::sleep(Duration::from_millis(100));
                    let t = Instant::now();
                    server.stop();
                    eprintln!("cycle {cycle}: stop took {:?}", t.elapsed());
                    std::thread::sleep(Duration::from_secs(1));
                }
                done.store(true, Ordering::SeqCst);
            })
        };
        {
            let done = done.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(120));
                if !done.load(Ordering::SeqCst) {
                    eprintln!("WATCHDOG: the scenario did not finish in 120 s — deadlock");
                    std::process::abort();
                }
            });
        }
        while !done.load(Ordering::SeqCst) {
            gtk::main_iteration_do(false);
            std::thread::sleep(Duration::from_millis(3));
        }
        worker.join().expect("worker panicked");
    }
}
