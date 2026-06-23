// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV geozone config (read). See docs/active/GEOZONES.md.
//
// Mirrors the Rust `GeozoneConfig` (commands/geozone.rs). `loaded` is the last snapshot read from the
// FC, rendered on the 2D/3D maps + listed in the Airspace Manager panel. Download is always-on at INAV
// connect (the store updates when the reads complete) and cleared on disconnect. Geozones are an INAV
// ≥8.0 feature; on older firmware `has_geozones` is false and `zones` is empty. Editing/writing is
// Phase 2 (a working/dirty copy + Save-to-FC) and not implemented yet.

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { settings } from '$lib/stores/settings';
import { mission, launchPoint, editMode, toDeg, hasLocation } from '$lib/stores/mission';
import { homePosition } from '$lib/stores/home';
import { resolveMissionAltitudes } from '$lib/helpers/terrainProfile';
import {
  checkMissionGeozones, EMPTY_MISSION_RESULT, type PathPt, type GeozoneMissionResult,
} from '$lib/helpers/geozoneMissionCheck';

/** Geozone shape (matches the firmware enum). */
export const GEOZONE_SHAPE_CIRCULAR = 0;
export const GEOZONE_SHAPE_POLYGON = 1;
/** Geozone type (matches the firmware enum). */
export const GEOZONE_TYPE_EXCLUSIVE = 0;
export const GEOZONE_TYPE_INCLUSIVE = 1;
/** Fence action (matches the firmware enum). */
export const GEOZONE_ACTION_NONE = 0;
export const GEOZONE_ACTION_AVOID = 1;
export const GEOZONE_ACTION_POSHOLD = 2;
export const GEOZONE_ACTION_RTH = 3;

/** One geozone vertex (lat/lon in degrees × 1e7). */
export interface GeoZoneVertex {
  lat: number;
  lon: number;
}

/** One geozone. `zone_type` 0 = exclusive (NFZ), 1 = inclusive (FZ). `shape` 0 = circular, 1 = polygon.
 *  `fence_action` 0 = none, 1 = avoid, 2 = pos-hold, 3 = RTH. Altitudes in cm: `min_alt_cm` 0 = ground,
 *  `max_alt_cm` 0 = no upper limit (∞). Circular zones carry `radius_cm` + a single centre vertex;
 *  polygons carry all corners and `radius_cm` is null. */
export interface GeoZone {
  id: number;
  zone_type: number;
  shape: number;
  min_alt_cm: number;
  max_alt_cm: number;
  is_sealevel_ref: boolean;
  fence_action: number;
  radius_cm: number | null;
  vertices: GeoZoneVertex[];
}

export interface GeozoneConfig {
  zones: GeoZone[];
  /** True when the geozone feature is available (INAV ≥8.0) — drives the UI's visibility. */
  has_geozones: boolean;
}

/** FC config limits (INAV `MAX_GEOZONES_IN_CONFIG` / `MAX_VERTICES_IN_CONFIG`). */
export const MAX_GEOZONES = 63;
export const MAX_VERTICES_TOTAL = 126;
/** Earth circumference (m) — for the web-mercator tile-width sizing of a freshly-added zone. */
const EARTH_CIRCUMFERENCE_M = 40075016.686;

/** Ground width (m) of one 256px web-mercator tile at the given latitude + zoom. New zones are sized
 *  in tiles so they look sensible at the current map scale (a circle = 2 tiles radius → fits a 4×4 tile
 *  grid; a polygon trapezoid ≈ 2×3 tiles). */
function tileWidthM(lat: number, zoom: number): number {
  return (EARTH_CIRCUMFERENCE_M * Math.cos((lat * Math.PI) / 180)) / 2 ** zoom;
}

/** Last snapshot read from the FC (the dirty-compare baseline). Null until first read / when not
 *  connected to a (geozone-capable) INAV FC. */
export const geozoneConfig = writable<GeozoneConfig | null>(null);

/** Editable working copy — drives the map overlays + the panel list so edits reflect live (mirrors
 *  `safehomeWorking`). Reset to the loaded snapshot on load / revert. */
export const geozoneWorking = writable<GeozoneConfig | null>(null);

/** Map edit-lock: when true the 2D map markers are editable (drag / popups). Toggled from the panel;
 *  defaults to locked so zones aren't moved by accident. Cleared on disconnect. */
export const geozoneEditing = writable<boolean>(false);

/** True when the working copy differs from the loaded snapshot (enables "Save to FC"). */
export const geozoneDirty = derived(
  [geozoneConfig, geozoneWorking],
  ([$loaded, $working]) =>
    !!$loaded && !!$working && JSON.stringify($loaded) !== JSON.stringify($working),
);

/** Read the full geozone config from the FC (INAV ≥8.0). Always called on connect (download
 *  always-on). On failure / non-INAV, clears to null. Resets the working copy to the fresh snapshot. */
export async function loadGeozoneConfig(): Promise<void> {
  try {
    const cfg = await invoke<GeozoneConfig>('geozone_read_all');
    geozoneConfig.set(cfg);
    geozoneWorking.set(structuredClone(cfg));
  } catch (e) {
    console.warn('[geozone] loadGeozoneConfig failed', e);
    geozoneConfig.set(null);
    geozoneWorking.set(null);
  }
}

/** "Save to FC": send the working copy as one batch + EEPROM + reboot. Geozones only apply after a
 *  reboot (INAV recomputes the internal zone structures at boot), so the FC restarts and the link drops
 *  — we do NOT re-read here; the reconnect handshake reloads the saved config. */
export async function saveGeozoneConfig(): Promise<void> {
  const cfg = get(geozoneWorking);
  if (!cfg) return;
  await invoke('geozone_write_all', { config: cfg });
}

/** Discard pending edits — reset the working copy to the loaded snapshot. */
export function revertGeozoneWorking(): void {
  const loaded = get(geozoneConfig);
  geozoneWorking.set(loaded ? structuredClone(loaded) : null);
}

/** Clear everything (on disconnect). */
export function clearGeozones(): void {
  geozoneConfig.set(null);
  geozoneWorking.set(null);
  geozoneEditing.set(false);
}

// ── Working-copy mutations (panel + map editing) ──────────────────────────────────────────────────
// All operate on `geozoneWorking`; "Save to FC" later writes the whole copy. Lat/lon are deg×1e7.

/** Update the matching zone in the working copy via a transform. */
function updateZone(id: number, fn: (z: GeoZone) => GeoZone): void {
  geozoneWorking.update((c) =>
    c ? { ...c, zones: c.zones.map((z) => (z.id === id ? fn(z) : z)) } : c,
  );
}

/** Lowest unused zone id (0..62), or null when all slots are full. */
export function nextFreeGeozoneId(): number | null {
  const cfg = get(geozoneWorking);
  const used = new Set((cfg?.zones ?? []).map((z) => z.id));
  for (let i = 0; i < MAX_GEOZONES; i++) if (!used.has(i)) return i;
  return null;
}

/** Add a new zone (circle or polygon) at the persisted map centre, sized in tiles for the current
 *  map zoom (circle = 2 tiles radius; polygon = a ~2×3 tile trapezoid, so it reads clearly as a
 *  polygon). Sane defaults otherwise (exclusive, action None, ground → no upper limit). Returns the
 *  new zone's id, or null if the table is full. */
export function addGeozone(shape: number): number | null {
  const id = nextFreeGeozoneId();
  if (id == null) return null;
  const map = get(settings).map;
  const [lat, lon] = map.center;
  const tw = tileWidthM(lat, map.zoom);
  // Local metres → deg×1e7 around the centre.
  const dLatE7 = (m: number) => Math.round((m / 111320) * 1e7);
  const dLonE7 = (m: number) => Math.round((m / (111320 * Math.cos((lat * Math.PI) / 180))) * 1e7);
  const latE7 = Math.round(lat * 1e7), lonE7 = Math.round(lon * 1e7);

  let vertices: GeoZoneVertex[];
  let radius_cm: number | null = null;
  if (shape === GEOZONE_SHAPE_CIRCULAR) {
    vertices = [{ lat: latE7, lon: lonE7 }];
    radius_cm = Math.round(2 * tw * 100); // 2 tiles radius (cm)
  } else {
    // Trapezoid ≈ 2 tiles wide (bottom) × 3 tiles tall, narrower at the top. CCW order; the save step
    // re-normalises winding anyway.
    const halfH = 1.5 * tw, wBottom = 1.0 * tw, wTop = 0.6 * tw;
    vertices = [
      { lat: latE7 - dLatE7(halfH), lon: lonE7 - dLonE7(wBottom) }, // bottom-left
      { lat: latE7 - dLatE7(halfH), lon: lonE7 + dLonE7(wBottom) }, // bottom-right
      { lat: latE7 + dLatE7(halfH), lon: lonE7 + dLonE7(wTop) },    // top-right
      { lat: latE7 + dLatE7(halfH), lon: lonE7 - dLonE7(wTop) },    // top-left
    ];
  }
  const zone: GeoZone = {
    id,
    zone_type: GEOZONE_TYPE_EXCLUSIVE,
    shape,
    min_alt_cm: 0,
    max_alt_cm: 0,
    is_sealevel_ref: false,
    fence_action: GEOZONE_ACTION_NONE,
    radius_cm,
    vertices,
  };
  geozoneWorking.update((c) => (c ? { ...c, zones: [...c.zones, zone] } : c));
  return id;
}

/** Delete a zone from the working copy. */
export function deleteGeozone(id: number): void {
  geozoneWorking.update((c) => (c ? { ...c, zones: c.zones.filter((z) => z.id !== id) } : c));
}

export function setGeozoneType(id: number, zoneType: number): void {
  updateZone(id, (z) => ({ ...z, zone_type: zoneType }));
}
export function setGeozoneAction(id: number, action: number): void {
  updateZone(id, (z) => ({ ...z, fence_action: action }));
}
export function setGeozoneAlts(id: number, minCm: number, maxCm: number): void {
  updateZone(id, (z) => ({ ...z, min_alt_cm: minCm, max_alt_cm: maxCm }));
}
export function setGeozoneSealevel(id: number, sealevel: boolean): void {
  updateZone(id, (z) => ({ ...z, is_sealevel_ref: sealevel }));
}

/** Move one vertex (deg×1e7). For a circle this moves the centre (vertex 0). */
export function setGeozoneVertex(id: number, index: number, latE7: number, lonE7: number): void {
  updateZone(id, (z) => ({
    ...z,
    vertices: z.vertices.map((v, i) => (i === index ? { lat: latE7, lon: lonE7 } : v)),
  }));
}

/** Insert a vertex (polygon only) after `afterIndex` — used by the map's edge midpoint handles. */
export function insertGeozoneVertex(id: number, afterIndex: number, latE7: number, lonE7: number): void {
  updateZone(id, (z) => {
    if (z.shape === GEOZONE_SHAPE_CIRCULAR) return z;
    const vertices = z.vertices.slice();
    vertices.splice(afterIndex + 1, 0, { lat: latE7, lon: lonE7 });
    return { ...z, vertices };
  });
}

/** Remove a polygon vertex (kept ≥3). */
export function removeGeozoneVertex(id: number, index: number): void {
  updateZone(id, (z) => {
    if (z.shape === GEOZONE_SHAPE_CIRCULAR || z.vertices.length <= 3) return z;
    return { ...z, vertices: z.vertices.filter((_, i) => i !== index) };
  });
}

/** Set a circular zone's radius (cm). */
export function setGeozoneRadius(id: number, radiusCm: number): void {
  updateZone(id, (z) => ({ ...z, radius_cm: Math.max(0, Math.round(radiusCm)) }));
}

// ── Mission ↔ geozone safety check (hints only; see helpers/geozoneMissionCheck.ts) ───────────────
/** Result of checking the planned mission against the FC geozones — drives the mission-editor warning
 *  bar + the red 2D/3D violation overlays. Recomputed (debounced) whenever an input changes. */
export const geozoneMissionResult = writable<GeozoneMissionResult>(EMPTY_MISSION_RESULT);

let checkGen = 0;
let checkTimer: ReturnType<typeof setTimeout> | undefined;
let checkMaxTimer: ReturnType<typeof setTimeout> | undefined;
async function recomputeMissionCheck(): Promise<void> {
  const cfg = get(geozoneWorking);
  if (!get(editMode) || !cfg?.has_geozones || cfg.zones.length === 0) {
    geozoneMissionResult.set(EMPTY_MISSION_RESULT);
    return;
  }
  const launch = get(launchPoint);
  const wps = get(mission).waypoints;
  const located = wps.some((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
  if (!launch && !located) {
    geozoneMissionResult.set(EMPTY_MISSION_RESULT);
    return;
  }
  // Reuse the exact mission altitude resolution (terrain-sampled launch ground + per-WP MSL) so the 3D
  // overlay sits on the mission line. `launchGround` (terrain MSL at launch) is the relM=0 reference.
  const gen = ++checkGen;
  const { alts, launchGround } = await resolveMissionAltitudes(wps, launch);
  if (gen !== checkGen) return; // superseded while awaiting terrain
  const base = launchGround ?? 0;

  const path: PathPt[] = [];
  if (launch) path.push({ lat: launch.lat, lon: launch.lng, relM: 0, altMsl: base });
  for (let i = 0; i < wps.length; i++) {
    const w = wps[i];
    if (!hasLocation(w.action) || (w.lat === 0 && w.lon === 0)) continue;
    const a = alts.get(i);
    if (!a) continue;
    path.push({ lat: toDeg(w.lat), lon: toDeg(w.lon), relM: a.altMsl - base, altMsl: a.altMsl });
  }
  const hp = get(homePosition);
  const home = hp.set ? { lat: hp.lat, lon: hp.lon } : null;
  geozoneMissionResult.set(checkMissionGeozones(cfg.zones, path, home, launchGround));
}

function runCheckNow(): void {
  if (checkTimer) { clearTimeout(checkTimer); checkTimer = undefined; }
  if (checkMaxTimer) { clearTimeout(checkMaxTimer); checkMaxTimer = undefined; }
  void recomputeMissionCheck();
}
// Debounced (150 ms) with a 400 ms max-wait so frequent telemetry re-emits (home/mission) can't keep
// resetting the timer and starve the recompute.
function scheduleMissionCheck(): void {
  if (checkTimer) clearTimeout(checkTimer);
  if (!checkMaxTimer) checkMaxTimer = setTimeout(runCheckNow, 400);
  checkTimer = setTimeout(runCheckNow, 150);
}

// Recompute on any input change. Module-level — the result store lives for the app lifetime and is
// cheap when not in edit mode (early return). The edit-mode toggle runs the check IMMEDIATELY so the
// violations appear the instant you start editing (not on the next map re-render); the rest are debounced.
editMode.subscribe(() => runCheckNow());
geozoneWorking.subscribe(scheduleMissionCheck);
mission.subscribe(scheduleMissionCheck);
launchPoint.subscribe(scheduleMissionCheck);
homePosition.subscribe(scheduleMissionCheck);
