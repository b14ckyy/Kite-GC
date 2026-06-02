// Mission-library helpers (frontend side).
// Builds the `LibraryMissionInput` payload for the DB: a content hash (identity, for dedup)
// plus computed geometry metadata. The hash is a SHA-256 of the SAME canonical serialization
// the provenance system uses (hashWaypoints), so DB identity and provenance stay consistent.
// See docs/dev/MISSION_LIBRARY_AND_DB.md.

import { hashWaypoints, hasLocation, toDeg, altToM, type Waypoint } from '$lib/stores/mission';
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

async function sha256Hex(s: string): Promise<string> {
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

export interface BuildMissionOpts {
  name?: string;
  notes?: string | null;
  sourceXml?: string | null;
  format?: string;
}

/** Build the DB save payload (identity hash + canonical waypoints + computed metadata). */
export async function buildMissionInput(
  wps: Waypoint[],
  opts: BuildMissionOpts = {},
): Promise<LibraryMissionInput> {
  const m = computeMissionMetadata(wps);
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
  };
}
