// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Airspace Manager (aeronautical data) frontend store — mirrors the backend `aero_fetch` snapshot and
// holds the per-layer 2D/3D visibility the panel controls. See docs/active/AIRSPACE_MANAGER.md.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

// ── Types (mirror the Rust `AeroData`) ──────────────────────────────
export interface AltLimit {
  valueM: number;
  datum: string; // "gnd" | "msl" | "std"
  label: string; // "FL90", "1600 ft MSL", "0 GND"
}
export interface Airspace {
  id: string;
  name: string;
  typeId: number;
  typeName: string;
  icaoClass: number;
  icaoClassName: string;
  lower: AltLimit;
  upper: AltLimit;
  /** Outer ring(s), each a list of `[lon, lat]`. */
  outlines: [number, number][][];
}
export interface AeroPoint {
  id: string;
  kind: 'obstacle' | 'airport' | 'rc';
  /** Raw OpenAIP type id (obstacle/airport); null for RC. */
  typeId: number | null;
  subtype: string;
  name: string;
  lat: number;
  lon: number;
  elevationM: number | null;
  heightM: number | null;
  extra: Record<string, string>;
}
export interface AeroData {
  airspaces: Airspace[];
  obstacles: AeroPoint[];
  airports: AeroPoint[];
  rcAirfields: AeroPoint[];
}

export interface AeroCacheStats {
  airspaces: number;
  obstacles: number;
  airports: number;
  rcAirfields: number;
  total: number;
  approxBytes: number;
  ageMs: number;
}

const EMPTY: AeroData = { airspaces: [], obstacles: [], airports: [], rcAirfields: [] };

export const aeroData = writable<AeroData>({ ...EMPTY });
export const aeroCacheStats = writable<AeroCacheStats | null>(null);

/** Last feature the panel asked to centre on (the 2D map pans to it). */
export const aeroFocus = writable<{ lat: number; lon: number } | null>(null);
export function focusAero(lat: number, lon: number): void {
  aeroFocus.set({ lat, lon });
}

// Per-layer 2D/3D visibility now lives in the persisted settings store (`settings.airspace.layers`,
// type `AeroLayers`) so it survives a restart — see $lib/stores/settings.ts.
// `geozones` is an INAV FC layer (not OpenAIP data) but shares the panel's 2D/3D toggle grid, so it
// lives in the same key union (and in `AeroLayers`). It is deliberately NOT in `ALL_AERO_LAYERS`, which
// drives the OpenAIP fetch loops.
export type AeroLayerKey = 'airspaces' | 'geozones' | 'obstacles' | 'airports' | 'rc';

/** Layer keys to fetch (the backend understands airspaces/obstacles/airports/rc). */
export const ALL_AERO_LAYERS: AeroLayerKey[] = ['airspaces', 'obstacles', 'airports', 'rc'];

/** Fetch the requested layers for a region (the backend serves from its RAM cache when it can). */
export async function fetchAero(
  provider: string, apiKey: string, lat: number, lon: number, radiusKm: number, layers: string[],
): Promise<void> {
  try {
    const d = await invoke<AeroData>('aero_fetch', { provider, apiKey, lat, lon, radiusKm, layers });
    aeroData.set(d);
    void refreshAeroCacheStats();
  } catch (e) {
    console.warn('aero_fetch failed:', e);
  }
}

export async function refreshAeroCacheStats(): Promise<void> {
  try { aeroCacheStats.set(await invoke<AeroCacheStats>('aero_cache_stats')); } catch { /* ignore */ }
}

export async function clearAeroCache(): Promise<void> {
  try { await invoke('aero_cache_clear'); } catch { /* ignore */ }
  aeroData.set({ ...EMPTY });
  void refreshAeroCacheStats();
}
