// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC safety evaluation (docs/active/RC_CONTROL.md safety section). Only **AUX_RC channels we control**
// matter: those latch in the FC and persist on GCS link loss (no failsafe), unlike MSP-RC channels
// (CH1–12) which fail safe. So we evaluate modes whose channel is BOTH in the AUX range AND configured
// in the active profile (i.e. we send a latching AUX_RC value for it):
//   • critical (ARM/RTH/FAILSAFE) → BLOCK all RC output (e.g. ARM latched on link loss = can't disarm).
//   • gps autonomous (CRUISE/WP/POSHOLD/ALTHOLD) → WARN (overridable only by MANUAL/RTH on link loss);
//     escalates to BLOCK if no MANUAL box is configured anywhere (no manual fallback).

import { boxCategory, boxName, BOX_MANUAL } from './inavModes';
import type { ModeRange } from '$lib/stores/rcFcConfig';

export interface RcSafetyIssue {
  channel: number;
  mode: string;
  /** 'critical' = ARM/RTH/FAILSAFE on AUX; 'gpsNoManual' = autonomous on AUX without a MANUAL fallback. */
  reason: 'critical' | 'gpsNoManual';
}

export interface RcSafety {
  /** Any block → RC output must be disabled entirely. */
  locked: boolean;
  blocks: RcSafetyIssue[];
  /** Autonomous modes on controlled AUX channels (overridable by MANUAL/RTH). */
  warnings: { channel: number; mode: string }[];
  /** Whether a MANUAL box is configured (the manual-control fallback). */
  manualConfigured: boolean;
}

export function evaluateRcSafety(
  modeRanges: ModeRange[],
  configuredChannels: number[],
  rawMax: number,
): RcSafety {
  const configured = new Set(configuredChannels);
  const manualConfigured = modeRanges.some((m) => m.permanent_id === BOX_MANUAL);
  const blocks: RcSafetyIssue[] = [];
  const warnings: { channel: number; mode: string }[] = [];

  for (const m of modeRanges) {
    if (m.channel <= rawMax) continue; // MSP-RC channel → fails safe, not a concern
    if (!configured.has(m.channel)) continue; // we don't latch this channel → not our concern
    const cat = boxCategory(m.permanent_id);
    const mode = boxName(m.permanent_id);
    if (cat === 'critical') {
      blocks.push({ channel: m.channel, mode, reason: 'critical' });
    } else if (cat === 'gps') {
      if (manualConfigured) warnings.push({ channel: m.channel, mode });
      else blocks.push({ channel: m.channel, mode, reason: 'gpsNoManual' });
    }
  }

  return { locked: blocks.length > 0, blocks, warnings, manualConfigured };
}
