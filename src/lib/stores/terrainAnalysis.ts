// Terrain Analysis panel — session state.
//
// In-memory only: a module-level writable survives panel close/reopen but is
// reset when the app process exits (per design — not written to disk).

import { writable } from 'svelte/store';

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
  /** Waypoint (planned mission) vs Track (flown live/blackbox) view */
  viewMode: TerrainViewMode;
  /** Y-axis reference: absolute MSL vs above-ground clearance */
  datum: TerrainDatum;
  /** Ground clearance (m) — target AGL (Terrain Follow) / minimum (Clearance Check) */
  groundClearance: number;
  /** Fixed-wing climb-angle limit (degrees) */
  climbAngleLimit: number;
  /** Apply the fixed-wing climb-angle limit */
  fixedWing: boolean;
  /** Average airspeed (m/s); 0 = off (no climb-rate readout) */
  airspeed: number;
  /** Vertical exaggeration factor (1 = auto-fit) — reserved, UI wired in a later phase */
  vExag: number;
  /** Correction range, by WP display number; 0 = auto (first / last) */
  rangeStart: number;
  rangeEnd: number;
  /** Terrain Follow: insert one WP per offending leg */
  addWaypoints: boolean;
  /** Visible distance window (m); null = full route */
  viewStart: number | null;
  viewEnd: number | null;
}

const INITIAL: TerrainAnalysisState = {
  open: false,
  compact: false,
  viewMode: 'waypoint',
  datum: 'msl',
  groundClearance: 50,
  climbAngleLimit: 12,
  fixedWing: false,
  airspeed: 0,
  vExag: 1,
  rangeStart: 0,
  rangeEnd: 0,
  addWaypoints: false,
  viewStart: null,
  viewEnd: null,
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
