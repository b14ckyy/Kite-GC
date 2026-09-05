// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! "A link is active" — the one thing the host OS is told about the connection state.
//!
//! What the OS does with it is platform-owned and lives here, so the connection code calls one
//! function and carries no target cfg:
//!
//! - **Android**: two things (Dev-Docs active/BACKGROUND_TELEMETRY.md). The display stays on
//!   while a link is up (`FLAG_KEEP_SCREEN_ON`, scoped to the app's window, released when the
//!   app leaves the foreground — no WAKE_LOCK). And a **foreground service** with a persistent
//!   notification runs for the life of the link, so minimising the app or switching to another
//!   one does not let Android freeze or reap the process — telemetry, the flight recorder and the
//!   track keep going. The notification carries the vehicle, battery, flight mode and distance to
//!   home, fed from `link_status` at 1 Hz. Screen-off is deliberately NOT held open: if Doze drops
//!   the link, that is an ordinary lost link.
//! - **iOS**: out of scope (the idle-timer equivalent is `UIApplication.isIdleTimerDisabled`, the
//!   background-BLE equivalent `UIBackgroundModes: bluetooth-central`; both belong to an iOS pass
//!   with hardware, if anyone takes it). No-op.
//! - **Desktop**: nothing — the OS screensaver is the user's business, and a GCS on a laptop is not
//!   the only thing on that screen.
//!
//! "Active" means a transport is open, whether or not bytes arrive: an idle link is still the link
//! the operator is waiting on. `link_up` on every connect, `link_down` on the user's disconnect and
//! on a lost link.

use crate::msp::types::FcInfo;

/// A link came up on `protocol` ("MSP" / "MAVLink" / "Telemetry"); the transport was registered
/// by `connect()` through `link_status::set_transport`.
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
        log::debug!("[screen] link up on {protocol} (idle-timer control not wired on iOS)");
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
        log::debug!("[screen] link down (idle-timer control not wired on iOS)");
    }
}
