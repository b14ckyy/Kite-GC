// Mission Planning Store
// Frontend state for INAV waypoint missions.
// Mirrors the Rust MissionStore, synced via Tauri invoke commands.

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

// ── Types (mirror Rust mission::types) ──────────────────────────────

export enum WpAction {
  Waypoint = 1,
  PosholdUnlim = 2,
  PosholdTime = 3,
  Rth = 4,
  SetPoi = 5,
  Jump = 6,
  SetHead = 7,
  Land = 8,
}

export const WP_ACTION_LABELS: Record<WpAction, string> = {
  [WpAction.Waypoint]: 'Waypoint',
  [WpAction.PosholdUnlim]: 'PosHold ∞',
  [WpAction.PosholdTime]: 'PosHold Time',
  [WpAction.Rth]: 'RTH',
  [WpAction.SetPoi]: 'Set POI',
  [WpAction.Jump]: 'Jump',
  [WpAction.SetHead]: 'Set Head',
  [WpAction.Land]: 'Land',
};

/** i18n translation keys for WP actions — use with $t() in .svelte files */
export const WP_ACTION_KEYS: Record<WpAction, string> = {
  [WpAction.Waypoint]: 'wpAction.waypoint',
  [WpAction.PosholdUnlim]: 'wpAction.posholdUnlim',
  [WpAction.PosholdTime]: 'wpAction.posholdTime',
  [WpAction.Rth]: 'wpAction.rth',
  [WpAction.SetPoi]: 'wpAction.setPoi',
  [WpAction.Jump]: 'wpAction.jump',
  [WpAction.SetHead]: 'wpAction.setHead',
  [WpAction.Land]: 'wpAction.land',
};

export const WP_FLAG_NORMAL = 0x00;
export const WP_FLAG_LAST = 0xa5;
export const WP_FLAG_FBH = 0x48;

/** Altitude reference mode (GCS-side). INAV only knows REL/AMSL; AGL is a
 *  planning-only mode resolved to AMSL on export. */
export const ALT_MODE_REL = 0;
export const ALT_MODE_AMSL = 1;
export const ALT_MODE_AGL = 2;

export interface Waypoint {
  number: number;
  action: WpAction;
  lat: number;
  lon: number;
  altitude: number;
  p1: number;
  p2: number;
  p3: number;
  flag: number;
  /** 0=REL, 1=AMSL, 2=AGL. For REL/AMSL mirrors p3 bit0. */
  alt_mode?: number;
}

export interface MissionInfo {
  max_waypoints: number;
  is_valid: boolean;
  wp_count: number;
}

export interface Mission {
  waypoints: Waypoint[];
  info: MissionInfo;
  dirty: boolean;
  /** Planned home/launch point from the .mission <mwp> meta (lat/lon degrees). */
  home?: { lat: number; lon: number };
}

// ── Helpers ─────────────────────────────────────────────────────────

/** Whether a WP action has a geographic position */
export function hasLocation(action: WpAction): boolean {
  return [WpAction.Waypoint, WpAction.PosholdUnlim, WpAction.PosholdTime, WpAction.SetPoi, WpAction.Land].includes(action);
}

/** Whether a WP action is a modifier (no position) */
export function isModifier(action: WpAction): boolean {
  return [WpAction.Jump, WpAction.Rth, WpAction.SetHead].includes(action);
}

/** Convert lat/lon * 1e7 integer to degrees */
export function toDeg(val: number): number {
  return val / 1e7;
}

/** Convert degrees to lat/lon * 1e7 integer */
export function fromDeg(deg: number): number {
  return Math.round(deg * 1e7);
}

/** Altitude from cm to metres */
export function altToM(cm: number): number {
  return cm / 100;
}

/** Altitude from metres to cm */
export function altFromM(m: number): number {
  return Math.round(m * 100);
}

// ── Store ───────────────────────────────────────────────────────────

function createEmptyMission(): Mission {
  return {
    waypoints: [],
    info: { max_waypoints: 0, is_valid: false, wp_count: 0 },
    dirty: false,
  };
}

export const mission = writable<Mission>(createEmptyMission());

/** Geo-waypoints only (for map markers) */
export const geoWaypoints = derived(mission, ($m) =>
  $m.waypoints.filter((wp) => hasLocation(wp.action))
);

/** Currently selected waypoint index (-1 = none) */
export const selectedWpIndex = writable<number>(-1);

/** Edit mode toggle */
export const editMode = writable<boolean>(false);

/** Planning-time launch/home reference point (GCS-side). Used as the base
 *  altitude for REL↔AGL conversions and terrain clearance of REL missions.
 *  Auto-placed when entering edit mode; user-movable. null = not set. */
export interface LaunchPoint { lat: number; lng: number; }
export const launchPoint = writable<LaunchPoint | null>(null);

// ── Multi-Mission Management ────────────────────────────────────────
// INAV multi-mission: up to 9 missions stored as sequential WP list on FC,
// each terminated by flag 0xa5. Max 120 WPs total across all missions.

export const MAX_MISSIONS = 9;
export const MAX_WAYPOINTS_TOTAL = 120;

/** 1-based active mission index */
export const activeMissionIndex = writable<number>(1);

/** Number of mission slots currently in use (starts at 1) */
export const missionCount = writable<number>(1);

/** Cached WP arrays for non-active mission slots (1-based index → waypoints) */
const missionSlots: Map<number, Waypoint[]> = new Map();

/** Total WP count across all missions */
export const totalWpCount = derived([mission, missionCount], ([$m, $count]) => {
  let total = $m.waypoints.length;
  for (const [idx, wps] of missionSlots) {
    if (idx !== get(activeMissionIndex)) {
      total += wps.length;
    }
  }
  return total;
});

/** Save current mission WPs to the slot cache */
function saveCurrentToSlot() {
  const idx = get(activeMissionIndex);
  const m = get(mission);
  missionSlots.set(idx, [...m.waypoints]);
}

/** Switch to a different mission tab */
export async function switchMission(newIdx: number): Promise<void> {
  const currentIdx = get(activeMissionIndex);
  if (newIdx === currentIdx) return;

  // Save current mission to slot cache
  saveCurrentToSlot();

  // Load target mission from slot cache (or empty)
  const targetWps = missionSlots.get(newIdx) || [];

  // Clear backend and re-add target WPs
  await invoke<Mission>('mission_clear');
  for (const wp of targetWps) {
    await invoke<Mission>('mission_add_wp', {
      action: wp.action, lat: wp.lat, lon: wp.lon,
      altitude: wp.altitude, p1: wp.p1, p2: wp.p2, p3: wp.p3,
    });
  }
  const m = await invoke<Mission>('mission_get');
  mission.set(m);
  activeMissionIndex.set(newIdx);
  selectedWpIndex.set(-1);
}

/** Add a new mission tab. Returns new mission index, or -1 if at limit. */
export function addMission(): number {
  const count = get(missionCount);
  if (count >= MAX_MISSIONS) return -1;
  const newIdx = count + 1;
  missionSlots.set(newIdx, []);
  missionCount.set(newIdx);
  return newIdx;
}

/** Remove a mission tab and its data. Returns the new active index. */
export async function removeMission(idx: number): Promise<number> {
  const count = get(missionCount);
  if (count <= 1) {
    // Last mission — just clear it
    await missionClear();
    return 1;
  }

  const currentIdx = get(activeMissionIndex);

  // Save current state before modifying slots
  if (idx !== currentIdx) {
    saveCurrentToSlot();
  }

  // Remove the target slot and shift higher slots down
  missionSlots.delete(idx);
  for (let i = idx; i < count; i++) {
    const wps = missionSlots.get(i + 1);
    if (wps) {
      missionSlots.set(i, wps);
      missionSlots.delete(i + 1);
    }
  }

  const newCount = count - 1;
  missionCount.set(newCount);

  // Determine new active index
  let newActive: number;
  if (idx === currentIdx) {
    newActive = Math.min(idx, newCount);
  } else if (currentIdx > idx) {
    newActive = currentIdx - 1;
  } else {
    newActive = currentIdx;
  }

  // Load the new active mission into the backend
  const targetWps = missionSlots.get(newActive) || [];
  await invoke<Mission>('mission_clear');
  for (const wp of targetWps) {
    await invoke<Mission>('mission_add_wp', {
      action: wp.action, lat: wp.lat, lon: wp.lon,
      altitude: wp.altitude, p1: wp.p1, p2: wp.p2, p3: wp.p3,
    });
  }
  const m = await invoke<Mission>('mission_get');
  mission.set(m);
  activeMissionIndex.set(newActive);
  selectedWpIndex.set(-1);
  return newActive;
}

/** Get total WP count across all missions (synchronous) */
export function getTotalWpCount(): number {
  let total = get(mission).waypoints.length;
  const currentIdx = get(activeMissionIndex);
  for (const [idx, wps] of missionSlots) {
    if (idx !== currentIdx) {
      total += wps.length;
    }
  }
  return total;
}

/** Reset multi-mission state (e.g., after file load or FC download) */
export function resetMultiMission(): void {
  missionSlots.clear();
  missionCount.set(1);
  activeMissionIndex.set(1);
}

/** Reset in-memory mission without touching the FC (used when switching autopilot systems) */
export function missionResetMemory(): void {
  mission.set(createEmptyMission());
}

// ── Backend Sync ────────────────────────────────────────────────────

/** Fetch current mission from backend */
export async function missionGet(): Promise<Mission> {
  const m = await invoke<Mission>('mission_get');
  mission.set(m);
  return m;
}

/** Clear mission */
export async function missionClear(): Promise<void> {
  const m = await invoke<Mission>('mission_clear');
  mission.set(createEmptyMission());
}

/** Add a waypoint */
export async function missionAddWp(
  action: WpAction,
  lat: number,
  lon: number,
  altitude: number,
  p1 = 0,
  p2 = 0,
  p3 = 0,
  altMode?: number
): Promise<Mission> {
  const m = await invoke<Mission>('mission_add_wp', {
    action, lat, lon, altitude, p1, p2, p3, altMode,
  });
  mission.set(m);
  return m;
}

/** Insert a waypoint at index */
export async function missionInsertWp(
  index: number,
  action: WpAction,
  lat: number,
  lon: number,
  altitude: number,
  p1 = 0,
  p2 = 0,
  p3 = 0,
  altMode?: number
): Promise<Mission> {
  const m = await invoke<Mission>('mission_insert_wp', {
    index, action, lat, lon, altitude, p1, p2, p3, altMode,
  });
  mission.set(m);
  return m;
}

/** Remove a waypoint by index */
export async function missionRemoveWp(index: number): Promise<Mission> {
  const m = await invoke<Mission>('mission_remove_wp', { index });
  mission.set(m);
  return m;
}

/** Update a waypoint at index */
export async function missionUpdateWp(
  index: number,
  wp: Waypoint
): Promise<Mission> {
  const m = await invoke<Mission>('mission_update_wp', {
    index,
    action: wp.action,
    lat: wp.lat,
    lon: wp.lon,
    altitude: wp.altitude,
    p1: wp.p1,
    p2: wp.p2,
    p3: wp.p3,
    flag: wp.flag,
    altMode: wp.alt_mode,
  });
  mission.set(m);
  return m;
}

/** Reorder a waypoint */
export async function missionReorderWp(from: number, to: number): Promise<Mission> {
  const m = await invoke<Mission>('mission_reorder_wp', { from, to });
  mission.set(m);
  return m;
}

/** Download mission from FC */
export async function missionDownload(fromEeprom = false): Promise<Mission> {
  const m = await invoke<Mission>('mission_download', { fromEeprom });
  mission.set(m);
  return m;
}

/** Upload mission to FC */
export async function missionUpload(saveEeprom = false): Promise<Mission> {
  const m = await invoke<Mission>('mission_upload', { saveEeprom });
  mission.set(m);
  return m;
}

/** Current launch point as [lat, lon] for export, or undefined if unset. */
function launchHomeArg(): [number, number] | undefined {
  const lp = get(launchPoint);
  return lp ? [lp.lat, lp.lng] : undefined;
}

/** Apply a loaded mission's <mwp> home to the launch-point store, if present. */
function applyLoadedHome(m: Mission) {
  if (m.home) launchPoint.set({ lat: m.home.lat, lng: m.home.lon });
}

/** Export mission as XML string (includes the launch point as <mwp> meta) */
export async function missionExportXml(): Promise<string> {
  return invoke<string>('mission_export_xml', { home: launchHomeArg() });
}

/** Import mission from XML string */
export async function missionImportXml(xml: string): Promise<Mission> {
  const m = await invoke<Mission>('mission_import_xml', { xml });
  mission.set(m);
  applyLoadedHome(m);
  return m;
}

/** Save mission to a .mission file (writes the launch point as <mwp> meta) */
export async function missionSaveFile(path: string): Promise<void> {
  await invoke<void>('mission_save_file', { path, home: launchHomeArg() });
}

/** Load mission from a .mission file */
export async function missionLoadFile(path: string): Promise<Mission> {
  const m = await invoke<Mission>('mission_load_file', { path });
  mission.set(m);
  applyLoadedHome(m);
  return m;
}
