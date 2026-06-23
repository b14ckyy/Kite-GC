// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// ArduPilot / PX4 rally points (MAVLink MAV_MISSION_TYPE_RALLY). Companion to the geofence store
// (stores/fence.ts) — RTL divert/return locations. Mirrors the Rust `RallyConfig` (commands/rally.rs).
// `loaded` = last FC snapshot (dirty baseline); `working` = the editable copy (rendered + edited);
// "Save to FC" uploads the working copy + writes the changed params, then re-reads. MAVLink-only.

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { settings } from '$lib/stores/settings';

/** Default altitude (m, relative to home) for a freshly-added rally point. */
const DEFAULT_RALLY_ALT_M = 50;

/** One rally point. lat/lon in degrees × 1e7; alt_cm relative to home. */
export interface RallyPoint {
  lat: number;
  lon: number;
  alt_cm: number;
}

export interface RallyParam { name: string; value: number }

export interface RallyConfig {
  points: RallyPoint[];
  params: RallyParam[];
  /** True when a MAVLink (ArduPilot/PX4) FC is connected — drives the UI's visibility. */
  has_rally: boolean;
}

/** Last snapshot from the FC (dirty baseline). */
export const rallyConfig = writable<RallyConfig | null>(null);
/** Editable working copy — drives the map markers + panel + editing. */
export const rallyWorking = writable<RallyConfig | null>(null);
/** Map edit-lock (markers draggable when true). */
export const rallyEditing = writable<boolean>(false);

export const rallyDirty = derived(
  [rallyConfig, rallyWorking],
  ([$loaded, $working]) =>
    !!$loaded && !!$working && JSON.stringify($loaded) !== JSON.stringify($working),
);

/** Download the rally points from the FC (MAVLink). Always called on connect; null on failure / non-MAVLink. */
export async function loadRallyConfig(): Promise<void> {
  try {
    const cfg = await invoke<RallyConfig>('rally_read_all');
    rallyConfig.set(cfg);
    rallyWorking.set(structuredClone(cfg));
  } catch (e) {
    console.warn('[rally] loadRallyConfig failed', e);
    rallyConfig.set(null);
    rallyWorking.set(null);
  }
}

/** "Save to FC": upload the working copy (points + params), then re-read so loaded == FC truth. */
export async function saveRallyConfig(): Promise<void> {
  const cfg = get(rallyWorking);
  if (!cfg) return;
  await invoke('rally_write_all', { config: cfg });
  await loadRallyConfig();
}

export function revertRallyWorking(): void {
  const loaded = get(rallyConfig);
  rallyWorking.set(loaded ? structuredClone(loaded) : null);
}

export function clearRally(): void {
  rallyConfig.set(null);
  rallyWorking.set(null);
  rallyEditing.set(false);
}

// ── Working-copy mutations (panel + map editing). Points are identified by array index. ─────────────
/** Add a new rally point at the current map centre. Returns its index, or null. */
export function addRallyPoint(): number | null {
  const cfg = get(rallyWorking);
  if (!cfg) return null;
  const [lat, lon] = get(settings).map.center;
  const point: RallyPoint = {
    lat: Math.round(lat * 1e7),
    lon: Math.round(lon * 1e7),
    alt_cm: DEFAULT_RALLY_ALT_M * 100,
  };
  rallyWorking.update((c) => (c ? { ...c, points: [...c.points, point] } : c));
  return cfg.points.length;
}

export function deleteRallyPoint(index: number): void {
  rallyWorking.update((c) => (c ? { ...c, points: c.points.filter((_, i) => i !== index) } : c));
}
export function setRallyPoint(index: number, latE7: number, lonE7: number): void {
  rallyWorking.update((c) =>
    c ? { ...c, points: c.points.map((p, i) => (i === index ? { ...p, lat: latE7, lon: lonE7 } : p)) } : c);
}
export function setRallyAlt(index: number, altCm: number): void {
  rallyWorking.update((c) =>
    c ? { ...c, points: c.points.map((p, i) => (i === index ? { ...p, alt_cm: Math.round(altCm) } : p)) } : c);
}
export function setRallyParam(name: string, value: number): void {
  rallyWorking.update((c) => {
    if (!c) return c;
    const params = c.params.some((p) => p.name === name)
      ? c.params.map((p) => (p.name === name ? { ...p, value } : p))
      : [...c.params, { name, value }];
    return { ...c, params };
  });
}
