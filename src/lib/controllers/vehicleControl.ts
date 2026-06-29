// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle-control controller — domain logic (no UI) for direct GCS command of a MAVLink vehicle
// (ArduPilot + PX4): flight-mode switch, arm/disarm, takeoff/land/RTL, Guided reposition, speed,
// mission start/pause. Wraps the Tauri commands, surfaces COMMAND_ACK results as feedback, and owns
// the Guided-toggle state + RC-presence lock. See docs/active/VEHICLE_CONTROL.md.

import { invoke } from '@tauri-apps/api/core';
import { writable, derived, get, type Readable } from 'svelte/store';
import { telemetry } from '$lib/stores/telemetry';
import { connection } from '$lib/stores/connection';
import { autopilotSystem } from '$lib/stores/autopilotContext';
import { arduVehicleClass, downloadArduMissionFromFc } from '$lib/stores/missionArdupilot';
import { rcEngaged } from '$lib/stores/rcEngage';
import { type MavMode, modesFor, guidedModeFor, matchActiveMode } from '$lib/helpers/mavModes';

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

async function runCommand(action: string, cmd: string, args?: Record<string, unknown>): Promise<boolean> {
  busyAction.set(action);
  try {
    await invoke(cmd, args);
    lastFeedback.set({ action, ok: true, message: '', ts: Date.now() });
    return true;
  } catch (e) {
    lastFeedback.set({ action, ok: false, message: String(e), ts: Date.now() });
    return false;
  } finally {
    busyAction.set(null);
  }
}

// ── Guided toggle + reposition params ───────────────────────────────────────

/** Map-interaction "Guided" intent: when on, a map click opens the Fly-Here popup. */
export const guidedActive = writable<boolean>(false);

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

// ArduPlane's GUIDED_CHANGE_HEADING sets a *sticky* heading override that bypasses waypoint nav and
// is only cleared by a mode change (current firmware does not clear it on DO_REPOSITION). We track it
// so a subsequent "Fly Here" can explicitly clear it first, otherwise the reposition is ignored.
let headingOverrideActive = false;

// ── Connection / vehicle gating ─────────────────────────────────────────────

/** True when connected over MAVLink (the only protocol the control panel supports in V1). */
export const controlAvailable: Readable<boolean> = derived(
  connection,
  (c) => c.status === 'connected' && c.protocolType === 'mavlink',
);

/** Best-effort "RC transmitter present" signal. We trust a valid receiver RSSI (`link.rssiPercent`
 *  is only populated when the FC reports a real RC RSSI). Without it, stick-required modes stay
 *  locked. Provisional — a more robust RX-present signal is a tracked open question. */
export const rcLinkPresent: Readable<boolean> = derived(
  telemetry,
  (t) => t.link.rssiPercent != null && t.link.rssiPercent > 0,
);

/** Whether stick-flown modes are safe to select: there must be a usable RC source — either a physical
 *  transmitter (the FC reports RC RSSI) OR Kite's own RC control is engaged, i.e. we're streaming the
 *  sticks to the FC (ArduPilot RC_CHANNELS_OVERRIDE / PX4 MANUAL_CONTROL). Without one, a stick mode
 *  would leave the vehicle with no control input, so the panel keeps those modes locked. */
export const stickModesUnlocked: Readable<boolean> = derived(
  [rcLinkPresent, rcEngaged],
  ([rc, eng]) => rc || eng.on,
);

/** Armed state from the unified arming flags (bit 2 = armed, matches the recorder convention). */
export const isArmed: Readable<boolean> = derived(telemetry, (t) => (t.armingFlags & 0x04) !== 0);

/** The FC's currently active mode (matched against the firmware/vehicle table), or undefined. */
export const activeMode: Readable<MavMode | undefined> = derived(
  [telemetry, autopilotSystem, arduVehicleClass],
  ([t, sys, cls]) => matchActiveMode(sys, cls, t.flightModeFlags),
);

// ── Commands ────────────────────────────────────────────────────────────────

/** Switch flight mode. Turns the Guided toggle on/off to match whether the target is the guided mode. */
export async function setMode(mode: MavMode): Promise<boolean> {
  const ok = await runCommand('setMode', 'mav_set_mode', { main: mode.main, sub: mode.sub });
  if (ok) { guidedActive.set(!!mode.guided); headingOverrideActive = false; } // mode re-entry clears the heading slew
  return ok;
}

export function arm(force = false): Promise<boolean> {
  return runCommand('arm', 'mav_arm', { arm: true, force });
}

export function disarm(force = false): Promise<boolean> {
  return runCommand('disarm', 'mav_arm', { arm: false, force });
}

/**
 * Take off to `altitude` (m). Guided takeoff requires the reposition-ready mode (GUIDED on ArduPilot)
 * and an already-armed vehicle. We switch to GUIDED first (ArduPilot) so the FC accepts NAV_TAKEOFF;
 * if the vehicle isn't armed the FC still rejects it and the error surfaces. PX4 takes off via the
 * NAV_TAKEOFF command directly.
 */
export async function takeoff(altitude: number): Promise<boolean> {
  if (get(autopilotSystem) === 'ardupilot') {
    const g = guidedModeFor('ardupilot', get(arduVehicleClass));
    if (g && get(activeMode)?.key !== g.key) {
      try {
        await invoke('mav_set_mode', { main: g.main, sub: g.sub });
        guidedActive.set(true);
      } catch {
        // fall through — let the takeoff attempt surface the real error
      }
    }
  }
  return runCommand('takeoff', 'mav_takeoff', { altitude });
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

/** Change the active target altitude — repositions to the current lat/lon at the new altitude
 *  (Guided). Needs a GPS fix; the vehicle should be in the guided/reposition-ready mode. */
export function changeAlt(alt: number): Promise<boolean> {
  const tel = get(telemetry);
  if (tel.fixType < 2) {
    lastFeedback.set({ action: 'changeAlt', ok: false, message: 'No GPS fix', ts: Date.now() });
    return Promise.resolve(false);
  }
  const p = get(guidedParams);
  return runCommand('changeAlt', 'mav_reposition', {
    lat: Math.round(tel.lat * 1e7),
    lon: Math.round(tel.lon * 1e7),
    alt,
    groundSpeed: p.speed,
    yaw: null,
    loiterRadius: p.loiterRadius,
  });
}

/** Set the fixed-wing loiter radius (metres) via PARAM_SET. The parameter name is firmware-specific:
 *  ArduPilot `WP_LOITER_RAD`, PX4 `NAV_LOITER_RAD`. */
export function setLoiterRadius(radius: number): Promise<boolean> {
  const name = get(autopilotSystem) === 'px4' ? 'NAV_LOITER_RAD' : 'WP_LOITER_RAD';
  return runCommand('setLoiterRadius', 'mav_set_param', { name, value: radius });
}

/** Set the home position to the vehicle's current location (DO_SET_HOME). */
export function setHomeHere(): Promise<boolean> {
  return runCommand('setHome', 'mav_set_home_here');
}

/** Abort a landing / go around (DO_GO_AROUND); altitude 0 = firmware default climb. */
export function abortLanding(): Promise<boolean> {
  return runCommand('abortLanding', 'mav_abort_landing', { altitude: 0 });
}

/**
 * VTOL transition (QuadPlane / VTOL). `toFw` = true → forward/fixed-wing flight, false → hover.
 * PX4 uses MAV_CMD_DO_VTOL_TRANSITION directly. ArduPlane's command path is AUTO-only, so we
 * transition by mode instead: forward → Guided (keeps GCS control), hover → QLOITER.
 */
export async function vtolTransition(toFw: boolean): Promise<boolean> {
  if (get(autopilotSystem) === 'px4') {
    return runCommand('vtolTransition', 'mav_vtol_transition', { toFw });
  }
  const key = toFw ? 'guided' : 'qloiter';
  const m = modesFor('ardupilot', 'quadplane').find((x) => x.key === key);
  if (!m) return false;
  const ok = await runCommand('vtolTransition', 'mav_set_mode', { main: m.main, sub: m.sub });
  if (ok) { guidedActive.set(!!m.guided); headingOverrideActive = false; }
  return ok;
}

/**
 * Set the Guided target heading (degrees). ArduPlane flies the course continuously
 * (GUIDED_CHANGE_HEADING); ArduCopter yaws the nose to it (CONDITION_YAW). ArduPilot only — PX4 has
 * no equivalent GCS heading command, so the panel hides this for PX4.
 */
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

/**
 * Toggle the Guided interaction. ON sends the firmware's reposition-ready mode (GUIDED on ArduPilot,
 * HOLD on PX4) once and arms the map click. OFF sends nothing to the FC — it just disables the map
 * interaction, so the vehicle stays in its current mode (no mode churn on toggle-off).
 */
export async function setGuided(on: boolean): Promise<boolean> {
  if (!on) {
    guidedActive.set(false);
    return true;
  }
  const mode = guidedModeFor(get(autopilotSystem), get(arduVehicleClass));
  if (!mode) {
    lastFeedback.set({ action: 'guided', ok: false, message: 'No guided mode for this vehicle', ts: Date.now() });
    return false;
  }
  const ok = await runCommand('guided', 'mav_set_mode', { main: mode.main, sub: mode.sub });
  guidedActive.set(ok);
  headingOverrideActive = false; // re-entering guided clears any heading slew
  return ok;
}

/** Guided "fly here" — reposition to a clicked point with the current Fly-Here params. If a fixed-wing
 *  heading override is active, clear it first (otherwise ArduPlane keeps flying the heading and ignores
 *  the new waypoint — see headingOverrideActive). */
export async function repositionTo(lat: number, lon: number, p: GuidedParams): Promise<boolean> {
  if (headingOverrideActive) {
    try { await invoke('mav_guided_clear_heading'); } catch { /* best effort */ }
    headingOverrideActive = false;
  }
  return runCommand('reposition', 'mav_reposition', {
    lat: Math.round(lat * 1e7),
    lon: Math.round(lon * 1e7),
    alt: p.alt,
    groundSpeed: p.speed,
    yaw: p.yaw,
    loiterRadius: p.loiterRadius,
  });
}
