// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Terrain Analysis panel — session state.
//
// In-memory only: a module-level writable survives panel close/reopen but is
// reset when the app process exits (per design — not written to disk).

import { writable } from 'svelte/store';
import type { RfRay } from '$lib/helpers/rfLink';

export type TerrainViewMode = 'waypoint' | 'track';
export type TerrainDatum = 'msl' | 'agl';

export interface LatLng {
  lat: number;
  lon: number;
}

export interface TerrainAnalysisState {
  /** Overlay visible */
  open: boolean;
  /** Compact mode: short top-docked strip so the map stays visible, with the
   *  chart cursor / placed marker mirrored onto the map */
  compact: boolean;
  /** Live Track mode: view follows the newest data (right edge); else free pan */
  follow: boolean;
  /** Waypoint (planned mission) vs Track (flown live/blackbox) view */
  viewMode: TerrainViewMode;
  /** Y-axis reference: absolute MSL vs above-ground clearance */
  datum: TerrainDatum;
  /** Ground clearance (m) — target AGL (Terrain Follow) / minimum (Clearance Check) */
  groundClearance: number;
  /** Terrain Correction sub-panel expanded (shows live preview) */
  correctionEnabled: boolean;
  /** Correction method */
  correctionMode: 'follow' | 'check';
  /** Fixed-wing climb-angle limit (degrees, 0 = off) */
  climbAngleLimit: number;
  /** Fixed-wing descent-angle limit (degrees, 0 = off) */
  descentAngleLimit: number;
  /** Apply the fixed-wing climb/descent-angle limits */
  fixedWing: boolean;
  /** Average airspeed (m/s); 0 = off (no climb-rate readout) */
  airspeed: number;
  /** Vertical exaggeration factor (1 = auto-fit) — reserved, UI wired in a later phase */
  vExag: number;
  /** Correction range, by WP display number; 0 = auto (first / last) */
  rangeStart: number;
  rangeEnd: number;
  /** Visible distance window (m); null = full route */
  viewStart: number | null;
  viewEnd: number | null;
  /** RF analysis — geometric LOS occlusion (naïve; ignored when rfFresnel is on) */
  rfLos: boolean;
  /** RF analysis — Fresnel-zone / knife-edge diffraction loss */
  rfFresnel: boolean;
  /** RF analysis — two-ray ground-reflection lobing (signed) */
  rfTworay: boolean;
  /** RF analysis — active band (MHz key): '5800' | '2400' | '900' | '433' */
  rfBand: '5800' | '2400' | '900' | '433';
  /** RF analysis — clutter/vegetation height (m) added to terrain for the obstacle analysis */
  rfClutterM: number;
  /** Show the logged-RSSI line in Track mode (toggle; auto-disabled in Waypoint mode / no RSSI) */
  rfShowRssi: boolean;
}

const INITIAL: TerrainAnalysisState = {
  open: false,
  compact: false,
  follow: true,
  viewMode: 'waypoint',
  datum: 'msl',
  groundClearance: 50,
  correctionEnabled: false,
  correctionMode: 'follow',
  climbAngleLimit: 12,
  descentAngleLimit: 8,
  fixedWing: false,
  airspeed: 0,
  vExag: 1,
  rangeStart: 0,
  rangeEnd: 0,
  viewStart: null,
  viewEnd: null,
  rfLos: false,
  rfFresnel: false,
  rfTworay: false,
  rfBand: '900',
  rfClutterM: 10,
  rfShowRssi: true,
};

export const terrainAnalysis = writable<TerrainAnalysisState>({ ...INITIAL });

export function patchTerrainAnalysis(patch: Partial<TerrainAnalysisState>): void {
  terrainAnalysis.update((s) => ({ ...s, ...patch }));
}

/** Reset only the zoom/pan window (e.g. on data change or "reset view"). */
export function resetTerrainView(): void {
  terrainAnalysis.update((s) => ({ ...s, viewStart: null, viewEnd: null }));
}

// ── Chart ↔ map cursor link ─────────────────────────────────────────
// `hover` follows the mouse over the profile (transient); `placed` is a marker
// the user pinned by clicking (persists even when the panel is closed, so it
// stays as a reference on the map while editing in mission control).

export const terrainCursor = writable<{ hover: LatLng | null; placed: LatLng | null }>({
  hover: null,
  placed: null,
});

export function setTerrainHover(p: LatLng | null): void {
  terrainCursor.update((s) => (s.hover === p ? s : { ...s, hover: p }));
}

/** Toggle the pinned marker: pin at `p` if none set, else clear it. */
export function toggleTerrainPlaced(p: LatLng): void {
  terrainCursor.update((s) => ({ ...s, placed: s.placed ? null : p }));
}

export function clearTerrainHover(): void {
  terrainCursor.update((s) => (s.hover ? { ...s, hover: null } : s));
}

export function clearTerrainPlaced(): void {
  terrainCursor.update((s) => (s.placed ? { ...s, placed: null } : s));
}

// ── RF map overlay ──────────────────────────────────────────────────
// Critical-point ray triangles (one per occupied 1° azimuth bin) published by the RF analysis for the
// 2D map overlay. Empty unless an RF method is active. The map gates display on Show-Map (compact) mode.
export const terrainRfRays = writable<RfRay[]>([]);

export function setTerrainRfRays(rays: RfRay[]): void {
  terrainRfRays.set(rays);
}
