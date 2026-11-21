// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle-control controller (no UI) for direct GCS command of a MAVLink vehicle (ArduPilot + PX4): wraps the
// Tauri commands, surfaces COMMAND_ACK results as feedback, and owns the Guided-toggle state + RC-presence
// lock. See docs/active/VEHICLE_CONTROL.md.

import { invoke } from '@tauri-apps/api/core';
import { writable, derived, get, type Readable } from 'svelte/store';
import { telemetry } from '$lib/stores/telemetry';
import { connection, isArduPilotLink, linkIsArduPilot } from '$lib/stores/connection';
import { activeVehicleId, activeVehicle } from '$lib/stores/vehicles';
import { autopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';
import { settings } from '$lib/stores/settings';
import { arduVehicleClass, downloadArduMissionFromFc } from '$lib/stores/missionArdupilot';
import { rcEngaged } from '$lib/stores/rcEngage';
import { activeFcModes } from '$lib/stores/fcModes';
import { type MavMode, type FcMode, modesFor, guidedModeFor, matchActiveMode, sameModeTable } from '$lib/helpers/mavModes';
import { vehicleSystem, vehicleClassOf } from '$lib/helpers/fleetStatus';

// ── Feedback (COMMAND_ACK surfacing) ────────────────────────────────────────

export interface CommandFeedback {
  /** i18n key suffix for the action label (control.action.<action>). */
  action: string;
  ok: boolean;
  /** Resolved message (English error from the backend, or '' on success). */
  message: string;
  ts: number;
}

export const lastFeedback = writable<CommandFeedback | null>(null);
/** Name of the action currently in flight (awaiting its ACK), or null. Drives per-button spinners. */
export const busyAction = writable<string | null>(null);

/** Every vehicle-facing backend command takes an optional `vehicleId`; always name the active vehicle
 *  explicitly so a selection change between click and dispatch can't retarget a command. */
function targeted(args?: Record<string, unknown>): Record<string, unknown> {
  return { vehicleId: get(activeVehicleId), ...(args ?? {}) };
}

async function runCommand(action: string, cmd: string, args?: Record<string, unknown>): Promise<boolean> {
  busyAction.set(action);
  try {
    await invoke(cmd, targeted(args));
    lastFeedback.set({ action, ok: true, message: '', ts: Date.now() });
    return true;
  } catch (e) {
    lastFeedback.set({ action, ok: false, message: String(e), ts: Date.now() });
    return false;
  } finally {
    busyAction.set(null);
  }
}

// ── On the ground or flying ─────────────────────────────────────────────────

/** Whether the active vehicle has taken off since it was armed, as its FC reports it. Stays set until
 *  the disarm: the question is whether a takeoff is still ahead, not whether the wheels are down. */
export const airborne = writable<boolean>(false);

const MAV_LANDED_STATE_ON_GROUND = 1;
const MAV_STATE_ACTIVE = 4;
const ARMED_FLAG = 0x04;
// The EXTENDED_SYS_STATE stream is requested at connect and only needed until the vehicle flies:
// the vehicles it is switched off for.
const landedStreamOff = new Set<string>();

/** The active vehicle's own system: `autopilotSystem` follows a vehicle switch a few IPC turns late. */
function activeSystem(): AutopilotSystem | null {
  const v = get(activeVehicle);
  return v ? vehicleSystem(v) : null;
}

function setLandedStream(on: boolean): void {
  const id = get(activeVehicleId);
  if (!id || activeSystem() == null || landedStreamOff.has(id) === !on) return;
  // "Full MAVLink Telemetry" leaves the stream rates to the FC.
  if (!on && get(settings).mavlinkFullTelemetry) return;
  if (on) landedStreamOff.delete(id);
  else landedStreamOff.add(id);
  invoke('mav_landed_state_stream', targeted({ on })).catch(() => {
    if (on) landedStreamOff.add(id);
    else landedStreamOff.delete(id);
  });
}

function markAirborne(): void {
  if (get(airborne) || (get(telemetry).armingFlags & ARMED_FLAG) === 0) return;
  airborne.set(true);
  setLandedStream(false);
}

/** MAV_LANDED_STATE of the active vehicle: in the air, taking off or landing (0 = no statement). */
export function ingestLandedState(state: number): void {
  if (state > MAV_LANDED_STATE_ON_GROUND) markAirborne();
}

/** MAV_STATE of the active vehicle's heartbeat. Only ArduPilot ties ACTIVE to flying (PX4: to armed). */
export function ingestSystemStatus(status: number): void {
  if (status === MAV_STATE_ACTIVE && activeSystem() === 'ardupilot') markAirborne();
}

/** `connected`: the vehicle may have had its stream switched off earlier — request it again. A new
 *  connection requests it by itself. */
function resetFlightState(connected: boolean): void {
  airborne.set(false);
  if (connected) setLandedStream(true);
  else landedStreamOff.clear();
}

// ── Guided toggle + reposition params ───────────────────────────────────────

/** Map-interaction "Guided" intent: when on, a map click opens the Fly-Here popup. */
export const guidedActive = writable<boolean>(false);

/** The active Guided "Fly Here" / loiter target (degrees), or null. Drives the map's loiter-target
 *  marker — shown while Guided is active and a target has been set, cleared on a mode change / leaving
 *  Guided. Protocol-agnostic (no MAVLink dependency) so INAV guided control can reuse it post-1.0. */
export const guidedTarget = writable<{ lat: number; lon: number } | null>(null);

export interface GuidedParams {
  /** Target altitude (m, relative to home). */
  alt: number;
  /** Ground speed (m/s); null = firmware default. */
  speed: number | null;
  /** Yaw heading (deg); null = keep current (multirotor only). */
  yaw: number | null;
  /** Loiter radius (m, fixed-wing only); null = default. */
  loiterRadius: number | null;
}

/** Last-used Fly-Here values, remembered for the next click (session-scoped). */
export const guidedParams = writable<GuidedParams>({ alt: 50, speed: null, yaw: null, loiterRadius: null });

/** The FC's configured default loiter radius (m, magnitude — `WP_LOITER_RAD` / `NAV_LOITER_RAD`),
 *  read once when Guided is activated. Drives the map's expected-loiter-track ring when no explicit
 *  Fly-Here radius is set. null = unknown / not applicable (multirotor). */
export const fcLoiterRadius = writable<number | null>(null);

// One param read per connection (the ingest path retriggers at 1 Hz and must not spam
// PARAM_REQUEST_READ, e.g. on a copter where the FC never reports WP_LOITER_RAD).
let loiterRadiusRequested = false;
connection.subscribe((c) => {
  if (c.status !== 'connected') {
    loiterRadiusRequested = false;
    fcLoiterRadius.set(null); // never carry a stale radius into the next session
    resetFlightState(false);
  }
});

/** Read the FC's default loiter radius for the ring (fixed-wing only — a copter holds the point). */
async function refreshFcLoiterRadius(): Promise<void> {
  loiterRadiusRequested = true;
  const cls = get(arduVehicleClass);
  if (cls !== 'plane' && cls !== 'quadplane') {
    fcLoiterRadius.set(null);
    return;
  }
  const name = get(autopilotSystem) === 'px4' ? 'NAV_LOITER_RAD' : 'WP_LOITER_RAD';
  try {
    const v = await invoke<number | null>('mav_read_param', targeted({ name }));
    // Negative = counter-clockwise loiter; the ring only needs the magnitude.
    fcLoiterRadius.set(v != null ? Math.abs(v) : null);
  } catch {
    fcLoiterRadius.set(null);
  }
}

/** FC-confirmed guided targets of EVERY vehicle (multi-vehicle), keyed by vehicle id, with the time
 *  the last POSITION_TARGET_GLOBAL_INT arrived. The map draws "who is heading where" from this; the
 *  active vehicle additionally goes through `ingestFcGuidedTarget` → `guidedTarget` for the Guided UI. */
export const guidedTargets = writable<ReadonlyMap<string, { lat: number; lon: number; ts: number }>>(new Map());

export function ingestVehicleGuidedTarget(vehicleId: string, lat: number, lon: number): void {
  guidedTargets.update((m) => {
    const cur = m.get(vehicleId);
    // Same ~0.1 m dedup as the active path, but the timestamp still refreshes so staleness works.
    if (cur && Math.abs(cur.lat - lat) < 1e-6 && Math.abs(cur.lon - lon) < 1e-6 && Date.now() - cur.ts < 2000) return m;
    const next = new Map(m);
    next.set(vehicleId, { lat, lon, ts: Date.now() });
    return next;
  });
}

/** FC-confirmed target (POSITION_TARGET_GLOBAL_INT, 1 Hz), accepted only in the FC's guided mode — in AUTO/RTL
 *  the same message carries the mission WP / home. Includes targets set by another GCS. */
export function ingestFcGuidedTarget(lat: number, lon: number): void {
  if (get(activeMode)?.guided !== true) return;
  // Connected mid-flight to a vehicle already in guided → the toggle was never pressed, so fetch
  // the loiter radius for the ring here (once per connection).
  if (!loiterRadiusRequested) void refreshFcLoiterRadius();
  const cur = get(guidedTarget);
  // ~0.1 m dedup so the 1 Hz push doesn't churn every subscriber with sub-metre jitter.
  if (cur && Math.abs(cur.lat - lat) < 1e-6 && Math.abs(cur.lon - lon) < 1e-6) return;
  guidedTarget.set({ lat, lon });
}

// ArduPlane's GUIDED_CHANGE_HEADING sets a *sticky* heading override that bypasses waypoint nav and
// is only cleared by a mode change (current firmware does not clear it on DO_REPOSITION). We track it
// so a subsequent "Fly Here" can explicitly clear it first, otherwise the reposition is ignored.
let headingOverrideActive = false;

// ── Connection / vehicle gating ─────────────────────────────────────────────

/** True when connected over MAVLink to ArduPilot/PX4 (the only autopilots the control panel supports
 *  in V1) — not on an INAV MSP-over-MAVLink link. */
export const controlAvailable: Readable<boolean> = isArduPilotLink;

/** "RC transmitter present": primarily the FC's RC_RECEIVER health bit (SYS_STATUS) — set for ANY live RC
 *  source, incl. a MAVLink-RC override without RSSI — else a reported RC RSSI (INAV). Without either,
 *  stick-required modes stay locked. */
export const rcLinkPresent: Readable<boolean> = derived(
  telemetry,
  (t) => t.sensorRcReceiver === 1 || (t.link.rssiPercent != null && t.link.rssiPercent > 0),
);

/** Stick-flown modes are selectable only with a usable RC source: a physical transmitter (rcLinkPresent) or
 *  Kite's own RC control streaming sticks (RC_CHANNELS_OVERRIDE / MANUAL_CONTROL) — else the vehicle would
 *  have no control input. */
export const stickModesUnlocked: Readable<boolean> = derived(
  [rcLinkPresent, rcEngaged],
  ([rc, eng]) => rc || eng.on,
);

/** Armed state from the unified arming flags (bit 2 = armed, matches the recorder convention). */
export const isArmed: Readable<boolean> = derived(telemetry, (t) => (t.armingFlags & 0x04) !== 0);

/** Whether `autopilotSystem` / `arduVehicleClass` already describe the active vehicle's mode numbering.
 *  On a vehicle switch `activeVehicleId` (and with it the FC mode list) changes at once, while system and
 *  class follow `connection` one or two IPC turns later. True when the active vehicle is not known yet. */
export const modeTableMatchesVehicle: Readable<boolean> = derived(
  [activeVehicle, autopilotSystem, arduVehicleClass],
  ([v, sys, cls]) => {
    if (!v) return true;
    const vSys = vehicleSystem(v);
    // Plane vs QuadPlane is the same table: the QuadPlane class comes from the Q_ENABLE probe, while
    // the vehicle summary of a QuadPlane reporting FIXED_WING still says "plane".
    return vSys != null && sameModeTable(vSys, vehicleClassOf(v), sys, cls);
  },
);

/** The active vehicle's FC mode list, only while system and class match that vehicle (see above) —
 *  otherwise null: the old class's table must not be filtered by the new vehicle's mode numbers. */
export const activeFcModesChecked: Readable<readonly FcMode[] | null> = derived(
  [activeFcModes, modeTableMatchesVehicle],
  ([fc, match]) => (match ? fc : null),
);

/** The FC's currently active mode (matched against the firmware/vehicle table, then the FC's own mode
 *  list), or undefined. Only on an ArduPilot/PX4 MAVLink link: elsewhere `flightModeFlags` is INAV's mode
 *  bitmask, whose values would "match" an ArduPilot mode number (bit 4 = Copter/Sub GUIDED → Fly Here). */
export const activeMode: Readable<MavMode | undefined> = derived(
  [telemetry, autopilotSystem, arduVehicleClass, connection, activeFcModesChecked],
  ([t, sys, cls, c, fc]) => (linkIsArduPilot(c) ? matchActiveMode(sys, cls, t.flightModeFlags, fc) : undefined),
);

// ── Commands ────────────────────────────────────────────────────────────────

/** Switch flight mode. Turns the Guided toggle on/off to match whether the target is the guided mode. */
export async function setMode(mode: MavMode): Promise<boolean> {
  const ok = await runCommand('setMode', 'mav_set_mode', { main: mode.main, sub: mode.sub });
  if (ok) {
    guidedActive.set(!!mode.guided);
    guidedTarget.set(null); // a mode change abandons the previous Guided target
    headingOverrideActive = false; // mode re-entry clears the heading slew
  }
  return ok;
}

export function arm(force = false): Promise<boolean> {
  return runCommand('arm', 'mav_arm', { arm: true, force });
}

export function disarm(force = false): Promise<boolean> {
  return runCommand('disarm', 'mav_arm', { arm: false, force });
}

/** Take off to `altitude` (m) via NAV_TAKEOFF (multirotor / QuadPlane / PX4; plain ArduPlane: `planeTakeoff`).
 *  ArduPilot switches to GUIDED first when the FC lists it as selectable; unarmed or without GUIDED the FC
 *  rejects the takeoff and the error surfaces. PX4 takes NAV_TAKEOFF directly. */
export async function takeoff(altitude: number): Promise<boolean> {
  guidedTarget.set(null); // fresh flight phase — no carried-over Guided target
  if (get(autopilotSystem) === 'ardupilot') {
    const g = guidedModeFor('ardupilot', get(arduVehicleClass), get(activeFcModesChecked));
    if (g && get(activeMode)?.key !== g.key) {
      try {
        await invoke('mav_set_mode', targeted({ main: g.main, sub: g.sub }));
        guidedActive.set(true);
      } catch {
        // fall through — let the takeoff attempt surface the real error
      }
    }
  }
  return runCommand('takeoff', 'mav_takeoff', { altitude });
}

/** Plain ArduPlane take-off: switch to TAKEOFF mode (NAV_TAKEOFF is the QuadPlane VTOL take-off). Disarmed the
 *  mode waits; once armed the motor runs up and the plane climbs to TKOFF_ALT, then loiters. `mode` is the
 *  table's TAKEOFF entry. */
export async function planeTakeoff(mode: MavMode): Promise<boolean> {
  const ok = await runCommand('takeoff', 'mav_set_mode', { main: mode.main, sub: mode.sub });
  if (ok) {
    guidedActive.set(false);
    guidedTarget.set(null);
    headingOverrideActive = false;
  }
  return ok;
}

export function land(): Promise<boolean> {
  // QuadPlane lands vertically via the QLAND mode (NAV_LAND is not the VTOL land path). Plain
  // ArduPlane fixed-wing has no land-now command (landing is an AUTO mission sequence / RTL) — the
  // panel hides the button there. Copter and PX4 (multirotor + fixed-wing Land mode) use NAV_LAND.
  if (get(autopilotSystem) === 'ardupilot' && get(arduVehicleClass) === 'quadplane') {
    const m = modesFor('ardupilot', 'quadplane').find((x) => x.key === 'qland');
    if (m) return runCommand('land', 'mav_set_mode', { main: m.main, sub: m.sub });
  }
  return runCommand('land', 'mav_land');
}

export function rtl(): Promise<boolean> {
  return runCommand('rtl', 'mav_rtl');
}

export function changeSpeed(speed: number, airspeed: boolean): Promise<boolean> {
  return runCommand('changeSpeed', 'mav_change_speed', { speedType: airspeed ? 0 : 1, speed });
}

/** Change the target altitude by repositioning to the EXISTING Guided target (the loiter centre) — using
 *  the momentary position would break a fixed-wing out of its loiter. Falls back to the current position
 *  only without a Guided target. Needs a GPS fix and the reposition-ready mode. */
export function changeAlt(alt: number): Promise<boolean> {
  const tel = get(telemetry);
  if (tel.fixType < 2) {
    lastFeedback.set({ action: 'changeAlt', ok: false, message: 'No GPS fix', ts: Date.now() });
    return Promise.resolve(false);
  }
  const target = get(guidedTarget);
  const lat = target ? target.lat : tel.lat;
  const lon = target ? target.lon : tel.lon;
  const p = get(guidedParams);
  return runCommand('changeAlt', 'mav_reposition', {
    lat: Math.round(lat * 1e7),
    lon: Math.round(lon * 1e7),
    alt,
    groundSpeed: p.speed,
    yaw: null,
    loiterRadius: p.loiterRadius,
  });
}

/** Set the fixed-wing loiter radius (metres) via PARAM_SET. The parameter name is firmware-specific:
 *  ArduPilot `WP_LOITER_RAD`, PX4 `NAV_LOITER_RAD`. */
export async function setLoiterRadius(radius: number): Promise<boolean> {
  const name = get(autopilotSystem) === 'px4' ? 'NAV_LOITER_RAD' : 'WP_LOITER_RAD';
  const ok = await runCommand('setLoiterRadius', 'mav_set_param', { name, value: radius });
  if (ok) fcLoiterRadius.set(Math.abs(radius)); // keep the map's loiter ring in sync with the write
  return ok;
}

/** Set the home position to the vehicle's current location (DO_SET_HOME). */
export function setHomeHere(): Promise<boolean> {
  return runCommand('setHome', 'mav_set_home_here');
}

/** Abort a landing / go around (DO_GO_AROUND); altitude 0 = firmware default climb. */
export function abortLanding(): Promise<boolean> {
  return runCommand('abortLanding', 'mav_abort_landing', { altitude: 0 });
}

/** VTOL transition: `toFw` → forward flight, else hover. PX4 uses MAV_CMD_DO_VTOL_TRANSITION; ArduPlane's
 *  command path is AUTO-only, so it transitions by mode: forward → Guided (keeps GCS control), hover → QLOITER. */
export async function vtolTransition(toFw: boolean): Promise<boolean> {
  if (get(autopilotSystem) === 'px4') {
    return runCommand('vtolTransition', 'mav_vtol_transition', { toFw });
  }
  const key = toFw ? 'guided' : 'qloiter';
  const m = modesFor('ardupilot', 'quadplane').find((x) => x.key === key);
  if (!m) return false;
  const ok = await runCommand('vtolTransition', 'mav_set_mode', { main: m.main, sub: m.sub });
  if (ok) { guidedActive.set(!!m.guided); guidedTarget.set(null); headingOverrideActive = false; }
  return ok;
}

/** Set the Guided target heading (deg): ArduPlane flies the course (GUIDED_CHANGE_HEADING), ArduCopter yaws
 *  to it (CONDITION_YAW). ArduPilot only — PX4 has no equivalent, the panel hides it there. */
export async function setHeading(heading: number): Promise<boolean> {
  const isPlane = get(arduVehicleClass) !== 'copter';
  const cmd = isPlane ? 'mav_guided_change_heading' : 'mav_condition_yaw';
  const ok = await runCommand('setHeading', cmd, { heading });
  // Only the fixed-wing GUIDED_CHANGE_HEADING is sticky; CONDITION_YAW (copter) just yaws the nose.
  if (ok && isPlane) headingOverrideActive = true;
  return ok;
}

export function missionStart(): Promise<boolean> {
  // PX4 has no MAV_CMD_MISSION_START handler — it runs/resumes missions by entering the Mission
  // flight mode. ArduPilot uses the MISSION_START command (begins/resumes from the current item).
  if (get(autopilotSystem) === 'px4') {
    const m = modesFor('px4', get(arduVehicleClass)).find((x) => x.key === 'mission');
    if (m) return runCommand('missionStart', 'mav_set_mode', { main: m.main, sub: m.sub });
  }
  return runCommand('missionStart', 'mav_mission_start');
}

/** Rewind the mission to the first item (MP's "Restart Mission" = MISSION_SET_CURRENT(0)). */
export function missionRestart(): Promise<boolean> {
  return runCommand('missionRestart', 'mav_mission_set_current', { seq: 0 });
}

/** Download the FC's mission into the working mission so the panel can command it (enables Set active
 *  WP). Not a COMMAND_ACK action, so it manages busy/feedback directly instead of via runCommand. */
export async function missionDownload(): Promise<boolean> {
  busyAction.set('missionDownload');
  try {
    await downloadArduMissionFromFc();
    lastFeedback.set({ action: 'missionDownload', ok: true, message: '', ts: Date.now() });
    return true;
  } catch (e) {
    lastFeedback.set({ action: 'missionDownload', ok: false, message: String(e), ts: Date.now() });
    return false;
  } finally {
    busyAction.set(null);
  }
}

/** Set the FC's active mission item to `seq` (FC item index, home-slot aware — see the panel). */
export function missionSetCurrent(seq: number): Promise<boolean> {
  return runCommand('setWp', 'mav_mission_set_current', { seq });
}

// Pause/continue is unreliable on ArduPilot (DO_PAUSE_CONTINUE often UNSUPPORTED); kept for PX4 /
// future use but not surfaced in the panel. Resume = switch to Auto; pause = switch to Loiter/Brake.
export function missionPause(pause: boolean): Promise<boolean> {
  return runCommand('missionPause', 'mav_mission_pause', { pause });
}

/** Toggle the Guided interaction. ON sends the reposition-ready mode (GUIDED on ArduPilot, HOLD on PX4) once
 *  and arms the map click; OFF sends nothing (no mode churn), it only disables the map interaction. */
export async function setGuided(on: boolean): Promise<boolean> {
  guidedTarget.set(null); // entering or leaving Guided starts with no target
  if (!on) {
    guidedActive.set(false);
    return true;
  }
  // With the FC's own mode list, only a GUIDED the FC offers — none means no Guided for this vehicle.
  const mode = guidedModeFor(get(autopilotSystem), get(arduVehicleClass), get(activeFcModesChecked));
  if (!mode) {
    lastFeedback.set({ action: 'guided', ok: false, message: 'No guided mode for this vehicle', ts: Date.now() });
    return false;
  }
  const ok = await runCommand('guided', 'mav_set_mode', { main: mode.main, sub: mode.sub });
  guidedActive.set(ok);
  headingOverrideActive = false; // re-entering guided clears any heading slew
  if (ok) {
    // Seed the target marker at the mode-entry point — ArduPlane loiters where GUIDED was entered, so this IS
    // the initial target; the FC's 1 Hz POSITION_TARGET_GLOBAL_INT then confirms or corrects it.
    const tel = get(telemetry);
    if (tel.fixType >= 2 && (tel.lat !== 0 || tel.lon !== 0)) {
      guidedTarget.set({ lat: tel.lat, lon: tel.lon });
    }
    void refreshFcLoiterRadius(); // for the expected-loiter-track ring
  }
  return ok;
}

/** Guided "fly here" — reposition to a clicked point with the current Fly-Here params. If a fixed-wing
 *  heading override is active, clear it first (otherwise ArduPlane keeps flying the heading and ignores
 *  the new waypoint — see headingOverrideActive). */
export async function repositionTo(lat: number, lon: number, p: GuidedParams): Promise<boolean> {
  if (headingOverrideActive) {
    try { await invoke('mav_guided_clear_heading', targeted()); } catch { /* best effort */ }
    headingOverrideActive = false;
  }
  const ok = await runCommand('reposition', 'mav_reposition', {
    lat: Math.round(lat * 1e7),
    lon: Math.round(lon * 1e7),
    alt: p.alt,
    groundSpeed: p.speed,
    yaw: p.yaw,
    loiterRadius: p.loiterRadius,
  });
  if (ok) guidedTarget.set({ lat, lon }); // drive the map's loiter-target marker
  return ok;
}

// ── Active-vehicle switch ── Guided UI state is really per vehicle: on a switch the shown target must be the
// NEW vehicle's FC-confirmed one (or none) and toggle / heading-override / loiter-radius bookkeeping restarts,
// so the previous Guided session can't leak into commands for this one.
let guidedSeededFor: string | null = null;
activeVehicleId.subscribe((id) => {
  if (id === guidedSeededFor) return;
  guidedSeededFor = id;
  const tgt = id ? get(guidedTargets).get(id) : undefined;
  guidedTarget.set(tgt ? { lat: tgt.lat, lon: tgt.lon } : null);
  guidedActive.set(false);
  headingOverrideActive = false;
  loiterRadiusRequested = false;
  fcLoiterRadius.set(null);
  resetFlightState(id != null);
});

// The disarm ends the flight: the next takeoff is ahead again.
let wasArmed = false;
isArmed.subscribe((armed) => {
  if (wasArmed && !armed) resetFlightState(true);
  wasArmed = armed;
});
