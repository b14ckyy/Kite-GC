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
import { invoke } from '@tauri-apps/api/core';
import { settings, type GcsMode } from '$lib/stores/settings';
import { userGeoLocation, userGeoAccuracyM, requestUserLocation, type LatLon } from '$lib/helpers/userLocation';
import { videoState } from '$lib/stores/video';
import { haversineDistance } from '$lib/utils/geo';
import { isAndroid } from '$lib/platform';

/** Current GCS position, or null (mode off / not yet resolved). */
export const gcsLocation = writable<LatLon | null>(null);
/** Accuracy radius (m) — shown as a circle only while the marker is selected. */
export const gcsAccuracyM = writable<number | null>(null);
/** True while a manual override is active (enables the Reset button). */
export const gcsManuallySet = writable(false);
/** True while continuous updates are paused for a running RTSP stream over Wi-Fi (Android)
 *  — the map hides the marker's "live" pulse dot then, since nothing is live. */
export const gcsWatchPaused = writable(false);

const GEO_OPTS: PositionOptions = { enableHighAccuracy: true, timeout: 10_000, maximumAge: 0 };
const CONT_MIN_MOVE_M = 20; // continuous: ignore sub-20 m jitter

let manualOverride: LatLon | null = null; // session-only hand placement (drag / "set GCS here")
let watchId: number | null = null;

function clearWatch() {
  if (watchId != null && typeof navigator !== 'undefined') navigator.geolocation.clearWatch(watchId);
  watchId = null;
}

/** Recompute the GCS position for off / manual (continuous is driven by the watch — except
 *  while paused for video, where the marker falls back to the one-shot OS fix if the watch
 *  never delivered, e.g. an RTSP stream auto-starting with the app). */
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
  } else if (mode === 'continuous' && pausedForVideo && !get(gcsLocation)) {
    // Paused before the watch ever produced a fix: show the session's one-shot OS location
    // so the marker exists at all. A position the watch DID deliver stays frozen instead.
    gcsLocation.set(get(userGeoLocation));
    gcsAccuracyM.set(get(userGeoAccuracyM));
  }
}

/** True while continuous updates are paused for a running RTSP stream over Wi-Fi (Android):
 *  each fused-location fix triggers a Wi-Fi scan (~every 10 s), every scan takes the radio
 *  off-channel, and the resulting RTP loss bursts corrupt the video until the next keyframe
 *  (measured on the Teclast M11). The marker freezes at its last position and the watch
 *  resumes the moment the stream stops. Streams over cellular/Ethernet are unaffected. */
let pausedForVideo = false;

function setPausedForVideo(on: boolean) {
  if (pausedForVideo === on) return;
  pausedForVideo = on;
  gcsWatchPaused.set(on);
  console.log(`[gcs] continuous location updates ${on ? 'paused (RTSP over Wi-Fi)' : 'resumed'}`);
  // Entering the pause with no location at all (stream auto-started with the app): run the
  // usual ONE-SHOT OS check — a single fix costs a single Wi-Fi scan blip, and without it
  // the marker would be missing for the whole stream.
  if (on && !get(userGeoLocation)) requestUserLocation();
  applyGcsMode(get(settings).gcsMode);
}

function startContinuous() {
  clearWatch();
  if (pausedForVideo) return; // frozen at the last position — see setPausedForVideo
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
  if (mode === 'continuous' && !pausedForVideo) startContinuous();
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

// Android: pause the continuous watch while an RTSP stream runs over Wi-Fi (see
// setPausedForVideo). The transport is checked once per stream start; a stream over
// cellular (or Ethernet) keeps live updates. An unknown route (some VPNs) counts as
// Wi-Fi — pausing needlessly is the safer error.
if (isAndroid) {
  let streamActive = false;
  videoState.subscribe((v) => {
    const active = v.enabled && v.kind === 'rtsp';
    if (active === streamActive) return;
    streamActive = active;
    if (!active) {
      setPausedForVideo(false);
      return;
    }
    invoke<boolean>('system_active_net_is_wifi')
      .then((wifi) => {
        if (wifi && streamActive) setPausedForVideo(true);
      })
      .catch(() => {
        if (streamActive) setPausedForVideo(true);
      });
  });
}
// In manual mode (no override), the GCS follows the resolved OS location + its accuracy.
userGeoLocation.subscribe(() => recompute());
userGeoAccuracyM.subscribe(() => recompute());
