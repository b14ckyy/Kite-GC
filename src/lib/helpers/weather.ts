// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { convertTemperature, convertSpeed, toTemperatureC, toSpeedMs } from '$lib/utils/units';
import type { InterfaceSettings } from '$lib/stores/settings';

const STANDARD_CONDITIONS = [
  'Clear',
  'Partly Cloudy',
  'Overcast',
  'Light Rain',
  'Moderate Rain',
  'Rain',
  'Snow',
  'Fog',
  'Stormy',
];

export function weatherTempDisplayFromC(tempC: string, settings: InterfaceSettings): string {
  if (tempC === '' || isNaN(Number(tempC))) return '';
  const v = convertTemperature(Number(tempC), settings.temperatureUnit).value;
  return String(Math.round(v * 10) / 10);
}

export function weatherWindDisplayFromMs(windMs: string, settings: InterfaceSettings): string {
  if (windMs === '' || isNaN(Number(windMs))) return '';
  const v = convertSpeed(Number(windMs), settings.speedUnit).value;
  return String(Math.round(v * 10) / 10);
}

export function weatherTempCFromDisplay(displayValue: string, settings: InterfaceSettings): string {
  if (displayValue === '' || isNaN(Number(displayValue))) return '';
  const v = toTemperatureC(Number(displayValue), settings.temperatureUnit);
  return String(Math.round(v * 10) / 10);
}

export function weatherWindMsFromDisplay(displayValue: string, settings: InterfaceSettings): string {
  if (displayValue === '' || isNaN(Number(displayValue))) return '';
  const v = toSpeedMs(Number(displayValue), settings.speedUnit);
  return String(Math.round(v * 10) / 10);
}

export function canonicalWeatherDescription(desc: string): string {
  const trimmed = desc.trim();
  if (!trimmed) return '';
  const match = STANDARD_CONDITIONS.find((v) => v.toLowerCase() === trimmed.toLowerCase());
  return match ?? trimmed;
}
