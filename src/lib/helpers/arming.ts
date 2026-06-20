// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Arming-state derivation for the toolbar arming indicator.
//
// INAV exposes a full armingFlag_e bitfield (MSP2_INAV_STATUS, already carried in telem.armingFlags):
// bit 2 = ARMED, bits 6..30 = ARMING_DISABLED_* reasons. So for INAV we get a real traffic light —
// armed (amber) / blocked (red, with reasons) / ready (green) — without any extra MSP query.
//
// MAVLink (ArduPilot/PX4) only carries the ARMED bit; the "not ready" reason comes from STATUSTEXT
// "PreArm: …" messages (tracked in stores/statusText → prearmReason).

import type { TelemetryData } from '$lib/stores/telemetry';

export const ARMED_BIT = 1 << 2;

/** INAV armingFlag_e disable bits → i18n reason key (suffix under `arming.reasons.*`). */
const INAV_DISABLE_FLAGS: { bit: number; key: string }[] = [
  { bit: 6, key: 'geozone' },
  { bit: 7, key: 'failsafe' },
  { bit: 8, key: 'notLevel' },
  { bit: 9, key: 'calibrating' },
  { bit: 10, key: 'overloaded' },
  { bit: 11, key: 'navUnsafe' },
  { bit: 12, key: 'compass' },
  { bit: 13, key: 'accel' },
  { bit: 14, key: 'armSwitch' },
  { bit: 15, key: 'hardware' },
  { bit: 16, key: 'boxFailsafe' },
  { bit: 18, key: 'rcLink' },
  { bit: 19, key: 'throttle' },
  { bit: 20, key: 'cli' },
  { bit: 21, key: 'cmsMenu' },
  { bit: 22, key: 'osdMenu' },
  { bit: 23, key: 'rollPitch' },
  { bit: 24, key: 'servoAutotrim' },
  { bit: 25, key: 'oom' },
  { bit: 26, key: 'invalidSetting' },
  { bit: 27, key: 'pwmError' },
  { bit: 28, key: 'noPrearm' },
  { bit: 29, key: 'dshotBeeper' },
  { bit: 30, key: 'landingDetected' },
];

export function isArmed(armingFlags: number): boolean {
  return (armingFlags & ARMED_BIT) !== 0;
}

/** The i18n reason-key suffixes for every ARMING_DISABLED_* bit currently set (INAV only). */
export function inavDisableReasonKeys(armingFlags: number): string[] {
  return INAV_DISABLE_FLAGS.filter((f) => (armingFlags & (1 << f.bit)) !== 0).map((f) => f.key);
}

export type ArmingLevel = 'armed' | 'ready' | 'notReady';

export interface ArmingStatus {
  level: ArmingLevel;
  /** INAV: i18n reason-key suffixes (`arming.reasons.*`). Empty for non-INAV. */
  reasonKeys: string[];
  /** MAVLink: the raw "PreArm: …" text when blocked, else null. */
  prearmText: string | null;
}

/**
 * Derive the arming traffic-light from telemetry + (for MAVLink) the latest prearm STATUSTEXT.
 * `isInav` selects the interpretation (INAV armingFlags bitfield vs MAVLink prearm signals) — pass it
 * from the `autopilotSystem` store, NOT telem.fcVariant (which is never updated frontend-side).
 * Returns null when there is no live telemetry (indicator hidden).
 */
export function armingStatus(t: TelemetryData, prearmText: string | null, isInav: boolean): ArmingStatus | null {
  if (!t.lastUpdate) return null;
  if (isArmed(t.armingFlags)) return { level: 'armed', reasonKeys: [], prearmText: null };

  if (isInav) {
    const reasonKeys = inavDisableReasonKeys(t.armingFlags);
    return { level: reasonKeys.length ? 'notReady' : 'ready', reasonKeys, prearmText: null };
  }

  // MAVLink (ArduPilot/PX4): the reliable "blocked" signal is a fresh "PreArm: …" STATUSTEXT — ArduPilot
  // only emits those while a prearm check is actually failing. The SYS_STATUS PREARM_CHECK bit is only an
  // ADDITIONAL blocked signal (prearmHealthy === 2): we deliberately do NOT trust prearmHealthy === 1 to
  // assert readiness, because some ArduPilot builds report the health bit set while prearm still fails
  // (observed: "READY" shown with live PreArm errors). Ready only when neither says blocked.
  const blocked = !!prearmText || t.prearmHealthy === 2;
  return blocked
    ? { level: 'notReady', reasonKeys: [], prearmText }
    : { level: 'ready', reasonKeys: [], prearmText: null };
}
