// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { writable, get } from 'svelte/store';
import { connection } from './connection';
import { cmdHasLocation, type VehicleClass } from '$lib/helpers/arduCommandCatalog';

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

// ── Vehicle class (drives the command catalog filter) ─────────────────
// Online: derived from the FC's variant and locked. Offline: the operator picks it (QuadPlane can't be
// auto-detected — it reports as ArduPlane — so it is an offline-only choice for now).

export const arduVehicleClass = writable<VehicleClass>('plane');

function variantToVehicleClass(variant: string): VehicleClass | null {
  const v = variant.toLowerCase();
  if (v.includes('plane')) return 'plane';     // QuadPlane also reports ArduPlane (manual override)
  if (v.includes('copter') || v.includes('heli')) return 'copter';
  if (v.includes('rover')) return 'rover';
  if (v.includes('boat')) return 'boat';
  if (v.includes('sub')) return 'sub';
  return null;
}

let _lastVariantForClass = '';
connection.subscribe((c) => {
  if (c.status !== 'connected' || !c.fcInfo) { _lastVariantForClass = ''; return; }
  if (c.fcInfo.fc_variant === _lastVariantForClass) return;
  _lastVariantForClass = c.fcInfo.fc_variant;
  const cls = variantToVehicleClass(c.fcInfo.fc_variant);
  if (cls) arduVehicleClass.set(cls);
});

// ── Modifier grouping (INAV-style list/editor presentation over the flat sequence) ──
// A "group" = a location command (a map waypoint) plus the trailing non-location commands that follow
// it in the sequence (its modifiers). Leading modifiers before the first waypoint group with `anchor:
// null`. Indices are the original flat-array indices (for editing).

export interface ArduGroup {
  anchorIdx: number;                          // -1 for a leading modifier-only group
  anchor: ArduWaypoint | null;
  modifiers: { idx: number; wp: ArduWaypoint }[];
}

export function groupArduMission(wps: ArduWaypoint[]): ArduGroup[] {
  const groups: ArduGroup[] = [];
  let current: ArduGroup | null = null;
  wps.forEach((wp, idx) => {
    if (cmdHasLocation(wp.command)) {
      current = { anchorIdx: idx, anchor: wp, modifiers: [] };
      groups.push(current);
    } else {
      if (!current) { current = { anchorIdx: -1, anchor: null, modifiers: [] }; groups.push(current); }
      current.modifiers.push({ idx, wp });
    }
  });
  return groups;
}

/** Index in the flat sequence just after the given group (where a new modifier is inserted). */
export function groupEndIndex(g: ArduGroup): number {
  if (g.modifiers.length) return g.modifiers[g.modifiers.length - 1].idx + 1;
  return g.anchorIdx + 1;
}

// ── Mutations ─────────────────────────────────────────────────────────

export function arduMissionClear(): void {
  arduMission.set([]);
  arduSelectedWpIndex.set(-1);
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
