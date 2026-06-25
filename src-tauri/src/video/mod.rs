// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Video subsystem (backend). v1 was frontend-only (webcam/USB via getUserMedia); this adds the
//! "native backend source": an RTSP → fragmented-MP4 loopback bridge fed by an on-demand ffmpeg.
//! See docs/active/RTSP_VIDEO.md.

pub mod ffmpeg;
pub mod go2rtc;
pub mod rtsp;

pub use go2rtc::Go2Rtc;
pub use rtsp::VideoBridge;
