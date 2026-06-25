// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video subsystem (backend). v1 was frontend-only (webcam/USB via getUserMedia); this adds the
//! "native backend source": live RTSP via the go2rtc engine (RTSP→WebRTC), with ffmpeg as go2rtc's
//! fallback RTSP reader for sources its native client can't handle. See docs/active/RTSP_VIDEO.md.

pub mod ffmpeg;
pub mod go2rtc;

pub use go2rtc::Go2Rtc;
