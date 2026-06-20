// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC channel layout — how channels split across the two MSP transports, derived from the connected FC
// (docs/active/RC_CONTROL.md §7). INAV 9.1+ (AUX_RC) → CH1–12 via MSP_SET_RAW_RC ("MSP-RC") + CH13–32
// via MSP2_INAV_SET_AUX_RC ("MSP-AUX"). INAV 8.0–9.0 → a single MSP-RC block, capped at 16 channels
// (bandwidth). Offline / unknown → assume the modern 9.1+ split (the richer layout; the user configures
// without an FC). Both the config editor and the live monitor group by this.

import { derived } from 'svelte/store';
import { connection } from './connection';

export interface RcLayout {
  /** true = two blocks (MSP-RC CH1–12 + MSP-AUX CH13–32); false = single MSP-RC block. */
  split: boolean;
  /** Highest channel sent via MSP_SET_RAW_RC. */
  rawMax: number;
  /** AUX_RC channel range (only meaningful when `split`). */
  auxMin: number;
  auxMax: number;
  /** Whether RC-over-MSP is available at all (INAV ≥8.0); informational. */
  supported: boolean;
}

export const RC_RAW_SPLIT_MAX = 12;
export const RC_RAW_SOLO_MAX = 16;
export const RC_AUX_MAX = 32;

export const rcLayout = derived(connection, ($c): RcLayout => {
  const connectedMsp = $c.status === 'connected' && $c.protocolType === 'msp';
  const features = $c.fcInfo?.features ?? null;
  // Offline/unknown → assume 9.1+ (split). When connected, follow the FC's AUX_RC capability.
  const hasAux = !connectedMsp || (features?.aux_rc ?? true);

  if (hasAux) {
    return { split: true, rawMax: RC_RAW_SPLIT_MAX, auxMin: RC_RAW_SPLIT_MAX + 1, auxMax: RC_AUX_MAX, supported: true };
  }
  return {
    split: false,
    rawMax: RC_RAW_SOLO_MAX,
    auxMin: RC_RAW_SOLO_MAX + 1,
    auxMax: RC_AUX_MAX,
    supported: features?.msp_rc ?? true,
  };
});
