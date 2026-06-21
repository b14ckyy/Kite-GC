// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC channel layout — how channels split into two groups, derived from the target platform + connected
// FC (docs/archive/MSP_RC_CONTROL.md §7, docs/active/MAVLINK_RC_CONTROL.md §4). The split is the same UI
// concept everywhere (a primary stick group + a secondary aux group), but the transport + ranges differ:
//
//   INAV 9.1+  → CH1–16 via MSP_SET_RAW_RC ("MSP-RC") + CH17–32 via MSP2_INAV_SET_AUX_RC ("MSP-AUX").
//   INAV 8.0–9.0 → a single MSP-RC block, capped at 16 channels.
//   ArduPilot  → one RC_CHANNELS_OVERRIDE frame; Primary CH1–8 / Secondary CH9–16 (NUM_RC_CHANNELS=16).
//                The two groups exist because the wire release-semantics differ at the 1–8 / 9–16 border.
//   PX4        → MANUAL_CONTROL (4 normalised axes); the channel model doesn't fit → not supported yet.
//   Offline / unknown → assume the platform's richest layout (the user configures without an FC).
//
// Why CH1–16 on RAW for INAV (not CH1–12, the firmware AUX_RC floor): AUX_RC values LATCH — once a
// channel is sent via AUX_RC it can't be released back to the TX/our control. Keeping the first 16
// channels (which carry the mode switches incl. GCS-OVERRIDE) on RAW_RC — gated by the override bitmask
// + override mode, so they revert cleanly when we stop — means we never lock ourselves out of toggling
// GCS-OVERRIDE. Both the config editor and the live monitor group by this.

import { derived } from 'svelte/store';
import { t } from 'svelte-i18n';
import { connection } from './connection';
import { rcPlatform, type RcPlatform } from './rcPlatform';

export interface RcLayout {
  /** Target platform — selects the transport adapter (rcStream) + group labels. */
  platform: RcPlatform;
  /** true = two groups (primary + secondary); false = a single group. */
  split: boolean;
  /** Highest channel in the primary group (INAV: RAW_RC max; ArduPilot: CH8). */
  rawMax: number;
  /** Secondary group channel range (only meaningful when `split`). */
  auxMin: number;
  auxMax: number;
  /** Whether RC injection is available for this platform/FC at all; informational. */
  supported: boolean;
}

// RAW_RC covers CH1–16 in both INAV layouts (see header). With AUX_RC the extra channels start at CH17.
export const RC_RAW_SPLIT_MAX = 16;
export const RC_RAW_SOLO_MAX = 16;
export const RC_AUX_MAX = 32;

// ArduPilot: Primary CH1–8 / Secondary CH9–16 (RC_CHANNELS_OVERRIDE carries CH1–18, INAV-style policy
// keeps mode switches on the lower, cleanly-releasable group).
export const RC_ARDU_PRIMARY_MAX = 8;
export const RC_ARDU_AUX_MAX = 16;

export const rcLayout = derived([connection, rcPlatform], ([$c, $platform]): RcLayout => {
  if ($platform === 'ardupilot') {
    return {
      platform: 'ardupilot',
      split: true,
      rawMax: RC_ARDU_PRIMARY_MAX,
      auxMin: RC_ARDU_PRIMARY_MAX + 1,
      auxMax: RC_ARDU_AUX_MAX,
      supported: true,
    };
  }
  if ($platform === 'px4') {
    // MANUAL_CONTROL: 4 normalised axes + aux — not a channel grid. The panel renders the dedicated
    // manual editor/monitor (ManualConfig/ManualStates) for PX4, so these channel-grid fields are unused;
    // `supported` just gates the engage UI on.
    return { platform: 'px4', split: false, rawMax: 4, auxMin: 5, auxMax: 4, supported: true };
  }

  // INAV / MSP.
  const connectedMsp = $c.status === 'connected' && $c.protocolType === 'msp';
  const features = $c.fcInfo?.features ?? null;
  // Offline/unknown → assume 9.1+ (split). When connected, follow the FC's AUX_RC capability.
  const hasAux = !connectedMsp || (features?.aux_rc ?? true);

  if (hasAux) {
    return { platform: 'inav', split: true, rawMax: RC_RAW_SPLIT_MAX, auxMin: RC_RAW_SPLIT_MAX + 1, auxMax: RC_AUX_MAX, supported: true };
  }
  return {
    platform: 'inav',
    split: false,
    rawMax: RC_RAW_SOLO_MAX,
    auxMin: RC_RAW_SOLO_MAX + 1,
    auxMax: RC_AUX_MAX,
    supported: features?.msp_rc ?? true,
  };
});

/** Localized labels for the two channel groups, per platform — INAV speaks RAW/AUX (the MSP transports),
 *  ArduPilot speaks Primary/Secondary (the CH1–8 / CH9–16 RC_CHANNELS_OVERRIDE bands). Used by the config
 *  editor and the live monitor so both group consistently. Derived (not inline in components) to keep the
 *  platform→label mapping in one place. */
export const rcGroupLabels = derived([rcLayout, t], ([$layout, $t]) =>
  $layout.platform === 'ardupilot'
    ? { primary: $t('rc.groupPrimary'), secondary: $t('rc.groupSecondary') }
    : { primary: $t('rc.groupRaw'), secondary: $t('rc.groupAux') },
);
