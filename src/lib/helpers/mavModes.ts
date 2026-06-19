// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink flight-mode tables for the vehicle-control panel (ArduPilot + PX4).
//
// `main`/`sub` are the firmware-specific custom_mode parts forwarded to MAV_CMD_DO_SET_MODE:
//   - ArduPilot: `main` = the flat Mode::Number, `sub` = 0.
//   - PX4: `main` = PX4_CUSTOM_MAIN_MODE_*, `sub` = PX4_CUSTOM_SUB_MODE_AUTO_* (0 if none).
//
// Numbers verified against ArduCopter mode.h, ArduPlane/Rover mode tables, and PX4 px4_custom_mode.h.
// `stick` flags modes that need transmitter sticks to fly — locked unless an RC link is present
// (a GCS-only switch into one of these without a TX is an instant crash). See docs/active/VEHICLE_CONTROL.md.

import type { VehicleClass } from './arduCommandCatalog';
import type { AutopilotSystem } from '$lib/stores/autopilotContext';

export interface MavMode {
  /** Stable key — i18n label resolved as `control.mode.<key>`, falling back to `name`. */
  key: string;
  /** English fallback / canonical name. */
  name: string;
  /** ArduPilot: flat custom_mode. PX4: main mode. */
  main: number;
  /** PX4 sub mode (0 for ArduPilot). */
  sub: number;
  /** Needs transmitter sticks to be safe — locked without an RC link. */
  stick: boolean;
  /** The reposition-ready / Guided mode driven by the Guided toggle. */
  guided?: boolean;
  /** VTOL-only (QuadPlane) — only offered for that class. */
  vtol?: boolean;
}

// ── ArduCopter (Mode::Number) ───────────────────────────────────────────────
const COPTER: MavMode[] = [
  { key: 'guided',    name: 'Guided',    main: 4,  sub: 0, stick: false, guided: true },
  { key: 'auto',      name: 'Auto',      main: 3,  sub: 0, stick: false },
  { key: 'loiter',    name: 'Loiter',    main: 5,  sub: 0, stick: false },
  { key: 'rtl',       name: 'RTL',       main: 6,  sub: 0, stick: false },
  { key: 'circle',    name: 'Circle',    main: 7,  sub: 0, stick: false },
  { key: 'land',      name: 'Land',      main: 9,  sub: 0, stick: false },
  { key: 'poshold',   name: 'PosHold',   main: 16, sub: 0, stick: false },
  { key: 'brake',     name: 'Brake',     main: 17, sub: 0, stick: false },
  { key: 'smartrtl',  name: 'SmartRTL',  main: 21, sub: 0, stick: false },
  { key: 'stabilize', name: 'Stabilize', main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',      main: 1,  sub: 0, stick: true },
  { key: 'althold',   name: 'AltHold',   main: 2,  sub: 0, stick: true },
  { key: 'sport',     name: 'Sport',     main: 13, sub: 0, stick: true },
];

// ── ArduPlane (Mode::Number). Q* modes are VTOL-only (QuadPlane). ────────────
const PLANE: MavMode[] = [
  { key: 'auto',      name: 'Auto',      main: 10, sub: 0, stick: false },
  { key: 'guided',    name: 'Guided',    main: 15, sub: 0, stick: false, guided: true },
  { key: 'rtl',       name: 'RTL',       main: 11, sub: 0, stick: false },
  { key: 'loiter',    name: 'Loiter',    main: 12, sub: 0, stick: false },
  { key: 'cruise',    name: 'Cruise',    main: 7,  sub: 0, stick: false },
  { key: 'circle',    name: 'Circle',    main: 1,  sub: 0, stick: false },
  { key: 'takeoff',   name: 'Takeoff',   main: 13, sub: 0, stick: false },
  { key: 'qrtl',      name: 'QRTL',      main: 21, sub: 0, stick: false, vtol: true },
  { key: 'qloiter',   name: 'QLoiter',   main: 19, sub: 0, stick: false, vtol: true },
  { key: 'qland',     name: 'QLand',     main: 20, sub: 0, stick: false, vtol: true },
  { key: 'qhover',    name: 'QHover',    main: 18, sub: 0, stick: false, vtol: true },
  { key: 'manual',    name: 'Manual',    main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',      main: 4,  sub: 0, stick: true },
  { key: 'fbwa',      name: 'FBWA',      main: 5,  sub: 0, stick: true },
  // FBWB: TECS auto-throttle holds altitude + airspeed, but there is NO autonomous course control
  // (the pilot steers via roll) — without an RC link it can't be steered, so it is stick-dependent.
  { key: 'fbwb',      name: 'FBWB',      main: 6,  sub: 0, stick: true },
  { key: 'stabilize', name: 'Stabilize', main: 2,  sub: 0, stick: true },
];

// ── ArduRover (Mode::Number) ─────────────────────────────────────────────────
const ROVER: MavMode[] = [
  { key: 'auto',      name: 'Auto',     main: 10, sub: 0, stick: false },
  { key: 'guided',    name: 'Guided',   main: 15, sub: 0, stick: false, guided: true },
  { key: 'rtl',       name: 'RTL',      main: 11, sub: 0, stick: false },
  { key: 'loiter',    name: 'Loiter',   main: 5,  sub: 0, stick: false },
  { key: 'smartrtl',  name: 'SmartRTL', main: 12, sub: 0, stick: false },
  { key: 'hold',      name: 'Hold',     main: 4,  sub: 0, stick: false },
  { key: 'manual',    name: 'Manual',   main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',     main: 1,  sub: 0, stick: true },
  { key: 'steering',  name: 'Steering', main: 3,  sub: 0, stick: true },
];

// ── PX4 (main_mode / auto sub_mode) ──────────────────────────────────────────
// HOLD (= AUTO_LOITER) is the reposition-ready state for the Guided toggle (PX4 has no "GUIDED").
const PX4: MavMode[] = [
  { key: 'hold',      name: 'Hold',       main: 4, sub: 3, stick: false, guided: true },
  { key: 'mission',   name: 'Mission',    main: 4, sub: 4, stick: false },
  { key: 'return',    name: 'Return',     main: 4, sub: 5, stick: false },
  { key: 'takeoff',   name: 'Takeoff',    main: 4, sub: 2, stick: false },
  { key: 'land',      name: 'Land',       main: 4, sub: 6, stick: false },
  { key: 'manual',    name: 'Manual',     main: 1, sub: 0, stick: true },
  { key: 'altitude',  name: 'Altitude',   main: 2, sub: 0, stick: true },
  { key: 'position',  name: 'Position',   main: 3, sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',       main: 5, sub: 0, stick: true },
  { key: 'stabilized', name: 'Stabilized', main: 7, sub: 0, stick: true },
];

/** All modes available for the given firmware + vehicle class (unfiltered by RC-presence). */
export function modesFor(system: AutopilotSystem, cls: VehicleClass): MavMode[] {
  if (system === 'px4') return PX4;
  // ArduPilot — pick the table by vehicle class.
  switch (cls) {
    case 'copter':    return COPTER;
    case 'quadplane': return PLANE;                          // plane + Q* modes
    case 'plane':     return PLANE.filter((m) => !m.vtol);   // hide VTOL-only modes on a plain plane
    case 'rover':
    case 'boat':      return ROVER;
    case 'sub':       return ROVER;                          // base set; Sub-specific modes TBD
    default:          return PLANE;
  }
}

/** The Guided/reposition-ready mode for the toggle (GUIDED on ArduPilot, HOLD on PX4). */
export function guidedModeFor(system: AutopilotSystem, cls: VehicleClass): MavMode | undefined {
  return modesFor(system, cls).find((m) => m.guided);
}

/** Decode a raw `custom_mode` into (main, sub) for the active firmware. */
export function decodeCustomMode(system: AutopilotSystem, customMode: number): { main: number; sub: number } {
  if (system === 'px4') {
    return { main: (customMode >>> 16) & 0xff, sub: (customMode >>> 24) & 0xff };
  }
  // ArduPilot: the whole custom_mode is the flat mode number.
  return { main: customMode >>> 0, sub: 0 };
}

/** Find the table entry matching the FC's raw `custom_mode`, or undefined if unknown. */
export function matchActiveMode(
  system: AutopilotSystem,
  cls: VehicleClass,
  customMode: number,
): MavMode | undefined {
  const { main, sub } = decodeCustomMode(system, customMode);
  return modesFor(system, cls).find((m) => m.main === main && m.sub === sub);
}
