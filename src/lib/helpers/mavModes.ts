// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink flight-mode tables for the vehicle-control panel (ArduPilot + PX4).
//
// `main`/`sub` are the firmware-specific custom_mode parts forwarded to MAV_CMD_DO_SET_MODE:
//   - ArduPilot: `main` = the flat Mode::Number, `sub` = 0.
//   - PX4: `main` = PX4_CUSTOM_MAIN_MODE_*, `sub` = PX4_CUSTOM_SUB_MODE_AUTO_* (0 if none).
//
// Numbers verified against ArduCopter mode.h, ArduPlane/Rover/Sub mode tables, and PX4 px4_custom_mode.h.
// `stick` flags modes that need transmitter sticks to fly — locked unless an RC link is present
// (a GCS-only switch into one of these without a TX is an instant crash). See docs/active/VEHICLE_CONTROL.md.
//
// ArduPilot 4.7+ also reports its own mode list (MAVLink AVAILABLE_MODES, read per vehicle by the
// backend — `stores/fcModes`). With that list the FC decides WHICH modes exist; these tables still
// decide label, order, stick lock and the Guided flag (`offeredModes`).

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
  /** Entered by the firmware itself, or a diagnostic: known for the active-mode readout, never offered —
   *  not even when the FC reports it as selectable. */
  neverOffer?: boolean;
  /** Exists only in firmware that also reports its mode list (ArduPilot 4.7+): offered when the FC
   *  lists it, never from the static fallback (older firmware would only refuse it). */
  listedOnly?: boolean;
  /** Returns or lands the vehicle — the quick grid asks for hold-to-confirm, like the RTL
   *  and Land buttons, instead of a single click. */
  confirm?: boolean;
}

/** One mode of the FC's own list (AVAILABLE_MODES, ArduPilot 4.7+) — the backend's `available-modes`. */
export interface FcMode {
  /** ArduPilot: the flat mode number. */
  customMode: number;
  /** MAV_MODE_PROPERTY bitmask. */
  properties: number;
  /** The firmware's own name. */
  name: string;
}

/** MAV_MODE_PROPERTY_NOT_USER_SELECTABLE — the FC refuses this mode from a GCS. */
const NOT_USER_SELECTABLE = 2;

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
  { key: 'smartrtl',  name: 'SmartRTL',  main: 21, sub: 0, stick: false, confirm: true },
  // Jumps to the mission's landing sequence.
  { key: 'autortl',   name: 'Auto RTL',  main: 27, sub: 0, stick: false, confirm: true },
  { key: 'follow',    name: 'Follow',    main: 23, sub: 0, stick: false },
  { key: 'stabilize', name: 'Stabilize', main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',      main: 1,  sub: 0, stick: true },
  { key: 'althold',   name: 'AltHold',   main: 2,  sub: 0, stick: true },
  { key: 'sport',     name: 'Sport',     main: 13, sub: 0, stick: true },
  { key: 'drift',     name: 'Drift',     main: 11, sub: 0, stick: true },
  { key: 'flowhold',  name: 'FlowHold',  main: 22, sub: 0, stick: true },
  { key: 'zigzag',    name: 'ZigZag',    main: 24, sub: 0, stick: true },
  { key: 'autotune',  name: 'AutoTune',  main: 15, sub: 0, stick: true },
  { key: 'systemid',  name: 'SystemID',  main: 25, sub: 0, stick: true },
  { key: 'flip',      name: 'Flip',      main: 14, sub: 0, stick: true },
  // Entered before arming, then thrown by hand.
  { key: 'throw',     name: 'Throw',     main: 18, sub: 0, stick: true },
  // Needs a stream of attitude targets (companion computer) — useless from the panel alone.
  { key: 'guidednogps', name: 'Guided NoGPS', main: 20, sub: 0, stick: true },
  { key: 'autorotate', name: 'Autorotate', main: 26, sub: 0, stick: true },
  { key: 'turtle',    name: 'Turtle',    main: 28, sub: 0, stick: true },
  { key: 'avoidadsb', name: 'Avoid ADSB', main: 19, sub: 0, stick: false, neverOffer: true },
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
  { key: 'autoland',  name: 'AutoLand',  main: 26, sub: 0, stick: false, listedOnly: true, confirm: true },
  { key: 'thermal',   name: 'Thermal',   main: 24, sub: 0, stick: false },
  { key: 'qrtl',      name: 'QRTL',      main: 21, sub: 0, stick: false, vtol: true, confirm: true },
  { key: 'qloiter',   name: 'QLoiter',   main: 19, sub: 0, stick: false, vtol: true },
  { key: 'qland',     name: 'QLand',     main: 20, sub: 0, stick: false, vtol: true, confirm: true },
  { key: 'loiterqland', name: 'Loiter QLand', main: 25, sub: 0, stick: false, vtol: true, confirm: true },
  // QHover holds altitude only — the pilot holds position, so it needs sticks like Copter's AltHold.
  { key: 'qhover',    name: 'QHover',    main: 18, sub: 0, stick: true,  vtol: true },
  { key: 'qstabilize', name: 'QStabilize', main: 17, sub: 0, stick: true, vtol: true },
  { key: 'qacro',     name: 'QAcro',     main: 23, sub: 0, stick: true,  vtol: true },
  { key: 'qautotune', name: 'QAutoTune', main: 22, sub: 0, stick: true,  vtol: true },
  { key: 'manual',    name: 'Manual',    main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',      main: 4,  sub: 0, stick: true },
  { key: 'fbwa',      name: 'FBWA',      main: 5,  sub: 0, stick: true },
  // FBWB: TECS auto-throttle holds altitude + airspeed, but there is NO autonomous course control
  // (the pilot steers via roll) — without an RC link it can't be steered, so it is stick-dependent.
  { key: 'fbwb',      name: 'FBWB',      main: 6,  sub: 0, stick: true },
  { key: 'stabilize', name: 'Stabilize', main: 2,  sub: 0, stick: true },
  { key: 'training',  name: 'Training',  main: 3,  sub: 0, stick: true },
  { key: 'autotune',  name: 'AutoTune',  main: 8,  sub: 0, stick: true },
  { key: 'avoidadsb', name: 'Avoid ADSB', main: 14, sub: 0, stick: false, neverOffer: true },
  { key: 'initialising', name: 'Initialising', main: 16, sub: 0, stick: false, neverOffer: true },
];

// ── ArduRover (Mode::Number) ─────────────────────────────────────────────────
const ROVER: MavMode[] = [
  { key: 'auto',      name: 'Auto',     main: 10, sub: 0, stick: false },
  { key: 'guided',    name: 'Guided',   main: 15, sub: 0, stick: false, guided: true },
  { key: 'rtl',       name: 'RTL',      main: 11, sub: 0, stick: false },
  { key: 'loiter',    name: 'Loiter',   main: 5,  sub: 0, stick: false },
  { key: 'smartrtl',  name: 'SmartRTL', main: 12, sub: 0, stick: false, confirm: true },
  { key: 'hold',      name: 'Hold',     main: 4,  sub: 0, stick: false },
  { key: 'circle',    name: 'Circle',   main: 9,  sub: 0, stick: false },
  { key: 'follow',    name: 'Follow',   main: 6,  sub: 0, stick: false },
  { key: 'dock',      name: 'Dock',     main: 8,  sub: 0, stick: false },
  { key: 'manual',    name: 'Manual',   main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',     main: 1,  sub: 0, stick: true },
  { key: 'steering',  name: 'Steering', main: 3,  sub: 0, stick: true },
  { key: 'simple',    name: 'Simple',   main: 7,  sub: 0, stick: true },
  { key: 'initialising', name: 'Initialising', main: 16, sub: 0, stick: false, neverOffer: true },
];

// ── ArduSub (Mode::Number) ───────────────────────────────────────────────────
const SUB: MavMode[] = [
  { key: 'guided',    name: 'Guided',     main: 4,  sub: 0, stick: false, guided: true },
  { key: 'auto',      name: 'Auto',       main: 3,  sub: 0, stick: false },
  { key: 'poshold',   name: 'PosHold',    main: 16, sub: 0, stick: false },
  { key: 'circle',    name: 'Circle',     main: 7,  sub: 0, stick: false },
  { key: 'surface',   name: 'Surface',    main: 9,  sub: 0, stick: false, confirm: true },
  { key: 'stabilize', name: 'Stabilize',  main: 0,  sub: 0, stick: true },
  { key: 'acro',      name: 'Acro',       main: 1,  sub: 0, stick: true },
  { key: 'depthhold', name: 'Depth Hold', main: 2,  sub: 0, stick: true },
  { key: 'manual',    name: 'Manual',     main: 19, sub: 0, stick: true },
  { key: 'surftrak',  name: 'Surftrak',   main: 21, sub: 0, stick: true },
  // Diagnostic — the FC lists it as selectable, the panel never offers it.
  { key: 'motordetect', name: 'Motor Detection', main: 20, sub: 0, stick: false, neverOffer: true },
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

/** The whole table of the firmware + vehicle class — every mode Kite knows, offered or not, VTOL modes
 *  included for any plane. For recognising the active mode (and the Debug Monitor). */
export function tableFor(system: AutopilotSystem, cls: VehicleClass): MavMode[] {
  if (system === 'px4') return PX4;
  switch (cls) {
    case 'copter':    return COPTER;
    case 'rover':
    case 'boat':      return ROVER;
    case 'sub':       return SUB;
    default:          return PLANE; // plane, quadplane
  }
}

/** Whether two firmware/class pairs use the same mode numbering — plane and QuadPlane do (one table),
 *  rover and boat do; copter and plane do not. */
export function sameModeTable(sysA: AutopilotSystem, clsA: VehicleClass, sysB: AutopilotSystem, clsB: VehicleClass): boolean {
  return tableFor(sysA, clsA) === tableFor(sysB, clsB);
}

/** The static list the panel offers for the given firmware + vehicle class (unfiltered by RC-presence):
 *  the table minus the never-offered modes, and minus the VTOL modes on a plain plane. */
export function modesFor(system: AutopilotSystem, cls: VehicleClass): MavMode[] {
  return tableFor(system, cls).filter((m) => !m.neverOffer && !m.listedOnly && !(cls === 'plane' && m.vtol));
}

/** A mode the FC reports that Kite has no table entry for (e.g. a Lua-registered one): the FC's own
 *  name, stick-locked — an unknown mode is assumed to need a pilot. */
function unknownFcMode(m: FcMode): MavMode {
  const n = m.customMode >>> 0;
  return { key: `fc${n}`, name: m.name || `#${n}`, main: n, sub: 0, stick: true };
}

/**
 * The modes the panel offers. Without an FC list (PX4, ArduPilot before 4.7, list not read yet) the
 * static `modesFor`. With one (ArduPilot): the table's modes the FC lists as user-selectable, in the
 * table's order, then the FC's modes the table does not know. A table mode the FC does not list is not
 * offered; a never-offered table mode stays out even when the FC lists it as selectable.
 */
export function offeredModes(system: AutopilotSystem, cls: VehicleClass, fc: readonly FcMode[] | null | undefined): MavMode[] {
  if (system !== 'ardupilot' || !fc || fc.length === 0) return modesFor(system, cls);
  const selectable = new Map<number, FcMode>();
  for (const m of fc) {
    if ((m.properties & NOT_USER_SELECTABLE) === 0) selectable.set(m.customMode >>> 0, m);
  }
  const table = tableFor(system, cls);
  const known = new Set(table.map((m) => m.main));
  const offered = table.filter((m) => !m.neverOffer && selectable.has(m.main));
  for (const [n, m] of selectable) {
    if (!known.has(n)) offered.push(unknownFcMode(m));
  }
  return offered;
}

/** The Guided/reposition-ready mode for the toggle (GUIDED on ArduPilot, HOLD on PX4). With an FC list
 *  (ArduPilot) only when the FC offers it — undefined means the vehicle has no usable guided mode. */
export function guidedModeFor(system: AutopilotSystem, cls: VehicleClass, fc?: readonly FcMode[] | null): MavMode | undefined {
  return offeredModes(system, cls, fc).find((m) => m.guided);
}

/** Decode a raw `custom_mode` into (main, sub) for the active firmware. */
export function decodeCustomMode(system: AutopilotSystem, customMode: number): { main: number; sub: number } {
  if (system === 'px4') {
    return { main: (customMode >>> 16) & 0xff, sub: (customMode >>> 24) & 0xff };
  }
  // ArduPilot: the whole custom_mode is the flat mode number.
  return { main: customMode >>> 0, sub: 0 };
}

/** Find the mode matching the FC's raw `custom_mode`: Kite's table first, then (ArduPilot) the FC's own
 *  list, so a mode Kite does not know shows the FC's name. Undefined if neither knows it. */
export function matchActiveMode(
  system: AutopilotSystem,
  cls: VehicleClass,
  customMode: number,
  fc?: readonly FcMode[] | null,
): MavMode | undefined {
  const { main, sub } = decodeCustomMode(system, customMode);
  const hit = tableFor(system, cls).find((m) => m.main === main && m.sub === sub);
  if (hit || system !== 'ardupilot' || !fc) return hit;
  const f = fc.find((m) => m.customMode >>> 0 === main);
  return f ? unknownFcMode(f) : undefined;
}
