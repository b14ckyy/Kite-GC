// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! What the host OS is told about the link: that one is active (Android foreground service) and whether
//! the display stays awake (`wants_awake`, every platform). The raw-recording rule exists for iOS but
//! deliberately applies to Android too — do not drop it there as redundant.
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
    /// This link has delivered a status at all — a passive feed only does once a frame with the arming
    /// state arrived, and a feed without such a frame never does.
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

/// Change the inputs and tell the OS when the outcome changed, or always with `force` (a fresh window
/// does not carry the old one's flag). Told under the lock so updates cannot arrive out of order;
/// no platform call takes this lock or blocks.
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

/// Desktop: one thread owns the inhibitor for the life of the process. Windows binds the request to the
/// thread that made it (`SetThreadExecutionState`), and callers here come and go with their links.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn keep_awake(on: bool) {
    use std::sync::{mpsc, OnceLock};
    static KEEPER: OnceLock<Option<mpsc::Sender<bool>>> = OnceLock::new();
    let keeper = KEEPER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<bool>();
        let spawned = std::thread::Builder::new().name("keep-awake".into()).spawn(move || {
            let request = |idle: bool| {
                keepawake::Builder::default()
                    .display(true)
                    .idle(idle)
                    .reason("Flight monitoring")
                    .app_name("Kite Ground Control")
                    .app_reverse_domain("com.kitegc.app")
                    .create()
            };
            let mut held: Option<keepawake::KeepAwake> = None;
            for on in rx {
                if !on {
                    // The crate's Linux release unwraps its D-Bus reply; a panic here would end the thread.
                    let old = held.take();
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(old))).is_err() {
                        log::warn!("[screen] releasing keep awake failed");
                    }
                } else if held.is_none() {
                    // Linux takes the display and the idle inhibit from two services and gives up both
                    // when one is missing — the display alone is what the rule is about.
                    held = request(true)
                        .or_else(|_| request(false))
                        .map_err(|e| log::warn!("[screen] keep awake failed: {e}"))
                        .ok();
                }
                log::info!("[screen] keep awake: {}", held.is_some());
            }
        });
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::warn!("[screen] no keep-awake thread: {e}");
                None
            }
        }
    });
    if let Some(tx) = keeper {
        if tx.send(on).is_err() {
            log::warn!("[screen] keep-awake thread gone");
        }
    }
}

/// Settings → "Stay awake without connection". The frontend sends it at startup and on every
/// change — the startup call also re-applies the current outcome to a window that was created
/// while the process lived on.
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
