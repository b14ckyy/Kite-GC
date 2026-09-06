// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Physical location of the user (for Night-Mode "auto" sunset timing). It does NOT need to be
// precise — city-level is plenty. Sources, in order: a persisted last-known value (restored on
// launch), an OS/browser geo check (on start + a manual button), and a connected UAV's GPS fix.
// It deliberately never tracks the live map/camera — orbiting the globe must not change it.
//
// The OS check is per-platform: Windows (WebView2) and Linux (WebKitGTK) use the Web Geolocation
// API, while macOS goes through native CoreLocation (location_macos.rs) because WKWebView ships no
// Geolocation API at all, so `navigator.geolocation` can never resolve there.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { telemetry } from '$lib/stores/telemetry';
import { homePosition } from '$lib/stores/home';
import { connection } from '$lib/stores/connection';
import { settings } from '$lib/stores/settings';
import { isValidGpsCoordinate } from '$lib/helpers/telemetry';
import { isMobile, isMacOS } from '$lib/platform';

export interface LatLon { lat: number; lon: number; }

// Seed from the persisted value so Night-Mode auto is correct immediately on launch,
// before any fresh geo/UAV fix arrives.
export const userGeoLocation = writable<LatLon | null>(get(settings).userLocation ?? null);
/** Accuracy radius (m) of the last OS fix, or null (persisted/UAV sources carry none). Used by the GCS
 *  marker's on-select accuracy circle. */
export const userGeoAccuracyM = writable<number | null>(null);

const GPS_HDOP_MAX = 10; // coarse location only — any usable fix qualifies
const GEO_OPTS: PositionOptions = { enableHighAccuracy: false, timeout: 8000, maximumAge: 3_600_000 };

/** Update the live store and persist for the next session. */
export function setUserLocation(lat: number, lon: number, source: string, accuracyM: number | null = null): void {
  userGeoLocation.set({ lat, lon });
  userGeoAccuracyM.set(accuracyM);
  settings.patch({ userLocation: { lat, lon } });
  console.log(`[geo] user location set via ${source}: ${lat.toFixed(3)}, ${lon.toFixed(3)}`);
}

/** Mobile (iOS/Android): native CoreLocation/Android location via the Tauri plugin. The OS permission
 *  prompt is labelled with the app name + Info.plist usage string, not the WebView origin ("localhost"
 *  is all the Web geolocation prompt can show). Loaded lazily so the plugin JS never reaches the desktop
 *  bundle's startup path. */
async function runGeoCheckNative(): Promise<void> {
  try {
    const geo = await import('@tauri-apps/plugin-geolocation');
    let perm = await geo.checkPermissions();
    if (perm.location !== 'granted') perm = await geo.requestPermissions(['location']);
    if (perm.location !== 'granted') {
      console.warn('[geo] native location permission not granted:', perm.location);
      return;
    }
    // The plugin's PositionOptions has no optional fields, so pass an explicit literal (same values
    // as GEO_OPTS: coarse fix, 8 s timeout, up to a 1 h cached position).
    const pos = await geo.getCurrentPosition({ enableHighAccuracy: false, timeout: 8000, maximumAge: 3_600_000 });
    setUserLocation(
      pos.coords.latitude, pos.coords.longitude, 'os-geolocation-native',
      Number.isFinite(pos.coords.accuracy) ? pos.coords.accuracy : null,
    );
  } catch (err) {
    console.warn('[geo] native geolocation failed, keeping last known:', err);
  }
}

/** macOS: read the newest CoreLocation fix from the backend (location_macos.rs). WKWebView has no
 *  Geolocation API at all, so the Web path below can never resolve there. The native module is the
 *  only source, and it pushes an `os-location` event as well for the live updates. */
async function runGeoCheckMacOs(): Promise<void> {
  try {
    // Start first, and on every manual check. The backend gives up when Location Services are off
    // system-wide, so without this a user who turns them on (or grants the permission) after launch
    // would have to restart the app to get a marker. The call is idempotent once running.
    await invoke('location_os_start');
    const fix = await invoke<{ lat: number; lon: number; accuracy_m: number | null } | null>(
      'location_os_last',
    );
    if (fix) setUserLocation(fix.lat, fix.lon, 'os-corelocation', fix.accuracy_m);
    // No fix yet is normal, not an error: CoreLocation is still resolving, or the permission prompt
    // is unanswered. The event listener below delivers it whenever it arrives.
  } catch (err) {
    console.warn('[geo] CoreLocation read failed, keeping last known:', err);
  }
}

function runGeoCheck(): void {
  // On mobile the vehicle's own GPS is the primary source (see the telemetry subscription below); the OS
  // check is a best-effort head start before a UAV connects. Route it through the native plugin so the
  // permission dialog carries the app's name.
  if (isMobile) { void runGeoCheckNative(); return; }
  // macOS desktop (isMacOS excludes iOS): native CoreLocation, see runGeoCheckMacOs.
  if (isMacOS) { void runGeoCheckMacOs(); return; }
  if (typeof navigator === 'undefined' || !navigator.geolocation) {
    console.warn('[geo] navigator.geolocation unavailable');
    return;
  }
  navigator.geolocation.getCurrentPosition(
    (pos) => setUserLocation(
      pos.coords.latitude, pos.coords.longitude, 'os-geolocation',
      Number.isFinite(pos.coords.accuracy) ? pos.coords.accuracy : null,
    ),
    (err) => console.warn('[geo] geolocation failed, keeping last known:', err.message),
    GEO_OPTS,
  );
}

let autoChecked = false;
/** One automatic OS geo check per app session (idempotent — safe from every map mount). */
export function ensureUserLocation(): void {
  if (autoChecked) return;
  autoChecked = true;
  runGeoCheck();
}

/** Manual trigger (settings button) — always runs a fresh OS geo check. */
export function requestUserLocation(): void {
  runGeoCheck();
}

// ── macOS: live CoreLocation updates ──
// The native module emits on every fix that moved more than its jitter threshold, which is what makes
// `gcsMode: continuous` track on macOS, where there is no watchPosition to drive it. Subscribed at
// import; the backend only emits when it has an authorised, valid fix.
if (isMacOS) {
  void listen<{ lat: number; lon: number; accuracy_m: number | null }>('os-location', (e) => {
    setUserLocation(e.payload.lat, e.payload.lon, 'os-corelocation', e.payload.accuracy_m);
  });
}

// ── Auto-update from a connected UAV's GPS (coarse is fine) ──
// Capture one good fix per connection so we don't thrash localStorage every telemetry frame.
let uavFixCaptured = false;
connection.subscribe((c) => { if (c.status !== 'connected') uavFixCaptured = false; });
telemetry.subscribe((t) => {
  if (uavFixCaptured) return;
  const hdopOk = t.gpsHdop <= 0 || t.gpsHdop < GPS_HDOP_MAX; // 0 = unknown → accept
  if (t.fixType >= 3 && hdopOk && isValidGpsCoordinate(t.lat, t.lon)) {
    uavFixCaptured = true;
    setUserLocation(t.lat, t.lon, 'uav-gps');
  }
});

/**
 * Best estimate of where the user physically is (for sunset timing).
 * Priority: stored/last-known geo → home position → persisted map centre. Never the live camera.
 */
export function resolveUserLocation(): LatLon {
  const geo = get(userGeoLocation);
  if (geo) return geo;

  const h = get(homePosition);
  if (h?.set && isValidGpsCoordinate(h.lat, h.lon)) return { lat: h.lat, lon: h.lon };

  const [lat, lon] = get(settings).map.center;
  return { lat, lon };
}
