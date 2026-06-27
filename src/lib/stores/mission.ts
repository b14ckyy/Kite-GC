// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mission Planning Store
// Frontend state for INAV waypoint missions.
// Mirrors the Rust MissionStore, synced via Tauri invoke commands.

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { connection } from './connection';
import { homePosition } from './home';
import { frameMissionOnMap } from './mapCamera';

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

/** DB id of the currently loaded/imported library mission (null = fresh / never saved).
 *  Set on DB-load, and on import when the content hash matches an existing row; drives the
 *  NEW vs OVERWRITE decision on save, and is the link target for arm-time recording saves.
 *  See docs/archive/MISSION_LIBRARY_AND_DB.md. */
export const loadedMissionId = writable<number | null>(null);

/** Geo-waypoints only (for map markers) */
export const geoWaypoints = derived(mission, ($m) =>
  $m.waypoints.filter((wp) => hasLocation(wp.action))
);

/** Primary selected waypoint index (-1 = none / multi-select). Drives the
 *  single-WP editor (panel detail + map bubble). */
export const selectedWpIndex = writable<number>(-1);

/** Multi-selection of waypoint indices (edit-mode multi-select / batch ops).
 *  The primary (`selectedWpIndex`) is the sole member when size === 1, else -1. */
export const selectedWpIndices = writable<Set<number>>(new Set());

function syncPrimary(s: Set<number>): void {
  selectedWpIndex.set(s.size === 1 ? (s.values().next().value as number) : -1);
}

/** Replace the selection with a single WP (also the editor primary). */
export function selectWpSingle(i: number): void {
  selectedWpIndices.set(new Set([i]));
  selectedWpIndex.set(i);
}

/** Toggle a WP in/out of the multi-selection (Ctrl-click / circle tap / map tap). */
export function toggleWpSelection(i: number): void {
  selectedWpIndices.update((s) => {
    const n = new Set(s);
    if (n.has(i)) n.delete(i);
    else n.add(i);
    syncPrimary(n);
    return n;
  });
}

/** Select an inclusive index range (Shift-click). */
export function selectWpRange(a: number, b: number): void {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  const n = new Set<number>();
  for (let i = lo; i <= hi; i++) n.add(i);
  selectedWpIndices.set(n);
  syncPrimary(n);
}

/** Clear the entire selection. */
export function clearWpSelection(): void {
  selectedWpIndices.set(new Set());
  selectedWpIndex.set(-1);
}

/** Batch-remove all selected waypoints (descending so indices stay valid). */
export async function removeSelectedWps(): Promise<void> {
  const ids = [...get(selectedWpIndices)].sort((a, b) => b - a);
  if (ids.length === 0) return;
  beginUndoGroup();
  try {
    for (const idx of ids) await missionRemoveWp(idx);
  } finally {
    endUndoGroup();
  }
  clearWpSelection();
}

/** Edit mode toggle */
export const editMode = writable<boolean>(false);

/** Whether the app is in blackbox-replay mode (mirrors +page `showPlayer`).
 *  In replay the mission visibility follows `showMission`; outside replay
 *  (planning / live) a loaded mission is always shown. */
export const replayActive = writable<boolean>(false);

/** Replay-only toggle for showing the loaded mission on the maps (2D + 3D).
 *  Only consulted while `replayActive`; default on. */
export const showMission = writable<boolean>(true);

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

// ── Mission provenance flags (FC / FILE / DB) ───────────────────────
// A flag is "valid" while the active mission's content still matches the snapshot
// captured at its sync event — so any edit invalidates the flags and Undo back to a
// synced state restores them, with no per-mutation bookkeeping.
// See docs/active/MISSION_TRACKING_AND_PROVENANCE.md.

export type MissionFlag = 'fc' | 'file' | 'db';

/** Stable content hash of a waypoint list (the WP `number` is positional → excluded).
 *  Exported so the mission library can derive the same identity (SHA-256 of this) — keeping
 *  the DB's `content_hash` consistent with provenance. See helpers/missionLibrary.ts. */
export function hashWaypoints(wps: Waypoint[]): string {
  return JSON.stringify(
    wps.map((w) => [w.action, w.lat, w.lon, w.altitude, w.p1, w.p2, w.p3, w.flag, w.alt_mode ?? 0]),
  );
}

/** Per-slot sync snapshots: slot index → { flag → content hash at sync time }. */
const syncRefs: Map<number, Partial<Record<MissionFlag, string>>> = new Map();
const provVersion = writable(0);
const bumpProv = () => provVersion.update((n) => n + 1);

/** Record the active mission as synced via `flag` (captures current content). For FC it
 *  also clears FC on every other slot — the FC then holds only this mission. */
export function markMissionSynced(flag: MissionFlag): void {
  const idx = get(activeMissionIndex);
  const h = hashWaypoints(get(mission).waypoints);
  if (flag === 'fc') {
    for (const [slot, refs] of syncRefs) if (slot !== idx) delete refs.fc;
  }
  const refs = syncRefs.get(idx) ?? {};
  refs[flag] = h;
  syncRefs.set(idx, refs);
  bumpProv();
}

/** Drop the FC flag from all slots (FC link gone). */
export function clearFcFlags(): void {
  for (const refs of syncRefs.values()) delete refs.fc;
  bumpProv();
}

export interface MissionFlags { fc: boolean; file: boolean; db: boolean; }

/** Live provenance flags for the *active* mission (content-compared to the snapshots). */
export const missionFlags = derived(
  [mission, activeMissionIndex, provVersion],
  ([$m, $idx]): MissionFlags => {
    const refs = syncRefs.get($idx);
    if (!refs || $m.waypoints.length === 0) return { fc: false, file: false, db: false };
    const h = hashWaypoints($m.waypoints);
    return { fc: refs.fc === h, file: refs.file === h, db: refs.db === h };
  },
);

/** "Modified" = the mission has content but matches none of its sync snapshots (unsaved
 *  relative to FC/FILE/DB). Content-based, so it clears again on Undo — unlike the old
 *  backend `dirty` flag, which stayed set after an edit+undo. */
export const missionModified = derived([mission, missionFlags], ([$m, $f]) =>
  $m.waypoints.length > 0 && !$f.fc && !$f.file && !$f.db,
);

// The FC flag is only valid while connected — drop it on any disconnect.
let prevConnStatus = 'disconnected';
connection.subscribe((c) => {
  if (prevConnStatus === 'connected' && c.status !== 'connected') clearFcFlags();
  prevConnStatus = c.status;
});

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
  clearWpSelection();
  frameMissionOnMap(); // switching the active multi-mission reframes the map (free pan only)
}

/** Add a new mission tab. Returns new mission index, or -1 if at limit. */
export function addMission(): number {
  const count = get(missionCount);
  if (count >= MAX_MISSIONS) return -1;
  pushUndo();
  const newIdx = count + 1;
  missionSlots.set(newIdx, []);
  missionCount.set(newIdx);
  return newIdx;
}

/** Remove a mission tab and its data. Returns the new active index. */
export async function removeMission(idx: number): Promise<number> {
  const count = get(missionCount);
  beginUndoGroup();
  try {
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
  clearWpSelection();
  return newActive;
  } finally {
    endUndoGroup();
  }
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
  loadedMissionId.set(null);
  clearUndoHistory();
}

/** Reset in-memory mission without touching the FC (used when switching autopilot systems) */
export function missionResetMemory(): void {
  mission.set(createEmptyMission());
  loadedMissionId.set(null);
  clearUndoHistory();
}

// ── Undo / Redo ─────────────────────────────────────────────────────
// Snapshot-based history covering ALL missions (active + cached slots) so
// cross-mission edits like "Move to Mission N" are undoable. The launch point
// is intentionally excluded — it is not part of what gets uploaded to the FC.
//
// One snapshot = one user action. The primitive mutators (add/insert/remove/
// update/reorder/clear) record a step automatically via pushUndo(). Multi-step
// actions (batch edit/delete, move, pattern append, terrain correction) wrap
// their primitives in beginUndoGroup()/endUndoGroup() so the whole action
// collapses into a single undo step.

interface MissionSnapshot {
  activeIdx: number;
  count: number;
  /** 1-based mission index → deep-copied waypoints, for every slot in use */
  missions: Map<number, Waypoint[]>;
}

const UNDO_LIMIT = 50;
let undoStack: MissionSnapshot[] = [];
let redoStack: MissionSnapshot[] = [];
/** >0 while a group/restore is running — suppresses primitive auto-recording. */
let undoSuspend = 0;

/** Reactive flags for the toolbar buttons / shortcut availability. */
export const canUndo = writable<boolean>(false);
export const canRedo = writable<boolean>(false);
function refreshUndoFlags(): void {
  canUndo.set(undoStack.length > 0);
  canRedo.set(redoStack.length > 0);
}

function cloneWps(wps: Waypoint[]): Waypoint[] {
  return wps.map((w) => ({ ...w }));
}

/** Capture the full multi-mission state. The active mission is read live from
 *  the `mission` store; the rest come from the slot cache. */
function captureState(): MissionSnapshot {
  const activeIdx = get(activeMissionIndex);
  const count = get(missionCount);
  const missions = new Map<number, Waypoint[]>();
  for (let i = 1; i <= count; i++) {
    const wps = i === activeIdx ? get(mission).waypoints : (missionSlots.get(i) ?? []);
    missions.set(i, cloneWps(wps));
  }
  return { activeIdx, count, missions };
}

/** Restore a captured state to the slot cache + backend (active mission). */
async function restoreState(snap: MissionSnapshot): Promise<void> {
  undoSuspend++; // restoring must not record further steps
  try {
    missionSlots.clear();
    for (const [i, wps] of snap.missions) missionSlots.set(i, cloneWps(wps));
    missionCount.set(snap.count);
    activeMissionIndex.set(snap.activeIdx);
    const activeWps = snap.missions.get(snap.activeIdx) ?? [];
    await missionSetWaypoints(cloneWps(activeWps));
    clearWpSelection();
  } finally {
    undoSuspend--;
  }
}

/** Push the CURRENT state onto the undo stack (call BEFORE a mutation). No-op
 *  while a group/restore is in progress. Clears the redo stack. */
export function pushUndo(): void {
  if (undoSuspend > 0) return;
  undoStack.push(captureState());
  if (undoStack.length > UNDO_LIMIT) undoStack.shift();
  redoStack = [];
  refreshUndoFlags();
}

/** Begin a grouped action: records one snapshot, then suspends primitive
 *  auto-recording so all inner mutations collapse into that single step. */
export function beginUndoGroup(): void {
  pushUndo();
  undoSuspend++;
}
/** End a grouped action (must be paired with beginUndoGroup). */
export function endUndoGroup(): void {
  if (undoSuspend > 0) undoSuspend--;
}

/** Undo the last recorded action. */
export async function undo(): Promise<void> {
  if (undoStack.length === 0) return;
  redoStack.push(captureState());
  const snap = undoStack.pop() as MissionSnapshot;
  await restoreState(snap);
  refreshUndoFlags();
}

/** Redo the last undone action. */
export async function redo(): Promise<void> {
  if (redoStack.length === 0) return;
  undoStack.push(captureState());
  const snap = redoStack.pop() as MissionSnapshot;
  await restoreState(snap);
  refreshUndoFlags();
}

/** Wipe the history (fresh baseline after a load/download/import). */
export function clearUndoHistory(): void {
  undoStack = [];
  redoStack = [];
  undoSuspend = 0;
  refreshUndoFlags();
}

// ── Backend Sync ────────────────────────────────────────────────────

/** Fetch current mission from backend */
export async function missionGet(): Promise<Mission> {
  const m = await invoke<Mission>('mission_get');
  mission.set(m);
  return m;
}

/** Replace ALL waypoints of the active mission in one backend call, preserving
 *  every field (including alt_mode). Used by undo/redo restore. */
export async function missionSetWaypoints(waypoints: Waypoint[]): Promise<Mission> {
  const m = await invoke<Mission>('mission_set', { waypoints });
  mission.set(m);
  return m;
}

/** Clear mission */
export async function missionClear(): Promise<void> {
  pushUndo();
  await invoke<Mission>('mission_clear');
  mission.set(createEmptyMission());
  loadedMissionId.set(null);
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
  pushUndo();
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
  pushUndo();
  const m = await invoke<Mission>('mission_insert_wp', {
    index, action, lat, lon, altitude, p1, p2, p3, altMode,
  });
  mission.set(m);
  return m;
}

/** Remove a waypoint by index */
export async function missionRemoveWp(index: number): Promise<Mission> {
  pushUndo();
  const m = await invoke<Mission>('mission_remove_wp', { index });
  mission.set(m);
  return m;
}

/**
 * Move a waypoint from the active mission to the END of another mission slot
 * (INAV multi-mission). The target mission is held in the frontend slot cache;
 * it loads into the backend when that tab is opened. No-op if the target is the
 * active mission or out of range.
 */
export async function moveWaypointToMission(wpIndex: number, targetIdx: number): Promise<void> {
  const activeIdx = get(activeMissionIndex);
  if (targetIdx === activeIdx || targetIdx < 1 || targetIdx > get(missionCount)) return;
  const wps = get(mission).waypoints;
  if (wpIndex < 0 || wpIndex >= wps.length) return;

  beginUndoGroup();
  try {
    // Append a copy to the target slot, then remove from the active mission.
    const target = missionSlots.get(targetIdx) ?? [];
    target.push({ ...wps[wpIndex] });
    missionSlots.set(targetIdx, target);

    await missionRemoveWp(wpIndex);
  } finally {
    endUndoGroup();
  }
  clearWpSelection();
}

/** Move ALL selected waypoints to the end of another mission slot (preserving
 *  their order). Used by the context menu so it works on a multi-selection. */
export async function moveSelectedWpsToMission(targetIdx: number): Promise<void> {
  const activeIdx = get(activeMissionIndex);
  if (targetIdx === activeIdx || targetIdx < 1 || targetIdx > get(missionCount)) return;
  const sel = [...get(selectedWpIndices)].sort((a, b) => a - b); // ascending = original order
  if (sel.length === 0) return;

  const wps = get(mission).waypoints;
  const toMove = sel.filter((i) => i >= 0 && i < wps.length).map((i) => ({ ...wps[i] }));

  beginUndoGroup();
  try {
    // Append (in order) to the target slot, then remove from the active mission
    // descending so the indices stay valid.
    const target = missionSlots.get(targetIdx) ?? [];
    for (const wp of toMove) target.push(wp);
    missionSlots.set(targetIdx, target);

    for (const i of [...sel].sort((a, b) => b - a)) await missionRemoveWp(i);
  } finally {
    endUndoGroup();
  }
  clearWpSelection();
}

/** Update a waypoint at index */
export async function missionUpdateWp(
  index: number,
  wp: Waypoint
): Promise<Mission> {
  pushUndo();
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
  pushUndo();
  const m = await invoke<Mission>('mission_reorder_wp', { from, to });
  mission.set(m);
  return m;
}

/** Download mission from FC */
export async function missionDownload(fromEeprom = false): Promise<Mission> {
  const m = await invoke<Mission>('mission_download', { fromEeprom });
  mission.set(m);
  applyMissionLaunchDefault(m, undefined, true); // FC has no embedded home → UAV HOME, else WP1
  clearUndoHistory();
  markMissionSynced('fc');
  loadedMissionId.set(null);
  frameMissionOnMap(); // loaded from the FC → frame it (free pan only)
  return m;
}

/** Query the FC's mission info (wp_count) without downloading — for the connect prompt. */
export async function missionFcInfo(): Promise<MissionInfo> {
  return invoke<MissionInfo>('mission_fc_info');
}

/** Waypoint-download progress emitted by the backend during an FC download (MSP + MAVLink). */
export interface MissionDownloadProgress {
  current: number;
  total: number;
}

/**
 * Subscribe to FC waypoint-download progress (`mission-download-progress`), for an "x of n" indicator.
 * Returns the unlisten fn — call it once the download settles (in a `finally`). Shared by the INAV and
 * ArduPilot mission panels (both backends emit the same event).
 */
export function onMissionDownloadProgress(cb: (p: MissionDownloadProgress) => void): Promise<() => void> {
  return listen<MissionDownloadProgress>('mission-download-progress', (e) => cb(e.payload));
}

/** Upload mission to FC */
export async function missionUpload(saveEeprom = false): Promise<Mission> {
  const m = await invoke<Mission>('mission_upload', { saveEeprom });
  mission.set(m);
  markMissionSynced('fc');
  return m;
}

/** Current launch point as [lat, lon] for export, or undefined if unset. */
function launchHomeArg(): [number, number] | undefined {
  const lp = get(launchPoint);
  return lp ? [lp.lat, lp.lng] : undefined;
}

/**
 * Default the planning launch/home point after loading a mission (DB / file / FC) so REL waypoint
 * altitudes resolve correctly even before editing.
 *  1. A home embedded WITH the mission (file `<mwp>` meta / DB row) is authoritative → always applied.
 *  2. Otherwise an existing launch point (e.g. user-placed in edit mode) is KEPT — loading a mission
 *     without its own home must not reset it.
 *  3. Only when none exists is one generated: live UAV HOME, else the first geo-waypoint (WP1).
 */
export function applyMissionLaunchDefault(m: Mission, embeddedHome?: { lat: number; lng: number }, resetLaunch = false) {
  // An authoritative FC home wins over everything: keep the launch reference pinned to the real home.
  const hp = get(homePosition);
  if (hp.set && hp.source === 'fc') { launchPoint.set({ lat: hp.lat, lng: hp.lon }); return; }
  if (embeddedHome) { launchPoint.set(embeddedHome); return; }
  if (m.home) { launchPoint.set({ lat: m.home.lat, lng: m.home.lon }); return; }
  // Keep a user-placed launch — but NOT across a fresh mission load (`resetLaunch`): each loaded mission
  // has its own home/first-WP reference, so a stale launch from a previous mission must not stick.
  if (!resetLaunch && get(launchPoint)) return;
  // Non-FC home fallback. On a fresh load this is SKIPPED: the only authoritative home (source 'fc') was
  // already handled above; what remains here is the 'manual' home, which just MIRRORS the previous
  // launchPoint (set via the launchPoint→homePosition link in +page) — using it would resurrect the
  // exact stale launch we're trying to reset (e.g. a Germany launch sticking to a Macedonia mission).
  const h = get(homePosition);
  if (!resetLaunch && h.set && (h.lat !== 0 || h.lon !== 0)) { launchPoint.set({ lat: h.lat, lng: h.lon }); return; }
  const fw = m.waypoints.find((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
  if (fw) launchPoint.set({ lat: toDeg(fw.lat), lng: toDeg(fw.lon) });
  else if (resetLaunch) launchPoint.set(null); // fresh load with no usable reference → drop the stale launch
}

/** Export mission as XML string (includes the launch point as <mwp> meta) */
export async function missionExportXml(): Promise<string> {
  return invoke<string>('mission_export_xml', { home: launchHomeArg() });
}

/** Import mission from XML string */
export async function missionImportXml(xml: string): Promise<Mission> {
  const m = await invoke<Mission>('mission_import_xml', { xml });
  mission.set(m);
  applyMissionLaunchDefault(m, undefined, true);
  clearUndoHistory();
  markMissionSynced('file');
  loadedMissionId.set(null);
  frameMissionOnMap(); // imported from a file → frame it (free pan only)
  return m;
}

/** Save mission to a .mission file (writes the launch point as <mwp> meta) */
export async function missionSaveFile(path: string): Promise<void> {
  await invoke<void>('mission_save_file', { path, home: launchHomeArg() });
  markMissionSynced('file');
}

/** Load mission from a .mission file */
export async function missionLoadFile(path: string): Promise<Mission> {
  const m = await invoke<Mission>('mission_load_file', { path });
  mission.set(m);
  applyMissionLaunchDefault(m, undefined, true);
  clearUndoHistory();
  markMissionSynced('file');
  loadedMissionId.set(null);
  frameMissionOnMap(); // loaded from a file → frame it (free pan only)
  return m;
}
