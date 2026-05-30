// Terrain profile builder for the Terrain Analysis panel.
//
// Turns the planned mission (or a flown track) + Copernicus terrain samples
// into a side-view profile: terrain line, flight/track altitude line (MSL),
// waypoint markers, and per-sample clearance. All altitudes are MSL (Copernicus
// EGM2008 ≈ MSL), consistent with the FC's GPS altitude and AMSL waypoints.

import { invoke } from '@tauri-apps/api/core';
import {
  WpAction,
  hasLocation,
  toDeg,
  ALT_MODE_AMSL,
  ALT_MODE_AGL,
  type Waypoint,
} from '$lib/stores/mission';

/** Raw sample returned by the `terrain_profile` backend command. */
interface RawSample {
  dist_m: number;
  lat: number;
  lon: number;
  elev_m: number | null;
}

export interface TerrainSample {
  dist: number;
  elev: number | null;
  lat: number;
  lon: number;
}

export interface PathPoint {
  dist: number;
  altMsl: number;
}

export interface ProfileMarker {
  /** index into mission.waypoints */
  index: number;
  /** display number (1-based, geo WPs only) */
  number: number;
  action: WpAction;
  dist: number;
  altMsl: number;
  ground: number | null;
  altMode: number;
}

export interface ProfileData {
  source: 'waypoint' | 'track';
  terrain: TerrainSample[];
  /** Flight/track altitude line (sparse: WP vertices or track points) */
  path: PathPoint[];
  /** Path MSL altitude interpolated at each terrain sample (aligned to `terrain`) */
  pathAtTerrain: (number | null)[];
  /** clearance = pathAtTerrain − terrain.elev, aligned to `terrain` */
  clearance: (number | null)[];
  markers: ProfileMarker[];
  totalDist: number;
  minClearance: number | null;
  minClearanceDist: number | null;
  maxClimbAngle: number | null;
  /** MSL y-range covering terrain + path */
  minElev: number;
  maxElev: number;
}

export const PROFILE_SPACING_M = 30;

const EARTH_R = 6371000;

function haversine(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const toRad = Math.PI / 180;
  const dLat = (bLat - aLat) * toRad;
  const dLon = (bLon - aLon) * toRad;
  const la1 = aLat * toRad;
  const la2 = bLat * toRad;
  const h =
    Math.sin(dLat / 2) ** 2 + Math.cos(la1) * Math.cos(la2) * Math.sin(dLon / 2) ** 2;
  return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** Cumulative distance (m) for each vertex of a [lat,lon] polyline. */
function cumulativeDistances(points: [number, number][]): number[] {
  const out = [0];
  for (let i = 1; i < points.length; i++) {
    out[i] = out[i - 1] + haversine(points[i - 1][0], points[i - 1][1], points[i][0], points[i][1]);
  }
  return out;
}

/** Nearest terrain sample elevation at a given cumulative distance. */
function groundAt(raw: RawSample[], dist: number): number | null {
  if (raw.length === 0) return null;
  let best = raw[0];
  let bestDelta = Math.abs(raw[0].dist_m - dist);
  for (const s of raw) {
    const delta = Math.abs(s.dist_m - dist);
    if (delta < bestDelta) {
      bestDelta = delta;
      best = s;
    }
  }
  return best.elev_m;
}

/** Linear interpolation of the (sparse) path altitude at an arbitrary distance. */
function pathAltAt(path: PathPoint[], dist: number): number | null {
  if (path.length === 0) return null;
  if (dist <= path[0].dist) return path[0].altMsl;
  const last = path[path.length - 1];
  if (dist >= last.dist) return last.altMsl;
  for (let i = 1; i < path.length; i++) {
    if (dist <= path[i].dist) {
      const a = path[i - 1];
      const b = path[i];
      const span = b.dist - a.dist || 1;
      const t = (dist - a.dist) / span;
      return a.altMsl + (b.altMsl - a.altMsl) * t;
    }
  }
  return last.altMsl;
}

/** Resolve a waypoint's alt_mode (falls back to P3 bit 0 for legacy data). */
function wpAltMode(wp: Waypoint): number {
  return wp.alt_mode ?? ((wp.p3 & 1) ? ALT_MODE_AMSL : 0);
}

/**
 * Max ground-relative climb angle (degrees) over a path.
 * Waypoint vertices are intentional → used as-is. Flown tracks carry altitude
 * jitter that spikes per-sample slopes toward 90°, so we low-pass the altitude
 * over a small window and only measure over segments of a minimum length.
 */
function computeMaxClimbAngle(path: PathPoint[], source: 'waypoint' | 'track'): number | null {
  if (path.length < 2) return null;

  if (source === 'waypoint') {
    let max: number | null = null;
    for (let i = 1; i < path.length; i++) {
      const dd = path[i].dist - path[i - 1].dist;
      if (dd > 0) {
        const ang = (Math.atan2(Math.abs(path[i].altMsl - path[i - 1].altMsl), dd) * 180) / Math.PI;
        if (max == null || ang > max) max = ang;
      }
    }
    return max;
  }

  // Track: low-pass altitude over a ±half-window, measure per ≥MIN_SEG_M segment.
  const HALF = 5; // 10-point window
  const MIN_SEG_M = 20;
  const n = path.length;
  const smooth: number[] = new Array(n);
  for (let i = 0; i < n; i++) {
    let sum = 0;
    let count = 0;
    for (let k = Math.max(0, i - HALF); k <= Math.min(n - 1, i + HALF); k++) {
      sum += path[k].altMsl;
      count++;
    }
    smooth[i] = sum / count;
  }
  let max: number | null = null;
  let anchor = 0;
  for (let i = 1; i < n; i++) {
    const dd = path[i].dist - path[anchor].dist;
    if (dd >= MIN_SEG_M) {
      const ang = (Math.atan2(Math.abs(smooth[i] - smooth[anchor]), dd) * 180) / Math.PI;
      if (max == null || ang > max) max = ang;
      anchor = i;
    }
  }
  return max;
}

/**
 * Bridge interior voids: terrain occasionally returns a null sample mid-route
 * (tile-edge sampling / nodata pixel). Linearly interpolate across interior
 * null runs so the terrain line and clearance stay continuous. Leading/trailing
 * nulls (genuine out-of-coverage at the route ends) are left as-is.
 */
function bridgeGaps(raw: RawSample[]): void {
  const n = raw.length;
  let i = 0;
  while (i < n) {
    if (raw[i].elev_m == null) {
      const prev = i - 1;
      let next = i;
      while (next < n && raw[next].elev_m == null) next++;
      if (prev >= 0 && next < n) {
        const a = raw[prev];
        const b = raw[next];
        const span = b.dist_m - a.dist_m || 1;
        for (let k = i; k < next; k++) {
          const tt = (raw[k].dist_m - a.dist_m) / span;
          raw[k].elev_m = (a.elev_m as number) + ((b.elev_m as number) - (a.elev_m as number)) * tt;
        }
      }
      i = next;
    } else {
      i++;
    }
  }
}

/** Fold terrain + path + markers into the final profile (clearance, ranges, climb). */
function finishProfile(
  source: 'waypoint' | 'track',
  raw: RawSample[],
  path: PathPoint[],
  markers: ProfileMarker[],
  totalDist: number,
): ProfileData {
  bridgeGaps(raw);
  const terrain: TerrainSample[] = raw.map((s) => ({ dist: s.dist_m, elev: s.elev_m, lat: s.lat, lon: s.lon }));

  const pathAtTerrain = terrain.map((s) => pathAltAt(path, s.dist));
  const clearance = terrain.map((s, i) => {
    const pa = pathAtTerrain[i];
    if (s.elev == null || pa == null) return null;
    return pa - s.elev;
  });

  let minClearance: number | null = null;
  let minClearanceDist: number | null = null;
  for (let i = 0; i < clearance.length; i++) {
    const c = clearance[i];
    if (c != null && (minClearance == null || c < minClearance)) {
      minClearance = c;
      minClearanceDist = terrain[i].dist;
    }
  }

  const maxClimbAngle = computeMaxClimbAngle(path, source);

  let minElev = Infinity;
  let maxElev = -Infinity;
  for (const s of terrain) {
    if (s.elev != null) {
      if (s.elev < minElev) minElev = s.elev;
      if (s.elev > maxElev) maxElev = s.elev;
    }
  }
  for (const p of path) {
    if (p.altMsl < minElev) minElev = p.altMsl;
    if (p.altMsl > maxElev) maxElev = p.altMsl;
  }
  if (!isFinite(minElev) || !isFinite(maxElev)) {
    minElev = 0;
    maxElev = 100;
  }

  return {
    source,
    terrain,
    path,
    pathAtTerrain,
    clearance,
    markers,
    totalDist,
    minClearance,
    minClearanceDist,
    maxClimbAngle,
    minElev,
    maxElev,
  };
}

/** Flight-path waypoints that define the profile route (geo WPs, excluding POI). */
function flightPathWaypoints(waypoints: Waypoint[]): { wp: Waypoint; index: number }[] {
  return waypoints
    .map((wp, index) => ({ wp, index }))
    .filter(({ wp }) => hasLocation(wp.action) && wp.action !== WpAction.SetPoi);
}

/**
 * Build a profile for the planned waypoint mission.
 * `launch` (lat/lng degrees) is the REL/AGL home reference; null = unavailable.
 * Returns null if there are fewer than 2 geo waypoints.
 */
export async function buildWaypointProfile(
  waypoints: Waypoint[],
  launch: { lat: number; lng: number } | null,
): Promise<ProfileData | null> {
  const flight = flightPathWaypoints(waypoints);
  if (flight.length < 2) return null;

  const points: [number, number][] = flight.map(({ wp }) => [toDeg(wp.lat), toDeg(wp.lon)]);
  const wpDist = cumulativeDistances(points);
  const totalDist = wpDist[wpDist.length - 1];

  const raw = await invoke<RawSample[]>('terrain_profile', {
    points,
    spacingM: PROFILE_SPACING_M,
  });

  // Home/launch ground reference for REL altitudes (fallback: first WP ground).
  let launchGround: number | null = null;
  if (launch) {
    launchGround = await invoke<number | null>('terrain_elevation', {
      lat: launch.lat,
      lon: launch.lng,
    });
  }
  if (launchGround == null) launchGround = groundAt(raw, 0);

  const markers: ProfileMarker[] = [];
  let displayNum = 1;
  flight.forEach(({ wp, index }, i) => {
    const altMode = wpAltMode(wp);
    const ground = groundAt(raw, wpDist[i]);
    const valM = wp.altitude / 100;
    let altMsl: number;
    if (altMode === ALT_MODE_AMSL) altMsl = valM;
    else if (altMode === ALT_MODE_AGL) altMsl = (ground ?? launchGround ?? 0) + valM;
    else altMsl = (launchGround ?? 0) + valM; // REL
    markers.push({ index, number: displayNum++, action: wp.action, dist: wpDist[i], altMsl, ground, altMode });
  });

  const path: PathPoint[] = markers.map((m) => ({ dist: m.dist, altMsl: m.altMsl }));
  return finishProfile('waypoint', raw, path, markers, totalDist);
}

/** A flown-track point (subset of TelemetryRecord). */
export interface TrackPoint {
  lat: number | null;
  lon: number | null;
  alt_m: number | null;
  timestamp_ms?: number;
}

/**
 * Build a profile for a flown track (live temp-log or loaded blackbox).
 * Returns null if there are fewer than 2 valid points.
 */
export async function buildTrackProfile(track: TrackPoint[]): Promise<ProfileData | null> {
  const pts = track.filter(
    (p): p is { lat: number; lon: number; alt_m: number } =>
      p.lat != null && p.lon != null && p.alt_m != null,
  );
  if (pts.length < 2) return null;

  const points: [number, number][] = pts.map((p) => [p.lat, p.lon]);
  const dist = cumulativeDistances(points);
  const totalDist = dist[dist.length - 1];

  const raw = await invoke<RawSample[]>('terrain_profile', {
    points,
    spacingM: PROFILE_SPACING_M,
  });

  const path: PathPoint[] = pts.map((p, i) => ({ dist: dist[i], altMsl: p.alt_m }));
  return finishProfile('track', raw, path, [], totalDist);
}
