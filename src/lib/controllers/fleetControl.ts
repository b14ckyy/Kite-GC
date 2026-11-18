// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Group command runner — sends ONE command to MANY vehicles and collects a per-vehicle result. Domain
// logic only (no UI); the bar/dialog decide WHAT to send and to WHOM, this module decides HOW.
//
// Dispatch rules (docs/02-design/features/fleet-control.design.md §3):
//  • The MAVLink link handler holds a single COMMAND_ACK forward slot per link, so two in-flight
//    commands to two vehicles on the SAME link would clobber each other's ACK. Targets are therefore
//    grouped by link: strictly sequential inside a link, links in parallel.
//  • Takeoff is fully serial across all links with a stagger delay between vehicles, so a formation
//    never leaves the ground at the same instant.
//  • Every backend call names its `vehicleId` explicitly — the active vehicle is irrelevant here.
//  • Nothing throws: the backend's error string (denied / unsupported / ACK timeout …) is recorded on
//    the item and the run continues with the next vehicle.

import { invoke } from '@tauri-apps/api/core';
import { writable, derived, get, type Readable } from 'svelte/store';
import type { VehicleSummary } from '$lib/stores/vehicles';
import { vehicles } from '$lib/stores/vehicles';
import { allTelemetry } from '$lib/stores/telemetry';
import { guidedModeFor } from '$lib/helpers/mavModes';
import { arduSetVehicleMission, type ArduWaypoint } from '$lib/stores/missionArdupilot';
import { offsetWaypoints } from '$lib/helpers/missionGeo';
import {
  type FleetModeIntent, type GroupCommandKind,
  orderedVehicles, vehicleColorFor, vehicleSystem, vehicleClassOf, isFixedWingClass, isMavlinkVehicle,
  isVehicleArmed, resolveIntentMode, vehicleActiveMode,
} from '$lib/helpers/fleetStatus';

export type { GroupCommandKind, FleetModeIntent };

export interface GroupCommandParams {
  /** takeoff: target altitude (m, relative). */
  altitude?: number;
  /** takeoff: delay between consecutive vehicles (ms). */
  staggerMs?: number;
  /** changeSpeed: m/s (airspeed for fixed-wing classes, ground speed otherwise). */
  speed?: number;
  /** setMode: abstract intent, resolved per vehicle. */
  intent?: FleetModeIntent;
  /** arm/disarm: bypass FC checks (the 21196 magic). */
  force?: boolean;
  /** missionUpload: the plan to send (the planner's current mission). */
  waypoints?: ArduWaypoint[];
  /** missionUpload: spacing between consecutive vehicles' copies (m, 0 = identical plan). */
  offsetM?: number;
  /** missionUpload: direction of that spacing (deg, 0 = north). */
  offsetBearingDeg?: number;
}

export type GroupItemStatus = 'pending' | 'running' | 'ok' | 'fail' | 'skipped';

export interface GroupRunItem {
  vehicleId: string;
  name: string;
  color: string;
  linkId: number;
  status: GroupItemStatus;
  /** Backend error text on `fail`, '' otherwise. */
  message: string;
}

export interface GroupRun {
  id: number;
  kind: GroupCommandKind;
  params: GroupCommandParams;
  items: GroupRunItem[];
  startedAt: number;
  finishedAt: number | null;
}

export interface GroupTarget {
  vehicle: VehicleSummary;
  color: string;
}

/** The latest (or in-progress) run; null before the first one. Drives the bar's busy state and the
 *  fleet panel's result list. */
export const groupRun = writable<GroupRun | null>(null);
export const groupBusy: Readable<boolean> = derived(groupRun, (r) => r != null && r.finishedAt == null);

let nextRunId = 1;

function patchItem(runId: number, vehicleId: string, patch: Partial<GroupRunItem>): void {
  groupRun.update((r) => {
    if (!r || r.id !== runId) return r;
    return { ...r, items: r.items.map((it) => (it.vehicleId === vehicleId ? { ...it, ...patch } : it)) };
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((res) => setTimeout(res, ms));
}

async function setModeFor(v: VehicleSummary, main: number, sub: number): Promise<void> {
  await invoke('mav_set_mode', { vehicleId: v.vehicleId, main, sub });
}

/** One vehicle, one command — the per-vehicle twin of the active-vehicle logic in vehicleControl.ts.
 *  `pos` = the vehicle's position in the target list (drives the per-vehicle plan offset on upload). */
async function sendOne(kind: GroupCommandKind, v: VehicleSummary, p: GroupCommandParams, pos: number): Promise<void> {
  const id = v.vehicleId;
  const sys = vehicleSystem(v);
  const cls = vehicleClassOf(v);
  switch (kind) {
    case 'missionUpload': {
      const base = p.waypoints ?? [];
      if (base.length === 0) throw new Error('No waypoints to upload');
      const plan = offsetWaypoints(base, (p.offsetM ?? 0) * pos, p.offsetBearingDeg ?? 0);
      // Mission protocol (not COMMAND_ACK) — still one transfer per link at a time, which the per-link
      // queue guarantees. On success that aircraft's slot holds exactly this plan, FC-synced.
      await invoke('ardu_mission_upload', { vehicleId: id, waypoints: plan });
      arduSetVehicleMission(id, plan, true);
      return;
    }
    case 'arm':
      await invoke('mav_arm', { vehicleId: id, arm: true, force: !!p.force });
      return;
    case 'disarm':
      await invoke('mav_arm', { vehicleId: id, arm: false, force: !!p.force });
      return;
    case 'takeoff': {
      // ArduPilot accepts NAV_TAKEOFF only in the Guided mode — switch first (failure surfaces via the
      // takeoff itself, exactly like the single-vehicle path).
      if (sys === 'ardupilot') {
        const g = guidedModeFor(sys, cls);
        const tv = get(allTelemetry).get(id);
        if (g && (!tv || vehicleActiveMode(v, tv)?.key !== g.key)) {
          try { await setModeFor(v, g.main, g.sub); } catch { /* let the takeoff report the real error */ }
        }
      }
      await invoke('mav_takeoff', { vehicleId: id, altitude: p.altitude ?? 50 });
      return;
    }
    case 'land': {
      if (sys === 'ardupilot' && cls === 'quadplane') {
        const m = resolveIntentMode(v, 'land');
        if (m) { await setModeFor(v, m.main, m.sub); return; }
      }
      await invoke('mav_land', { vehicleId: id });
      return;
    }
    case 'rtl':
      await invoke('mav_rtl', { vehicleId: id });
      return;
    case 'hold': {
      const m = resolveIntentMode(v, 'hold');
      if (!m) throw new Error('No hold mode for this vehicle');
      await setModeFor(v, m.main, m.sub);
      return;
    }
    case 'missionStart': {
      // PX4 has no MISSION_START handler — it runs missions by entering the Mission mode.
      if (sys === 'px4') {
        const m = resolveIntentMode(v, 'mission');
        if (m) { await setModeFor(v, m.main, m.sub); return; }
      }
      await invoke('mav_mission_start', { vehicleId: id });
      return;
    }
    case 'changeSpeed':
      await invoke('mav_change_speed', {
        vehicleId: id,
        speedType: isFixedWingClass(cls) ? 0 : 1,
        speed: p.speed ?? 10,
      });
      return;
    case 'setMode': {
      if (!p.intent) throw new Error('No mode selected');
      const m = resolveIntentMode(v, p.intent);
      if (!m) throw new Error('Mode not supported by this vehicle');
      await setModeFor(v, m.main, m.sub);
      return;
    }
  }
}

async function runItem(runId: number, kind: GroupCommandKind, v: VehicleSummary, p: GroupCommandParams, pos: number): Promise<void> {
  patchItem(runId, v.vehicleId, { status: 'running', message: '' });
  try {
    await sendOne(kind, v, p, pos);
    patchItem(runId, v.vehicleId, { status: 'ok' });
  } catch (e) {
    patchItem(runId, v.vehicleId, { status: 'fail', message: e instanceof Error ? e.message : String(e) });
  }
}

/** Send `kind` to every target. Resolves when every vehicle has a final status. A second call while a
 *  run is in progress is refused (returns the running one) — the bar disables itself meanwhile. */
export async function runGroupCommand(
  kind: GroupCommandKind,
  targets: readonly GroupTarget[],
  params: GroupCommandParams = {},
): Promise<GroupRun> {
  const running = get(groupRun);
  if (running && running.finishedAt == null) return running;

  const runId = nextRunId++;
  const run: GroupRun = {
    id: runId,
    kind,
    params,
    items: targets.map(({ vehicle, color }) => ({
      vehicleId: vehicle.vehicleId, name: vehicle.name, color, linkId: vehicle.linkId, status: 'pending', message: '',
    })),
    startedAt: Date.now(),
    finishedAt: null,
  };
  groupRun.set(run);

  if (kind === 'takeoff') {
    // Fully serial + stagger, regardless of link.
    const stagger = Math.max(0, params.staggerMs ?? 1000);
    for (let i = 0; i < targets.length; i++) {
      if (i > 0 && stagger > 0) await sleep(stagger);
      await runItem(runId, kind, targets[i].vehicle, params, i);
    }
  } else {
    const byLink = new Map<number, { v: VehicleSummary; pos: number }[]>();
    targets.forEach((t, pos) => {
      const list = byLink.get(t.vehicle.linkId) ?? [];
      list.push({ v: t.vehicle, pos });
      byLink.set(t.vehicle.linkId, list);
    });
    await Promise.all(
      [...byLink.values()].map(async (queue) => {
        for (const { v, pos } of queue) await runItem(runId, kind, v, params, pos);
      }),
    );
  }

  groupRun.update((r) => (r && r.id === runId ? { ...r, finishedAt: Date.now() } : r));
  return get(groupRun) ?? run;
}

/** Re-send the last run's command to just its failed vehicles (still present in the registry). */
export async function retryFailed(): Promise<GroupRun | null> {
  const last = get(groupRun);
  if (!last || last.finishedAt == null) return null;
  const known = get(vehicles);
  const ordered = orderedVehicles(known);
  const targets: GroupTarget[] = [];
  for (const it of last.items) {
    if (it.status !== 'fail') continue;
    const v = known.get(it.vehicleId);
    if (v) targets.push({ vehicle: v, color: vehicleColorFor(ordered, v.vehicleId) });
  }
  if (!targets.length) return null;
  return runGroupCommand(last.kind, targets, last.params);
}

/** Targets for the emergency "RTL all": every armed MAVLink vehicle, selection ignored. */
export function rtlAllTargets(): GroupTarget[] {
  const known = get(vehicles);
  const telem = get(allTelemetry);
  const ordered = orderedVehicles(known);
  const out: GroupTarget[] = [];
  for (const v of ordered) {
    if (!isMavlinkVehicle(v) || !vehicleSystem(v)) continue;
    const tv = telem.get(v.vehicleId);
    if (!tv || !isVehicleArmed(tv)) continue;
    out.push({ vehicle: v, color: vehicleColorFor(ordered, v.vehicleId) });
  }
  return out;
}

/** Resolve a set of ids to ordered, coloured targets (ids unknown to the registry are dropped). */
export function targetsFor(ids: Iterable<string>): GroupTarget[] {
  const known = get(vehicles);
  const ordered = orderedVehicles(known);
  const want = new Set(ids);
  return ordered
    .filter((v) => want.has(v.vehicleId))
    .map((v) => ({ vehicle: v, color: vehicleColorFor(ordered, v.vehicleId) }));
}
