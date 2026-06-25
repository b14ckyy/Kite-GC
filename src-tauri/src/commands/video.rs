// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video commands — the go2rtc RTSP→WebRTC engine + its ffmpeg fallback dependency.
//! See docs/active/RTSP_VIDEO.md.

use tauri::{AppHandle, Emitter, State};

use crate::video::{ffmpeg, go2rtc, Go2Rtc};

/// Fixed go2rtc stream name for the single live RTSP feed.
const STREAM_NAME: &str = "kite";

/// ffmpeg version string (`ffmpeg -version` first line), or null if it isn't installed yet. ffmpeg is
/// the fallback RTSP reader for go2rtc (sources its native client can't read), not always required.
#[tauri::command]
pub fn video_ffmpeg_status() -> Option<String> {
    ffmpeg::version()
}

/// Download ffmpeg into the app-data `bin/` dir (Windows). Emits `ffmpeg-download-progress`
/// (`{ pct, msg }`). Returns the installed path. go2rtc is pointed at this path, so a freshly
/// downloaded ffmpeg is picked up on the next stream start without restarting go2rtc.
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

// ── go2rtc / WebRTC (the live RTSP path) ─────────────────────────────

/// go2rtc presence string (version/installed), or null if not installed yet.
#[tauri::command]
pub fn video_go2rtc_status() -> Option<String> {
    go2rtc::status()
}

/// Download go2rtc into the app-data `bin/` dir (Windows). Emits `go2rtc-download-progress`
/// (`{ pct, msg }`). Returns the installed path.
#[tauri::command]
pub async fn video_go2rtc_download(app_handle: AppHandle) -> Result<String, String> {
    let report = |pct: u8, msg: &str| {
        let _ = app_handle.emit(
            "go2rtc-download-progress",
            serde_json::json!({ "pct": pct, "msg": msg }),
        );
    };
    let path = go2rtc::download(report).await?;
    Ok(path.to_string_lossy().to_string())
}

/// Start (or refresh) the go2rtc RTSP→WebRTC stream for `url`. Ensures go2rtc is running and
/// registers the source. The browser then negotiates WebRTC via `video_webrtc_offer`.
///
/// `use_ffmpeg`: register the source via go2rtc's bundled-ffmpeg reader instead of its native RTSP
/// client. The `input=rtsp/udp` template uses ffmpeg WITHOUT a forced `-rtsp_transport`, which is the
/// only mode that reads quirky servers (e.g. obs-rtspserver, which 461s any forced transport). Used
/// as the automatic fallback when the native client fails.
#[tauri::command]
pub async fn video_webrtc_start(
    url: String,
    use_ffmpeg: bool,
    engine: State<'_, Go2Rtc>,
) -> Result<(), String> {
    let port = engine.ensure_running()?;
    let src = if use_ffmpeg {
        format!("ffmpeg:{url}#input=rtsp/udp#video=copy")
    } else {
        url.clone()
    };
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("http://127.0.0.1:{port}/api/streams"))
        .query(&[("name", STREAM_NAME), ("src", src.as_str())])
        .send()
        .await
        .map_err(|e| format!("go2rtc add-stream failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("go2rtc add-stream HTTP {}", resp.status()));
    }
    Ok(())
}

/// Exchange a browser WebRTC SDP offer with go2rtc and return the SDP answer (proxied to avoid CORS).
#[tauri::command]
pub async fn video_webrtc_offer(sdp: String, engine: State<'_, Go2Rtc>) -> Result<String, String> {
    let port = engine
        .port()
        .ok_or("go2rtc is not running — start the stream first")?;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/api/webrtc"))
        .query(&[("src", STREAM_NAME)])
        .json(&serde_json::json!({ "type": "offer", "sdp": sdp }))
        .send()
        .await
        .map_err(|e| format!("go2rtc WebRTC offer failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Surface go2rtc's own error text (e.g. RTSP connect failure / codec mismatch).
        return Err(format!("go2rtc WebRTC offer HTTP {status}: {}", body.trim()));
    }
    let answer: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("go2rtc answer parse failed: {e} (body: {body})"))?;
    answer
        .get("sdp")
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string())
        .ok_or("go2rtc answer has no SDP".to_string())
}

/// Stop the WebRTC stream (kills the local go2rtc process). Idempotent.
#[tauri::command]
pub fn video_webrtc_stop(engine: State<'_, Go2Rtc>) -> Result<(), String> {
    engine.stop();
    Ok(())
}
