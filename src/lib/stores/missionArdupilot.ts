// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { writable, derived, get } from 'svelte/store';
import { connection } from './connection';
import { settings } from './settings';
import { CMD, cmdHasLocation, type VehicleClass } from '$lib/helpers/arduCommandCatalog';

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

export type MavFrame = 0 | 3 | 10;

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
  arduSelectedWpIndex.set(-1);
  arduLoadedMissionId.set(null);
}

export function arduAddWp(wp: ArduWaypoint): void {
  arduMission.update(wps => [...wps, wp]);
}

export function arduRemoveWp(index: number): void {
  arduMission.update(wps => wps.filter((_, i) => i !== index));
  arduSelectedWpIndex.update(i => (i >= index ? Math.max(-1, i - 1) : i));
}

export function arduUpdateWp(index: number, wp: ArduWaypoint): void {
  arduMission.update(wps => {
    const next = [...wps];
    next[index] = wp;
    return next;
  });
}

export function arduMoveWp(from: number, to: number): void {
  arduMission.update(wps => {
    if (from < 0 || to < 0 || from >= wps.length || to >= wps.length) return wps;
    const next = [...wps];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    return next;
  });
}

// ── .waypoints file format ────────────────────────────────────────────

export function serializeWaypoints(wps: ArduWaypoint[]): string {
  const lines = ['QGC WPL 110'];
  wps.forEach((wp, i) => {
    const current = i === 0 ? 1 : 0;
    const latDeg  = (wp.lat / 1e7).toFixed(8);
    const lonDeg  = (wp.lon / 1e7).toFixed(8);
    const ac      = wp.autocontinue ? 1 : 0;
    lines.push([
      i, current, wp.frame, wp.command,
      wp.param1.toFixed(6), wp.param2.toFixed(6),
      wp.param3.toFixed(6), wp.param4.toFixed(6),
      latDeg, lonDeg, wp.alt.toFixed(3), ac,
    ].join('\t'));
  });
  return lines.join('\n') + '\n';
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
      param1:       parseFloat(c[4]),
      param2:       parseFloat(c[5]),
      param3:       parseFloat(c[6]),
      param4:       parseFloat(c[7]),
      lat:          Math.round(parseFloat(c[8]) * 1e7),
      lon:          Math.round(parseFloat(c[9]) * 1e7),
      alt:          parseFloat(c[10]),
      autocontinue: c[11].trim() === '1',
    });
  }
  return wps;
}
