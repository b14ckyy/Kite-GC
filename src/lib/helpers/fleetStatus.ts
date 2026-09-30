// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Per-vehicle status derivations for the fleet panel and the group command runner. The singleton
// derived stores in `controllers/vehicleControl.ts` (`isArmed`, `activeMode`, …) describe the ACTIVE
// vehicle only; everything here takes an explicit (VehicleSummary, TelemetryData) pair so it works for
// any vehicle in `allTelemetry`. Pure functions — no stores, no UI.
// See docs/02-design/features/fleet-control.design.md §2.2.

import type { VehicleSummary } from '$lib/stores/vehicles';
import type { TelemetryData } from '$lib/stores/telemetry';
import type { AutopilotSystem } from '$lib/stores/autopilotContext';
import { cmdValidForVehicle, cmdValidForPx4, type VehicleClass } from '$lib/helpers/arduCommandCatalog';
import { detectVehicleClass, type ArduWaypoint } from '$lib/stores/missionArdupilot';
import { type MavMode, modesFor, matchActiveMode, guidedModeFor } from '$lib/helpers/mavModes';
import { vehicleColor } from '$lib/helpers/vehicleColors';
import { ARMING_FLAG_ARMED, MIN_FIX_SATELLITES } from '$lib/helpers/telemetry';

/** Same (link, sysid) ordering the map fleet layer and LinkManager use, so colours agree everywhere. */
export function orderedVehicles(known: ReadonlyMap<string, VehicleSummary>): VehicleSummary[] {
  return [...known.values()].sort((a, b) => a.linkId - b.linkId || a.sysid - b.sysid);
}

export function vehicleColorFor(ordered: readonly VehicleSummary[], vehicleId: string): string {
  const idx = ordered.findIndex((v) => v.vehicleId === vehicleId);
  return vehicleColor(idx < 0 ? 0 : idx);
}

/** Firmware family from the per-vehicle variant string, or null for anything we can't command. */
export function vehicleSystem(v: VehicleSummary): AutopilotSystem | null {
  const variant = v.fcVariant.toLowerCase();
  if (variant === 'px4') return 'px4';
  if (variant.startsWith('ardu')) return 'ardupilot';
  return null;
}

export function vehicleClassOf(v: VehicleSummary): VehicleClass {
  return detectVehicleClass(v.fcVariant, v.mavType) ?? 'copter';
}

export function isMavlinkVehicle(v: VehicleSummary): boolean {
  return v.protocol.toLowerCase() === 'mavlink';
}

export function vehicleModes(v: VehicleSummary): MavMode[] {
  const sys = vehicleSystem(v);
  return sys ? modesFor(sys, vehicleClassOf(v)) : [];
}

export function vehicleActiveMode(v: VehicleSummary, tv: TelemetryData): MavMode | undefined {
  const sys = vehicleSystem(v);
  return sys ? matchActiveMode(sys, vehicleClassOf(v), tv.flightModeFlags) : undefined;
}

export function isVehicleArmed(tv: TelemetryData): boolean {
  return tv.lastUpdate > 0 && (tv.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
}

export function isFixedWingClass(cls: VehicleClass): boolean {
  return cls === 'plane' || cls === 'quadplane';
}

export type LinkState = 'ok' | 'stale' | 'lost';
export const LINK_STALE_MS = 3_000;
export const LINK_LOST_MS = 10_000;

/** Freshness of a vehicle's telemetry — the only per-vehicle liveness signal the frontend has
 *  (`VehicleSummary.lastSeen` is a discovery timestamp, not a heartbeat clock). */
export function linkState(tv: TelemetryData | undefined, now = Date.now()): LinkState {
  if (!tv || !tv.lastUpdate) return 'lost';
  const age = now - tv.lastUpdate;
  if (age > LINK_LOST_MS) return 'lost';
  if (age > LINK_STALE_MS) return 'stale';
  return 'ok';
}

/** Failsafe as the FC reports it through the canonical mode registry. */
export function isFailsafe(tv: TelemetryData): boolean {
  const p = tv.flightMode?.primary ?? '';
  return p === 'failsafe' || p === 'failsafe_rth';
}

export const LOW_BATTERY_PCT = 20;

// ── Abstract ("intent") modes for mixed-firmware fleets ─────────────────────

export type FleetModeIntent = 'hold' | 'return' | 'mission' | 'land' | 'guided';
export const FLEET_MODE_INTENTS: readonly FleetModeIntent[] = ['hold', 'return', 'mission', 'land', 'guided'];

/** Resolve an abstract intent to this vehicle's concrete mode table entry, or undefined when the
 *  firmware/class has no safe equivalent (a plain plane has no land-now mode, a rover no land, …). */
export function resolveIntentMode(v: VehicleSummary, intent: FleetModeIntent): MavMode | undefined {
  const sys = vehicleSystem(v);
  if (!sys) return undefined;
  const cls = vehicleClassOf(v);
  const table = modesFor(sys, cls);
  const byKey = (key: string) => table.find((m) => m.key === key);
  if (sys === 'px4') {
    switch (intent) {
      case 'hold': return byKey('hold');
      case 'return': return byKey('return');
      case 'mission': return byKey('mission');
      case 'land': return byKey('land');
      case 'guided': return guidedModeFor(sys, cls);
    }
  }
  switch (intent) {
    case 'hold': return byKey(cls === 'rover' || cls === 'boat' || cls === 'sub' ? 'hold' : 'loiter');
    case 'return': return byKey('rtl');
    case 'mission': return byKey('auto');
    case 'land':
      if (cls === 'copter') return byKey('land');
      if (cls === 'quadplane') return byKey('qland');
      return undefined;
    case 'guided': return guidedModeFor(sys, cls);
  }
}

// ── Pre-flight checks for a group command ───────────────────────────────────

export type GroupCommandKind =
  | 'arm' | 'disarm' | 'takeoff' | 'land' | 'rtl' | 'hold' | 'missionStart' | 'changeSpeed' | 'setMode' | 'missionUpload';

/** Whether every command in `wps` is valid for this vehicle's firmware + class (upload sanity). */
export function planValidForVehicle(v: VehicleSummary, wps: readonly ArduWaypoint[]): boolean {
  const sys = vehicleSystem(v);
  if (!sys) return false;
  const cls = vehicleClassOf(v);
  return wps.every((w) => (sys === 'px4' ? cmdValidForPx4(w.command, cls) : cmdValidForVehicle(w.command, cls)));
}

export type CheckLevel = 'ok' | 'warn' | 'fail';

export interface PreflightResult {
  level: CheckLevel;
  /** i18n key suffixes under `fleet.check.*`. */
  reasons: string[];
}

/** Gate a single vehicle for `kind`. The operator picked these vehicles on purpose, so the default is
 *  to INCLUDE them: `fail` (unticked by default in the confirm dialog, can be opted back in) is reserved
 *  for cases where sending is structurally pointless — not a MAVLink/known-firmware vehicle, link lost,
 *  mode not available. Vehicle STATE (not armed yet, no GPS fix, low battery, pre-arm blockers) is a
 *  `warn`: ticked, but flagged so the operator sees what the FC is likely to reject. Note PX4 arms
 *  itself on a takeoff command, and ArduPilot surfaces its own denial via COMMAND_ACK. */
export function preflightChecks(
  kind: GroupCommandKind,
  v: VehicleSummary,
  tv: TelemetryData | undefined,
  intent?: FleetModeIntent,
  now = Date.now(),
  plan?: readonly ArduWaypoint[],
): PreflightResult {
  const fails: string[] = [];
  const warns: string[] = [];
  if (kind === 'missionUpload' && plan && plan.length === 0) fails.push('emptyPlan');

  if (!isMavlinkVehicle(v)) fails.push('notMavlink');
  if (!vehicleSystem(v)) fails.push('unknownFirmware');
  const ls = linkState(tv, now);
  if (ls === 'lost') fails.push('linkLost');
  else if (ls === 'stale') warns.push('linkStale');

  if (tv) {
    const armed = isVehicleArmed(tv);
    const gpsOk = tv.fixType >= 2 && tv.numSat >= MIN_FIX_SATELLITES; // unified scale: 2 = 3D
    const lowBat = tv.batteryPercentage > 0 && tv.batteryPercentage < LOW_BATTERY_PCT;
    switch (kind) {
      case 'arm':
        if (armed) warns.push('alreadyArmed');
        if (!gpsOk) warns.push('noGpsFix');
        if (lowBat) warns.push('lowBattery');
        if (tv.prearmHealthy === 2) warns.push('prearmBlocked');
        break;
      case 'disarm':
        if (!armed) warns.push('notArmed');
        break;
      case 'takeoff':
        if (!armed) warns.push('notArmed');
        if (!gpsOk) warns.push('noGpsFix');
        if (lowBat) warns.push('lowBattery');
        break;
      case 'land':
        if (!armed) warns.push('notArmed');
        // ArduPilot: only Copter (NAV_LAND) and QuadPlane (QLAND) have a land-now path — a plain plane
        // lands via an AUTO sequence / RTL, a rover has nothing to land.
        if (vehicleSystem(v) === 'ardupilot') {
          const cls = vehicleClassOf(v);
          if (cls !== 'copter' && cls !== 'quadplane') fails.push('modeUnsupported');
        }
        break;
      case 'rtl':
      case 'hold':
      case 'missionStart':
      case 'changeSpeed':
        if (!armed) warns.push('notArmed');
        break;
      case 'setMode':
        if (intent && !resolveIntentMode(v, intent)) fails.push('modeUnsupported');
        break;
      case 'missionUpload':
        // Replacing the mission of a flying aircraft is legal but rarely intended.
        if (armed) warns.push('armedUpload');
        if (plan && plan.length > 0 && !planValidForVehicle(v, plan)) warns.push('cmdInvalid');
        break;
    }
  }

  if (fails.length) return { level: 'fail', reasons: [...fails, ...warns] };
  if (warns.length) return { level: 'warn', reasons: warns };
  return { level: 'ok', reasons: [] };
}
