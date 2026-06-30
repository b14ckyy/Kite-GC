// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

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
  /** A jump cut: a discontinuity in the route (path/terrain break here). */
  cut?: boolean;
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
  /** A revisit caused by a simulated jump loop (no map dot; same WP as `index`). */
  repeat: boolean;
  /** The resume point after a jump cut — shown with a dot to mark where the route continues. */
  resume: boolean;
  /** The endpoint of a jump-back leg (the jump target) — shown with a distinct marker. */
  jumpTarget: boolean;
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
  /** Logged RSSI (Track mode), nearest track point per terrain sample; all null otherwise. */
  rssi: (number | null)[];
  markers: ProfileMarker[];
  /** Distances (m) where the route is cut by a simulated jump loop. */
  cuts: number[];
  /** Distance ranges of jump-back legs (coloured distinctly, like the map). */
  jumpLegs: { start: number; end: number }[];
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

/** Nearest terrain elevation at a given cumulative distance (skips jump cuts). */
function groundAt(terrain: TerrainSample[], dist: number): number | null {
  let best: TerrainSample | null = null;
  let bestDelta = Infinity;
  for (const s of terrain) {
    if (s.cut) continue;
    const delta = Math.abs(s.dist - dist);
    if (delta < bestDelta) {
      bestDelta = delta;
      best = s;
    }
  }
  return best ? best.elev : null;
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

/** Per-waypoint resolved MSL altitude + terrain ground (for the 3D mission). */
export interface WpMsl { altMsl: number; ground: number | null; }

/**
 * Resolve each geo-waypoint's altitude to MSL (AMSL = value, AGL = terrain+value,
 * REL = launch-ground+value) and sample the terrain ground beneath it (for the
 * 3D drop-lines). Returns a map keyed by the waypoint's index in `waypoints`.
 * Lightweight vs. buildWaypointProfile: one terrain sample per geo-WP, no jump
 * expansion or dense profiling.
 */
export async function resolveMissionAltitudes(
  waypoints: Waypoint[],
  launch: { lat: number; lng: number } | null,
): Promise<{ alts: Map<number, WpMsl>; launchGround: number | null }> {
  const out = new Map<number, WpMsl>();

  // Gather every point that needs a terrain sample (launch + each geo WP), then
  // resolve them all in a single batched IPC call instead of one round-trip per
  // waypoint — the dominant cost when the 3D mission overlay is (re)built.
  const points: [number, number][] = [];
  if (launch) points.push([launch.lat, launch.lng]);
  const launchIdx = launch ? 0 : -1;
  const wpPointIdx = new Map<number, number>(); // mission index → index into `points`
  for (let i = 0; i < waypoints.length; i++) {
    const wp = waypoints[i];
    if (!hasLocation(wp.action) || (wp.lat === 0 && wp.lon === 0)) continue;
    wpPointIdx.set(i, points.length);
    points.push([toDeg(wp.lat), toDeg(wp.lon)]);
  }

  const grounds = points.length > 0
    ? await invoke<(number | null)[]>('terrain_elevations', { points })
    : [];

  const launchGround: number | null = launchIdx >= 0 ? grounds[launchIdx] ?? null : null;
  for (const [i, pi] of wpPointIdx) {
    const wp = waypoints[i];
    const ground = grounds[pi] ?? null;
    const mode = wpAltMode(wp);
    const valM = wp.altitude / 100;
    let altMsl: number;
    if (mode === ALT_MODE_AMSL) altMsl = valM;
    else if (mode === ALT_MODE_AGL) altMsl = (ground ?? launchGround ?? 0) + valM;
    else altMsl = (launchGround ?? 0) + valM; // REL
    out.set(i, { altMsl, ground });
  }
  return { alts: out, launchGround };
}

/**
 * Max ground-relative climb angle (degrees) over a path.
 * Waypoint vertices are intentional → used as-is. Flown tracks carry altitude
 * jitter that spikes per-sample slopes toward 90°, so we low-pass the altitude
 * over a small window and only measure over segments of a minimum length.
 */
function computeMaxClimbAngle(
  path: PathPoint[],
  source: 'waypoint' | 'track',
  cuts: number[],
): number | null {
  if (path.length < 2) return null;

  if (source === 'waypoint') {
    let max: number | null = null;
    for (let i = 1; i < path.length; i++) {
      const dd = path[i].dist - path[i - 1].dist;
      // skip legs that straddle a jump cut (not a real flight leg)
      if (cuts.some((c) => c > path[i - 1].dist && c < path[i].dist)) continue;
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

/** Nearest value (by cumulative distance) from a sparse {dist, val} series — for per-sample RSSI. */
function nearestByDist(dist: number[], vals: (number | null)[], target: number): number | null {
  if (dist.length === 0) return null;
  let lo = 0;
  let hi = dist.length - 1;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (dist[m] < target) lo = m + 1;
    else hi = m;
  }
  // lo is the first index with dist[lo] >= target; compare with its predecessor
  if (lo > 0 && Math.abs(dist[lo - 1] - target) <= Math.abs(dist[lo] - target)) lo -= 1;
  return vals[lo] ?? null;
}

/** Fold terrain + path + markers into the final profile (clearance, ranges, climb). */
function finishProfile(
  source: 'waypoint' | 'track',
  terrain: TerrainSample[],
  path: PathPoint[],
  markers: ProfileMarker[],
  totalDist: number,
  cuts: number[],
  jumpLegs: { start: number; end: number }[],
  terrainRssi: (number | null)[] | null = null,
): ProfileData {
  const pathAtTerrain = terrain.map((s) => (s.cut ? null : pathAltAt(path, s.dist)));
  const clearance = terrain.map((s, i) => {
    const pa = pathAtTerrain[i];
    if (s.cut || s.elev == null || pa == null) return null;
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

  const maxClimbAngle = computeMaxClimbAngle(path, source, cuts);

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
    rssi: terrainRssi ?? terrain.map(() => null),
    markers,
    cuts,
    jumpLegs,
    totalDist,
    minClearance,
    minClearanceDist,
    maxClimbAngle,
    minElev,
    maxElev,
  };
}

/** Geo waypoints that define the profile route (have a position, excluding POI). */
function isGeoAction(action: WpAction): boolean {
  return hasLocation(action) && action !== WpAction.SetPoi;
}

interface RouteEntry {
  wp: Waypoint;
  index: number; // mission index
  number: number; // display number (1-based, geo WPs)
  repeat: boolean; // a jump revisit (no map dot; same WP)
  cutBefore: boolean; // a jump cut precedes this entry
  jumpTarget: boolean; // this entry is a jump-back branch target
  resume: boolean; // this entry resumes the route after a cut
}

/**
 * Expand the waypoint sequence into a route, simulating *one* loop per jump.
 * `4J2` (WP4 then jump to WP2) plots 4→2 (branch), a cut, then resumes 4→5 —
 * no duplicate WP dots, and the jump-back leg's terrain is covered.
 */
function expandRoute(waypoints: Waypoint[], markerNumbers?: number[]): RouteEntry[] {
  // Display numbers default to a geo-only running count (INAV: modifiers aren't counted). A caller can
  // override per-waypoint — ArduPilot/PX4 number *every* mission item, so a converted geo WP keeps its
  // original item number (the dropped DO_ commands leave gaps), matching the mission panel.
  const numbers: number[] = new Array(waypoints.length).fill(0);
  let dn = 0;
  for (let i = 0; i < waypoints.length; i++) {
    if (isGeoAction(waypoints[i].action)) numbers[i] = markerNumbers ? (markerNumbers[i] ?? ++dn) : ++dn;
  }

  const route: RouteEntry[] = [];
  const simulated = new Set<number>();
  for (let i = 0; i < waypoints.length; i++) {
    const wp = waypoints[i];
    if (wp.action === WpAction.Jump) {
      if (simulated.has(i)) continue;
      simulated.add(i);
      // Resolve target the same way the map does: p1 is the 1-based absolute
      // waypoint number, so the array index is p1 − 1.
      const tIdx = wp.p1 - 1;
      const last = route.length ? route[route.length - 1] : null;
      if (tIdx >= 0 && tIdx < waypoints.length && last && tIdx < last.index && isGeoAction(waypoints[tIdx].action)) {
        // branch to the jump target, then resume at the WP before the jump
        route.push({ wp: waypoints[tIdx], index: tIdx, number: numbers[tIdx], repeat: true, cutBefore: false, jumpTarget: true, resume: false });
        route.push({ wp: waypoints[last.index], index: last.index, number: last.number, repeat: true, cutBefore: true, jumpTarget: false, resume: true });
      }
      continue;
    }
    if (isGeoAction(wp.action)) {
      route.push({ wp, index: i, number: numbers[i], repeat: false, cutBefore: false, jumpTarget: false, resume: false });
    }
  }
  return route;
}

const CUT_GAP_M = 60;

/**
 * Build a profile for the planned waypoint mission.
 * `launch` (lat/lng degrees) is the REL/AGL home reference; null = unavailable.
 * Returns null if there are fewer than 2 geo waypoints.
 */
export async function buildWaypointProfile(
  waypoints: Waypoint[],
  launch: { lat: number; lng: number } | null,
  markerNumbers?: number[],
): Promise<ProfileData | null> {
  const route = expandRoute(waypoints, markerNumbers);
  if (route.filter((e) => !e.repeat).length < 2) return null;

  // Split into continuous segments at jump cuts
  const segments: RouteEntry[][] = [];
  let cur: RouteEntry[] = [];
  for (const e of route) {
    if (e.cutBefore && cur.length) {
      segments.push(cur);
      cur = [];
    }
    cur.push(e);
  }
  if (cur.length) segments.push(cur);

  let launchGround: number | null = null;
  if (launch) {
    launchGround = await invoke<number | null>('terrain_elevation', { lat: launch.lat, lon: launch.lng });
  }

  // Sample terrain per segment, stitching distances with a gap at each cut
  const entryDist: number[] = new Array(route.length);
  const terrain: TerrainSample[] = [];
  const cuts: number[] = [];
  let offset = 0;
  let ri = 0;
  for (let s = 0; s < segments.length; s++) {
    const seg = segments[s];
    const pts: [number, number][] = seg.map((e) => [toDeg(e.wp.lat), toDeg(e.wp.lon)]);
    const local = cumulativeDistances(pts);
    for (let j = 0; j < seg.length; j++) entryDist[ri++] = offset + local[j];

    const raw = await invoke<RawSample[]>('terrain_profile', { points: pts, spacingM: PROFILE_SPACING_M });
    bridgeGaps(raw);
    for (const sm of raw) terrain.push({ dist: offset + sm.dist_m, elev: sm.elev_m, lat: sm.lat, lon: sm.lon });

    const segLen = local[local.length - 1];
    if (s < segments.length - 1) {
      const cutDist = offset + segLen + CUT_GAP_M / 2;
      cuts.push(cutDist);
      const tail = pts[pts.length - 1];
      terrain.push({ dist: cutDist, elev: null, lat: tail[0], lon: tail[1], cut: true });
      offset += segLen + CUT_GAP_M;
    } else {
      offset += segLen;
    }
  }
  const totalDist = offset;
  if (launchGround == null) launchGround = groundAt(terrain, 0);

  const markers: ProfileMarker[] = [];
  const path: PathPoint[] = [];
  const jumpLegs: { start: number; end: number }[] = [];
  for (let k = 0; k < route.length; k++) {
    const e = route[k];
    const altMode = wpAltMode(e.wp);
    const ground = groundAt(terrain, entryDist[k]);
    const valM = e.wp.altitude / 100;
    let altMsl: number;
    if (altMode === ALT_MODE_AMSL) altMsl = valM;
    else if (altMode === ALT_MODE_AGL) altMsl = (ground ?? launchGround ?? 0) + valM;
    else altMsl = (launchGround ?? 0) + valM; // REL
    markers.push({
      index: e.index,
      number: e.number,
      action: e.wp.action,
      dist: entryDist[k],
      altMsl,
      ground,
      altMode,
      repeat: e.repeat,
      resume: e.resume,
      jumpTarget: e.jumpTarget,
    });
    path.push({ dist: entryDist[k], altMsl });
    // the leg from the WP before the jump (k-1) to its branch target (k)
    if (e.jumpTarget && k > 0) jumpLegs.push({ start: entryDist[k - 1], end: entryDist[k] });
  }

  return finishProfile('waypoint', terrain, path, markers, totalDist, cuts, jumpLegs);
}

/** A flown-track point (subset of TelemetryRecord). */
export interface TrackPoint {
  lat: number | null;
  lon: number | null;
  alt_m: number | null;
  timestamp_ms?: number;
  /** Logged signal strength at this fix (Track mode replay/live) — for the measured-link line. */
  rssi?: number | null;
}

/**
 * Build a profile for a flown track (live temp-log or loaded blackbox).
 * Returns null if there are fewer than 2 valid points.
 */
export async function buildTrackProfile(track: TrackPoint[]): Promise<ProfileData | null> {
  const pts = track.filter(
    (p): p is TrackPoint & { lat: number; lon: number; alt_m: number } =>
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
  bridgeGaps(raw);
  const terrain: TerrainSample[] = raw.map((s) => ({ dist: s.dist_m, elev: s.elev_m, lat: s.lat, lon: s.lon }));

  const path: PathPoint[] = pts.map((p, i) => ({ dist: dist[i], altMsl: p.alt_m }));

  // Per-terrain-sample RSSI: the nearest track fix's value (only if the track carries any RSSI).
  const trackRssi = pts.map((p) => p.rssi ?? null);
  const hasRssi = trackRssi.some((v) => v != null);
  const terrainRssi = hasRssi ? terrain.map((s) => (s.cut ? null : nearestByDist(dist, trackRssi, s.dist))) : null;

  return finishProfile('track', terrain, path, [], totalDist, [], [], terrainRssi);
}

/**
 * Incremental profiler for the live flown track. Samples terrain only for the
 * *new* points each update (with the last known point as the segment start for
 * continuity), accumulating terrain + path; the cheap JS folding (clearance,
 * min, climb) is recomputed over the whole accumulation each time.
 */
export class LiveTrackProfiler {
  private terrain: TerrainSample[] = [];
  private path: PathPoint[] = [];
  private processed = 0;
  private lastLat = NaN;
  private lastLon = NaN;
  private lastDist = 0;
  private keepM = 0; // 0 = unbounded
  private triggerM = 0; // compact only once the retained span exceeds this

  /**
   * Bound the retained history: once more than `triggerM` metres have accumulated, trim back
   * to the most recent `keepM` metres. The wide trigger→keep gap means the O(n) compaction runs
   * only every (triggerM − keepM) metres of travel — roughly once every few minutes — so the
   * per-tick cost stays flat and the arrays never grow without bound, no matter how long the
   * replay runs. Only consumers that display a recent window (the Live-AGL HUD) set this; the
   * full-track Terrain-Analysis panel leaves it unbounded. The `processed` cursor counts the
   * input array, so trimming the internal arrays doesn't disturb the incremental sampling.
   */
  setWindow(keepM: number, triggerM: number): void {
    this.keepM = keepM;
    this.triggerM = triggerM;
  }

  reset(): void {
    this.terrain = [];
    this.path = [];
    this.processed = 0;
    this.lastLat = NaN;
    this.lastLon = NaN;
    this.lastDist = 0;
  }

  async update(track: { lat: number; lon: number; alt_m: number }[]): Promise<ProfileData | null> {
    if (track.length < this.processed) this.reset(); // restarted (new arm)

    const fresh = track.slice(this.processed);
    this.processed = track.length;

    if (fresh.length > 0) {
      const havePrev = !Number.isNaN(this.lastLat);
      const seg: [number, number][] = [];
      if (havePrev) seg.push([this.lastLat, this.lastLon]);
      for (const p of fresh) seg.push([p.lat, p.lon]);

      if (seg.length >= 2) {
        const raw = await invoke<RawSample[]>('terrain_profile', {
          points: seg,
          spacingM: PROFILE_SPACING_M,
        });
        bridgeGaps(raw);
        // raw[0] (dist 0) == seg[0]; skip it only if it duplicates an existing sample.
        const skipFirst = havePrev && this.terrain.length > 0;
        for (let i = skipFirst ? 1 : 0; i < raw.length; i++) {
          this.terrain.push({
            dist: this.lastDist + raw[i].dist_m,
            elev: raw[i].elev_m,
            lat: raw[i].lat,
            lon: raw[i].lon,
          });
        }
      }

      let cum = this.lastDist;
      let prevLat = this.lastLat;
      let prevLon = this.lastLon;
      for (const p of fresh) {
        if (!Number.isNaN(prevLat)) cum += haversine(prevLat, prevLon, p.lat, p.lon);
        this.path.push({ dist: cum, altMsl: p.alt_m });
        prevLat = p.lat;
        prevLon = p.lon;
      }
      this.lastLat = prevLat;
      this.lastLon = prevLon;
      this.lastDist = cum;

      // Compact only once the retained span passes `triggerM`, then drop back to `keepM` — so
      // the O(n) splice runs once every few km of travel, not every tick (near-zero impact).
      if (this.keepM > 0 && this.path.length > 0 && this.lastDist - this.path[0].dist > this.triggerM) {
        const minDist = this.lastDist - this.keepM;
        let ti = 0;
        while (ti < this.terrain.length && this.terrain[ti].dist < minDist) ti++;
        this.terrain.splice(0, ti);
        let pi = 0;
        while (pi < this.path.length && this.path[pi].dist < minDist) pi++;
        this.path.splice(0, pi);
      }
    }

    if (this.path.length < 2) return null;
    return finishProfile('track', this.terrain, this.path, [], this.lastDist, [], []);
  }
}
