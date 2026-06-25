# RTSP Video Input — Feature Plan

> STATUS: active · 2026-06-26. **Shipped (local, OBS-verified):** live RTSP video via go2rtc → WebRTC.
> Awaiting a real-source test (DJI). The original ffmpeg→fMP4→`<video>` approach was abandoned (see
> "Why WebRTC"); the discovery/download infra it introduced is reused for go2rtc.

## Goal
Display a live **RTSP** video feed (FPV / DJI PC-capture / IP camera / any RTSP source) inside Kite's
existing video surfaces (panel, floating window, widget, map-swap, native PiP), at low latency.

## Why WebRTC (and not ffmpeg → fragmented-MP4)
The first approach piped ffmpeg's fMP4 over a loopback HTTP server into a `<video>`. It **works but
can't do low-latency live**, and we burned through every variant proving it:
- **Progressive `<video src>`** buffers and drifts — latency grows unbounded; seeking to the live edge
  freezes it (the loopback has no HTTP Range support).
- **MSE** (manual `SourceBuffer` + back-buffer eviction + live-edge seek) bounded latency but stuttered,
  and sharing one decode across sinks needs `captureStream()` from a hidden `<video>`, which the webview
  throttles → jitter.
- Multiple `<video>` sinks each opened their own loopback connection, and the one-ffmpeg-per-connection
  bridge killed the previous feed on each new connection → constant restart.

The browser is simply the wrong place to re-implement a live player. **WebRTC** is the right tool: the
webview plays it **natively and hardware-accelerated**, and `RTCPeerConnection` yields a real
`MediaStream` — which drops straight into the existing camera sink path (`srcObject`, shared across all
sinks, one decode). Measured **~200 ms** end-to-end vs the OBS preview with a sane encoder.

## Architecture (shipped)
**go2rtc** (AlexxIT/go2rtc) is the RTSP→WebRTC engine — a single Go binary, discovered/downloaded
exactly like `blackbox_decode`/ffmpeg (`flightlog::decoder::install_dir`, GitHub-release zip).

- **`src-tauri/src/video/go2rtc.rs`** — discovery (`find_go2rtc`) · `status()` · `download()` (Windows
  `go2rtc_win64.zip`) · `Go2Rtc` managed state running **one** local instance bound to `127.0.0.1` with
  **ephemeral free ports** for the API, the internal RTSP server, **and WebRTC** (a busy default WebRTC
  port 8555 makes pion's UDP mux nil → process-killing panic, go2rtc #1851/#1855). Config is written as
  JSON (valid YAML) and also points go2rtc at our bundled ffmpeg (`ffmpeg.bin`) for the fallback.
- **Commands** (`commands/video.rs`): `video_go2rtc_status` / `video_go2rtc_download`
  (`go2rtc-download-progress` event) · `video_webrtc_start(url, transport, use_ffmpeg)` (ensures go2rtc,
  `PUT /api/streams`) · `video_webrtc_offer(sdp)` (proxies the WHEP-style `POST /api/webrtc?src=` SDP
  exchange — **Rust-side to avoid CORS**) · `video_webrtc_stop` (kills the process).
- **Frontend** (`stores/video.ts`): `startRtsp()` runs a browser `RTCPeerConnection` (recvonly video),
  proxies the SDP offer/answer through Rust, and sets the resulting `MediaStream` as the shared
  `videoStream`. All four sinks already bind `srcObject` → no fMP4/MSE/captureStream anywhere.

### Native-first with automatic ffmpeg fallback
go2rtc's native RTSP client is tried first (lowest overhead — the normal path for real cameras/DJI). If
it fails, we automatically retry the source as `ffmpeg:<url>#input=rtsp/udp#video=copy` — go2rtc's
`rtsp/udp` template runs ffmpeg **without a forced `-rtsp_transport`**, the only mode that reads quirky
servers like **obs-rtspserver** (which `461`s any forced transport — TCP *and* UDP — and only yields to
ffmpeg's UDP→TCP auto-retry dance). The panel shows which reader is live (`video.via.native|ffmpeg`).

### Source codec note (jitter)
WebRTC tolerates **B-frames** poorly → "frames out of order" / jitter. Real FPV/DJI/IP-camera streams
are Baseline (no B-frames) and play smoothly; OBS must be set to **B-frames = 0** (+ baseline/main,
ultra-low-latency) to behave. This is an encoder property of the source, not something we can fix in the
pipeline for a copy stream.

## Source router
`stores/video.ts` exposes `{ kind: 'camera'|'rtsp', … }`; the camera path is `getUserMedia`, the rtsp
path is go2rtc/WebRTC. Both end as one shared `videoStream`, so panel/floating/widget/map-swap/PiP are
unchanged. The RTSP URL persists (the transport selector was removed — go2rtc/ffmpeg negotiate it).

## Done since the first cut
- **Cleanup:** removed the dead ffmpeg→fMP4 loopback bridge (`video/rtsp.rs`, `VideoBridge`,
  `video_rtsp_start/stop`) and the now-irrelevant transport dropdown. `video_ffmpeg_status/download`
  are kept — repurposed as the fallback-reader dependency.
- **ffmpeg fallback dependency:** go2rtc's config now always points `ffmpeg.bin` at the resolved path
  *or the path the guided download will write to*, so a later download is picked up without restarting
  go2rtc. The Video panel checks ffmpeg and offers a non-blocking download (separate from the required
  go2rtc download) when it's missing.

## Open / follow-ups
- **Native-source test with DJI / a real IP camera** (the native go2rtc path; OBS only exercised the
  ffmpeg fallback).
- **Robustness:** auto-reconnect on drop, error/spinner states, HEVC sources, audio.
- **Licensing/size note in BUILD.md** for the bundled go2rtc (+ ffmpeg) binaries.

## Risks
- WebRTC support: solid in WebView2 (Windows). WebKitGTK (Linux) uses a GStreamer WebRTC backend —
  verify when Linux is in scope.
- go2rtc/pion stability: pin ephemeral ports (done) to avoid the UDP-mux panic.
