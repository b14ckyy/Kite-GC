// Radar tracking store
// Mirrors the consolidated `radar-vehicles` snapshot emitted by the Rust radar subsystem (foreign
// vehicles: ADS-B / FormationFlight / radio telemetry). Independent of the main telemetry store.
// See docs/active/RADAR_TRACKING_CORE.md / ...PANEL_AND_MAP.md.

import { writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { haversineDistance, bearing } from '$lib/utils/geo';

// ── Types (mirror the Rust serde output) ────────────────────────────

export type VehicleSystem = 'adsb' | 'formationFlight' | 'radio';
export type AltRef = 'baro_msl' | 'geo_msl' | 'relative' | 'unknown';

export interface TrackedVehicle {
  id: string;
  system: VehicleSystem;
  sources: string[];
  callsign: string | null;
  lat: number;
  lon: number;
  altM: number | null;
  altRef: AltRef;
  headingDeg: number | null;
  groundSpeedMs: number | null;
  verticalSpeedMs: number | null;
  category: string | null;
  signal: number | null;
  squawk: number | null;
  lastSeenMs: number;
  validPos: boolean;
  extra?: Record<string, string>;
}

export interface RadarSnapshot {
  adsb: TrackedVehicle[];
  formationFlight: TrackedVehicle[];
  radio: TrackedVehicle[];
  lastUpdate: number;
}

/** A vehicle augmented with values derived from the user's location (frontend-only). */
export interface EnrichedVehicle extends TrackedVehicle {
  /** Great-circle distance from the user (m), or null if no user location. */
  distanceM: number | null;
  /** Bearing from the user to the vehicle (deg, 0–360), or null. */
  bearingDeg: number | null;
}

const EMPTY: RadarSnapshot = { adsb: [], formationFlight: [], radio: [], lastUpdate: 0 };

export const radarVehicles = writable<RadarSnapshot>({ ...EMPTY });

export function resetRadar() {
  radarVehicles.set({ ...EMPTY });
}

// ── Event listener ──────────────────────────────────────────────────

let unlisten: UnlistenFn | undefined;

export async function startRadarListeners() {
  stopRadarListeners();
  unlisten = await listen<RadarSnapshot>('radar-vehicles', (event) => {
    radarVehicles.set(event.payload);
  });
}

export function stopRadarListeners() {
  unlisten?.();
  unlisten = undefined;
}

// ── Backend config push ─────────────────────────────────────────────

export interface RadarBackendConfig {
  enabled: boolean;
  /** Dev-only synthetic source (ignored by release backend). */
  sim: boolean;
  /** `[lat, lon]` centre for the dev sim, or null. */
  simCenter: [number, number] | null;
}

/** Push the radar config to the backend (starts/stops the pipeline). Idempotent. */
export async function configureRadar(config: RadarBackendConfig): Promise<void> {
  try {
    await invoke('radar_configure', { config });
  } catch (e) {
    console.warn('radar_configure failed:', e);
  }
}

// ── Enrichment (distance / bearing from the user) ───────────────────

/** Add user-relative distance/bearing. `user` is the resolved user location, or null. */
export function enrichVehicle(
  v: TrackedVehicle,
  user: { lat: number; lon: number } | null,
): EnrichedVehicle {
  if (!user || !v.validPos) {
    return { ...v, distanceM: null, bearingDeg: null };
  }
  return {
    ...v,
    distanceM: haversineDistance(user.lat, user.lon, v.lat, v.lon),
    bearingDeg: bearing(user.lat, user.lon, v.lat, v.lon),
  };
}

/** Enrich + sort a system's list by distance (nulls last). */
export function enrichList(
  list: TrackedVehicle[],
  user: { lat: number; lon: number } | null,
): EnrichedVehicle[] {
  return list
    .map((v) => enrichVehicle(v, user))
    .sort((a, b) => {
      if (a.distanceM == null) return 1;
      if (b.distanceM == null) return -1;
      return a.distanceM - b.distanceM;
    });
}
