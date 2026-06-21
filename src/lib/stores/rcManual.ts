// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// PX4 manual-control mapping (docs/active/MAVLINK_RC_CONTROL.md §5). PX4's RC injection is NOT
// per-channel µs like INAV/ArduPilot — it's `MANUAL_CONTROL` (#69): four normalised axes (pitch x /
// roll y / thrust z / yaw r, each [-1000,1000]) + button bitfields + up to six continuous aux axes.
// So PX4 uses this dedicated, simpler model instead of the channel map: pick a HID axis for each of the
// four sticks (+ optional aux1–6) and assign HID buttons to MANUAL_CONTROL button numbers (1–32). The
// FC maps each button to an action itself (per-vehicle), so we only ever send the bitfield.
//
// Inputs are resolved with the same A/B/H frame as the channel engine (reuses rcEngine.makeFrame). The
// evaluator runs on every HID frame (so live) and on config edits (so the editor reflects immediately).

import { writable, get } from 'svelte/store';
import { hidSnapshot, type HidSnapshot } from './hid';
import { makeFrame } from './rcEngine';

/** A stick-axis assignment: which HID axis drives it, with invert + a small centre deadband. */
export interface ManualAxisMap {
  input: string; // 'A1'… or '' = unassigned
  invert: boolean;
  deadband: number; // 0..1 fraction of full scale
}

/** A continuous aux-axis assignment (aux1–6). No deadband — these are knobs/sliders, not centre sticks. */
export interface ManualAuxMap {
  input: string;
  invert: boolean;
}

/** A button assignment: a HID button/hat → a MANUAL_CONTROL button number (1–32). */
export interface ManualButtonMap {
  input: string; // 'B1' / 'H1' … or ''
  button: number; // 1..32
}

/** The full PX4 manual map (one per profile). */
export interface ManualMap {
  roll: ManualAxisMap;
  pitch: ManualAxisMap;
  throttle: ManualAxisMap;
  yaw: ManualAxisMap;
  aux: ManualAuxMap[]; // up to 6, position i → aux(i+1)
  buttons: ManualButtonMap[];
}

/** The computed MANUAL_CONTROL setpoint pushed to the backend / shown in the monitor. */
export interface ManualOutput {
  x: number; // pitch, -1000..1000
  y: number; // roll
  z: number; // thrust (-1000 = 0 %, 0 = mid, 1000 = full)
  r: number; // yaw
  aux: number[]; // length 6, -1000..1000
  buttons: number; // bits for MC buttons 1..16
  buttons2: number; // bits for MC buttons 17..32
  ext: number; // enabled_extensions bitmask (aux1..6 → bits 2..7)
}

const emptyAxis = (): ManualAxisMap => ({ input: '', invert: false, deadband: 0.02 });

export function defaultManualMap(): ManualMap {
  return { roll: emptyAxis(), pitch: emptyAxis(), throttle: emptyAxis(), yaw: emptyAxis(), aux: [], buttons: [] };
}

/** The manual map currently being edited (mirrors `currentChannels` for the channel path). */
export const rcManual = writable<ManualMap>(defaultManualMap());

/** Live computed setpoint — read by the monitor and the stream pump. */
export const manualOutput = writable<ManualOutput>({
  x: 0, y: 0, z: 0, r: 0, aux: [0, 0, 0, 0, 0, 0], buttons: 0, buttons2: 0, ext: 0,
});

type Frame = ReturnType<typeof makeFrame>;

/** Resolve a stick axis to -1000..1000 (invert + deadband applied). Unassigned → 0. */
function axisVal(frame: Frame, m: ManualAxisMap): number {
  if (!m.input) return 0;
  let v = frame.axis(m.input); // -1..1
  if (m.invert) v = -v;
  if (Math.abs(v) < m.deadband) v = 0;
  return Math.round(Math.max(-1, Math.min(1, v)) * 1000);
}

function auxVal(frame: Frame, m: ManualAuxMap): number {
  if (!m.input) return 0;
  let v = frame.axis(m.input);
  if (m.invert) v = -v;
  return Math.round(Math.max(-1, Math.min(1, v)) * 1000);
}

function recompute(snap: HidSnapshot | null): void {
  const m = get(rcManual);
  // No HID frame yet → everything centred (still publish so the monitor/stream have a defined state).
  const frame = snap ? makeFrame(snap) : null;

  const aux = [0, 0, 0, 0, 0, 0];
  let ext = 0;
  (m.aux ?? []).slice(0, 6).forEach((a, i) => {
    if (a.input) {
      aux[i] = frame ? auxVal(frame, a) : 0;
      ext |= 1 << (2 + i); // aux1..6 occupy enabled_extensions bits 2..7
    }
  });

  let buttons = 0;
  let buttons2 = 0;
  if (frame) {
    for (const b of m.buttons) {
      if (!b.input || b.button < 1 || b.button > 32) continue;
      if (!frame.button(b.input)) continue;
      if (b.button <= 16) buttons |= 1 << (b.button - 1);
      else buttons2 |= 1 << (b.button - 17);
    }
  }

  manualOutput.set({
    x: frame ? axisVal(frame, m.pitch) : 0,
    y: frame ? axisVal(frame, m.roll) : 0,
    z: frame ? axisVal(frame, m.throttle) : 0,
    r: frame ? axisVal(frame, m.yaw) : 0,
    aux,
    buttons,
    buttons2,
    ext,
  });
}

// Live: recompute on every HID frame, and whenever the map is edited (so the editor reflects at once).
hidSnapshot.subscribe((snap) => recompute(snap));
rcManual.subscribe(() => recompute(get(hidSnapshot)));
