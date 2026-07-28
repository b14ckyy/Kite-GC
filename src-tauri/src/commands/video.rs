// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video commands — the go2rtc RTSP→WebRTC engine + its ffmpeg fallback dependency.
//! See docs/active/RTSP_VIDEO.md.
//!
//! **Threading:** every command here that spawns a helper process, waits on one, or tears one down is
//! marked `#[tauri::command(async)]`. Tauri runs plain `fn` commands on the **main thread**, so a
//! device enumeration behind a wedged capture driver (or a `--version` call on a binary Gatekeeper /
//! Defender is still scanning) would freeze the whole UI. Only trivially-cheap commands stay sync.

use tauri::{AppHandle, Emitter, State};

use std::sync::Arc;

use crate::video::mjpeg_server::{EndedHook, MjpegSource, RtspTranscode};
use crate::video::{ffmpeg, go2rtc, native, Go2Rtc};

/// Fixed go2rtc stream name for the single live feed.
const STREAM_NAME: &str = "kite";

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

/// How long the MJPEG endpoint gets to produce its first byte before the stream copy is judged
/// unusable. Generous — it covers spawning ffmpeg and the RTSP handshake — because it is paid once,
/// at connect, and never touches a frame afterwards.
const COPY_PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(4);

/// Register `src` under the fixed stream name. go2rtc patches its config in place, so this also
/// replaces an existing registration.
async fn register_source(client: &reqwest::Client, port: u16, src: &str) -> Result<(), String> {
    let resp = client
        .put(format!("http://127.0.0.1:{port}/api/streams"))
        .query(&[("name", STREAM_NAME), ("src", src)])
        .send()
        .await
        .map_err(|e| format!("go2rtc add-stream failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("go2rtc add-stream HTTP {}", resp.status()));
    }
    Ok(())
}

/// Whether go2rtc's MJPEG endpoint actually yields frames for the registered source.
///
/// It has to be a byte check, not a status check: asked for MJPEG from a stream that carries none,
/// go2rtc answers **HTTP 200 and then sends nothing** (measured; it logs `add consumer error=…`
/// internally). A status check would read that as success and leave the `<img>` on a silent, empty
/// response — which is exactly the failure the MJPEG fallback used to die of.
async fn mjpeg_endpoint_delivers(client: &reqwest::Client, port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/api/stream.mjpeg?src={STREAM_NAME}");
    let Ok(mut resp) = client.get(&url).send().await else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    matches!(
        tokio::time::timeout(COPY_PROBE_BUDGET, resp.chunk()).await,
        Ok(Ok(Some(chunk))) if !chunk.is_empty()
    )
}

/// ffmpeg version string (`ffmpeg -version` first line), or null if it isn't installed yet. ffmpeg is
/// the fallback RTSP reader for go2rtc (sources its native client can't read), not always required.
#[tauri::command(async)]
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
#[tauri::command(async)]
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
/// `mjpeg`: the feed will be consumed as MJPEG over HTTP (the fallback for WebViews without WebRTC).
/// A source that already carries MJPEG is stream-copied; anything else is transcoded — see below.
///
/// `use_ffmpeg`: register the source via go2rtc's bundled-ffmpeg reader instead of its native RTSP
/// client. The `input=rtsp/udp` template uses ffmpeg WITHOUT a forced `-rtsp_transport`, which is the
/// only mode that reads quirky servers (e.g. obs-rtspserver, which 461s any forced transport). Used
/// as the automatic fallback when the native client fails.
///
/// `allow_hw_decode`: caller veto for the hardware decoder on the MJPEG path (default: allowed). The
/// frontend turns it off after repeated failures, so a host that passes the probe but cannot actually
/// keep a live stream on the hardware decoder still ends up with a working picture.
#[tauri::command]
pub async fn video_webrtc_start(
    url: String,
    use_ffmpeg: bool,
    mjpeg: bool,
    allow_hw_decode: Option<bool>,
    engine: State<'_, Go2Rtc>,
) -> Result<String, String> {
    // Resolve the hardware probes FIRST and on a blocking thread: each runs short ffmpeg processes,
    // and `ensure_running` below writes the go2rtc config, which reads the same (cached) verdicts to
    // emit the transcode templates — doing it in the other order would run those ffmpeg spawns on the
    // async runtime's thread. They are resolved *here*, explicitly, rather than being left to a `||`
    // in the expression below, because that would decide the order by accident.
    // V4L2 M2M is the Pi-class path (decode only); VAAPI is the desktop-GPU one and does the whole
    // chain — see `video::ffmpeg` for why a half-hardware chain is deliberately not an option.
    let (hw_v4l2, hw_vaapi) = tauri::async_runtime::spawn_blocking(|| {
        let v4l2 = crate::video::ffmpeg::v4l2_h264_decode_available();
        // Only ask about VAAPI when V4L2 had no answer, mirroring the go2rtc config writer's own
        // `if/else if` — a board with a working SoC decoder never reaches the VAAPI template, so
        // probing it there is two ffmpeg spawns for a verdict nothing reads. On a Raspberry Pi that
        // is not merely wasted: `/dev/dri/renderD*` exists (the V3D render node) with no VAAPI driver
        // behind it, so the probe runs against hardware that cannot answer.
        let vaapi = !v4l2 && crate::video::ffmpeg::vaapi_render_node().is_some();
        (v4l2, vaapi)
    })
    .await
    .unwrap_or((false, false));
    let port = engine.ensure_running()?;
    let hw = mjpeg && allow_hw_decode.unwrap_or(true) && (hw_v4l2 || hw_vaapi);
    // V4L2 M2M has no MJPEG encoder, so a Pi keeps go2rtc's software one; VAAPI does both halves.
    // The two are mutually exclusive by construction (see the probe order above).
    let hw_mjpeg_encode = hw_vaapi;
    // Bounded: never let a wedged go2rtc freeze the frontend's reconnect loop.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // `mjpeg` = the consumer will be go2rtc's `/api/stream.mjpeg` endpoint, which can only serve a
    // stream that actually carries an MJPEG track. An ordinary H.264 camera does not — but a source
    // that already sends MJPEG does, and then transcoding it is decode+re-encode for nothing.
    // Measured against a UAV-Link in MJPEG passthrough, same source and same reader: `#video=copy`
    // costs 7.4 % of a core, `#video=mjpeg` costs 47.6 %. So try the copy first and keep it if the
    // endpoint actually delivers.
    //
    // The MJPEG path stays on the ffmpeg reader either way, and always takes the permissive
    // `input=rtsp/udp` template (ffmpeg with NO forced -rtsp_transport) rather than honouring the
    // transport choice: that is the only variant that also reads UDP-only servers, and a connection
    // left on "Auto" would otherwise fail to open one at all. go2rtc's own native client is not an
    // option even for a pure MJPEG source — it fails such servers at `SETUP` (measured), because the
    // problem there is the transport, not the codec.
    if mjpeg {
        let copy_src = format!("ffmpeg:{url}#input=rtsp/udp#video=copy");
        register_source(&client, port, &copy_src).await?;
        if mjpeg_endpoint_delivers(&client, port).await {
            log::info!("[video] source already carries MJPEG — stream-copied, no transcode");
            return Ok("copy".to_string());
        }
        log::debug!("[video] no MJPEG track in the source — registering the transcode instead");
    }
    let src = if mjpeg && hw {
        // Hardware transcode. The arguments live in NAMED templates written into the go2rtc config
        // (`kite_hw_input` / `kite_hw_mjpeg`, see `video::go2rtc`) — spelling them out inline is not an
        // option because go2rtc rejects any source containing a space with HTTP 400 ("source with
        // spaces may be insecure"), which used to make this branch fail the stream registration
        // outright on exactly the hardware it was written for. Everything else matches the software
        // template, so a stream that reads on one reads on the other.
        //
        // On Pi-class SoCs only the decode is hardware (no V4L2 M2M MJPEG encoder exists) and the
        // encoder stays go2rtc's `mjpeg`; with VAAPI both halves are, and the frames never leave the GPU.
        let enc = if hw_mjpeg_encode { "kite_hw_mjpeg" } else { "mjpeg" };
        format!("ffmpeg:{url}#input=kite_hw_input#video={enc}")
    } else if mjpeg {
        format!("ffmpeg:{url}#input=rtsp/udp#video=mjpeg")
    } else if use_ffmpeg {
        format!("ffmpeg:{url}#input=rtsp/udp#video=copy")
    } else {
        url.clone()
    };
    register_source(&client, port, &src).await?;
    // Report what was actually registered, so the UI states the running pipeline instead of the
    // host's capabilities. Without `mjpeg` go2rtc repackages the stream and nothing transcodes; the
    // `copy` case returned above never reaches here.
    Ok(match (mjpeg, hw, hw_mjpeg_encode) {
        (false, _, _) => "none",
        (true, true, true) => "vaapi",
        (true, true, false) => "v4l2m2m",
        (true, false, _) => "software",
    }
    .to_string())
}

/// Exchange a browser WebRTC SDP offer with go2rtc and return the SDP answer (proxied to avoid CORS).
#[tauri::command]
pub async fn video_webrtc_offer(sdp: String, engine: State<'_, Go2Rtc>) -> Result<String, String> {
    let port = engine
        .port()
        .ok_or("go2rtc is not running — start the stream first")?;
    // Bounded: go2rtc blocks this answer until the producer probes the source — on a wedged/dead
    // RTSP server that wait is unbounded and froze the frontend's reconnect loop. 15 s is enough
    // for any healthy source (probe is normally <2 s).
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
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

/// Stop the WebRTC stream (kills the local go2rtc process). Idempotent. Async: the graceful teardown
/// does a blocking DELETE + a settle delay before the kill (~1 s worst case).
#[tauri::command(async)]
pub fn video_webrtc_stop(engine: State<'_, Go2Rtc>) -> Result<(), String> {
    engine.stop();
    Ok(())
}

/// Return the go2rtc API port if the engine is running, or null.
/// Used by the frontend to construct HTTP fallback URLs (MJPEG, etc.)
/// when RTCPeerConnection is unavailable.
#[tauri::command]
pub fn video_go2rtc_port(engine: State<'_, Go2Rtc>) -> Option<u16> {
    engine.port()
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

/// Start the embedded MJPEG server on an RTSP source — the image path, **without go2rtc**.
///
/// go2rtc drives an `ffmpeg:` source by having ffmpeg publish back into it over RTSP/TCP, so a stream
/// that already carries MJPEG is packetised into RTP/JPEG (RFC 2435), reassembled and repacked as
/// HTTP multipart. Measured over the same 120 s against a UAV-Link: the source had **zero** arrival
/// gaps above 200 ms and go2rtc's output had **69**, each ~338 ms — a fixed buffer flushing, and the
/// cause of the freezes testers reported. Reading the source once and broadcasting `-f mpjpeg`
/// measures as clean as the source itself. RFC 2435 also only carries baseline JPEG at 4:2:0/4:2:2,
/// so this path additionally reaches MJPEG sources the republish rejected outright.
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
