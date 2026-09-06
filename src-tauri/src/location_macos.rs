// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// macOS operator location via CoreLocation, for the GCS marker ("Your Location").
//
// Why this exists at all: macOS is the only desktop platform where the Web Geolocation API is
// unavailable, so `navigator.geolocation` (helpers/userLocation.ts) can never resolve there.
// WKWebView ships no Geolocation API: wry implements the permission prompt only for its Android
// backend, WebKit exposes no delegate for it, and tauri-plugin-geolocation's desktop backend is
// a stub returning a default position. Windows (WebView2) and Linux (WebKitGTK) both support the Web
// API, and iOS/Android go through the native plugin, which is why only macOS needs this.
//
// Implemented in-crate against the objc2 CoreLocation bindings, the same shape as the IOKit HID
// backend in hid/macos.rs: no Swift, no extra plugin. Coarse accuracy on purpose, because the marker
// needs to know which town you are in (userLocation.ts documents city-level as plenty), and asking
// for less precision keeps CoreLocation off the GPS/Wi-Fi-scan fast path.
//
// Threading: CLLocationManager must be created on a thread with a live run loop and delivers its
// delegate callbacks there, so construction is hopped onto the main thread via Tauri. The manager
// and delegate are parked in a process-global to stay alive for the app's lifetime. `delegate` is a
// WEAK property on CLLocationManager, so dropping our reference would silently stop the callbacks.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, AllocAnyThread, DefinedClass};
use objc2_core_location::{
    kCLLocationAccuracyKilometer, CLAuthorizationStatus, CLLocation, CLLocationManager,
    CLLocationManagerDelegate,
};
use objc2_foundation::{NSArray, NSError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Event name the frontend listens on for every fresh OS fix.
const EVENT: &str = "os-location";

/// Only re-emit once the position moved this far, mirroring the frontend's own anti-jitter gate
/// (`CONT_MIN_MOVE_M` in stores/gcsLocation.ts). Coarse fixes wander by tens of metres at rest.
const MIN_MOVE_M: f64 = 20.0;

/// A resolved operator position. Field names match what `helpers/userLocation.ts` expects.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct OsFix {
    pub lat: f64,
    pub lon: f64,
    /// Horizontal accuracy radius in metres, or None when CoreLocation reports it as invalid.
    pub accuracy_m: Option<f64>,
}

/// Last fix seen, so a frontend that starts (or reloads) after the fix arrived can still read it.
static LAST_FIX: Mutex<Option<OsFix>> = Mutex::new(None);

/// Keeps the manager + delegate alive for the process. See the threading note above: the delegate is
/// a weak property, so this is what stops the callbacks from going silent.
static KEEPALIVE: Mutex<Option<Keepalive>> = Mutex::new(None);

struct Keepalive {
    _manager: Retained<CLLocationManager>,
    _delegate: Retained<LocationDelegate>,
}

// The CoreLocation objects are `!Send`, but they are only ever touched on the main thread (created
// there, and the delegate callbacks arrive there). This wrapper exists purely to park them in a
// static; nothing here dereferences them from another thread.
unsafe impl Send for Keepalive {}

struct Ivars {
    /// Emitting goes through the AppHandle, which is Send + Sync and cheap to clone.
    app: AppHandle,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "KiteLocationDelegate"]
    #[ivars = Ivars]
    struct LocationDelegate;

    unsafe impl NSObjectProtocol for LocationDelegate {}

    unsafe impl CLLocationManagerDelegate for LocationDelegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update_locations(
            &self,
            _manager: &CLLocationManager,
            locations: &NSArray<CLLocation>,
        ) {
            // CoreLocation batches; the newest fix is last.
            let Some(loc) = locations.lastObject() else { return };
            let coord = unsafe { loc.coordinate() };
            if !unsafe { coord.is_valid() } {
                return;
            }
            let acc = unsafe { loc.horizontalAccuracy() };
            let fix = OsFix {
                lat: coord.latitude,
                lon: coord.longitude,
                // A negative horizontal accuracy means the coordinate is invalid per Apple's docs.
                accuracy_m: (acc >= 0.0).then_some(acc),
            };

            {
                let mut last = LAST_FIX.lock().unwrap();
                if let Some(prev) = *last {
                    if distance_m(prev.lat, prev.lon, fix.lat, fix.lon) < MIN_MOVE_M {
                        return; // unchanged within the jitter band: keep the event stream quiet
                    }
                }
                *last = Some(fix);
            }

            log::info!("[location] CoreLocation fix: {:.3}, {:.3}", fix.lat, fix.lon);
            let _ = self.ivars().app.emit(EVENT, fix);
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn did_fail(&self, _manager: &CLLocationManager, error: &NSError) {
            // Denied authorisation and "no fix yet" both land here. Warn rather than error: the GCS
            // marker is optional and every other feature works without it.
            log::warn!("[location] CoreLocation failed: {}", error.localizedDescription());
        }

        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn did_change_authorization(&self, manager: &CLLocationManager) {
            let status = unsafe { manager.authorizationStatus() };
            match status {
                CLAuthorizationStatus::AuthorizedAlways
                | CLAuthorizationStatus::AuthorizedWhenInUse => {
                    log::info!("[location] CoreLocation authorised, starting updates");
                    unsafe { manager.startUpdatingLocation() };
                }
                CLAuthorizationStatus::Denied | CLAuthorizationStatus::Restricted => {
                    log::warn!(
                        "[location] location access denied. The GCS marker falls back to the \
                         vehicle's GPS fix (System Settings > Privacy & Security > Location Services)"
                    );
                }
                // NotDetermined: the prompt is still on screen, this fires again once answered.
                _ => {}
            }
        }
    }
);

/// Great-circle distance in metres. Only used against the 20 m jitter gate, so the spherical
/// approximation is far more precision than the comparison needs.
fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// The most recent OS fix, if CoreLocation has produced one. Lets the frontend read a fix that
/// arrived before it subscribed (or before a WebView reload) instead of waiting for the next event.
#[tauri::command]
pub fn location_os_last() -> Option<OsFix> {
    *LAST_FIX.lock().unwrap()
}

/// Start (or retry starting) CoreLocation from the frontend, behind the "Detect my location" button.
///
/// Needed because `start()` gives up when Location Services are off system-wide, and app setup is the
/// only other caller: a user who enables Location Services or grants the permission after launch
/// would otherwise have to restart the app. `start()` is idempotent, so this is free once running.
#[tauri::command]
pub fn location_os_start(app: AppHandle) {
    start(&app);
}

/// Start CoreLocation and request authorisation. Idempotent: a second call is a no-op, so this is
/// safe to call from app setup and again from a manual "detect my location" button.
///
/// Not generic over the runtime: the delegate parks an `AppHandle` in its ivars to emit from, and
/// that has to be one concrete type to live in a `static`. The app builds on the default Wry runtime.
pub fn start(app: &AppHandle) {
    if KEEPALIVE.lock().unwrap().is_some() {
        return;
    }
    // Location Services can be off system-wide, in which case authorisation would never resolve.
    if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
        log::warn!("[location] Location Services are disabled system-wide. GCS marker will rely on \
                    the vehicle's GPS fix");
        return;
    }

    let handle = app.app_handle().clone();
    let hop = app.run_on_main_thread(move || {
        let mut slot = KEEPALIVE.lock().unwrap();
        if slot.is_some() {
            return; // raced with another start() while the hop was queued
        }
        let delegate = LocationDelegate::alloc().set_ivars(Ivars { app: handle });
        let delegate: Retained<LocationDelegate> = unsafe { objc2::msg_send![super(delegate), init] };

        let manager = unsafe { CLLocationManager::new() };
        unsafe {
            manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // Coarse is deliberate (see the module note): town-level is all the marker needs.
            manager.setDesiredAccuracy(kCLLocationAccuracyKilometer);
            manager.setDistanceFilter(MIN_MOVE_M);
            // Fires locationManagerDidChangeAuthorization once answered, which is where updates
            // actually start. Already-authorised apps get that callback immediately.
            manager.requestWhenInUseAuthorization();
        }
        *slot = Some(Keepalive { _manager: manager, _delegate: delegate });
        log::info!("[location] CoreLocation started, awaiting authorisation");
    });
    if let Err(e) = hop {
        log::warn!("[location] could not reach the main thread to start CoreLocation: {e}");
    }
}
