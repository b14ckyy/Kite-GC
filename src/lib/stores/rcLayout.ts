// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC channel layout — how channels split across the two MSP transports, derived from the connected FC
// (docs/active/RC_CONTROL.md §7). INAV 9.1+ (AUX_RC) → CH1–16 via MSP_SET_RAW_RC ("MSP-RC") + CH17–32
// via MSP2_INAV_SET_AUX_RC ("MSP-AUX"). INAV 8.0–9.0 → a single MSP-RC block, capped at 16 channels.
// Offline / unknown → assume the modern 9.1+ split (the richer layout; the user configures without an FC).
//
// Why CH1–16 on RAW (not CH1–12, the firmware AUX_RC floor): AUX_RC values LATCH — once a channel is
// sent via AUX_RC it can't be released back to the TX/our control. Keeping the first 16 channels (which
// carry the mode switches incl. GCS-OVERRIDE) on RAW_RC — gated by the override bitmask + override mode,
// so they revert cleanly when we stop — means we never lock ourselves out of toggling GCS-OVERRIDE. It
// also gives the same full RAW-RC config on every INAV version, with the extra AUX channels on top from
// 9.1+. Both the config editor and the live monitor group by this.

import { derived } from 'svelte/store';
import { connection } from './connection';

export interface RcLayout {
  /** true = two blocks (MSP-RC CH1–16 + MSP-AUX CH17–32); false = single MSP-RC block. */
  split: boolean;
  /** Highest channel sent via MSP_SET_RAW_RC. */
  rawMax: number;
  /** AUX_RC channel range (only meaningful when `split`). */
  auxMin: number;
  auxMax: number;
  /** Whether RC-over-MSP is available at all (INAV ≥8.0); informational. */
  supported: boolean;
}

// RAW_RC covers CH1–16 in both layouts (see header). With AUX_RC the extra channels start at CH17.
export const RC_RAW_SPLIT_MAX = 16;
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
