import type {
  AltitudeUnit,
  DistanceUnit,
  SpeedUnit,
  TemperatureUnit,
  VerticalSpeedUnit,
} from '$lib/stores/settings';

export interface ConvertedValue {
  value: number;
  unit: string;
}

export function convertSpeed(ms: number, unit: SpeedUnit): ConvertedValue {
  switch (unit) {
    case 'kmh':
      return { value: ms * 3.6, unit: 'km/h' };
    case 'mph':
      return { value: ms * 2.2369362921, unit: 'mi/h' };
    case 'fts':
      return { value: ms * 3.280839895, unit: 'ft/s' };
    case 'kt':
      return { value: ms * 1.9438444924, unit: 'kt' };
    case 'ms':
    default:
      return { value: ms, unit: 'm/s' };
  }
}

export function convertAltitude(meters: number, unit: AltitudeUnit): ConvertedValue {
  if (unit === 'ft') {
    return { value: meters * 3.280839895, unit: 'ft' };
  }
  return { value: meters, unit: 'm' };
}

export function convertDistance(meters: number, unit: DistanceUnit): ConvertedValue {
  if (unit === 'imperial') {
    const feet = meters * 3.280839895;
    if (feet >= 5280) {
      return { value: feet / 5280, unit: 'mi' };
    }
    return { value: feet, unit: 'ft' };
  }

  if (meters >= 1000) {
    return { value: meters / 1000, unit: 'km' };
  }
  return { value: meters, unit: 'm' };
}

/**
 * Length for *input* fields (line spacing, radius, …): a single fixed unit
 * (m or ft), never auto-switching to km/mi mid-edit.
 */
export function convertLength(meters: number, unit: DistanceUnit): ConvertedValue {
  return unit === 'imperial'
    ? { value: meters * 3.280839895, unit: 'ft' }
    : { value: meters, unit: 'm' };
}

export function toLengthM(value: number, unit: DistanceUnit): number {
  return unit === 'imperial' ? value / 3.280839895 : value;
}

export function toAltitudeM(value: number, unit: AltitudeUnit): number {
  return unit === 'ft' ? value / 3.280839895 : value;
}

export function convertVerticalSpeed(ms: number, unit: VerticalSpeedUnit): ConvertedValue {
  if (unit === 'fts') {
    return { value: ms * 3.280839895, unit: 'ft/s' };
  }
  return { value: ms, unit: 'm/s' };
}

export function convertTemperature(celsius: number, unit: TemperatureUnit): ConvertedValue {
  if (unit === 'f') {
    return { value: (celsius * 9) / 5 + 32, unit: '°F' };
  }
  return { value: celsius, unit: '°C' };
}

export function toSpeedMs(value: number, unit: SpeedUnit): number {
  switch (unit) {
    case 'kmh':
      return value / 3.6;
    case 'mph':
      return value / 2.2369362921;
    case 'fts':
      return value / 3.280839895;
    case 'kt':
      return value / 1.9438444924;
    case 'ms':
    default:
      return value;
  }
}

export function toTemperatureC(value: number, unit: TemperatureUnit): number {
  if (unit === 'f') {
    return ((value - 32) * 5) / 9;
  }
  return value;
}

export function formatConverted(value: ConvertedValue, digits = 1): string {
  return `${value.value.toFixed(digits)} ${value.unit}`;
}
