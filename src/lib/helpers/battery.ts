// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Battery-level derivation for the toolbar battery indicator (primary battery / instance 0).
//
// We only ever trust the FC's **native** percentage — INAV's `percentageRemaining` (always sent;
// INAV does its own chemistry-aware estimation) / MAVLink `battery_remaining` (when capacity is
// configured). Carried in telem.batteryPercentage; 0/absent = unknown. When there is no native %,
// we show the **voltage** instead — never a guessed %: a fixed per-cell curve can't tell LiPo / Li-Ion
// / LiFe apart, so an estimate would be misleading.

import type { TelemetryData } from '$lib/stores/telemetry';

const CELL_NOMINAL_V = 3.8; // for estimating the cell count (tooltip only) when the FC doesn't report it

export interface BatteryLevel {
  percent: number | null;     // 0..100 (FC-native), or null → no native % available (show voltage)
  voltage: number;
  cells: number;              // reported, else estimated (tooltip only)
  mAhDrawn: number;
}

function clampPct(n: number): number {
  return Math.max(0, Math.min(100, Math.round(n)));
}

/** Resolve the battery level, or null when no battery telemetry is present. `percent` is null when the
 *  FC reports no native charge % — the indicator then shows the voltage instead of a guess. */
export function batteryLevel(t: TelemetryData): BatteryLevel | null {
  if (!t.lastUpdate || t.voltage <= 0) return null;
  const cells = t.cellCount > 0 ? t.cellCount : Math.max(1, Math.round(t.voltage / CELL_NOMINAL_V));
  return {
    percent: t.batteryPercentage > 0 ? clampPct(t.batteryPercentage) : null,
    voltage: t.voltage,
    cells,
    mAhDrawn: t.mAhDrawn,
  };
}

/** Fill colour by level: green > 50 %, amber 20–50 %, red ≤ 20 %. */
export function batteryColor(percent: number): string {
  if (percent <= 20) return '#d40000';
  if (percent <= 50) return '#e8a317';
  return '#59aa29';
}
