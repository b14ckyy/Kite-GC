// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mission-library helpers (frontend side).
// Builds the `LibraryMissionInput` payload for the DB: a content hash (identity, for dedup)
// plus computed geometry metadata. The hash is a SHA-256 of the SAME canonical serialization
// the provenance system uses (hashWaypoints), so DB identity and provenance stay consistent.
// See docs/archive/MISSION_LIBRARY_AND_DB.md.

import { get } from 'svelte/store';
import { hashWaypoints, hasLocation, toDeg, altToM, WpAction, launchPoint, type Waypoint } from '$lib/stores/mission';
import type { LibraryMissionInput } from '$lib/stores/flightlogTypes';
import { missionDbFindByHash } from '$lib/stores/flightlog';
import { computeRouteStats, type RoutePoint } from '$lib/helpers/missionStats';

const EARTH_R = 6371000;
const D2R = Math.PI / 180;

/** Great-circle distance in metres (matches the Rust haversine_m). */
function haversineM(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const dLat = (bLat - aLat) * D2R;
  const dLon = (bLon - aLon) * D2R;
  const h =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(aLat * D2R) * Math.cos(bLat * D2R) * Math.sin(dLon / 2) ** 2;
  return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** SHA-256 hex of a string. Shared with the ArduPilot library builder so both protocols derive
 *  their DB `content_hash` the same way. */
export async function sha256Hex(s: string): Promise<string> {
  const buf = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s));
  return [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** Stable content-hash identity of a mission (SHA-256 of the provenance serialization). */
export function missionContentHash(wps: Waypoint[]): Promise<string> {
  return sha256Hex(hashWaypoints(wps));
}

/** The library mission id matching these waypoints by content hash, or null (no match).
 *  Used by the import flow to set `loadedMissionId` when an imported mission already exists. */
export async function findLibraryMissionId(wps: Waypoint[], dbPath: string): Promise<number | null> {
  const hash = await missionContentHash(wps);
  const m = await missionDbFindByHash(hash, dbPath);
  return m ? m.id : null;
}

export interface MissionMetadata {
  wpCount: number;
  totalDistanceM: number | null;
  altDiffM: number | null;
  maxAltM: number | null;
  minAltM: number | null;
  bndbox: {
    minLat: number | null;
    minLon: number | null;
    maxLat: number | null;
    maxLon: number | null;
  };
}

/** Geometry metadata over the geo-waypoints (legs only; launch→WP1 not included). */
export function computeMissionMetadata(wps: Waypoint[]): MissionMetadata {
  const geo = wps.filter((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));

  let totalDistanceM: number | null = null;
  let maxAltM: number | null = null;
  let minAltM: number | null = null;
  const bndbox = { minLat: null, minLon: null, maxLat: null, maxLon: null } as MissionMetadata['bndbox'];

  if (geo.length > 0) {
    let dist = 0;
    let prevLat: number | null = null;
    let prevLon: number | null = null;
    let maxA = -Infinity;
    let minA = Infinity;
    let minLat = Infinity;
    let minLon = Infinity;
    let maxLat = -Infinity;
    let maxLon = -Infinity;

    for (const w of geo) {
      const lat = toDeg(w.lat);
      const lon = toDeg(w.lon);
      const altM = altToM(w.altitude);
      if (prevLat !== null && prevLon !== null) dist += haversineM(prevLat, prevLon, lat, lon);
      prevLat = lat;
      prevLon = lon;
      maxA = Math.max(maxA, altM);
      minA = Math.min(minA, altM);
      minLat = Math.min(minLat, lat);
      maxLat = Math.max(maxLat, lat);
      minLon = Math.min(minLon, lon);
      maxLon = Math.max(maxLon, lon);
    }

    totalDistanceM = geo.length > 1 ? dist : 0;
    maxAltM = maxA;
    minAltM = minA;
    bndbox.minLat = minLat;
    bndbox.minLon = minLon;
    bndbox.maxLat = maxLat;
    bndbox.maxLon = maxLon;
  }

  const altDiffM = maxAltM !== null && minAltM !== null ? maxAltM - minAltM : null;

  return { wpCount: wps.length, totalDistanceM, altDiffM, maxAltM, minAltM, bndbox };
}

export interface MissionStats {
  /** Number of geographic waypoints in the active mission part (up to the first Land/RTH). */
  geoCount: number;
  /** Sum of straight-line leg distances between geo-WPs (launch→WP1 not included). */
  legDistanceM: number;
  /** Sum of positive altitude changes across the legs. */
  climbM: number;
  /** Sum of altitude losses across the legs (positive number). */
  descentM: number;
  /** Flight time in seconds, or null when not every leg has an explicitly-set WP speed (we don't
   *  guess a cruise speed). With <2 geo-WPs it's also null. */
  estTimeS: number | null;
  /** A PosHold-∞ is present → the real flight time is unbounded (the value is a lower bound). */
  hasUnlimitedHold: boolean;
}

/** Editor-facing mission stats: distance, climb/descent totals and (when known) the flight time.
 *  Only the active mission part counts (everything up to and including the first Land/RTH);
 *  JUMP repetition is not expanded, so loops read as a single straight-through pass. The time is
 *  reported only when waypoints set an explicit cruise speed (INAV `p1`, cm/s) — see `computeRouteStats`. */
export function computeMissionStats(wps: Waypoint[]): MissionStats {
  const endIdx = wps.findIndex((w) => w.action === WpAction.Land || w.action === WpAction.Rth);
  const active = endIdx >= 0 ? wps.slice(0, endIdx + 1) : wps;
  const geo = active.filter((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));

  // Carry-forward cruise speed: a Waypoint/Land with explicit p1 (>0, cm/s) sets the speed for the leg
  // ending at it and onward (mirrors INAV). Unset stays null → the time reads as "unknown".
  let curSpeedMs: number | null = null;
  const points: RoutePoint[] = geo.map((w) => {
    if ((w.action === WpAction.Waypoint || w.action === WpAction.Land) && w.p1 > 0) {
      curSpeedMs = w.p1 / 100;
    }
    return { lat: toDeg(w.lat), lon: toDeg(w.lon), altM: altToM(w.altitude), legSpeedMs: curSpeedMs };
  });

  let holdS = 0;
  let hasUnlimitedHold = false;
  for (const w of active) {
    if (w.action === WpAction.PosholdTime) holdS += Math.max(0, w.p1);
    else if (w.action === WpAction.PosholdUnlim) hasUnlimitedHold = true;
  }

  const s = computeRouteStats(points, holdS);
  return {
    geoCount: s.geoCount,
    legDistanceM: s.legDistanceM,
    climbM: s.climbM,
    descentM: s.descentM,
    estTimeS: s.timeS,
    hasUnlimitedHold,
  };
}

export interface BuildMissionOpts {
  name?: string;
  notes?: string | null;
  sourceXml?: string | null;
  format?: string;
  /** Launch/home point to store; defaults to the current `launchPoint` store. `null` = none. */
  home?: { lat: number; lng: number } | null;
}

/** Build the DB save payload (identity hash + canonical waypoints + computed metadata). */
export async function buildMissionInput(
  wps: Waypoint[],
  opts: BuildMissionOpts = {},
): Promise<LibraryMissionInput> {
  const m = computeMissionMetadata(wps);
  // Persist the planned launch/home point (the REL-altitude + 3D-preview reference) — the
  // same point the .mission file export writes as <mwp> meta. opts.home overrides the store.
  const home = opts.home !== undefined ? opts.home : get(launchPoint);
  return {
    content_hash: await missionContentHash(wps),
    name: opts.name ?? '',
    format: opts.format ?? 'inav',
    waypoints_json: JSON.stringify(wps),
    source_xml: opts.sourceXml ?? null,
    wp_count: m.wpCount,
    total_distance_m: m.totalDistanceM,
    alt_diff_m: m.altDiffM,
    max_alt_m: m.maxAltM,
    min_alt_m: m.minAltM,
    bndbox_min_lat: m.bndbox.minLat,
    bndbox_min_lon: m.bndbox.minLon,
    bndbox_max_lat: m.bndbox.maxLat,
    bndbox_max_lon: m.bndbox.maxLon,
    notes: opts.notes ?? null,
    home_lat: home?.lat ?? null,
    home_lon: home?.lng ?? null,
  };
}
