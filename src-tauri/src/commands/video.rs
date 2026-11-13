// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video commands — the MediaMTX RTSP→WebRTC engine + its ffmpeg fallback dependency.
//! See docs/active/RTSP_VIDEO.md.
//!
//! **Threading:** every command here that spawns a helper process, waits on one, or tears one down is
//! marked `#[tauri::command(async)]`. Tauri runs plain `fn` commands on the **main thread**, so a
//! device enumeration behind a wedged capture driver (or a `--version` call on a binary Gatekeeper /
//! Defender is still scanning) would freeze the whole UI. Only trivially-cheap commands stay sync.

use tauri::{AppHandle, Emitter, State};

use std::sync::Arc;

use crate::video::mediamtx::StreamSpec;
use crate::video::mjpeg_server::{EndedHook, MjpegSource, RtspTranscode};
use crate::video::{ffmpeg, mediamtx, native, MediaMtx};

/// Emitted when a running feed's source dies (ffmpeg exited, read error) — never on our own stop.
///
/// The `<img>` sink cannot report this itself on WebKit: measured on 2.52.5, a multipart `<img>`
/// fires one `load` for the whole stream and then **no** `error` and no `abort` when the server
/// closes mid-stream, leaving the element on a dead `src` with `complete` still true. That is the
/// whole reconnect trigger for the image path, so it comes from the backend instead, where the fact
/// is known for certain and identically on every platform.
pub const MJPEG_ENDED_EVENT: &str = "video-mjpeg-ended";

/// Turn the MJPEG server's runtime-agnostic "the source died" callback into that event. The server
/// deliberately knows nothing about Tauri — see `EndedHook` for why that module has to stay linkable
/// without the window runtime.
fn ended_hook(app: &AppHandle) -> EndedHook {
    let app = app.clone();
    Arc::new(move || {
        let _ = app.emit(MJPEG_ENDED_EVENT, ());
    })
}

/// ffmpeg version string (`ffmpeg -version` first line), or null if it isn't installed yet. ffmpeg is
/// the fallback RTSP reader for MediaMTX (sources its native client can't pull), not always required.
#[tauri::command(async)]
pub fn video_ffmpeg_status() -> Option<String> {
    ffmpeg::version()
}

/// Whether the ffmpeg MJPEG transcode of an RTSP source would run on hardware on this machine (V4L2
/// M2M decode on Pi-class boards, VAAPI on a desktop GPU; Linux only). Decides whether the Video panel
/// shows "Disable hardware acceleration" at all. Answers `false` without probing while ffmpeg is
/// missing: the probes cache their verdict for the process, and "no ffmpeg yet" must not become
/// "no hardware" for an ffmpeg installed a minute later.
#[tauri::command(async)]
pub fn video_hw_transcode_available() -> bool {
    if ffmpeg::version().is_none() {
        return false;
    }
    ffmpeg::v4l2_h264_decode_available() || ffmpeg::vaapi_render_node().is_some()
}

/// Download ffmpeg into the app-data `bin/` dir (Windows). Emits `ffmpeg-download-progress`
/// (`{ pct, msg }`). Returns the installed path. The fallback reader resolves ffmpeg per stream
/// start, so a fresh download is picked up without restarting anything.
#[tauri::command]
pub async fn video_ffmpeg_download(app_handle: AppHandle) -> Result<String, String> {
    let report = |pct: u8, msg: &str| {
        let _ = app_handle.emit(
            "ffmpeg-download-progress",
            serde_json::json!({ "pct": pct, "msg": msg }),
        );
    };
    let path = ffmpeg::download(report).await?;
    Ok(path.to_string_lossy().to_string())
}

// ── MediaMTX / WebRTC (the live RTSP path) ───────────────────────────

/// Engine presence string (version/installed), or null if not installed yet.
#[tauri::command(async)]
pub fn video_engine_status() -> Option<String> {
    mediamtx::status()
}

/// Download the pinned MediaMTX into the app-data `bin/` dir. Emits `video-engine-download-progress`
/// (`{ pct, msg }`). Returns the installed path.
#[tauri::command]
pub async fn video_engine_download(app_handle: AppHandle) -> Result<String, String> {
    let report = |pct: u8, msg: &str| {
        let _ = app_handle.emit(
            "video-engine-download-progress",
            serde_json::json!({ "pct": pct, "msg": msg }),
        );
    };
    let path = mediamtx::download(report).await?;
    Ok(path.to_string_lossy().to_string())
}

/// Start (or refresh) the RTSP→WebRTC stream for `url`: MediaMTX pulls the source itself and the
/// browser then negotiates WHEP via `video_webrtc_offer`. Returns once the source is actually
/// connected and its tracks are known — "the source is unreachable" surfaces here, distinct from a
/// WebRTC failure.
///
/// `transport`: `udp` | `tcp` | `auto` — how MediaMTX' native RTSP client pulls the source (the Pi
/// is UDP-only; `auto` lets MediaMTX negotiate).
///
/// `use_ffmpeg`: read the source with our ffmpeg (no forced transport — the only mode quirky
/// servers like obs-rtspserver accept) and publish it into MediaMTX over RTSP/UDP. The automatic
/// fallback when the native pull fails; never the default, because the extra hop costs ~22 ms
/// (measured) and the publish leg is the part that ever went wrong historically.
#[tauri::command]
pub async fn video_webrtc_start(
    url: String,
    transport: String,
    use_ffmpeg: bool,
    engine: State<'_, Arc<MediaMtx>>,
) -> Result<(), String> {
    let spec = StreamSpec {
        url,
        transport: match transport.as_str() {
            "udp" => "udp".into(),
            "tcp" => "tcp".into(),
            _ => "automatic".into(),
        },
        use_ffmpeg,
    };
    // `start` spawns processes and polls readiness (up to ~11 s worst case) — keep it off the async
    // runtime's threads. The state is managed as an `Arc` precisely so it can cross into
    // `spawn_blocking`; only the verdict comes back.
    let engine = Arc::clone(engine.inner());
    tauri::async_runtime::spawn_blocking(move || engine.start(spec))
        .await
        .map_err(|e| format!("engine start task failed: {e}"))?
}

/// Exchange a browser WebRTC SDP offer via MediaMTX' WHEP endpoint and return the SDP answer
/// (proxied to avoid CORS and so the frontend never needs to know a port).
#[tauri::command]
pub async fn video_webrtc_offer(sdp: String, engine: State<'_, Arc<MediaMtx>>) -> Result<String, String> {
    let port = engine
        .whep_port()
        .ok_or("the video engine is not running — start the stream first")?;
    // Bounded: never let a wedged engine freeze the frontend's reconnect loop. The source is
    // already connected by the time this runs (video_webrtc_start waits for readiness), so this
    // only covers the WHEP negotiation itself.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let resp = client
        .post(format!("http://127.0.0.1:{port}/kite/whep"))
        .header("Content-Type", "application/sdp")
        .body(sdp)
        .send()
        .await
        .map_err(|e| format!("WHEP offer failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Surface MediaMTX' own error text (e.g. no compatible codec).
        return Err(format!("WHEP offer HTTP {status}: {}", body.trim()));
    }
    // WHEP answers with the SDP as the response body (Content-Type application/sdp).
    if body.trim().is_empty() {
        return Err("WHEP answer is empty".to_string());
    }
    Ok(body)
}

/// Stop the WebRTC stream (kills the local MediaMTX process and its ffmpeg publisher, if any).
/// Idempotent. Async: process teardown waits on the children.
#[tauri::command(async)]
pub fn video_webrtc_stop(engine: State<'_, Arc<MediaMtx>>) -> Result<(), String> {
    engine.stop();
    Ok(())
}

// ── Native capture (V4L2 / DirectShow / AVFoundation) ─────────────────

/// Enumerate native capture devices (USB/HDMI dongles etc.) for the "Advanced" source. Uses the OS
/// hardware layer via ffmpeg (Linux V4L2, Windows DirectShow, macOS AVFoundation). Empty on
/// unsupported platforms / when ffmpeg is missing.
#[tauri::command(async)]
pub fn video_list_native_devices() -> Vec<native::NativeDevice> {
    native::list_devices()
}

/// Probe a device's supported capture modes (codec + resolution range + fps range). Best-effort: V4L2
/// reports no framerate (0 = unknown) and AVFoundation returns nothing — the frontend then falls back
/// to the curated FPV catalog.
#[tauri::command(async)]
pub fn video_probe_device(id: String) -> Vec<native::CaptureMode> {
    native::probe(&id)
}

/// Start the embedded MJPEG HTTP server capturing from a native device with the chosen mode
/// (codec/resolution/framerate). MJPEG input is stream-copied; anything else is transcoded. Returns
/// the local URL plus the transcode mode actually used, killing any previous server first.
///
/// The mode is reported rather than inferred in the UI: "can this host do hardware" and "is this
/// stream using it" are different questions, and showing the first as if it were the second told the
/// user "Hardware" for a feed that was in fact a plain stream copy.
///
/// Only returns `Ok` once the capture actually produced its first bytes — a device that rejects the
/// requested mode used to leave the UI showing "live" over a black frame (see `MjpegServer::start`).
#[tauri::command(async)]
pub fn video_native_mjpeg_start(
    app: AppHandle,
    id: String,
    codec: String,
    width: u32,
    height: u32,
    fps: u32,
    mjpeg: State<'_, crate::video::MjpegServer>,
) -> Result<serde_json::Value, String> {
    let spec = native::CaptureSpec { id, codec, width, height, fps };
    // Native capture has no hardware transcode path: an MJPEG camera is stream-copied (nothing left
    // to accelerate) and the raw-input case measured only ~21 % better on VAAPI because the upload
    // eats most of the gain, so it stays in software.
    let transcode = if native::needs_transcode(&spec.codec) { "software" } else { "copy" };
    let port = mjpeg.start(ended_hook(&app), &MjpegSource::Device(&spec))?;
    Ok(serde_json::json!({ "url": format!("http://127.0.0.1:{port}/"), "transcode": transcode }))
}

/// Start the embedded MJPEG server on an RTSP source — the image path, **without the engine**.
///
/// The old go2rtc chain republished an already-MJPEG stream as RTP/JPEG over loopback RTSP/TCP and
/// back; measured over the same 120 s against a UAV-Link, the source had **zero** arrival gaps above
/// 200 ms and the engine's output had **69**, each ~338 ms — the TCP-publish stall, and the cause of
/// the freezes testers reported. Reading the source once and broadcasting `-f mpjpeg` measures as
/// clean as the source itself. RFC 2435 also only carries baseline JPEG at 4:2:0/4:2:2, so this path
/// additionally reaches MJPEG sources a republish would reject outright.
///
/// `require_copy` is the caller's way of saying "only take this path if the source really is MJPEG":
/// after a failed WebRTC negotiation, settling for a transcode would be a permanent downgrade.
#[tauri::command(async)]
pub fn video_rtsp_mjpeg_start(
    app: AppHandle,
    url: String,
    require_copy: bool,
    allow_hw_decode: Option<bool>,
    mjpeg: State<'_, crate::video::MjpegServer>,
) -> Result<serde_json::Value, String> {
    let reply = |port: u16, t: RtspTranscode| {
        serde_json::json!({ "url": format!("http://127.0.0.1:{port}/"), "transcode": t.label() })
    };

    // Try the stream copy first. A source that already sends MJPEG is cheaper by a wide margin
    // (measured on this very stream: 7.4 % of a core against 47.6 % for a transcode), and trying is
    // the only way to know — the mpjpeg muxer rejects anything that isn't MJPEG, so the attempt costs
    // a failed spawn rather than a probe.
    let copy = MjpegSource::Rtsp { url: &url, transcode: RtspTranscode::Copy };
    match mjpeg.start(ended_hook(&app), &copy) {
        Ok(port) => {
            log::info!("[video] RTSP source already carries MJPEG — stream-copied, no transcode");
            return Ok(reply(port, RtspTranscode::Copy));
        }
        Err(e) if require_copy => return Err(format!("source does not carry MJPEG: {e}")),
        Err(e) => log::debug!("[video] no MJPEG track in the source ({e}) — transcoding instead"),
    }

    // V4L2 M2M is the Pi-class path (hardware decode only, no MJPEG encoder exists for it); VAAPI is
    // the desktop-GPU one and does the whole chain. Probed in that order — on a Raspberry Pi a render
    // node exists with no VAAPI driver behind it, so asking there probes hardware that cannot answer.
    let transcode = if !allow_hw_decode.unwrap_or(true) {
        RtspTranscode::Software
    } else if crate::video::ffmpeg::v4l2_h264_decode_available() {
        RtspTranscode::V4l2m2m
    } else if let Some(node) = crate::video::ffmpeg::vaapi_render_node() {
        RtspTranscode::Vaapi(node)
    } else {
        RtspTranscode::Software
    };
    let port = mjpeg.start(ended_hook(&app), &MjpegSource::Rtsp { url: &url, transcode })?;
    log::info!("[video] RTSP MJPEG transcode running ({})", transcode.label());
    Ok(reply(port, transcode))
}

/// Stop the embedded MJPEG server if running. Async: kills ffmpeg and joins the broadcast threads,
/// which can sit in a blocking client write for up to `CLIENT_WRITE_TIMEOUT`.
#[tauri::command(async)]
pub fn video_native_mjpeg_stop(mjpeg: State<'_, crate::video::MjpegServer>) -> Result<(), String> {
    mjpeg.stop();
    Ok(())
}

// ── Native RTSP client (Kite's own, in-process — MOBILE_RTSP.md P1) ───────────

/// Start the in-process native RTSP client on `url` — no MediaMTX, no ffmpeg. An MJPEG
/// source is served over the local multipart MJPEG port (`mode: "mjpeg"` + `url`); an
/// H264/HEVC source decodes natively into the hole-punch sink on Windows (`mode: "sink"`
/// + `codec`, no URL — the frontend cuts the CSS hole and syncs the rect via the sink
/// commands below). `transport`: udp | tcp | auto (UDP first, automatic TCP-interleaved
/// fallback when no RTP arrives).
#[tauri::command(async)]
pub fn video_rtsp_native_start(
    app: AppHandle,
    url: String,
    transport: String,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<serde_json::Value, String> {
    // The decode sink renders into a child window of the main window, below the WebView.
    #[cfg(target_os = "windows")]
    let parent = {
        use tauri::Manager;
        app.get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as isize)
    };
    #[cfg(not(target_os = "windows"))]
    let parent: Option<isize> = None;
    match native_rtsp.start(ended_hook(&app), &url, &transport, parent)? {
        crate::video::rtsp_native::Started::Mjpeg { port } => Ok(serde_json::json!({
            "mode": "mjpeg",
            "url": format!("http://127.0.0.1:{port}/"),
            // "copy" is the honest verdict: the frames pass through untouched.
            "transcode": "copy",
        })),
        crate::video::rtsp_native::Started::Sink { codec } => Ok(serde_json::json!({
            // "none": nothing transcodes anywhere — AUs go straight into the HW decoder.
            "mode": "sink",
            "transcode": "none",
            "codec": codec,
        })),
    }
}

/// Publish the calling window's video surfaces to the native decode sink: every DOM hole it wants
/// the picture in, with the surface's FULL box `x/y/w/h` (the video's aspect-fit layout) and its
/// VISIBLE box `cx/cy/cw/ch` (what is left after the DOM's clipping ancestors — the sink CUTS the
/// picture there instead of shrinking it). All in PHYSICAL px of that window's client area.
///
/// The list REPLACES what this window published before, so visibility is implicit: a surface that
/// stops being listed is gone, and an empty list means this window shows nothing. Tauri hands us
/// the calling window, so a second window (the detached video window) can never overwrite the main
/// window's holes. Cheap; a no-op while no sink runs.
#[tauri::command]
pub fn video_rtsp_native_sink_surfaces(
    surfaces: Vec<crate::video::surface::SurfaceRect>,
    window: tauri::Window,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) {
    native_rtsp.sink_surfaces(window.label(), surfaces);
}

/// Smoothing-buffer depth for the native decode sink (frames, 0 = present on decode —
/// the latency-first default). Shares the panel's "Smoothing buffer" stepper with the
/// WebRTC/MJPEG paths; no-op while no sink runs.
#[tauri::command]
pub fn video_rtsp_native_sink_buffer(
    frames: u32,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) {
    native_rtsp.sink_buffer(frames);
}

/// Horizontal mirror / 180° rotation of the native decode sink's picture (the DOM sinks
/// do this with a CSS transform, which cannot touch the native layer).
#[tauri::command]
pub fn video_rtsp_native_sink_orient(
    mirror: bool,
    rotate180: bool,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) {
    native_rtsp.sink_orient(mirror, rotate180);
}

/// Live counters of the running native RTSP stream: client side (transport, RTP
/// received/lost/reordered/late, frames/dropped, bytes) plus the decode sink's numbers
/// (`sink: {presented, width, height, error, codec}`) when that route is active.
/// `{active: false}` while nothing runs. Polled 1 Hz by the frontend for the Debug
/// Monitor, the fps readout, aspect ratio and sink stall detection.
#[tauri::command]
pub fn video_rtsp_native_stats(
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> serde_json::Value {
    native_rtsp
        .debug_stats()
        .unwrap_or_else(|| serde_json::json!({ "active": false }))
}

/// Stop the in-process native RTSP client if running. Idempotent.
#[tauri::command(async)]
pub fn video_rtsp_native_stop(
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<(), String> {
    native_rtsp.stop();
    Ok(())
}

/// Dev-only (Linux debug builds): put a coloured stand-in into the hole-punch host so the
/// window transparency, hole geometry and scroll-clip can be verified without a decoder
/// (MOBILE_RTSP.md P2.3 stage A). Elsewhere: not available.
#[tauri::command]
pub fn video_linux_hole_spike(on: bool) -> Result<(), String> {
    #[cfg(all(target_os = "linux", debug_assertions))]
    {
        crate::video::linux_host::spike(on);
        Ok(())
    }
    #[cfg(not(all(target_os = "linux", debug_assertions)))]
    {
        let _ = on;
        Err("not available on this build".to_string())
    }
}

// ── Detached video window (VIDEO_MULTISINK_WINDOW.md PR B) ────────────

#[cfg(desktop)]
/// Label of the detached video window. The surface router in that window publishes its holes under
/// this label, so the sink can tell them apart from the main window's.
pub const DETACHED_LABEL: &str = "video";

/// How long `video_detached_open` waits for the window to come up, and in what steps.
#[cfg(desktop)]
const READY_POLLS: u32 = 60;
#[cfg(desktop)]
const READY_POLL_MS: u64 = 50;

#[cfg(desktop)]
/// Emitted to the main window when the detached video window is gone — its own close button,
/// Alt+F4, or our `video_detached_close`. The frontend docks the picture back in (D6).
pub const DETACHED_CLOSED_EVENT: &str = "video-detached-closed";

/// Spawn the detached video window: a transparent, undecorated, always-on-top window (D4, D8, D12)
/// running the `/video` route, which draws the same glass frame as the in-app floating window and
/// registers ONE hole for the native decode sink.
///
/// Geometry is ours, not the window-state plugin's (see §5.5 — the plugin restores a size onto a
/// monitor that may be gone): `x/y` and `w/h` are PHYSICAL px, already validated against the
/// available monitors by the caller. Opening it twice just focuses the existing window.
#[tauri::command(async)]
pub fn video_detached_open(
    app: AppHandle,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    fullscreen: bool,
) -> Result<(), String> {
    // Desktop only — a second window is a desktop idea, and half the window API this needs
    // (`title` on the builder, `set_focus`, `destroy`) does not exist on mobile. The phone shows
    // the docked window or the widget, never both (D10), and never a second window.
    #[cfg(desktop)]
    {
        use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

        if let Some(win) = app.get_webview_window(DETACHED_LABEL) {
            let _ = win.set_focus();
            return Ok(());
        }
        // Built hidden: a transparent window flashes its background before the page paints, and the
        // exact box is applied below in physical px (the builder only takes logical ones).
        #[allow(unused_mut)]
        let mut builder = WebviewWindowBuilder::new(&app, DETACHED_LABEL, WebviewUrl::App("video".into()))
            .title("Kite Ground Control — Video")
            .decorations(false)
            .transparent(true)
            .resizable(true)
            .always_on_top(true)
            .visible(false)
            .inner_size(640.0, 360.0)
            .min_inner_size(200.0, 120.0);
        // Every WebView2 in a process shares ONE environment, and asking for a second one with different
        // browser arguments fails with ERROR_INVALID_STATE (0x8007139F) — the webview is then never
        // created while the window creation still reports success. The main window sets its own
        // arguments in tauri.windows.conf.json, so this window has to ask for exactly the same ones;
        // read from the config rather than repeated here, so the two cannot drift apart.
        #[cfg(windows)]
        if let Some(args) = app
            .config()
            .app
            .windows
            .first()
            .and_then(|w| w.additional_browser_args.clone())
        {
            builder = builder.additional_browser_args(&args);
        }
        let win = builder
            .build()
            .map_err(|e| format!("detached video window: {e}"))?;
        // `create_window` hands the event loop a closure and returns: the window is built a moment
        // later on the main thread, and a failure THERE is only logged, never returned. So wait until
        // the window answers before reporting success — otherwise a window that was never created looks
        // exactly like a working one from here (which is how the WebView2 mismatch above stayed hidden).
        let mut ready = false;
        for _ in 0..READY_POLLS {
            if win.is_visible().is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(READY_POLL_MS));
        }
        if !ready {
            log::warn!("[video] detached window was not created — see the error above this line");
            let _ = win.destroy();
            return Err("the video window could not be created".to_string());
        }
        let _ = win.set_position(PhysicalPosition::new(x, y));
        let _ = win.set_size(PhysicalSize::new(w.max(200), h.max(120)));
        if fullscreen {
            let _ = win.set_fullscreen(true);
        }
        // The native handle BEFORE the page can publish a surface: the sink drops surfaces from a window
        // it has no handle for, and a still window publishes nothing again until it moves.
        #[cfg(target_os = "windows")]
        match win.hwnd() {
            Ok(handle) => {
                app.state::<crate::video::rtsp_native::NativeRtsp>()
                    .register_window(DETACHED_LABEL, handle.0 as isize);
                // The same handle carries the aspect lock (`video_detached_aspect`): it holds the
                // shape from inside the OS resize loop, where the page cannot reach.
                crate::video::win_aspect::install(handle.0 as isize);
            }
            Err(e) => log::warn!("[video] detached window has no native handle ({e}) — no picture there"),
        }
        // Linux: the detached window needs a video layer tree of its own. A GStreamer sink's widget
        // cannot move between windows, so that window gets its own host — and its own pipeline in
        // `linux_sink`, fed the same access units from the one RTSP connection.
        #[cfg(target_os = "linux")]
        crate::video::linux_host::install_for(&app, DETACHED_LABEL);
        let app_handle = app.clone();
        win.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                app_handle
                    .state::<crate::video::rtsp_native::NativeRtsp>()
                    .forget_window(DETACHED_LABEL);
                let _ = app_handle.emit_to("main", DETACHED_CLOSED_EVENT, ());
            }
        });
        let _ = win.show();
        // Re-assert it after the window is realised: the builder flag is applied during creation,
        // and this is the one property the window exists for (D12).
        let _ = win.set_always_on_top(true);
        #[cfg(target_os = "macos")]
        crate::video::apple_host::float_window(DETACHED_LABEL.to_string());
        log::info!("[video] detached window opened at {x},{y} {w}x{h} (fullscreen={fullscreen})");
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, x, y, w, h, fullscreen);
        Err("the detached video window is desktop-only".to_string())
    }
}

/// Put the detached video window back above the others. Needed after every fullscreen toggle:
/// tao ends its borderless-fullscreen mode by setting `NSNormalWindowLevel` unconditionally, so a
/// window that was pinned comes back out of fullscreen at the normal level and sinks behind
/// whatever the user clicks next (Marc, macOS, 2026-09-08). The viewer calls this itself, because
/// it is the only side that knows a toggle happened.
#[tauri::command(async)]
pub fn video_detached_pin_top(app: AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        if let Some(win) = app.get_webview_window(DETACHED_LABEL) {
            let _ = win.set_always_on_top(true);
        }
        // ...and on macOS at the level AppKit actually means — see `float_window`.
        #[cfg(target_os = "macos")]
        crate::video::apple_host::float_window(DETACHED_LABEL.to_string());
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(())
    }
}

/// Close the detached video window if it is open. Idempotent — the picture docks back into the app
/// when the `video-detached-closed` event lands.
#[tauri::command(async)]
pub fn video_detached_close(app: AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        // Surfaces first, window second. On Linux that window owns a GStreamer pipeline rendering
        // into a widget INSIDE it, and withdrawing the surfaces is what stops that pipeline. The
        // last frames may still be in flight when the window goes — a stopping pipeline's
        // complaints are swallowed rather than ending the stream (see `linux_sink`'s bus loop).
        app.state::<crate::video::rtsp_native::NativeRtsp>()
            .forget_window(DETACHED_LABEL);
        if let Some(win) = app.get_webview_window(DETACHED_LABEL) {
            win.destroy().map_err(|e| e.to_string())?;
        }
        // Belt and braces: whatever route the pipeline took down, this window's video layer must
        // not outlive it (`linux_host` explains what an inherited one does to the next window).
        #[cfg(target_os = "linux")]
        crate::video::linux_host::uninstall(DETACHED_LABEL);
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(())
    }
}

/// What the detached window's page handles itself: the rects of its overlay buttons (while they
/// are on screen) and of its resize corner, in CSS px, plus whether dragging the picture may move
/// the window at all. Linux reads them from the GTK press handler — see `video::linux_drag`, which
/// also explains why the move cannot go through `startDragging()` there. A no-op elsewhere.
#[tauri::command]
pub fn video_detached_chrome(zones: DetachedChrome) {
    #[cfg(target_os = "linux")]
    crate::video::linux_drag::set_zones(zones.drag, zones.grip, zones.chrome);
    #[cfg(not(target_os = "linux"))]
    let _ = zones;
}

/// Hold the detached window to the picture's shape while the user drags its corner. The page can
/// only put the size right AFTER a drag — inside the OS's resize loop every correction it makes is
/// overwritten by the next mouse move, which is what made the frame snap into shape when the
/// pointer stopped. Each platform enforces it where its resize actually happens: a `WM_SIZING`
/// subclass on Windows, GTK's geometry hints on Linux, AppKit's content aspect ratio on macOS.
///
/// `aspect` is the picture's width/height and `ring` the page's chrome around it in physical px;
/// `aspect <= 0` releases the window, which is what fullscreen wants.
#[tauri::command]
pub fn video_detached_aspect(app: AppHandle, aspect: f64, ring: f64) {
    #[cfg(target_os = "windows")]
    {
        let _ = &app;
        crate::video::win_aspect::set_shape(aspect, ring);
    }
    #[cfg(target_os = "linux")]
    crate::video::linux_drag::set_aspect(&app, DETACHED_LABEL, aspect, ring);
    #[cfg(target_os = "macos")]
    {
        let _ = &app;
        crate::video::apple_host::set_aspect(DETACHED_LABEL.to_string(), aspect, ring);
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    let _ = (app, aspect, ring);
}

/// Shake the detached window's size once, so its WebView gets a fresh framebuffer.
///
/// WebKitGTK's DMABUF renderer paints garbage into the first buffer the Pi's v3d driver hands it —
/// scanline corruption over the whole page, reproduced on that hardware for months (the Jarvis
/// dashboard carries the same workaround). It is not permanent: ANY change of the draw area's size
/// forces the buffer to be built again and the picture is clean from then on. Kite's main window
/// never shows it because it repaints constantly and clears the buffer by itself; the detached
/// window's page is static and almost entirely transparent, so nothing ever forces a new buffer and
/// the corruption stays for the life of the window (Marc, Pi 5, 2026-09-08).
///
/// Windowed, two pixels wider and back: the aspect lock pulls the height along, so one axis is
/// enough and the window lands exactly where it started. Fullscreen, the compositor owns the size —
/// the only size change left to a client is leaving fullscreen and going back in.
///
/// ARM Linux only: no other platform has this driver.
#[tauri::command]
pub fn video_detached_nudge(app: AppHandle) {
    #[cfg(target_os = "linux")]
    {
        use tauri::Manager;

        // A runtime test, not a `cfg`: this way the workaround is compiled and type-checked on every
        // Linux, and const-folded away where it can never apply.
        if !cfg!(target_arch = "aarch64") {
            return;
        }
        let Some(win) = app.get_webview_window(DETACHED_LABEL) else { return };
        // Off the caller's thread: both halves need the compositor to have acted in between.
        std::thread::spawn(move || {
            let settle = std::time::Duration::from_millis(200);
            if win.is_fullscreen().unwrap_or(false) {
                // Leaving fullscreen is not necessarily a SIZE change, and only a size change
                // rebuilds the buffer. A window that was CREATED fullscreen — Kite restarted with
                // the picture detached and full screen — has no windowed size to fall back to and
                // comes out of fullscreen exactly as large as it went in, so the garbage stayed
                // until Marc resized the window by hand (Pi 5, 2026-09-08). Force the change, then
                // put the size back before returning to fullscreen: what the window remembers as
                // its windowed box must not shrink just because we shook it.
                let _ = win.set_fullscreen(false);
                std::thread::sleep(settle);
                if let Ok(size) = win.inner_size() {
                    let smaller = tauri::PhysicalSize::new(
                        size.width.saturating_sub(64).max(320),
                        size.height.saturating_sub(64).max(240),
                    );
                    let _ = win.set_size(smaller);
                    std::thread::sleep(settle);
                    let _ = win.set_size(size);
                    std::thread::sleep(settle);
                }
                let _ = win.set_fullscreen(true);
                return;
            }
            let Ok(size) = win.inner_size() else { return };
            let _ = win.set_size(tauri::PhysicalSize::new(size.width + 2, size.height));
            std::thread::sleep(settle);
            let _ = win.set_size(size);
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

/// The page's layout, as [`video_detached_chrome`] takes it. Only Linux reads the rects — the other
/// platforms take their window gestures from the page itself and never need to know.
#[derive(serde::Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct DetachedChrome {
    /// Dragging the picture moves the window (false while fullscreen).
    pub drag: bool,
    /// The resize corner, `[x, y, w, h]`.
    pub grip: Option<[f64; 4]>,
    /// Rects the page wants the press for.
    pub chrome: Vec<[f64; 4]>,
}
