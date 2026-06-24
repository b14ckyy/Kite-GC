// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Geozone ↔ mission safety check (hints only, never a blocker). Given the FC geozones and the planned
// mission path, flags when the path would breach a No-Flight-Zone or leave an inclusion zone — mirroring
// INAV's own rules. See docs/active/GEOZONES.md.
//
// Altitude-aware: everything is normalised to "metres above the launch/home ground" (relM). INAV
// geozones with isSealevelRef=0 are relative to launch (so they map straight onto relM); AMSL zones and
// AMSL waypoints are converted with the launch ground elevation (`homeAmsl`). Terrain-following (AGL)
// waypoints are approximated as launch-relative (no per-sample terrain query) — fine for a hint.
//
// Lateral geometry uses a uniform sampling of each leg (handles polygons, circles, and the union of
// overlapping inclusion zones — the "corridor" — naturally: a point is allowed if ANY inclusion zone
// contains it in 3D).

import type { GeoZone } from '$lib/stores/geozone';
import { haversineDistance } from '$lib/utils/geo';

// Firmware enum values (kept local to avoid an import cycle with the store).
const SHAPE_CIRCLE = 0;
const TYPE_INCLUSIVE = 1;

/** A point on the mission path: position (degrees), height above the launch/home ground (`relM`, used
 *  for the zone-band check) and absolute MSL (`altMsl`, used to place the 3D red overlay at the exact
 *  mission height = `altMsl + geoidOffset`). */
export interface PathPt {
  lat: number;
  lon: number;
  relM: number;
  altMsl: number;
}

export interface ViolationSegment {
  a: PathPt;
  b: PathPt;
}

export interface GeozoneMissionResult {
  /** The check ran (mission edit mode + at least one zone present). */
  active: boolean;
  /** Launch or home lies inside an inclusion zone → inclusion zones are enforced (else ignored, as INAV does). */
  inclusiveActive: boolean;
  /** The path leaves the inclusion-zone union (only meaningful when inclusiveActive). */
  inclusiveViolated: boolean;
  /** Launch or home lies inside an NFZ (arming may be blocked there). */
  nfzLaunchInside: boolean;
  /** The path crosses an NFZ. */
  nfzPathViolated: boolean;
  /** Violating legs (for the red 2D/3D overlay). */
  segments: ViolationSegment[];
}

export const EMPTY_MISSION_RESULT: GeozoneMissionResult = {
  active: false, inclusiveActive: false, inclusiveViolated: false,
  nfzLaunchInside: false, nfzPathViolated: false, segments: [],
};

/** Ray-casting point-in-polygon (lon = x, lat = y); ring vertices in degrees. */
function pointInPolygon(lat: number, lon: number, ring: [number, number][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [yi, xi] = ring[i]; // [lat, lon]
    const [yj, xj] = ring[j];
    if ((xi > lon) !== (xj > lon) && lat < ((yj - yi) * (lon - xi)) / (xj - xi) + yi) inside = !inside;
  }
  return inside;
}

/** Lateral containment of a point (degrees) in a zone's footprint. */
function zoneContainsLatLon(zone: GeoZone, lat: number, lon: number): boolean {
  if (zone.vertices.length === 0) return false;
  if (zone.shape === SHAPE_CIRCLE) {
    const c = zone.vertices[0];
    const r = (zone.radius_cm ?? 0) / 100;
    return r > 0 && haversineDistance(c.lat / 1e7, c.lon / 1e7, lat, lon) <= r;
  }
  if (zone.vertices.length < 3) return false;
  return pointInPolygon(lat, lon, zone.vertices.map((v) => [v.lat / 1e7, v.lon / 1e7] as [number, number]));
}

/** Zone altitude band in the launch-relative frame (m). `null` homeAmsl + an AMSL zone → unbounded
 *  (conservative: treat altitude as always overlapping so the lateral check still warns). */
function zoneBandRelM(zone: GeoZone, homeAmsl: number | null): [number, number] {
  const minM = zone.min_alt_cm / 100;
  const maxM = zone.max_alt_cm / 100;
  if (zone.is_sealevel_ref) {
    if (homeAmsl == null) return [-Infinity, Infinity];
    return [minM - homeAmsl, maxM === 0 ? Infinity : maxM - homeAmsl];
  }
  return [minM, maxM === 0 ? Infinity : maxM];
}

/** Full 3D containment: laterally inside AND the height is within the zone's band. */
function zoneContains3D(zone: GeoZone, lat: number, lon: number, relM: number, homeAmsl: number | null): boolean {
  if (!zoneContainsLatLon(zone, lat, lon)) return false;
  const [lo, hi] = zoneBandRelM(zone, homeAmsl);
  return relM >= lo && relM <= hi;
}

/** Samples (inclusive of both endpoints) along a leg, ~every 40 m, capped. */
function sampleLeg(a: PathPt, b: PathPt): PathPt[] {
  const lenM = haversineDistance(a.lat, a.lon, b.lat, b.lon);
  const steps = Math.min(150, Math.max(2, Math.ceil(lenM / 40)));
  const out: PathPt[] = [];
  for (let k = 0; k <= steps; k++) {
    const t = k / steps;
    out.push({
      lat: a.lat + (b.lat - a.lat) * t,
      lon: a.lon + (b.lon - a.lon) * t,
      relM: a.relM + (b.relM - a.relM) * t,
      altMsl: a.altMsl + (b.altMsl - a.altMsl) * t,
    });
  }
  return out;
}

/**
 * Run the check. `path` is launch (relM 0) followed by the located waypoints; `home` is an extra ground
 * point to test for inside-zone (or null). `homeAmsl` is the launch ground elevation (m, MSL) or null.
 */
export function checkMissionGeozones(
  zones: GeoZone[],
  path: PathPt[],
  home: { lat: number; lon: number } | null,
  homeAmsl: number | null,
): GeozoneMissionResult {
  const inclusive = zones.filter((z) => z.zone_type === TYPE_INCLUSIVE);
  const exclusive = zones.filter((z) => z.zone_type !== TYPE_INCLUSIVE);

  // Inclusion zones are only enforced when launch/home is inside one laterally (INAV: armed outside → off).
  const launch = path[0];
  const inclusiveActive =
    (!!launch && inclusive.some((z) => zoneContainsLatLon(z, launch.lat, launch.lon))) ||
    (!!home && inclusive.some((z) => zoneContainsLatLon(z, home.lat, home.lon)));

  // Launch/home inside an NFZ (ground level → relM 0).
  const groundInNfz = (p: { lat: number; lon: number }) =>
    exclusive.some((z) => zoneContains3D(z, p.lat, p.lon, 0, homeAmsl));
  const nfzLaunchInside = (!!launch && groundInNfz(launch)) || (!!home && groundInNfz(home));

  const segments: ViolationSegment[] = [];
  let nfzPathViolated = false;
  let inclusiveViolated = false;

  for (let i = 0; i + 1 < path.length; i++) {
    const a = path[i], b = path[i + 1];
    let bad = false;
    for (const s of sampleLeg(a, b)) {
      const inNfz = exclusive.some((z) => zoneContains3D(z, s.lat, s.lon, s.relM, homeAmsl));
      if (inNfz) { bad = true; nfzPathViolated = true; }
      if (inclusiveActive && !inclusive.some((z) => zoneContains3D(z, s.lat, s.lon, s.relM, homeAmsl))) {
        bad = true; inclusiveViolated = true;
      }
      if (inNfz && inclusiveViolated) break; // already flagged both
    }
    if (bad) segments.push({ a, b });
  }

  return { active: true, inclusiveActive, inclusiveViolated, nfzLaunchInside, nfzPathViolated, segments };
}

/**
 * Live single-point breach check (3D) for the in-flight breach toast. Mirrors the mission check but for
 * the current UAV position: inside any NFZ → `nfz`; if inclusion zones are enforced (home inside one,
 * INAV-style) and the point is outside the inclusion union → `inclusion`. `point.relM` is altitude above
 * launch; `homeAmsl` the launch ground (m MSL) for AMSL zone bands.
 */
export function checkLiveGeozoneBreach(
  zones: GeoZone[],
  point: { lat: number; lon: number; relM: number },
  home: { lat: number; lon: number } | null,
  homeAmsl: number | null,
): { nfz: boolean; inclusion: boolean } {
  const inclusive = zones.filter((z) => z.zone_type === TYPE_INCLUSIVE);
  const exclusive = zones.filter((z) => z.zone_type !== TYPE_INCLUSIVE);
  const nfz = exclusive.some((z) => zoneContains3D(z, point.lat, point.lon, point.relM, homeAmsl));
  const inclusiveActive = !!home && inclusive.some((z) => zoneContainsLatLon(z, home.lat, home.lon));
  const inclusion =
    inclusiveActive && !inclusive.some((z) => zoneContains3D(z, point.lat, point.lon, point.relM, homeAmsl));
  return { nfz, inclusion };
}
