// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! What the host OS is told about the link: that one is active, and whether the display has to
//! stay awake for it.
//!
//! Android: run the foreground service with the notification for the life of the link (Dev-Docs
//! active/BACKGROUND_TELEMETRY.md). Screen-off is not held open — a Doze-dropped link is an
//! ordinary lost link. Desktop: nothing.
//!
//! Android + iOS: keep the display awake while there is something to watch (`wants_awake`) — the
//! vehicle is armed, or the link cannot tell (a telemetry feed without an arming flag), or the
//! continuous raw recording runs, or the user asked for it in the settings. A connected but
//! disarmed vehicle leaves the display to the OS timeout. Android: the window flag
//! FLAG_KEEP_SCREEN_ON (no WAKE_LOCK); iOS: `UIApplication.idleTimerDisabled`. Both keep the display
//! on AND undimmed — neither system offers "on, but may dim" — and both end with the foreground.
//! Neither switches a display back ON: a screen that went dark while disarmed stays dark at arming.
//!
//! The raw-recording rule exists for iOS, which cannot record with the display off (Android can,
//! through its foreground service); it applies to both so that a phone and an iPad behave alike
//! (Marc, 2026-11-15) — do not drop it on Android as redundant.
//!
//! "Active" = a transport is open, bytes or not. `link_up` on every connect; `link_down` on the
//! user's disconnect of the last link and on a lost MSP link (a lost MAVLink link arrives through
//! the frontend's disconnect).
//!
//! GATE(fleet): ONE state for all links. With several vehicles the last status wins and a second
//! connect replaces the first link's `raw_always`; per-link state needs the link id in
//! `link_status::on_status` (multi-vehicle work).

use std::sync::Mutex;

use crate::msp::types::FcInfo;

/// What decides whether the display stays awake.
#[derive(Clone, Copy, Default)]
struct Awake {
    /// A transport is open.
    link: bool,
    /// The last status of this link said "armed".
    armed: bool,
    /// This link has delivered a status at all. A passive telemetry feed only produces one once the
    /// frame that carries the arming state has arrived (CRSF flight mode, FrSky modes sensor, LTM
    /// S-frame, ArduPilot passthrough 0x5001) — a feed without that frame never does.
    status_seen: bool,
    /// The link records the raw stream continuously (not per flight).
    raw_always: bool,
    /// Settings → "Stay awake without connection".
    always: bool,
}

/// The display stays awake: always when the user asked for it; with a link while the vehicle is
/// armed, while the arming state is unknown, or while the continuous raw recording runs.
fn wants_awake(a: &Awake) -> bool {
    a.always || (a.link && (a.armed || !a.status_seen || a.raw_always))
}

static AWAKE: Mutex<(Awake, bool)> = Mutex::new((
    Awake { link: false, armed: false, status_seen: false, raw_always: false, always: false },
    false,
));

/// Change the inputs and tell the OS when the outcome changed — or in any case with `force` (a
/// fresh window does not carry the flag of the one before it). The OS is told while the lock is
/// held, so two updates cannot reach it in the wrong order; neither platform call takes this lock
/// or blocks (both post to the main thread).
fn update(force: bool, change: impl FnOnce(&mut Awake)) {
    let mut g = AWAKE.lock().unwrap_or_else(|p| p.into_inner());
    change(&mut g.0);
    let on = wants_awake(&g.0);
    if on == g.1 && !force {
        return;
    }
    g.1 = on;
    keep_awake(on);
}

#[cfg(target_os = "android")]
fn keep_awake(on: bool) {
    log::info!("[screen] keep awake: {on}");
    if let Err(e) = crate::android::screen::keep_on(on) {
        log::warn!("[screen] keep-on({on}) failed: {e}");
    }
}

#[cfg(target_os = "ios")]
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// `UIApplication.shared.isIdleTimerDisabled` — UIKit state, main thread only.
#[cfg(target_os = "ios")]
fn keep_awake(on: bool) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    let Some(app) = APP.get() else {
        log::warn!("[screen] keep awake({on}) before the app handle is known");
        return;
    };
    log::info!("[screen] keep awake: {on}");
    let hop = app.run_on_main_thread(move || unsafe {
        let shared: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if !shared.is_null() {
            let _: () = msg_send![shared, setIdleTimerDisabled: on];
        }
    });
    if let Err(e) = hop {
        log::warn!("[screen] keep awake({on}) failed: {e}");
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn keep_awake(_on: bool) {}

/// Settings → "Stay awake without connection". The frontend sends it at startup and on every
/// change — the startup call also re-applies the current outcome to a window that was created
/// while the process lived on. Desktop builds accept it and do nothing.
#[tauri::command]
pub fn set_stay_awake(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    let _ = APP.set(app);
    #[cfg(not(target_os = "ios"))]
    let _ = app;
    update(true, |a| a.always = enabled);
    Ok(())
}

/// A link came up on `protocol` ("MSP" / "MAVLink" / "Telemetry"); the transport label was
/// registered by `connect()` through `link_status::set_transport`. `raw_always`: this link records
/// the raw stream continuously.
pub fn link_up(fc: &FcInfo, protocol: &str, raw_always: bool) {
    crate::link_status::on_link_up(fc, protocol);
    update(false, |a| {
        a.link = true;
        a.armed = false;
        a.status_seen = false;
        a.raw_always = raw_always;
    });
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::android::link_service::start() {
            log::warn!("[link-service] start failed: {e}");
        }
    }
}

/// A link delivered a status (called by `link_status::on_status`, at telemetry rate: one lock and
/// a compare unless the outcome flips).
pub fn on_status(armed: bool) {
    update(false, |a| {
        a.armed = armed;
        a.status_seen = true;
    });
}

/// The link is gone — user disconnect or lost.
pub fn link_down() {
    crate::link_status::on_link_down();
    #[cfg(target_os = "android")]
    {
        if let Err(e) = crate::android::link_service::stop() {
            log::warn!("[link-service] stop failed: {e}");
        }
    }
    update(false, |a| {
        a.link = false;
        a.armed = false;
        a.status_seen = false;
        a.raw_always = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_only_when_there_is_something_to_watch() {
        let none = Awake::default();
        assert!(!wants_awake(&none));
        // The setting alone, without a link.
        assert!(wants_awake(&Awake { always: true, ..none }));
        // A fresh link has not said yet whether the vehicle is armed.
        let link = Awake { link: true, ..none };
        assert!(wants_awake(&link));
        // Connected and disarmed: the OS timeout applies.
        let disarmed = Awake { status_seen: true, ..link };
        assert!(!wants_awake(&disarmed));
        assert!(wants_awake(&Awake { armed: true, ..disarmed }));
        assert!(wants_awake(&Awake { raw_always: true, ..disarmed }));
        // Nothing of a link counts without the link.
        assert!(!wants_awake(&Awake { link: false, armed: true, raw_always: true, ..disarmed }));
    }
}
