// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video commands (docs/active/RTSP_VIDEO.md). Every command that spawns, waits on or tears down a
//! helper process is `#[tauri::command(async)]`: plain `fn` commands run on the MAIN thread, where a
//! wedged capture driver or a binary still being virus-scanned would freeze the whole UI.

use tauri::{AppHandle, Emitter, State};

use std::sync::Arc;

use crate::video::mediamtx::StreamSpec;
use crate::video::mjpeg_server::{EndedHook, MjpegSource, RtspTranscode};
use crate::video::{ffmpeg, mediamtx, native, MediaMtx};

/// Emitted when a running feed's source dies — never on our own stop. WebKit's multipart `<img>` never
/// fires `error` when the server closes mid-stream, so this is the image path's only reconnect trigger.
pub const MJPEG_ENDED_EVENT: &str = "video-mjpeg-ended";

/// The MJPEG server's runtime-agnostic "source died" callback → that event (see `EndedHook`).
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

/// Whether the ffmpeg MJPEG transcode of an RTSP source would run on hardware (Linux only). Answers
/// `false` without probing while ffmpeg is missing: the probes cache their verdict for the process, and
/// "no ffmpeg yet" must not become "no hardware" for an ffmpeg installed a minute later.
#[tauri::command(async)]
pub fn video_hw_transcode_available() -> bool {
    if ffmpeg::version().is_none() {
        return false;
    }
    ffmpeg::v4l2_h264_decode_available() || ffmpeg::vaapi_render_node().is_some()
}

/// Download ffmpeg into the app-data `bin/` dir (Windows), emitting `ffmpeg-download-progress`. The
/// fallback reader resolves ffmpeg per stream start, so no restart is needed.
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

/// Start (or refresh) the RTSP→WebRTC stream; returns once the source is connected, so "unreachable" is
/// told apart from a WebRTC failure. `use_ffmpeg`: read with our ffmpeg and publish into MediaMTX — the
/// fallback for quirky servers, never the default (an extra hop of ~22 ms).
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
    // `start` spawns processes and polls readiness (up to ~11 s) — keep it off the async runtime's
    // threads; the state is an `Arc` so it can cross into `spawn_blocking`.
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
    // Bounded: a wedged engine must never freeze the frontend's reconnect loop.
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

/// Enumerate native capture devices via ffmpeg (V4L2 / DirectShow / AVFoundation); empty without ffmpeg.
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

/// Start the embedded MJPEG server on a native device; returns the URL and the transcode mode actually
/// used (reported, not inferred: "can do hardware" ≠ "uses it"). `Ok` only once the capture produced
/// its first bytes — a rejected mode must not show "live" over a black frame.
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
    // No hardware transcode for native capture: MJPEG is stream-copied, and raw input gained only ~21 %
    // on VAAPI (the upload eats the gain).
    let transcode = if native::needs_transcode(&spec.codec) { "software" } else { "copy" };
    let port = mjpeg.start(ended_hook(&app), &MjpegSource::Device(&spec))?;
    Ok(serde_json::json!({ "url": format!("http://127.0.0.1:{port}/"), "transcode": transcode }))
}

/// Start the embedded MJPEG server on an RTSP source — the image path, WITHOUT the engine, whose loopback
/// republish stalled ~338 ms at a time (and RFC 2435 cannot carry every MJPEG). `require_copy`: only if
/// the source really is MJPEG — after a failed WebRTC negotiation a transcode is a permanent downgrade.
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

    // Try the stream copy first: far cheaper for an MJPEG source, and the mpjpeg muxer rejects anything
    // else, so trying costs one failed spawn rather than a probe.
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

/// Start the in-process native RTSP client: an MJPEG source is served over the local MJPEG port
/// (`mode: "mjpeg"`), H264/HEVC decodes natively into the hole-punch sink on Windows (`mode: "sink"`).
/// `transport` auto = UDP first, TCP-interleaved when no RTP arrives.
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

/// Publish the calling window's video surfaces to the native sink: per hole the FULL box and the VISIBLE
/// box (the sink cuts, never shrinks), in PHYSICAL px. The list REPLACES this window's earlier one (empty
/// = shows nothing); keyed by the calling window, so two windows cannot overwrite each other.
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

/// Live counters of the native RTSP stream plus the decode sink's (`sink: {…}`); `{active: false}`
/// while nothing runs. Polled at 1 Hz (Debug Monitor, fps, aspect, sink stall detection).
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

/// Spawn the detached video window (transparent, undecorated, always on top) on the `/video` route.
/// Geometry is ours, not the window-state plugin's (it restores sizes onto vanished monitors): PHYSICAL
/// px, validated by the caller. Opening it twice focuses the existing window.
#[tauri::command(async)]
pub fn video_detached_open(
    app: AppHandle,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    fullscreen: bool,
) -> Result<(), String> {
    // Desktop only: half the window API this needs does not exist on mobile, and the phone never shows a
    // second window.
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
        // All WebView2s of a process share ONE environment; a second one with other browser arguments fails
        // (0x8007139F) while the window creation still reports success. So this window reads exactly the main
        // window's arguments from the config.
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
        // `create_window` builds the window later on the main thread and only logs a failure there — wait until
        // it answers, or a window that was never created looks like a working one.
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

/// Put the detached video window back on top — needed after every fullscreen toggle: tao leaves
/// borderless fullscreen at `NSNormalWindowLevel`, so a pinned window sinks behind the next click.
/// The viewer calls this, as only it knows a toggle happened.
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

        // Surfaces first, window second: on Linux withdrawing the surfaces stops the pipeline rendering INSIDE
        // that window (late frames' complaints are swallowed, see `linux_sink`).
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

/// What the detached window's page handles itself (button rects, resize corner, whether a drag may move
/// the window), in CSS px. Read by Linux's GTK press handler (`video::linux_drag`); a no-op elsewhere.
#[tauri::command]
pub fn video_detached_chrome(zones: DetachedChrome) {
    #[cfg(target_os = "linux")]
    crate::video::linux_drag::set_zones(zones.drag, zones.grip, zones.chrome);
    #[cfg(not(target_os = "linux"))]
    let _ = zones;
}

/// Hold the detached window to the picture's shape while its corner is dragged — inside the OS resize
/// loop the page cannot correct the size. Per platform: `WM_SIZING`, GTK geometry hints, AppKit aspect.
/// `aspect` = width/height, `ring` = the page's chrome in physical px; `aspect <= 0` releases it.
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

/// Shake the detached window's size once so its WebView gets a fresh framebuffer: WebKitGTK's DMABUF
/// renderer paints garbage into the first buffer of the Pi's v3d driver, and only a size change
/// rebuilds it (the static detached page never forces one). ARM Linux only.
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
                // Leaving fullscreen is not necessarily a size change (a window CREATED fullscreen has no windowed
                // size), so force one — then restore the size before going back, so the remembered windowed box does
                // not shrink.
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

/// `<Videos>/KiteGC` — where recordings go unless the user chose a folder.
fn default_record_dir() -> std::path::PathBuf {
    dirs::video_dir().or_else(dirs::home_dir).unwrap_or_default().join("KiteGC")
}

#[tauri::command]
pub fn video_record_default_dir() -> String {
    default_record_dir().to_string_lossy().to_string()
}

/// The video folder: the user's choice, else the default. Recordings are written into it and looked
/// for below it.
pub(crate) fn record_dir(chosen: Option<&str>) -> std::path::PathBuf {
    match chosen.map(str::trim) {
        Some(d) if !d.is_empty() => std::path::PathBuf::from(d),
        _ => default_record_dir(),
    }
}

/// Recording switched on in the settings: keep the pre-roll so a recording can start at a keyframe.
#[tauri::command]
pub fn video_record_enable(on: bool, native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>) {
    native_rtsp.recorder().set_enabled(on);
}

fn file_name_part(vehicle: &str) -> String {
    let cleaned: String = vehicle.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() { "Video".to_string() } else { cleaned.chars().take(40).collect() }
}

/// The flight database of the running recording: an app exit while armed has no frontend to name it.
static RECORD_DB: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

fn flight_db_file(db_path: Option<String>) -> std::path::PathBuf {
    crate::flightlog::db::resolve_db_path(&db_path.unwrap_or_default(), crate::commands::flightlog::portable_mode())
}

/// App exit: close a running recording and leave it waiting, so the flight resumed at the next start
/// still gets it.
pub fn finish_recording_at_exit(native_rtsp: &crate::video::rtsp_native::NativeRtsp) {
    let files = native_rtsp.recorder().stop();
    if let Some(db_file) = RECORD_DB.lock().unwrap_or_else(|e| e.into_inner()).take() {
        crate::flightlog::video_link::add_waiting(&db_file, &files);
    }
}

/// Start recording into `dir` (empty = the default folder). The file is named `<vehicle>_<local time>`.
#[tauri::command(async)]
pub fn video_record_start(
    dir: Option<String>,
    vehicle: Option<String>,
    db_path: Option<String>,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<(), String> {
    *RECORD_DB.lock().unwrap_or_else(|e| e.into_inner()) = Some(flight_db_file(db_path));
    let dir = record_dir(dir.as_deref());
    let name = format!(
        "{}_{}",
        file_name_part(vehicle.as_deref().unwrap_or_default()),
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    );
    native_rtsp.recorder().start(&dir, &name)
}

/// Stop, close the file(s) and leave them waiting for the flight they belong to (`flightlog::video_link`).
#[tauri::command(async)]
pub fn video_record_stop(
    db_path: Option<String>,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Vec<crate::video::record::RecordedVideo> {
    let files = native_rtsp.recorder().stop();
    RECORD_DB.lock().unwrap_or_else(|e| e.into_inner()).take();
    let db_file = flight_db_file(db_path);
    crate::flightlog::video_link::add_waiting(&db_file, &files);
    match crate::flightlog::db::open_database(&db_file) {
        Ok(conn) => crate::flightlog::video_link::attach_to_saved_flights(&conn, &db_file),
        Err(e) => log::warn!("[video] recording stays unattached, flight DB not opened: {e}"),
    }
    files
}

#[tauri::command]
pub fn video_record_status(native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>) -> crate::video::record::RecordingStatus {
    native_rtsp.recorder().status()
}

/// Open the recordings of `flight_id` on the decode sink; a live stream ends. Fails when the flight has
/// no recording below the video folder.
#[tauri::command(async)]
pub fn video_replay_open(
    app: AppHandle,
    flight_id: i64,
    db_path: Option<String>,
    video_dir: Option<String>,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<serde_json::Value, String> {
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    {
        let db_file = flight_db_file(db_path);
        let conn = crate::flightlog::db::open_database(&db_file).map_err(|e| format!("Database error: {e}"))?;
        let parts = crate::flightlog::video_link::replay_parts(&conn, flight_id, &record_dir(video_dir.as_deref()))
            .map_err(|e| format!("Database error: {e}"))?;
        if parts.is_empty() {
            return Err("the flight has no recording on disk".into());
        }
        #[cfg(target_os = "windows")]
        let parent = {
            use tauri::Manager;
            app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize)
        };
        #[cfg(not(target_os = "windows"))]
        let parent: Option<isize> = {
            let _ = &app;
            None
        };
        let (codec, width, height) = native_rtsp.replay_open(parts, parent)?;
        Ok(serde_json::json!({ "codec": codec, "width": width, "height": height }))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = (&app, flight_id, &db_path, &video_dir, &native_rtsp);
        Err("not available on this platform".into())
    }
}

/// The replay's position: flight time (ms from the flight's start), running or not, speed. Returns whether
/// a recording covers it and whether its picture was handed to the sink; `null` while none is open.
#[tauri::command]
pub fn video_replay_sync(
    flight_ms: f64,
    playing: bool,
    rate: f64,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Option<serde_json::Value> {
    native_rtsp
        .replay_sync(flight_ms, playing, rate)
        .map(|(covered, ready)| serde_json::json!({ "covered": covered, "ready": ready }))
}
