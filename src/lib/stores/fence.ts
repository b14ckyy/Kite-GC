// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// ArduPilot / PX4 geofence (MAVLink MAV_MISSION_TYPE_FENCE). See docs/active/GEOFENCE.md. Mirrors the
// Rust `FenceConfig` (commands/fence.rs) and the geozone store's shape so the map editor can be shared.
// `loaded` = last FC snapshot (dirty baseline); `working` = the editable copy (rendered + edited);
// "Save to FC" uploads the working copy + writes the changed params, then re-reads. MAVLink-only.

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { settings } from '$lib/stores/settings';

export const FENCE_KIND_INCLUSION = 0;
export const FENCE_KIND_EXCLUSION = 1;
export const FENCE_SHAPE_POLYGON = 0;
export const FENCE_SHAPE_CIRCLE = 1;

/** Earth circumference (m) — web-mercator tile sizing for a freshly-added zone (shared idea w/ geozones). */
const EARTH_CIRCUMFERENCE_M = 40075016.686;
const DEFAULT_POLY_TILES = { halfH: 1.5, wBottom: 1.0, wTop: 0.6 };
const DEFAULT_CIRCLE_TILES = 2;
function tileWidthM(lat: number, zoom: number): number {
  return (EARTH_CIRCUMFERENCE_M * Math.cos((lat * Math.PI) / 180)) / 2 ** zoom;
}

export interface FenceVertex { lat: number; lon: number }

/** One fence region. `kind` 0 = inclusion, 1 = exclusion. `shape` 0 = polygon, 1 = circle (centre +
 *  `radius_cm`). Fences have no per-zone altitude/action (those are global params). */
export interface FenceZone {
  kind: number;
  shape: number;
  radius_cm: number | null;
  vertices: FenceVertex[];
}

export interface FenceParam { name: string; value: number }

export interface FenceConfig {
  zones: FenceZone[];
  return_point: FenceVertex | null;
  params: FenceParam[];
  /** True when a MAVLink (ArduPilot/PX4) FC is connected — drives the UI's visibility. */
  has_fence: boolean;
}

/** Last snapshot from the FC (dirty baseline). */
export const fenceConfig = writable<FenceConfig | null>(null);
/** Editable working copy — drives the map overlay + panel + editing. */
export const fenceWorking = writable<FenceConfig | null>(null);
/** Map edit-lock (handles editable when true). */
export const fenceEditing = writable<boolean>(false);

export const fenceDirty = derived(
  [fenceConfig, fenceWorking],
  ([$loaded, $working]) =>
    !!$loaded && !!$working && JSON.stringify($loaded) !== JSON.stringify($working),
);

/** Download the fence from the FC (MAVLink). Always called on connect; null on failure / non-MAVLink. */
export async function loadFenceConfig(): Promise<void> {
  try {
    const cfg = await invoke<FenceConfig>('fence_read_all');
    fenceConfig.set(cfg);
    fenceWorking.set(structuredClone(cfg));
  } catch (e) {
    console.warn('[fence] loadFenceConfig failed', e);
    fenceConfig.set(null);
    fenceWorking.set(null);
  }
}

/** "Save to FC": upload the working copy (geometry + params), then re-read so loaded == FC truth. */
export async function saveFenceConfig(): Promise<void> {
  const cfg = get(fenceWorking);
  if (!cfg) return;
  await invoke('fence_write_all', { config: cfg });
  await loadFenceConfig();
}

export function revertFenceWorking(): void {
  const loaded = get(fenceConfig);
  fenceWorking.set(loaded ? structuredClone(loaded) : null);
}

export function clearFence(): void {
  fenceConfig.set(null);
  fenceWorking.set(null);
  fenceEditing.set(false);
}

// ── Working-copy mutations (panel + map editing). Zones are identified by array index. ──────────────
function updateZone(index: number, fn: (z: FenceZone) => FenceZone): void {
  fenceWorking.update((c) => (c ? { ...c, zones: c.zones.map((z, i) => (i === index ? fn(z) : z)) } : c));
}

/** Add a new fence zone (kind × shape) at the map centre, sized to the current zoom (like geozones). */
export function addFenceZone(kind: number, shape: number): number | null {
  const cfg = get(fenceWorking);
  if (!cfg) return null;
  const map = get(settings).map;
  const [lat, lon] = map.center;
  const tw = tileWidthM(lat, map.zoom);
  const latRad = (lat * Math.PI) / 180;
  const dLatE7 = (m: number) => Math.round((m / 111320) * 1e7);
  const dLonE7 = (m: number) => Math.round((m / (111320 * Math.cos(latRad))) * 1e7);
  const latE7 = Math.round(lat * 1e7), lonE7 = Math.round(lon * 1e7);

  let zone: FenceZone;
  if (shape === FENCE_SHAPE_CIRCLE) {
    zone = { kind, shape, radius_cm: Math.round(DEFAULT_CIRCLE_TILES * tw * 100), vertices: [{ lat: latE7, lon: lonE7 }] };
  } else {
    const { halfH, wBottom, wTop } = DEFAULT_POLY_TILES;
    zone = {
      kind, shape, radius_cm: null,
      vertices: [
        { lat: latE7 - dLatE7(halfH * tw), lon: lonE7 - dLonE7(wBottom * tw) },
        { lat: latE7 - dLatE7(halfH * tw), lon: lonE7 + dLonE7(wBottom * tw) },
        { lat: latE7 + dLatE7(halfH * tw), lon: lonE7 + dLonE7(wTop * tw) },
        { lat: latE7 + dLatE7(halfH * tw), lon: lonE7 - dLonE7(wTop * tw) },
      ],
    };
  }
  fenceWorking.update((c) => (c ? { ...c, zones: [...c.zones, zone] } : c));
  return cfg.zones.length; // index of the new zone
}

export function deleteFenceZone(index: number): void {
  fenceWorking.update((c) => (c ? { ...c, zones: c.zones.filter((_, i) => i !== index) } : c));
}
export function setFenceKind(index: number, kind: number): void {
  updateZone(index, (z) => ({ ...z, kind }));
}
export function setFenceVertex(index: number, vi: number, latE7: number, lonE7: number): void {
  updateZone(index, (z) => ({ ...z, vertices: z.vertices.map((v, i) => (i === vi ? { lat: latE7, lon: lonE7 } : v)) }));
}
export function insertFenceVertex(index: number, afterVi: number, latE7: number, lonE7: number): void {
  updateZone(index, (z) => {
    if (z.shape === FENCE_SHAPE_CIRCLE) return z;
    const vertices = z.vertices.slice();
    vertices.splice(afterVi + 1, 0, { lat: latE7, lon: lonE7 });
    return { ...z, vertices };
  });
}
export function removeFenceVertex(index: number, vi: number): void {
  updateZone(index, (z) => {
    if (z.shape === FENCE_SHAPE_CIRCLE || z.vertices.length <= 3) return z;
    return { ...z, vertices: z.vertices.filter((_, i) => i !== vi) };
  });
}
export function setFenceRadius(index: number, radiusCm: number): void {
  updateZone(index, (z) => ({ ...z, radius_cm: Math.max(0, Math.round(radiusCm)) }));
}
export function setFenceParam(name: string, value: number): void {
  fenceWorking.update((c) => {
    if (!c) return c;
    const params = c.params.some((p) => p.name === name)
      ? c.params.map((p) => (p.name === name ? { ...p, value } : p))
      : [...c.params, { name, value }];
    return { ...c, params };
  });
}
