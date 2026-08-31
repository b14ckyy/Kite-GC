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

/// Start the in-process native RTSP client on `url` and serve its frames over the local
/// multipart MJPEG port — no MediaMTX, no ffmpeg. MJPEG sources only until the P2.1 decode
/// sinks; an H264/HEVC source fails with a message saying so. `transport`: udp | tcp |
/// auto (UDP first, automatic TCP-interleaved fallback when no RTP arrives).
#[tauri::command(async)]
pub fn video_rtsp_native_start(
    app: AppHandle,
    url: String,
    transport: String,
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<serde_json::Value, String> {
    let port = native_rtsp.start(ended_hook(&app), &url, &transport)?;
    // "copy" is the honest verdict: the frames pass through untouched, nothing transcodes.
    Ok(serde_json::json!({ "url": format!("http://127.0.0.1:{port}/"), "transcode": "copy" }))
}

/// Stop the in-process native RTSP client if running. Idempotent.
#[tauri::command(async)]
pub fn video_rtsp_native_stop(
    native_rtsp: State<'_, crate::video::rtsp_native::NativeRtsp>,
) -> Result<(), String> {
    native_rtsp.stop();
    Ok(())
}
