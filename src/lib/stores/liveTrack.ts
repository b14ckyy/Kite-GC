// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Live flown track — accumulated in RAM while armed, for the Terrain Analyzer
// (and future live widgets). lat/lon + MSL altitude over time; cleared on each
// new arm. Independent of the map trail (which keeps lat/lon only) and of the
// flight-log DB (this exists regardless of the recording setting).

import { writable } from 'svelte/store';

export interface LiveTrackPoint {
  lat: number;
  lon: number;
  alt_m: number; // MSL
  timestamp_ms: number;
}

export const liveTrack = writable<LiveTrackPoint[]>([]);

/** Don't add a point unless the craft moved at least this far (matches map trail). */
const MIN_DIST_M = 5;
const EARTH_R = 6371000;

function haversine(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const toRad = Math.PI / 180;
  const dLat = (bLat - aLat) * toRad;
  const dLon = (bLon - aLon) * toRad;
  const la1 = aLat * toRad;
  const la2 = bLat * toRad;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(la1) * Math.cos(la2) * Math.sin(dLon / 2) ** 2;
  return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
}

export function appendLivePoint(lat: number, lon: number, alt_m: number, timestamp_ms: number): void {
  liveTrack.update((arr) => {
    if (arr.length > 0) {
      const last = arr[arr.length - 1];
      if (haversine(last.lat, last.lon, lat, lon) < MIN_DIST_M) return arr;
    }
    // Mutate in place (O(1) append); `update` still notifies subscribers.
    arr.push({ lat, lon, alt_m, timestamp_ms });
    return arr;
  });
}

export function clearLiveTrack(): void {
  liveTrack.set([]);
}
