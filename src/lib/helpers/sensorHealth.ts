// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Sensor-health tiles shared by the desktop toolbar's sensor bar and the phone's warning chip:
// one tile per sensor the airframe reports (state !== 0), so the list adapts to the aircraft
// (rangefinder/pitot appear only when equipped). State 0=NONE / 1=OK / 2|3=fault. GPS additionally
// goes amber while the fix is below 3D. Fed by SYS_STATUS (MAVLink) or MSP_SENSOR_STATUS (INAV) —
// both land in the same telemetry fields.

import type { TelemetryData } from '$lib/stores/telemetry';

export interface SensorTile {
  key: string;
  state: number;
  label: string;
  tooltip: string;
  /** Amber even though the sensor is healthy (GPS without a 3D fix). */
  warn: boolean;
}

/** Translator shape — `$t` from svelte-i18n (the helper stays framework-free). */
type Translate = (key: string) => string;

function gpsFixLabel(telem: TelemetryData, t: Translate): string {
  if (!telem.lastUpdate || telem.fixType === 0) return t('gps.noFix');
  const types: Record<number, string> = { 1: t('gps.fix2d'), 2: t('gps.fix3d'), 3: t('gps.fix3dDgps') };
  return types[telem.fixType] || `FIX:${telem.fixType}`;
}

export function sensorTiles(telem: TelemetryData, t: Translate): SensorTile[] {
  return [
    { key: 'gyro', state: telem.sensorGyro, label: t('sensors.gyro'), tooltip: t('sensors.gyroTooltip'), warn: false },
    { key: 'acc', state: telem.sensorAcc, label: t('sensors.acc'), tooltip: t('sensors.accTooltip'), warn: false },
    { key: 'mag', state: telem.sensorMag, label: t('sensors.mag'), tooltip: t('sensors.magTooltip'), warn: false },
    { key: 'baro', state: telem.sensorBaro, label: t('sensors.baro'), tooltip: t('sensors.baroTooltip'), warn: false },
    { key: 'gps', state: telem.sensorGps, label: t('sensors.gps'), tooltip: `GPS: ${gpsFixLabel(telem, t)} ${telem.numSat}S`, warn: telem.sensorGps === 1 && telem.fixType < 2 },
    { key: 'rangefinder', state: telem.sensorRangefinder, label: t('sensors.rangefinder'), tooltip: t('sensors.rangefinderTooltip'), warn: false },
    { key: 'pitot', state: telem.sensorPitot, label: t('sensors.pitot'), tooltip: t('sensors.pitotTooltip'), warn: false },
  ].filter((s) => s.state !== 0);
}

/** EKF estimator tile label (ArduPilot only — INAV never sets ekfStatus). Shows the active core. */
export function ekfLabel(telem: TelemetryData): string {
  return telem.ekfType === 2 ? 'EKF2' : telem.ekfType === 3 ? 'EKF3' : 'EKF';
}

/** Tiles that deserve attention: faulted (state ≥ 2) or amber (warn). The phone shows only these. */
export function sensorProblems(telem: TelemetryData, t: Translate): SensorTile[] {
  return sensorTiles(telem, t).filter((s) => s.state >= 2 || s.warn);
}
