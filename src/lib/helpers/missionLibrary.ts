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

/** Default cruise speed (m/s) assumed for legs where no waypoint sets an explicit speed.
 *  INAV WPs carry an optional per-WP speed (cm/s in p1); when left at default the GCS has no
 *  authoritative value, so the time estimate falls back to this constant and is flagged approximate. */
export const DEFAULT_CRUISE_MS = 5;

export interface MissionStats {
  /** Number of geographic waypoints in the active mission part (up to the first Land/RTH). */
  geoCount: number;
  /** Sum of straight-line leg distances between geo-WPs (launch→WP1 not included). */
  legDistanceM: number;
  /** Sum of positive altitude changes across the legs. */
  climbM: number;
  /** Sum of altitude losses across the legs (positive number). */
  descentM: number;
  /** Estimated flight time in seconds (legs / cruise speed + hold times), or null with <2 geo-WPs. */
  estTimeS: number | null;
  /** A leg fell back to {@link DEFAULT_CRUISE_MS} (no explicit WP speed) → the time is rough. */
  estTimeApprox: boolean;
  /** A PosHold-∞ is present → the real flight time is unbounded (estimate is a lower bound). */
  hasUnlimitedHold: boolean;
  /** The assumed cruise speed (m/s), surfaced for the tooltip. */
  assumedCruiseMs: number;
}

/** Editor-facing mission stats: distance, climb/descent totals and an estimated flight time.
 *  Only the active mission part counts (everything up to and including the first Land/RTH);
 *  JUMP repetition is not expanded, so loops read as a single straight-through pass. */
export function computeMissionStats(wps: Waypoint[]): MissionStats {
  const endIdx = wps.findIndex((w) => w.action === WpAction.Land || w.action === WpAction.Rth);
  const active = endIdx >= 0 ? wps.slice(0, endIdx + 1) : wps;
  const geo = active.filter((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));

  let legDistanceM = 0;
  let climbM = 0;
  let descentM = 0;
  let estTimeS = 0;
  let estTimeApprox = false;
  let curSpeedMs = DEFAULT_CRUISE_MS;
  let sawExplicitSpeed = false;

  let prev: Waypoint | null = null;
  for (const w of geo) {
    // Carry-forward speed: a Waypoint/Land with an explicit p1 (>0) sets the cruise speed,
    // which persists onto following legs until overridden (mirrors INAV's behaviour).
    if ((w.action === WpAction.Waypoint || w.action === WpAction.Land) && w.p1 > 0) {
      curSpeedMs = w.p1 / 100;
      sawExplicitSpeed = true;
    }
    if (prev) {
      const d = haversineM(toDeg(prev.lat), toDeg(prev.lon), toDeg(w.lat), toDeg(w.lon));
      legDistanceM += d;
      const dAlt = altToM(w.altitude) - altToM(prev.altitude);
      if (dAlt > 0) climbM += dAlt;
      else descentM += -dAlt;
      const spd = curSpeedMs > 0 ? curSpeedMs : DEFAULT_CRUISE_MS;
      estTimeS += d / spd;
      if (!sawExplicitSpeed) estTimeApprox = true;
    }
    prev = w;
  }

  let hasUnlimitedHold = false;
  for (const w of active) {
    if (w.action === WpAction.PosholdTime) estTimeS += Math.max(0, w.p1);
    else if (w.action === WpAction.PosholdUnlim) hasUnlimitedHold = true;
  }

  return {
    geoCount: geo.length,
    legDistanceM,
    climbM,
    descentM,
    estTimeS: geo.length > 1 ? estTimeS : null,
    estTimeApprox,
    hasUnlimitedHold,
    assumedCruiseMs: DEFAULT_CRUISE_MS,
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
