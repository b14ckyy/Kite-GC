// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// On-screen RC sticks for iPad (Phase 4). Desktop drives RC injection from a HID joystick through
// rcEngine (config-mapped) → rcStream. Mobile has no joystick, so this store feeds the SAME rcStream
// pump directly from two touch sticks with a fixed Mode-2 / AETR layout:
//   CH1 roll, CH2 pitch, CH3 throttle, CH4 yaw, CH5 arm (AUX1).
// It writes both channelValues (INAV MSP RAW_RC / ArduPilot RC_CHANNELS_OVERRIDE) and manualOutput
// (PX4 MANUAL_CONTROL), so the existing platform-aware pump handles the wire format, send rate and the
// 500 ms deadman. Nothing here persists to the desktop profiles.

import { get, writable } from 'svelte/store';
import { channelValues } from './rcEngine';
import { rcEngaged } from './rcEngage';
import { manualOutput } from './rcManual';
import { currentChannels } from './rcProfiles';
import type { RcChannelMap } from '$lib/helpers/rcMethods';

/** Live stick positions, each −1..1. Throttle is −1 (idle, bottom) .. +1 (full, top). */
export interface StickState {
  roll: number;
  pitch: number;
  throttle: number;
  yaw: number;
}

const CENTERED: StickState = { roll: 0, pitch: 0, throttle: -1, yaw: 0 };

export const mobileSticks = writable<StickState>({ ...CENTERED });
/** Arm switch (CH5 / AUX1). Latched - the user toggles it explicitly. */
export const mobileArm = writable<boolean>(false);
/** True while the touch-stick controller owns RC output (engaged). */
export const mobileRcActive = writable<boolean>(false);

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
/** −1..1 → 1000..2000 µs (centre 1500). */
const axisUs = (v: number) => Math.round(1500 + clamp(v, -1, 1) * 500);

// Fixed AETR channel map (CH1..CH5). The pump only checks each channel is present here; the values come
// from channelValues below. Passthrough configs are placeholders to satisfy the RcChannelMap type.
function mobileChannelMap(): RcChannelMap {
  const pt = (input: string) => ({ kind: 'passthrough' as const, input, invert: false, deadband: 0.02 });
  return { 1: pt('A1'), 2: pt('A2'), 3: pt('A3'), 4: pt('A4'), 5: pt('A5') };
}

/** Recompute the channel frame + PX4 setpoint from the current sticks + arm switch. */
function recompute(): void {
  const s = get(mobileSticks);
  const armed = get(mobileArm);

  // INAV / ArduPilot channel frame (µs), consumed by rcStream buildRaw / buildOverride.
  channelValues.set({
    1: axisUs(s.roll),
    2: axisUs(s.pitch),
    3: axisUs(s.throttle),
    4: axisUs(s.yaw),
    5: armed ? 2000 : 1000,
  });

  // PX4 MANUAL_CONTROL setpoint (−1000..1000). z (thrust) maps the throttle stick directly.
  manualOutput.set({
    x: Math.round(clamp(s.pitch, -1, 1) * 1000),
    y: Math.round(clamp(s.roll, -1, 1) * 1000),
    z: Math.round(clamp(s.throttle, -1, 1) * 1000),
    r: Math.round(clamp(s.yaw, -1, 1) * 1000),
    aux: [0, 0, 0, 0, 0, 0],
    buttons: 0,
    buttons2: 0,
    ext: 0,
  });
}

let unsubs: Array<() => void> = [];

/** Engage the touch-stick controller: install the mobile channel map, wire the sticks to the pump, and
 *  start streaming (rcStream reacts to rcEngaged). Always starts from a safe state (idle throttle,
 *  disarmed). */
export function startMobileRc(): void {
  if (get(mobileRcActive)) return;
  mobileSticks.set({ ...CENTERED });
  mobileArm.set(false);
  currentChannels.set(mobileChannelMap());
  recompute();
  unsubs = [mobileSticks.subscribe(recompute), mobileArm.subscribe(recompute)];
  mobileRcActive.set(true);
  rcEngaged.set({ on: true, mode: 'msp' });
}

/** Disengage: stop the stream, disarm, and clear the mobile channel map. */
export function stopMobileRc(): void {
  if (!get(mobileRcActive)) return;
  rcEngaged.set({ on: false, mode: null });
  unsubs.forEach((u) => u());
  unsubs = [];
  mobileArm.set(false);
  mobileSticks.set({ ...CENTERED });
  currentChannels.set({});
  mobileRcActive.set(false);
}
