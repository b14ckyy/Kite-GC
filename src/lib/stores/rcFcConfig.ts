// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Live INAV config relevant to GCS RC injection (docs/archive/MSP_RC_CONTROL.md). Read on demand from the
// FC via `rc_read_fc_config` (receiver_type + msp_override_channels + mode ranges). Feeds the mode
// labels under channels and the RC safety locks/warnings. Null until read / when not MSP-connected.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

/** One configured mode-activation range (mirrors the Rust `ModeRange`). */
export interface ModeRange {
  permanent_id: number;
  /** 1-based RC channel (AUX1 = CH5). */
  channel: number;
  range_min: number;
  range_max: number;
}

export interface RcFcConfig {
  /** 0 = NONE, 1 = SERIAL, 2 = MSP. */
  receiver_type: number;
  /** Override bitmask (CH1 = bit 0); null if the FC lacks the setting. */
  msp_override_channels: number | null;
  mode_ranges: ModeRange[];
}

export const rcFcConfig = writable<RcFcConfig | null>(null);

/** Read the FC config (MSP/INAV only). Clears to null on failure / wrong protocol. */
export async function loadRcFcConfig(): Promise<void> {
  try {
    rcFcConfig.set(await invoke<RcFcConfig>('rc_read_fc_config'));
  } catch (e) {
    console.warn('[rc] loadRcFcConfig failed', e);
    rcFcConfig.set(null);
  }
}

// ── PX4: COM_RC_IN_MODE ───────────────────────────────────────────────────────────────────────
// PX4 ignores MANUAL_CONTROL unless COM_RC_IN_MODE allows a MAVLink/joystick source (0 = RC only and
// 4 = sticks disabled block it). Read ONCE in the on-connect sequence (connectionController), after the
// fence/rally downloads: the MAVLink handler has a single param-receiver slot, and a second reader used to
// displace the first one, which then failed at once (its unconditional unregister also cleared the newer
// reader's slot). `params_rt` now serialises the slot, so overlapping reads wait for each other instead.

/** Live COM_RC_IN_MODE of the connected PX4 vehicle; null while unknown / not PX4. */
export const px4RcInMode = writable<number | null>(null);
/** Error of the last "Allow joystick" write (shown in the RC panel); null when none / cleared. */
export const px4RcInModeError = writable<string | null>(null);

/** Read COM_RC_IN_MODE from the FC (PX4 only, best-effort). */
export async function loadPx4RcInMode(): Promise<void> {
  try {
    const v = await invoke<number | null>('mav_read_param', { name: 'COM_RC_IN_MODE' });
    if (v != null) {
      px4RcInMode.set(Math.round(v));
      px4RcInModeError.set(null); // a fresh value supersedes a stale "could not set" line
    }
  } catch (e) {
    console.warn('[rc] COM_RC_IN_MODE read failed', e);
    px4RcInMode.set(null);
  }
}

/** Set COM_RC_IN_MODE = 2 ("RC and Joystick with fallback"), then re-read. PX4 persists parameters itself. */
export async function allowPx4JoystickInput(): Promise<void> {
  px4RcInModeError.set(null);
  try {
    await invoke('mav_set_param', { name: 'COM_RC_IN_MODE', value: 2 });
    await loadPx4RcInMode();
  } catch (e) {
    console.warn('[rc] COM_RC_IN_MODE set failed', e);
    px4RcInModeError.set(String(e));
  }
}

/** Set the FC's msp_override_channels bitmask at runtime (not saved), then re-read. */
export async function setOverrideBitmask(mask: number): Promise<void> {
  await invoke('rc_set_override_bitmask', { mask });
  await loadRcFcConfig();
}
