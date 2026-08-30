// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! "A link is active" — the one thing the host OS is told about the connection state.
//!
//! What the OS does with it is platform-owned and lives here, so the connection code calls one
//! function and carries no target cfg:
//!
//! - **Android**: keep the display on while a link is up, back to the normal screen timeout once
//!   it is down — nav-app behaviour. A ground station is watched, not touched: minutes pass between
//!   interactions, and the screen blanking mid-flight is exactly when the telemetry matters most. It
//!   is the window flag (`FLAG_KEEP_SCREEN_ON`), scoped to the app's own window, released the moment
//!   the app leaves the foreground — no WAKE_LOCK permission, nothing kept awake in the background.
//! - **iOS**: the same idea is `UIApplication.isIdleTimerDisabled`. Not wired yet — UIKit is
//!   main-thread-only and `objc2-ui-kit` is not in the dependency set; it belongs to the iOS pass
//!   with hardware to test on. Until then the call is a no-op there.
//! - **Desktop**: nothing — the OS screensaver is the user's business, and a GCS on a laptop is not
//!   the only thing on that screen.
//!
//! "Active" means a transport is open, whether or not bytes arrive: an idle link is still the link
//! the operator is waiting on. Called on every connect, on the user's disconnect, and on a lost link.

pub fn link_active(active: bool) {
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::android::screen::keep_on(active) {
            log::warn!("[screen] keep-on({active}) failed: {e}");
        }
    }
    #[cfg(target_os = "ios")]
    {
        log::debug!("[screen] link active = {active} (idle-timer control not wired on iOS yet)");
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = active;
    }
}
