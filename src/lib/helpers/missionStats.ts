// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared mission route statistics — distance, climb/descent and flight time, computed identically for
// INAV and ArduPilot/PX4 so both mission panels show the same footer. The per-stack callers normalise
// their waypoints into `RoutePoint`s (carrying the cruise speed in effect for each leg) and call
// `computeRouteStats`. Flight time is returned ONLY when every travel leg has an explicitly-set cruise
// speed (INAV per-WP `p1`, ArduPilot `DO_CHANGE_SPEED`); otherwise it's `null` — we never guess a
// cruise speed and show a misleading estimate.

const EARTH_R = 6371000;

/** Great-circle distance in metres between two lat/lon points (degrees). */
export function haversineM(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const d2r = Math.PI / 180;
  const dLat = (bLat - aLat) * d2r;
  const dLon = (bLon - aLon) * d2r;
  const h =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(aLat * d2r) * Math.cos(bLat * d2r) * Math.sin(dLon / 2) ** 2;
  return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** One flight-path node, already resolved by the per-stack adapter. */
export interface RoutePoint {
  lat: number; // degrees
  lon: number; // degrees
  altM: number; // metres (reference-agnostic — used only for climb/descent deltas)
  /** Cruise speed (m/s) in effect for the leg ENDING at this point, or null when unknown. */
  legSpeedMs: number | null;
}

export interface RouteStats {
  /** Number of flight-path nodes. */
  geoCount: number;
  legDistanceM: number;
  climbM: number;
  descentM: number;
  /** Total flight time (s), or null when any travel leg's speed is unknown (we don't guess). */
  timeS: number | null;
}

/**
 * Distance / climb / descent / time over an ordered list of flight-path nodes. `extraHoldS` is added
 * to the time (e.g. INAV timed PosHolds) — but only when the travel time itself is known, since a total
 * is meaningless without it.
 */
export function computeRouteStats(points: RoutePoint[], extraHoldS = 0): RouteStats {
  let legDistanceM = 0;
  let climbM = 0;
  let descentM = 0;
  let timeS = 0;
  let timeKnown = true;

  let prev: RoutePoint | null = null;
  for (const p of points) {
    if (prev) {
      const d = haversineM(prev.lat, prev.lon, p.lat, p.lon);
      legDistanceM += d;
      const dAlt = p.altM - prev.altM;
      if (dAlt > 0) climbM += dAlt;
      else descentM += -dAlt;
      if (p.legSpeedMs && p.legSpeedMs > 0) timeS += d / p.legSpeedMs;
      else timeKnown = false;
    }
    prev = p;
  }

  return {
    geoCount: points.length,
    legDistanceM,
    climbM,
    descentM,
    timeS: points.length > 1 && timeKnown ? timeS + extraHoldS : null,
  };
}
