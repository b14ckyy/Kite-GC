// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { connection } from './connection';
import { settings } from './settings';
import { CMD, cmdDef, cmdHasLocation, type VehicleClass } from '$lib/helpers/arduCommandCatalog';

// ── MAV_CMD constants ─────────────────────────────────────────────────
export const MAV_CMD_NAV_WAYPOINT        = 16;
export const MAV_CMD_NAV_LOITER_UNLIM   = 17;
export const MAV_CMD_NAV_LOITER_TURNS   = 18;
export const MAV_CMD_NAV_LOITER_TIME    = 19;
export const MAV_CMD_NAV_RETURN_TO_LAUNCH = 20;
export const MAV_CMD_NAV_LAND           = 21;
export const MAV_CMD_NAV_TAKEOFF        = 22;
export const MAV_CMD_DO_JUMP            = 177;
export const MAV_CMD_DO_CHANGE_SPEED    = 178;
export const MAV_CMD_DO_SET_ROI         = 201;
export const MAV_CMD_CONDITION_DELAY    = 112;

// ── MAV_FRAME constants ───────────────────────────────────────────────
export const MAV_FRAME_GLOBAL                = 0;   // absolute AMSL
export const MAV_FRAME_GLOBAL_RELATIVE_ALT   = 3;   // relative to home
export const MAV_FRAME_GLOBAL_TERRAIN_ALT    = 10;  // terrain-relative
export const MAV_FRAME_MISSION               = 2;   // no position — pure action items (DO_* / CONDITION_*)

export type MavFrame = 0 | 2 | 3 | 10;

// ── Types ─────────────────────────────────────────────────────────────

export interface ArduWaypoint {
  command: number;
  frame: MavFrame;
  param1: number;
  param2: number;
  param3: number;
  param4: number;
  lat: number;           // degrees * 1e7 (same internal convention as INAV)
  lon: number;
  alt: number;           // metres float
  autocontinue: boolean;
}

export const MAV_CMD_LABELS: Record<number, string> = {
  [MAV_CMD_NAV_WAYPOINT]:          'Waypoint',
  [MAV_CMD_NAV_LOITER_UNLIM]:      'Loiter ∞',
  [MAV_CMD_NAV_LOITER_TURNS]:      'Loiter Turns',
  [MAV_CMD_NAV_LOITER_TIME]:       'Loiter Time',
  [MAV_CMD_NAV_RETURN_TO_LAUNCH]:  'RTL',
  [MAV_CMD_NAV_LAND]:              'Land',
  [MAV_CMD_NAV_TAKEOFF]:           'Takeoff',
  [MAV_CMD_DO_JUMP]:               'Jump',
  [MAV_CMD_DO_CHANGE_SPEED]:       'Change Speed',
  [MAV_CMD_DO_SET_ROI]:            'Set ROI',
  [MAV_CMD_CONDITION_DELAY]:       'Delay',
};

export const MAV_CMD_SHORT: Record<number, string> = {
  [MAV_CMD_NAV_WAYPOINT]:          'WPT',
  [MAV_CMD_NAV_LOITER_UNLIM]:      'LTR∞',
  [MAV_CMD_NAV_LOITER_TURNS]:      'LTRT',
  [MAV_CMD_NAV_LOITER_TIME]:       'LTRS',
  [MAV_CMD_NAV_RETURN_TO_LAUNCH]:  'RTL',
  [MAV_CMD_NAV_LAND]:              'LND',
  [MAV_CMD_NAV_TAKEOFF]:           'TKO',
  [MAV_CMD_DO_JUMP]:               'JMP',
  [MAV_CMD_DO_CHANGE_SPEED]:       'SPD',
  [MAV_CMD_DO_SET_ROI]:            'ROI',
  [MAV_CMD_CONDITION_DELAY]:       'DLY',
};

/** Commands that carry a lat/lon position */
const COMMANDS_WITH_LOCATION = new Set([
  MAV_CMD_NAV_WAYPOINT,
  MAV_CMD_NAV_LOITER_UNLIM,
  MAV_CMD_NAV_LOITER_TURNS,
  MAV_CMD_NAV_LOITER_TIME,
  MAV_CMD_NAV_LAND,
  MAV_CMD_NAV_TAKEOFF,
  MAV_CMD_DO_SET_ROI,
]);

export function arduHasLocation(cmd: number): boolean {
  return COMMANDS_WITH_LOCATION.has(cmd);
}

/** Commands that show a loiter circle on the map */
export function arduIsLoiter(cmd: number): boolean {
  return cmd === MAV_CMD_NAV_LOITER_UNLIM ||
         cmd === MAV_CMD_NAV_LOITER_TURNS ||
         cmd === MAV_CMD_NAV_LOITER_TIME;
}

// ── Stores ────────────────────────────────────────────────────────────

export const arduMission        = writable<ArduWaypoint[]>([]);
export const arduSelectedWpIndex = writable<number>(-1);

/** Multi-selection of waypoint indices (edit-mode multi-select / batch ops). The primary
 *  (`arduSelectedWpIndex`) is the sole member when size === 1, else -1. Mirrors the INAV model
 *  (stores/mission.ts) so both stacks behave identically. */
export const arduSelectedWpIndices = writable<Set<number>>(new Set());

function syncArduPrimary(s: Set<number>): void {
  arduSelectedWpIndex.set(s.size === 1 ? (s.values().next().value as number) : -1);
}

/** Replace the selection with a single WP (also the editor primary). */
export function arduSelectWpSingle(i: number): void {
  arduSelectedWpIndices.set(new Set([i]));
  arduSelectedWpIndex.set(i);
}

/** Toggle a WP in/out of the multi-selection (Ctrl/⌘-click, badge tap). */
export function arduToggleWpSelection(i: number): void {
  arduSelectedWpIndices.update((s) => {
    const n = new Set(s);
    if (n.has(i)) n.delete(i);
    else n.add(i);
    syncArduPrimary(n);
    return n;
  });
}

/** Select an inclusive index range (Shift-click). */
export function arduSelectWpRange(a: number, b: number): void {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  const n = new Set<number>();
  for (let i = lo; i <= hi; i++) n.add(i);
  arduSelectedWpIndices.set(n);
  syncArduPrimary(n);
}

/** Clear the entire selection. */
export function arduClearWpSelection(): void {
  arduSelectedWpIndices.set(new Set());
  arduSelectedWpIndex.set(-1);
}

/** Batch-remove all selected waypoints (descending so indices stay valid), as one undo step. */
export function arduRemoveSelectedWps(): void {
  const ids = [...get(arduSelectedWpIndices)].sort((a, b) => b - a);
  if (ids.length === 0) return;
  arduBeginUndoGroup();
  try {
    for (const idx of ids) arduRemoveWp(idx);
  } finally {
    arduEndUndoGroup();
  }
  arduClearWpSelection();
}
export const arduEditMode       = writable<boolean>(false);

/** DB id of the currently loaded/imported library mission (null = fresh / never saved).
 *  ArduPilot/PX4 counterpart to `mission.ts` `loadedMissionId`: set on DB-load and on import when
 *  the content hash matches an existing row; the link target for arm-time/End-Flight recording
 *  saves. See docs/active/ARDUPILOT_MISSION_LIBRARY.md. */
export const arduLoadedMissionId = writable<number | null>(null);

// ── Mission provenance (FC / FILE / DB), mirroring the INAV store ──────────────
// Each flag holds the serialized snapshot of the mission at the moment it was last synced via that
// channel; a flag is "lit" when the current mission still matches its snapshot (content-compared, so
// it clears on edit and returns on undo). The FC flag also gates FC-relative actions like "Set active
// WP". Single-mission, so a plain ref map (no per-slot like INAV).
export type ArduSyncFlag = 'fc' | 'file' | 'db';
const arduSyncRefs = new Map<ArduSyncFlag, string>();
const arduProvVersion = writable(0);

/** Record the current mission as synced via `flag` (call after FC download/upload, file open, DB load/save). */
export function markArduMissionSynced(flag: ArduSyncFlag, wps: ArduWaypoint[]): void {
  arduSyncRefs.set(flag, serializeWaypoints(wps));
  arduProvVersion.update((n) => n + 1);
}

/** Drop the FC flag (link gone) — the on-FC mission is unknown after a disconnect. */
export function clearArduFcFlag(): void {
  if (arduSyncRefs.delete('fc')) arduProvVersion.update((n) => n + 1);
}

export interface ArduMissionFlags { fc: boolean; file: boolean; db: boolean; }

/** Live provenance flags for the current mission (content-compared to the snapshots). */
export const arduMissionFlags = derived(
  [arduMission, arduProvVersion],
  ([m]): ArduMissionFlags => {
    if (m.length === 0) return { fc: false, file: false, db: false };
    const h = serializeWaypoints(m);
    return { fc: arduSyncRefs.get('fc') === h, file: arduSyncRefs.get('file') === h, db: arduSyncRefs.get('db') === h };
  },
);

/** "Modified" = has content but matches none of its sync snapshots (unsaved relative to FC/FILE/DB). */
export const arduMissionModified = derived(
  [arduMission, arduMissionFlags],
  ([m, f]) => m.length > 0 && !f.fc && !f.file && !f.db,
);

/** Whether the current mission still matches what's on the FC — gates "Set active WP". */
export const arduMissionFcSynced = derived(arduMissionFlags, (f) => f.fc);

// ── Vehicle class (drives the command catalog filter) ─────────────────
// Online: derived from the FC's variant and locked. Offline: the operator picks it (QuadPlane can't be
// auto-detected — it reports as ArduPlane — so it is an offline-only choice for now).

const VEHICLE_CLASSES: VehicleClass[] = ['plane', 'copter', 'quadplane', 'rover', 'boat', 'sub'];

function coerceVehicleClass(s: string): VehicleClass {
  return (VEHICLE_CLASSES as string[]).includes(s) ? (s as VehicleClass) : 'plane';
}

export const arduVehicleClass = writable<VehicleClass>(coerceVehicleClass(get(settings).lastArduVehicleClass));

/** Set the vehicle class from the offline selector and remember it. Ignored while connected (the class
 *  is then locked to the detected FC). */
export function setArduVehicleClass(cls: VehicleClass): void {
  if (get(connection).status === 'connected') return;
  arduVehicleClass.set(cls);
  settings.patch({ lastArduVehicleClass: cls });
}

/** MAV_TYPE → vehicle class. PX4 reports an accurate MAV_TYPE for every airframe, so the class is read
 *  straight from it (unlike ArduPilot, where a QuadPlane lies and reports MAV_TYPE_FIXED_WING). */
function mavTypeToClass(mavType: number): VehicleClass | null {
  if (mavType === 1) return 'plane';                              // FIXED_WING
  if ([2, 3, 4, 13, 14, 15].includes(mavType)) return 'copter';  // quad/coaxial/heli/hexa/octo/tri
  if (mavType === 10) return 'rover';                             // GROUND_ROVER
  if (mavType === 11) return 'boat';                              // SURFACE_BOAT
  if (mavType === 12) return 'sub';                              // SUBMARINE
  if (mavType >= 19 && mavType <= 25) return 'quadplane';        // VTOL family
  return null;
}

/** Map the FC's variant + MAVLink vehicle type to a class. For **PX4** the class comes straight from
 *  the MAV_TYPE (PX4 reports it accurately). For **ArduPilot** the MAV_TYPE is only a reliable QuadPlane
 *  signal (a QuadPlane reports fc_variant "ArduPlane" but a VTOL_* MAV_TYPE); otherwise the per-vehicle
 *  fc_variant string ("ArduPlane"/"ArduCopter"/…) is authoritative. */
function detectVehicleClass(variant: string, mavType: number | null | undefined): VehicleClass | null {
  if (variant.toLowerCase() === 'px4') return mavType != null ? mavTypeToClass(mavType) : null;
  // MAV_TYPE VTOL range (19–25: tailsitter duo/quad, tiltrotor, …) → QuadPlane.
  if (mavType != null && mavType >= 19 && mavType <= 25) return 'quadplane';
  const v = variant.toLowerCase();
  if (v.includes('plane')) return 'plane';
  if (v.includes('copter') || v.includes('heli')) return 'copter';
  if (v.includes('rover')) return 'rover';
  if (v.includes('boat')) return 'boat';
  if (v.includes('sub')) return 'sub';
  return null;
}

let _lastVariantForClass = '';
connection.subscribe((c) => {
  if (c.status !== 'connected' || !c.fcInfo) { _lastVariantForClass = ''; return; }
  const key = `${c.fcInfo.fc_variant}/${c.fcInfo.mav_type ?? ''}`;
  if (key === _lastVariantForClass) return;
  _lastVariantForClass = key;
  const cls = detectVehicleClass(c.fcInfo.fc_variant, c.fcInfo.mav_type);
  if (cls) arduVehicleClass.set(cls);
});

// Drop the FC provenance flag on any disconnect — the on-FC mission is then unknown (mirrors INAV).
let _arduPrevConn = 'disconnected';
connection.subscribe((c) => {
  if (_arduPrevConn === 'connected' && c.status !== 'connected') clearArduFcFlag();
  _arduPrevConn = c.status;
});

// ── Modifier grouping (INAV-style list/editor presentation over the flat sequence) ──
// A "group" = a location command (a map waypoint) plus the non-location commands attached to it. Most
// modifiers (DO_*/CONDITION_*, executed on arrival / gating the next leg) TRAIL their waypoint. A
// JUMP_TAG is the exception: it's a jump *target* — the FC resumes at the next nav waypoint after the
// tag — so it LEADS the waypoint it marks (grouped with the FOLLOWING location, not the preceding one),
// which matches both the FC behaviour and "the tag belongs to this waypoint". Leading modifiers before
// the first waypoint group with `anchor: null`. Indices are the original flat-array indices (for editing).

export interface ArduGroup {
  anchorIdx: number;                          // -1 for a leading modifier-only group
  anchor: ArduWaypoint | null;
  modifiers: { idx: number; wp: ArduWaypoint }[];
}

export function groupArduMission(wps: ArduWaypoint[]): ArduGroup[] {
  const groups: ArduGroup[] = [];
  let current: ArduGroup | null = null;
  let leading: { idx: number; wp: ArduWaypoint }[] = []; // JUMP_TAGs awaiting their target waypoint

  wps.forEach((wp, idx) => {
    if (cmdHasLocation(wp.command)) {
      current = { anchorIdx: idx, anchor: wp, modifiers: [...leading] }; // leading tags belong to this WP
      leading = [];
      groups.push(current);
    } else if (wp.command === CMD.JUMP_TAG) {
      leading.push({ idx, wp }); // defer: attach to the NEXT location (the jump target)
    } else {
      if (!current) { current = { anchorIdx: -1, anchor: null, modifiers: [] }; groups.push(current); }
      current.modifiers.push({ idx, wp });
    }
  });
  // Trailing JUMP_TAG(s) with no following waypoint → attach to the last group (fallback).
  if (leading.length) {
    if (!current) { current = { anchorIdx: -1, anchor: null, modifiers: [] }; groups.push(current); }
    current.modifiers.push(...leading);
  }
  return groups;
}

/** Index in the flat sequence just after the given group's TRAILING items (where a new trailing
 *  modifier is inserted). Leading modifiers (JUMP_TAGs, idx < anchorIdx) are ignored here. */
export function groupEndIndex(g: ArduGroup): number {
  let end = g.anchorIdx;
  for (const m of g.modifiers) if (m.idx > end) end = m.idx;
  return end + 1;
}

// ── Mutations ─────────────────────────────────────────────────────────

export function arduMissionClear(): void {
  arduMission.set([]);
  arduClearWpSelection();
  arduLoadedMissionId.set(null);
  arduClearUndoHistory(); // a clear is a fresh baseline
}

export function arduAddWp(wp: ArduWaypoint): void {
  arduRecordUndo();
  arduMission.update(wps => [...wps, wp]);
}

export function arduRemoveWp(index: number): void {
  arduRecordUndo();
  arduMission.update(wps => wps.filter((_, i) => i !== index));
  arduSelectedWpIndex.update(i => (i >= index ? Math.max(-1, i - 1) : i));
}

export function arduUpdateWp(index: number, wp: ArduWaypoint): void {
  arduRecordUndo();
  arduMission.update(wps => {
    const next = [...wps];
    next[index] = wp;
    return next;
  });
}

export function arduMoveWp(from: number, to: number): void {
  arduRecordUndo();
  arduMission.update(wps => {
    if (from < 0 || to < 0 || from >= wps.length || to >= wps.length) return wps;
    const next = [...wps];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    return next;
  });
}

// ── Undo / redo ────────────────────────────────────────────────────────────
// Snapshot-based history of the (single) ArduPilot/PX4 mission — the equivalent of the INAV mission
// history. The primitive mutators above record a snapshot BEFORE they change the array; multi-step
// actions (a survey-pattern append, a marker drag) wrap their primitives in a begin/end group so the
// whole action collapses into one undo step. Cleared on load/download/import (a fresh baseline).
const ARDU_UNDO_LIMIT = 50;
let arduUndoStack: ArduWaypoint[][] = [];
let arduRedoStack: ArduWaypoint[][] = [];
let arduUndoSuspend = 0;

export const arduCanUndo = writable<boolean>(false);
export const arduCanRedo = writable<boolean>(false);
function arduSyncUndoFlags(): void {
  arduCanUndo.set(arduUndoStack.length > 0);
  arduCanRedo.set(arduRedoStack.length > 0);
}
const cloneWps = (wps: ArduWaypoint[]): ArduWaypoint[] => wps.map(w => ({ ...w }));

/** Push the CURRENT mission onto the undo stack (call BEFORE a mutation). No-op while a group or a
 *  restore is in progress. Clears the redo stack. */
export function arduRecordUndo(): void {
  if (arduUndoSuspend > 0) return;
  arduUndoStack.push(cloneWps(get(arduMission)));
  if (arduUndoStack.length > ARDU_UNDO_LIMIT) arduUndoStack.shift();
  arduRedoStack = [];
  arduSyncUndoFlags();
}

/** Begin a grouped action: one snapshot now, then suspend per-primitive recording until end. */
export function arduBeginUndoGroup(): void {
  arduRecordUndo();
  arduUndoSuspend++;
}
export function arduEndUndoGroup(): void {
  if (arduUndoSuspend > 0) arduUndoSuspend--;
}

export function arduUndo(): void {
  if (arduUndoStack.length === 0) return;
  arduRedoStack.push(cloneWps(get(arduMission)));
  const snap = arduUndoStack.pop() as ArduWaypoint[];
  arduUndoSuspend++;
  arduMission.set(snap);
  arduUndoSuspend--;
  arduClearWpSelection();
  arduSyncUndoFlags();
}

export function arduRedo(): void {
  if (arduRedoStack.length === 0) return;
  arduUndoStack.push(cloneWps(get(arduMission)));
  const snap = arduRedoStack.pop() as ArduWaypoint[];
  arduUndoSuspend++;
  arduMission.set(snap);
  arduUndoSuspend--;
  arduClearWpSelection();
  arduSyncUndoFlags();
}

/** Wipe the history (fresh baseline after a load / download / import / clear). */
export function arduClearUndoHistory(): void {
  arduUndoStack = [];
  arduRedoStack = [];
  arduUndoSuspend = 0;
  arduSyncUndoFlags();
}

/** Download the mission from the connected MAVLink FC (ArduPilot/PX4), replace the working mission and
 *  mark it FC-synced (which gates "Set active WP"). Returns the waypoint count. Shared by the mission
 *  panel and the Vehicle Control panel; camera framing stays with the caller (a UI concern). */
export async function downloadArduMissionFromFc(): Promise<number> {
  const wps = await invoke<ArduWaypoint[]>('ardu_mission_download');
  arduMission.set(wps);
  arduClearWpSelection();
  arduLoadedMissionId.set(null); // from the FC → not a library mission
  arduClearUndoHistory();        // downloaded mission = fresh undo baseline
  markArduMissionSynced('fc', wps);
  return wps.length;
}

// ── Upload framing ────────────────────────────────────────────────────
/** ArduPilot's `AP_Mission::stored_in_location()` — the mission commands whose x/y/z are a coordinate
 *  (NAV_* incl. GUIDED_ENABLE / PAYLOAD_PLACE, DO_SET_HOME, DO_LAND_START, DO_RETURN_PATH_START,
 *  DO_GO_AROUND, DO_SET_ROI / _LOCATION). RTL (20) is kept in the set so it goes out unchanged. */
const ARDUPILOT_LOCATION_CMDS: ReadonlySet<number> = new Set([
  16, 17, 18, 19, 20, 21, 22, 30, 31, 82, 84, 85, 92, 94, 179, 188, 189, 191, 195, 201,
]);

/** Frame for a MISSION upload: every command the catalog knows that is NOT a location command goes out
 *  in MAV_FRAME_MISSION. PX4 parses any global frame as a position item and rejects lat 0 with
 *  MAV_MISSION_INVALID_PARAM5_X (Mission Planner writes frame 0 on such items); ArduPilot accepts
 *  either frame for them. Commands the catalog does not know keep the frame they came with — a wrong
 *  guess would turn a relative altitude into an absolute one on ArduPilot, which reads MAV_FRAME_MISSION
 *  like GLOBAL for a location command. Fence / rally uploads never pass through here. */
export function framesForUpload(wps: readonly ArduWaypoint[]): ArduWaypoint[] {
  return wps.map((w) =>
    cmdDef(w.command) && !ARDUPILOT_LOCATION_CMDS.has(w.command) && w.frame !== MAV_FRAME_MISSION
      ? { ...w, frame: MAV_FRAME_MISSION }
      : w,
  );
}

// ── .waypoints file format ────────────────────────────────────────────

/** Number → fixed-point text that never throws: a null/NaN (PX4 "not set", a stale download, a file
 *  with "nan") becomes 0. This serializer also runs inside a store subscriber (provenance flags) — an
 *  exception there froze the whole UI once, because svelte/store's notify queue is left half-flushed. */
function fixed(v: number | null | undefined, digits: number): string {
  return (typeof v === 'number' && Number.isFinite(v) ? v : 0).toFixed(digits);
}

export function serializeWaypoints(wps: ArduWaypoint[]): string {
  const lines = ['QGC WPL 110'];
  wps.forEach((wp, i) => {
    const current = i === 0 ? 1 : 0;
    const latDeg  = fixed(wp.lat / 1e7, 8);
    const lonDeg  = fixed(wp.lon / 1e7, 8);
    const ac      = wp.autocontinue ? 1 : 0;
    lines.push([
      i, current, wp.frame, wp.command,
      fixed(wp.param1, 6), fixed(wp.param2, 6),
      fixed(wp.param3, 6), fixed(wp.param4, 6),
      latDeg, lonDeg, fixed(wp.alt, 3), ac,
    ].join('\t'));
  });
  return lines.join('\n') + '\n';
}

/** "nan" (QGC) / blanks → 0: NaN cannot cross the JSON IPC boundary, and 0 means "not set". */
function num(s: string): number {
  const v = parseFloat(s);
  return Number.isFinite(v) ? v : 0;
}

export function parseWaypoints(text: string): ArduWaypoint[] {
  const lines = text.trim().split('\n');
  if (!lines[0]?.startsWith('QGC WPL')) throw new Error('Not a QGC WPL file');
  const wps: ArduWaypoint[] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    const c = line.split('\t');
    if (c.length < 12) continue;
    wps.push({
      command:      parseInt(c[3]),
      frame:        parseInt(c[2]) as MavFrame,
      param1:       num(c[4]),
      param2:       num(c[5]),
      param3:       num(c[6]),
      param4:       num(c[7]),
      lat:          Math.round(num(c[8]) * 1e7),
      lon:          Math.round(num(c[9]) * 1e7),
      alt:          num(c[10]),
      autocontinue: c[11].trim() === '1',
    });
  }
  return wps;
}

// ── QGroundControl .plan file format (PX4 ecosystem) ──────────────────
// A QGC "Plan" is a JSON container: `mission.items[]` of SimpleItems (command / frame /
// params[7] / autoContinue — exactly the ArduWaypoint fields, with lat/lon/alt riding in
// params[4..6]), plus a plannedHomePosition and geoFence/rallyPoints sections. QGC saves
// ONLY this format since v3.2 (it still *imports* .waypoints), so missions coming out of
// the PX4 ecosystem arrive as .plan files. Import scope for 1.0:
//  * ComplexItems (Survey, Corridor Scan, …) are QGC-computed patterns that only QGC can
//    expand into waypoints → refuse the whole file with a readable error rather than
//    silently dropping flight path.
//  * geoFence / rallyPoints sections are ignored — Kite handles fence and rally as
//    separate subsystems; a .plan EXPORT that bundles them is a planned 1.1 feature.
//  * plannedHomePosition is ignored — like the MAVLink path, the working mission holds
//    only real waypoints (ArduPilot's home slot 0 is synthesized on upload, PX4 has no
//    home slot at all; see `reserve_home` in mavlink_proto/mission.rs).
//  * null params (QGC's "no preference", typically yaw) become 0 — the same value
//    Kite-authored missions carry in those fields. NaN cannot cross the Tauri IPC
//    boundary (JSON has no NaN; the Rust side deserializes plain f32).

/** Which system a .plan targets: `mission.firmwareType` 12 (MAV_AUTOPILOT_PX4) → px4, everything
 *  else (3 = ArduPilotMega, absent, unparseable) → ardupilot. Both share the mission stack — this
 *  only steers which family tab the global drag-drop router activates. */
export function planFirmwareTarget(text: string): 'ardupilot' | 'px4' {
  try {
    const fw = (JSON.parse(text) as { mission?: { firmwareType?: number } })?.mission?.firmwareType;
    return fw === 12 ? 'px4' : 'ardupilot';
  } catch {
    return 'ardupilot';
  }
}

/** Replace the working mission with waypoints loaded from a file — the shared post-parse sequence
 *  of every file-import path (panel dialog, global drag-drop): fresh selection, not a library
 *  mission, fresh undo baseline, FILE provenance. */
export function loadArduMissionFromFile(wps: ArduWaypoint[]): void {
  arduMission.set(wps);
  arduSelectedWpIndex.set(-1);
  arduLoadedMissionId.set(null);
  arduClearUndoHistory();
  markArduMissionSynced('file', wps);
}

export function parsePlanFile(text: string): ArduWaypoint[] {
  let root: unknown;
  try {
    root = JSON.parse(text);
  } catch {
    throw new Error('Not a valid .plan file (broken JSON)');
  }
  const plan = root as { fileType?: string; mission?: { items?: unknown[] } };
  if (plan?.fileType !== 'Plan' || !Array.isArray(plan.mission?.items)) {
    throw new Error('Not a QGroundControl .plan file');
  }
  const wps: ArduWaypoint[] = [];
  for (const raw of plan.mission.items) {
    const item = raw as {
      type?: string;
      command?: number;
      frame?: number;
      autoContinue?: boolean;
      params?: (number | null)[];
    };
    if (item?.type !== 'SimpleItem') {
      throw new Error(
        `unsupported "${item?.type ?? 'unknown'}" item — pattern items (Survey, Corridor Scan, …) exist only inside QGroundControl`,
      );
    }
    if (typeof item.command !== 'number' || !Array.isArray(item.params) || item.params.length < 7) {
      throw new Error('malformed mission item in .plan file');
    }
    const p = (i: number): number => {
      const v = item.params?.[i];
      return typeof v === 'number' && Number.isFinite(v) ? v : 0;
    };
    wps.push({
      command: item.command,
      frame: (item.frame ?? MAV_FRAME_GLOBAL_RELATIVE_ALT) as MavFrame,
      param1: p(0),
      param2: p(1),
      param3: p(2),
      param4: p(3),
      lat: Math.round(p(4) * 1e7),
      lon: Math.round(p(5) * 1e7),
      alt: p(6),
      autocontinue: item.autoContinue !== false,
    });
  }
  return wps;
}
