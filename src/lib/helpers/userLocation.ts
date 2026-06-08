// Physical location of the user (for Night-Mode "auto" sunset timing). It does NOT need to be
// precise — city-level is plenty. Sources, in order: a persisted last-known value (restored on
// launch), an OS/browser geo check (on start + a manual button), and a connected UAV's GPS fix.
// It deliberately never tracks the live map/camera — orbiting the globe must not change it.

import { writable, get } from 'svelte/store';
import { telemetry } from '$lib/stores/telemetry';
import { homePosition } from '$lib/stores/home';
import { connection } from '$lib/stores/connection';
import { settings } from '$lib/stores/settings';
import { isValidGpsCoordinate } from '$lib/helpers/telemetry';

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

function runGeoCheck(): void {
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
