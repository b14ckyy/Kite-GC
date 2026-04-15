// Telemetry state store
// Holds real-time telemetry data received from the flight controller

import { writable } from 'svelte/store';

export interface TelemetryData {
  // GPS
  lat: number;
  lon: number;
  altMsl: number;
  groundSpeed: number;
  numSat: number;
  fixType: number;

  // Attitude
  roll: number;
  pitch: number;
  yaw: number;

  // Battery
  voltage: number;
  current: number;
  mAhDrawn: number;
  batteryPercentage: number;

  // Status
  armingFlags: number;
  flightMode: string;

  // Timestamps
  lastUpdate: number;
}

const defaultTelemetry: TelemetryData = {
  lat: 0, lon: 0, altMsl: 0, groundSpeed: 0, numSat: 0, fixType: 0,
  roll: 0, pitch: 0, yaw: 0,
  voltage: 0, current: 0, mAhDrawn: 0, batteryPercentage: 0,
  armingFlags: 0, flightMode: 'UNKNOWN',
  lastUpdate: 0,
};

export const telemetry = writable<TelemetryData>({ ...defaultTelemetry });

export function resetTelemetry() {
  telemetry.set({ ...defaultTelemetry });
}
