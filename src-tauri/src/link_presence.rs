// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! "A link is active" — the one thing the host OS is told about the connection state.
//!
//! Android: keep the display on (window flag, no WAKE_LOCK) and run the foreground service with
//! the notification for the life of the link (Dev-Docs active/BACKGROUND_TELEMETRY.md). Screen-off
//! is not held open — a Doze-dropped link is an ordinary lost link. iOS: out of scope, no-op.
//! Desktop: nothing.
//!
//! "Active" = a transport is open, bytes or not. `link_up` on every connect, `link_down` on the
//! user's disconnect and on a lost link.

use crate::msp::types::FcInfo;

/// A link came up on `protocol` ("MSP" / "MAVLink" / "Telemetry"); the transport label was
/// registered by `connect()` through `link_status::set_transport`.
pub fn link_up(fc: &FcInfo, protocol: &str) {
    crate::link_status::on_link_up(fc, protocol);
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::android::screen::keep_on(true) {
            log::warn!("[screen] keep-on(true) failed: {e}");
        }
        if let Err(e) = crate::android::link_service::start() {
            log::warn!("[link-service] start failed: {e}");
        }
    }
    #[cfg(target_os = "ios")]
    {
        log::debug!("[screen] link up on {protocol} (not wired on iOS)");
    }
}

/// The link is gone — user disconnect or lost.
pub fn link_down() {
    crate::link_status::on_link_down();
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::android::link_service::stop() {
            log::warn!("[link-service] stop failed: {e}");
        }
        if let Err(e) = crate::android::screen::keep_on(false) {
            log::warn!("[screen] keep-on(false) failed: {e}");
        }
    }
    #[cfg(target_os = "ios")]
    {
        log::debug!("[screen] link down (not wired on iOS)");
    }
}
