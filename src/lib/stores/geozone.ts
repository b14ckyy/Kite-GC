// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV geozone config (read). See docs/active/GEOZONES.md.
//
// Mirrors the Rust `GeozoneConfig` (commands/geozone.rs). `loaded` is the last snapshot read from the
// FC, rendered on the 2D/3D maps + listed in the Airspace Manager panel. Download is always-on at INAV
// connect (the store updates when the reads complete) and cleared on disconnect. Geozones are an INAV
// ≥8.0 feature; on older firmware `has_geozones` is false and `zones` is empty. Editing/writing is
// Phase 2 (a working/dirty copy + Save-to-FC) and not implemented yet.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

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

/** Last snapshot read from the FC — drives the map overlays + the panel list. Null until first read /
 *  when not connected to a (geozone-capable) INAV FC. */
export const geozoneConfig = writable<GeozoneConfig | null>(null);

/** Read the full geozone config from the FC (INAV ≥8.0). Always called on connect (download
 *  always-on). On failure / non-INAV, clears to null. */
export async function loadGeozoneConfig(): Promise<void> {
  try {
    const cfg = await invoke<GeozoneConfig>('geozone_read_all');
    geozoneConfig.set(cfg);
  } catch (e) {
    console.warn('[geozone] loadGeozoneConfig failed', e);
    geozoneConfig.set(null);
  }
}

/** Clear everything (on disconnect). */
export function clearGeozones(): void {
  geozoneConfig.set(null);
}
