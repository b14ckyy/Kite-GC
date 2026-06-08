// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Ground-station (GCS) location — the marker on the map + the radar / FormationFlight reference point.
//
// It is NOT a second location detector: the OS detection lives in `userGeoLocation` ("Your Location",
// shared with Night-Mode). The GCS location is just a VIEW of that, per mode:
//  - off:        no marker, no reference.
//  - manual:     the resolved OS location, which the user may override by dragging / "set GCS here";
//                "Reset" clears the override and snaps back to the OS location (no re-detect).
//  - continuous: follows the OS location API live; the marker only moves on a > 20 m change (anti-jitter).

import { writable, get } from 'svelte/store';
import { settings, type GcsMode } from '$lib/stores/settings';
import { userGeoLocation, userGeoAccuracyM, type LatLon } from '$lib/helpers/userLocation';
import { haversineDistance } from '$lib/utils/geo';

/** Current GCS position, or null (mode off / not yet resolved). */
export const gcsLocation = writable<LatLon | null>(null);
/** Accuracy radius (m) — shown as a circle only while the marker is selected. */
export const gcsAccuracyM = writable<number | null>(null);
/** True while a manual override is active (enables the Reset button). */
export const gcsManuallySet = writable(false);

const GEO_OPTS: PositionOptions = { enableHighAccuracy: true, timeout: 10_000, maximumAge: 0 };
const CONT_MIN_MOVE_M = 20; // continuous: ignore sub-20 m jitter

let manualOverride: LatLon | null = null; // session-only hand placement (drag / "set GCS here")
let watchId: number | null = null;

function clearWatch() {
  if (watchId != null && typeof navigator !== 'undefined') navigator.geolocation.clearWatch(watchId);
  watchId = null;
}

/** Recompute the GCS position for off / manual (continuous is driven by the watch). */
function recompute() {
  const mode = get(settings).gcsMode;
  if (mode === 'off') {
    gcsLocation.set(null);
    gcsAccuracyM.set(null);
  } else if (mode === 'manual') {
    if (manualOverride) {
      gcsLocation.set(manualOverride);
      gcsAccuracyM.set(null); // a hand-placed point has no measured accuracy
    } else {
      gcsLocation.set(get(userGeoLocation));
      gcsAccuracyM.set(get(userGeoAccuracyM));
    }
  }
}

function startContinuous() {
  clearWatch();
  if (typeof navigator === 'undefined' || !navigator.geolocation) return;
  watchId = navigator.geolocation.watchPosition(
    (pos) => {
      const next = { lat: pos.coords.latitude, lon: pos.coords.longitude };
      const cur = get(gcsLocation);
      if (!cur || haversineDistance(cur.lat, cur.lon, next.lat, next.lon) > CONT_MIN_MOVE_M) {
        gcsLocation.set(next);
      }
      gcsAccuracyM.set(Number.isFinite(pos.coords.accuracy) ? pos.coords.accuracy : null);
    },
    (err) => console.warn('[gcs] watch failed:', err.message),
    GEO_OPTS,
  );
}

function applyGcsMode(mode: GcsMode) {
  clearWatch();
  if (mode === 'continuous') startContinuous();
  else recompute();
}

/** Manual placement (drag end / "Set GCS here") — overrides the OS location until Reset. */
export function setGcsManual(lat: number, lon: number) {
  manualOverride = { lat, lon };
  gcsManuallySet.set(true);
  if (get(settings).gcsMode === 'manual') {
    gcsLocation.set(manualOverride);
    gcsAccuracyM.set(null);
  }
}

/** Reset the manual placement → snap back to the OS-resolved location ("Your Location"). No re-detect. */
export function resetGcsManual() {
  manualOverride = null;
  gcsManuallySet.set(false);
  recompute();
}

// React to the mode (and apply it once on first import).
let lastMode: GcsMode | null = null;
settings.subscribe((s) => {
  if (s.gcsMode !== lastMode) {
    lastMode = s.gcsMode;
    applyGcsMode(s.gcsMode);
  }
});
// In manual mode (no override), the GCS follows the resolved OS location + its accuracy.
userGeoLocation.subscribe(() => recompute());
userGeoAccuracyM.subscribe(() => recompute());
