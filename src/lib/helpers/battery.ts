// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Battery-level derivation for the toolbar battery indicator.
//
// Source priority (mirrors what the user asked for):
//  1. FC-native percentage — INAV's `percentageRemaining` (capacity-used vs configured capacity, or its
//     own voltage curve) / MAVLink `battery_remaining`. Carried in telem.batteryPercentage; 0 = unknown.
//  2. Otherwise a per-cell voltage estimate over the usable LiPo window (3.3 V empty … 4.2 V full).

import type { TelemetryData } from '$lib/stores/telemetry';

const CELL_FULL_V = 4.2;
const CELL_EMPTY_V = 3.3;
const CELL_NOMINAL_V = 3.8; // for estimating the cell count when the FC doesn't report it

export interface BatteryLevel {
  percent: number;            // 0..100
  voltage: number;
  cells: number;              // reported, else estimated
  mAhDrawn: number;
  source: 'native' | 'voltage';
}

function clampPct(n: number): number {
  return Math.max(0, Math.min(100, Math.round(n)));
}

/** Resolve the battery level, or null when no battery telemetry is present. */
export function batteryLevel(t: TelemetryData): BatteryLevel | null {
  if (!t.lastUpdate || t.voltage <= 0) return null;
  const cells = t.cellCount > 0 ? t.cellCount : Math.max(1, Math.round(t.voltage / CELL_NOMINAL_V));

  if (t.batteryPercentage > 0) {
    return { percent: clampPct(t.batteryPercentage), voltage: t.voltage, cells, mAhDrawn: t.mAhDrawn, source: 'native' };
  }
  const vCell = t.voltage / cells;
  const percent = clampPct(((vCell - CELL_EMPTY_V) / (CELL_FULL_V - CELL_EMPTY_V)) * 100);
  return { percent, voltage: t.voltage, cells, mAhDrawn: t.mAhDrawn, source: 'voltage' };
}

/** Fill colour by level: green > 50 %, amber 20–50 %, red ≤ 20 %. */
export function batteryColor(percent: number): string {
  if (percent <= 20) return '#d40000';
  if (percent <= 50) return '#e8a317';
  return '#59aa29';
}
