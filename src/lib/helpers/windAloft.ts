// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Wind-by-height data of the weather widget (service event and `weather_records.data_json`) and the
// pure maths on it.

export interface WindLevel {
  heightAglM: number;
  speedMs: number;
  /** Direction the wind comes FROM. */
  dirDeg: number;
}

export interface WindStep {
  /** Start of the 15-minute step, Unix seconds. */
  time: number;
  weatherCode: number | null;
  isDay: boolean;
  gustMs: number | null;
  tempC: number | null;
  /** Ascending by height. */
  levels: WindLevel[];
}

export interface WindAloft {
  elevationM: number;
  steps: WindStep[];
}

export interface Wind {
  speedMs: number;
  dirDeg: number;
}

/** Wind at `heightM` above ground: linear between the two neighbouring levels (direction along the
 *  shorter arc), the lowest / highest level outside them. */
export function windAt(levels: WindLevel[], heightM: number): Wind | null {
  if (levels.length === 0) return null;
  const first = levels[0];
  const last = levels[levels.length - 1];
  if (heightM <= first.heightAglM) return { speedMs: first.speedMs, dirDeg: first.dirDeg };
  if (heightM >= last.heightAglM) return { speedMs: last.speedMs, dirDeg: last.dirDeg };
  const i = levels.findIndex((l) => l.heightAglM >= heightM);
  const a = levels[i - 1];
  const b = levels[i];
  const f = (heightM - a.heightAglM) / (b.heightAglM - a.heightAglM);
  const turn = ((b.dirDeg - a.dirDeg + 540) % 360) - 180;
  return {
    speedMs: a.speedMs + (b.speedMs - a.speedMs) * f,
    dirDeg: (a.dirDeg + turn * f + 360) % 360,
  };
}

/** Index of the step that covers `nowMs` (the last one that has started), 0 before the first. */
export function currentStep(steps: WindStep[], nowMs: number): number {
  let idx = 0;
  for (let i = 0; i < steps.length; i++) if (steps[i].time * 1000 <= nowMs) idx = i;
  return idx;
}

export type WeatherKind = 'clear' | 'partly' | 'cloudy' | 'fog' | 'rain' | 'showers' | 'snow' | 'thunder';

/** WMO weather interpretation code → icon family. */
export function weatherKind(code: number | null): WeatherKind | null {
  if (code == null) return null;
  if (code <= 1) return 'clear';
  if (code === 2) return 'partly';
  if (code === 3) return 'cloudy';
  if (code === 45 || code === 48) return 'fog';
  if (code >= 95) return 'thunder';
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return 'snow';
  if (code >= 80 && code <= 82) return 'showers';
  if (code >= 51 && code <= 67) return 'rain';
  return 'cloudy';
}
