// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mission-library helpers for the MAVLink-mission family (ArduPilot now, PX4 later).
// The ArduPilot-side counterpart to helpers/missionLibrary.ts (INAV): a content hash (identity,
// for dedup) plus computed geometry metadata, building the same `LibraryMissionInput` payload.
//
// Identity rule (mirrors INAV): the hash covers the mission waypoint items only. ArduPilot home is
// NOT part of the `arduMission` store — the FC reserves mission slot 0 for home and the download
// drops it (mavlink_proto/mission.rs), so the launch/home position is naturally excluded from the
// dedup hash, exactly like INAV's separate launch point. See docs/active/ARDUPILOT_MISSION_LIBRARY.md.

import type { LibraryMissionInput } from '$lib/stores/flightlogTypes';
import { arduHasLocation, type ArduWaypoint } from '$lib/stores/missionArdupilot';
import { sha256Hex } from './missionLibrary';
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

/** Stable content-hash identity of an ArduPilot mission (the WP `seq` is positional → not stored;
 *  home is not in the array → naturally excluded). */
export function hashArduWaypoints(wps: ArduWaypoint[]): string {
  return JSON.stringify(
    wps.map((w) => [w.command, w.frame, w.param1, w.param2, w.param3, w.param4, w.lat, w.lon, w.alt]),
  );
}

/** SHA-256 of the canonical ArduPilot mission serialization (the DB `content_hash`). */
export function arduMissionContentHash(wps: ArduWaypoint[]): Promise<string> {
  return sha256Hex(hashArduWaypoints(wps));
}

/** The library mission id matching these waypoints by content hash, or null (no match). */
export async function findArduLibraryMissionId(wps: ArduWaypoint[], dbPath: string): Promise<number | null> {
  const hash = await arduMissionContentHash(wps);
  const m = await missionDbFindByHash(hash, dbPath);
  return m ? m.id : null;
}

export interface ArduMissionMetadata {
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

/** Geometry metadata over the geo (location) commands. ArduPilot altitudes are already metres. */
export function computeArduMissionMetadata(wps: ArduWaypoint[]): ArduMissionMetadata {
  const geo = wps.filter((w) => arduHasLocation(w.command) && !(w.lat === 0 && w.lon === 0));

  let totalDistanceM: number | null = null;
  let maxAltM: number | null = null;
  let minAltM: number | null = null;
  const bndbox = { minLat: null, minLon: null, maxLat: null, maxLon: null } as ArduMissionMetadata['bndbox'];

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
      const lat = w.lat / 1e7;
      const lon = w.lon / 1e7;
      const altM = w.alt;
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

export interface BuildArduMissionOpts {
  name?: string;
  notes?: string | null;
  /** Concrete system stored in the DB `format` column ('ardupilot' | 'px4'). */
  format?: string;
}

/** Build the DB save payload for an ArduPilot/PX4 mission (identity hash + canonical items +
 *  computed metadata). `home_lat/lon` are null — ArduPilot home lives FC-side, not in the mission. */
export async function buildArduMissionInput(
  wps: ArduWaypoint[],
  opts: BuildArduMissionOpts = {},
): Promise<LibraryMissionInput> {
  const m = computeArduMissionMetadata(wps);
  return {
    content_hash: await arduMissionContentHash(wps),
    name: opts.name ?? '',
    format: opts.format ?? 'ardupilot',
    waypoints_json: JSON.stringify(wps),
    source_xml: null,
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
    home_lat: null,
    home_lon: null,
  };
}
