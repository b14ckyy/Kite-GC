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

/** Set the FC's msp_override_channels bitmask at runtime (not saved), then re-read. */
export async function setOverrideBitmask(mask: number): Promise<void> {
  await invoke('rc_set_override_bitmask', { mask });
  await loadRcFcConfig();
}
