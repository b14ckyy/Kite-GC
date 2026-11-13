// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Small geodesy helpers for moving ArduPilot/PX4 mission plans as a whole: a metric offset along a
// bearing → a lat/lon delta, applied to every location command (coordinate-less items and 0,0
// placeholders — "take off in place", DO_ commands — are left untouched).
// See docs/02-design/features/fleet-mission.design.md §2.3.

import type { ArduWaypoint } from '$lib/stores/missionArdupilot';
import { cmdHasLocation } from '$lib/helpers/arduCommandCatalog';

const EARTH_R_M = 6_371_000;

/** Degrees of latitude/longitude for a north/east displacement (m) at latitude `latDeg`. */
export function metresToLatLonDelta(latDeg: number, dNorthM: number, dEastM: number): { dLat: number; dLon: number } {
  const dLat = (dNorthM / EARTH_R_M) * (180 / Math.PI);
  const cos = Math.cos((latDeg * Math.PI) / 180);
  const dLon = cos > 1e-9 ? (dEastM / (EARTH_R_M * cos)) * (180 / Math.PI) : 0;
  return { dLat, dLon };
}

/** Reference latitude of a plan: the first location command with a real coordinate, or null. */
export function planReferenceLat(wps: readonly ArduWaypoint[]): number | null {
  for (const wp of wps) {
    if (cmdHasLocation(wp.command) && (wp.lat !== 0 || wp.lon !== 0)) return wp.lat / 1e7;
  }
  return null;
}

/** Translate every located waypoint by `distM` along `bearingDeg` (0 = north, 90 = east). A zero
 *  distance returns a copy of the input. */
export function offsetWaypoints(wps: readonly ArduWaypoint[], distM: number, bearingDeg: number): ArduWaypoint[] {
  if (!distM) return wps.map((w) => ({ ...w }));
  const refLat = planReferenceLat(wps);
  if (refLat == null) return wps.map((w) => ({ ...w }));
  const b = (bearingDeg * Math.PI) / 180;
  const { dLat, dLon } = metresToLatLonDelta(refLat, distM * Math.cos(b), distM * Math.sin(b));
  return translateWaypoints(wps, Math.round(dLat * 1e7), Math.round(dLon * 1e7));
}

/** Translate located waypoints by an integer 1e7-degree delta (the store's coordinate unit). */
export function translateWaypoints(wps: readonly ArduWaypoint[], dLat1e7: number, dLon1e7: number, only?: ReadonlySet<number>): ArduWaypoint[] {
  return wps.map((w, i) => {
    if (only && !only.has(i)) return { ...w };
    if (!cmdHasLocation(w.command) || (w.lat === 0 && w.lon === 0)) return { ...w };
    return { ...w, lat: w.lat + dLat1e7, lon: w.lon + dLon1e7 };
  });
}
