// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video subsystem (backend). v1 was frontend-only (webcam/USB via getUserMedia); this adds the
//! "native backend source": live RTSP via the MediaMTX engine (RTSP→WebRTC), with ffmpeg as the
//! fallback reader for sources its native client can't handle. See docs/active/RTSP_VIDEO.md.
//!
//! Linux V4L2 capture devices (HDMI dongles, etc.) that aren't exposed via getUserMedia are
//! enumerated and ingested through the ffmpeg→MJPEG pipeline (`mjpeg_server`).

pub mod ffmpeg;
pub mod mediamtx;
pub mod native;
/// V4L2 device enumeration is Linux-only (reads `/sys/class/video4linux`); `native` uses it there.
#[cfg(target_os = "linux")]
pub mod v4l2;
pub mod mjpeg_server;
pub mod rtsp;
pub mod rtsp_native;
/// Windows H264/HEVC decode + render sink for the hole-punch surface (MOBILE_RTSP.md P2.1).
#[cfg(target_os = "windows")]
pub mod win_sink;
/// DEV-only P2.1 hole-punch spike (see MOBILE_RTSP.md) — removed once the native sink lands.
#[cfg(all(target_os = "windows", debug_assertions))]
pub mod holepunch;

pub use mediamtx::MediaMtx;
pub use mjpeg_server::MjpegServer;
