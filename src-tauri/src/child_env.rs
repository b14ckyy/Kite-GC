// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Keep the AppImage's private library environment out of the processes we spawn.
//!
//! An AppImage runs with `LD_LIBRARY_PATH` pointed at its own bundled libraries, and **every child
//! process inherits that**. A system binary started from inside then loads the AppImage's (older,
//! built-on-Ubuntu) libraries instead of its own, and dies before doing any work. Measured on a
//! Raspberry Pi 5 with the shipped AppImage:
//!
//! ```text
//! /usr/bin/ffmpeg: symbol lookup error: /lib/aarch64-linux-gnu/libavutil.so.59:
//!                  undefined symbol: vaMapBuffer2
//! ```
//!
//! To Kite that looks exactly like "ffmpeg is not installed" — and since the MJPEG fallback is the
//! only RTSP path on a WebView without WebRTC, video stops working entirely. It also detonates at a
//! distance: go2rtc is our child, so the ffmpeg *it* starts inherits the same broken environment.
//! The trigger is a host library update (the symbol above appeared when newer `libva` arrived), so
//! an AppImage that worked yesterday can break without anything in Kite changing.
//!
//! Our helpers (`ffmpeg`, `go2rtc`, `blackbox_decode`) and the system tools we call (`tar`, `ps`)
//! are all self-contained or system-linked: none of them wants the AppImage's libraries. Stripping
//! the loader variables is therefore always the right call — outside an AppImage it is a no-op.

/// Remove the AppImage loader environment from `cmd`, restoring the pre-AppImage values that
/// linuxdeploy's AppRun saves in `*_ORIG` when they exist. No-op unless we run from an AppImage.
pub fn sanitize(cmd: &mut std::process::Command) {
    if std::env::var_os("APPDIR").is_none() {
        return;
    }
    for (var, orig) in [
        ("LD_LIBRARY_PATH", "LD_LIBRARY_PATH_ORIG"),
        ("LD_PRELOAD", "LD_PRELOAD_ORIG"),
    ] {
        match std::env::var_os(orig) {
            Some(v) if !v.is_empty() => cmd.env(var, v),
            _ => cmd.env_remove(var),
        };
    }
}
