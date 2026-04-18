// Telemetry state store
// Holds real-time telemetry data received from the flight controller.
// Listens to Tauri events emitted by the MSP scheduler thread.

import { writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface TelemetryData {
  // GPS
  lat: number;
  lon: number;
  altMsl: number;
  groundSpeed: number;
  numSat: number;
  fixType: number;
  course: number;

  // Attitude
  roll: number;
  pitch: number;
  yaw: number;

  // Altitude (baro/nav)
  altitude: number;
  vario: number;

  // Airspeed
  airspeed: number;

  // Battery / Analog
  voltage: number;
  current: number;
  mAhDrawn: number;
  rssi: number;
  power: number;
  batteryPercentage: number;
  cellCount: number;

  // Status
  armingFlags: number;
  cpuLoad: number;
  sensorStatus: number;

  // Sensor hardware status (from MSP_SENSOR_STATUS 151)
  // Values: 0=NONE, 1=OK, 2=UNAVAILABLE, 3=UNHEALTHY
  sensorGyro: number;
  sensorAcc: number;
  sensorMag: number;
  sensorBaro: number;
  sensorGps: number;
  sensorRangefinder: number;
  sensorPitot: number;
  sensorOpflow: number;

  // Flight mode & navigation
  activeFlightModeFlags: number;
  navState: number;

  // Timestamps
  lastUpdate: number;
}

const defaultTelemetry: TelemetryData = {
  lat: 0, lon: 0, altMsl: 0, groundSpeed: 0, numSat: 0, fixType: 0, course: 0,
  roll: 0, pitch: 0, yaw: 0,
  altitude: 0, vario: 0,
  airspeed: 0,
  voltage: 0, current: 0, mAhDrawn: 0, rssi: 0, power: 0, batteryPercentage: 0, cellCount: 0,
  armingFlags: 0, cpuLoad: 0, sensorStatus: 0,
  sensorGyro: 0, sensorAcc: 0, sensorMag: 0, sensorBaro: 0,
  sensorGps: 0, sensorRangefinder: 0, sensorPitot: 0, sensorOpflow: 0,
  activeFlightModeFlags: 0, navState: 0,
  lastUpdate: 0,
};

export const telemetry = writable<TelemetryData>({ ...defaultTelemetry });

export function resetTelemetry() {
  telemetry.set({ ...defaultTelemetry });
}

// ── Event listeners for scheduler telemetry ─────────────────────────

let unlisteners: UnlistenFn[] = [];

export async function startTelemetryListeners() {
  // Clean up any existing listeners
  stopTelemetryListeners();

  unlisteners.push(
    await listen<{ roll: number; pitch: number; yaw: number }>('telemetry-attitude', (event) => {
      telemetry.update((t) => ({
        ...t,
        roll: event.payload.roll,
        pitch: event.payload.pitch,
        yaw: event.payload.yaw,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<{
      fix_type: number; num_sat: number;
      lat: number; lon: number; alt_msl: number;
      ground_speed: number; course: number;
    }>('telemetry-gps', (event) => {
      const p = event.payload;
      telemetry.update((t) => ({
        ...t,
        fixType: p.fix_type,
        numSat: p.num_sat,
        lat: p.lat,
        lon: p.lon,
        altMsl: p.alt_msl,
        groundSpeed: p.ground_speed,
        course: p.course,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<{ altitude: number; vario: number }>('telemetry-altitude', (event) => {
      telemetry.update((t) => ({
        ...t,
        altitude: event.payload.altitude,
        vario: event.payload.vario,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<{
      voltage: number; mah_drawn: number; rssi: number; current: number;
      power: number; battery_percentage: number; cell_count: number;
    }>(
      'telemetry-analog',
      (event) => {
        const p = event.payload;
        telemetry.update((t) => ({
          ...t,
          voltage: p.voltage,
          current: p.current,
          mAhDrawn: p.mah_drawn,
          rssi: p.rssi,
          power: p.power,
          batteryPercentage: p.battery_percentage,
          cellCount: p.cell_count,
          lastUpdate: Date.now(),
        }));
      }
    )
  );

  unlisteners.push(
    await listen<{ arming_flags: number; flight_mode_flags: number; cpu_load: number; sensor_status: number }>('telemetry-status', (event) => {
      telemetry.update((t) => ({
        ...t,
        armingFlags: event.payload.arming_flags,
        activeFlightModeFlags: event.payload.flight_mode_flags,
        cpuLoad: event.payload.cpu_load,
        sensorStatus: event.payload.sensor_status,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<{
      gyro: number; acc: number; mag: number; baro: number;
      gps: number; rangefinder: number; pitot: number; opflow: number;
    }>('telemetry-sensor-status', (event) => {
      const p = event.payload;
      telemetry.update((t) => ({
        ...t,
        sensorGyro: p.gyro,
        sensorAcc: p.acc,
        sensorMag: p.mag,
        sensorBaro: p.baro,
        sensorGps: p.gps,
        sensorRangefinder: p.rangefinder,
        sensorPitot: p.pitot,
        sensorOpflow: p.opflow,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<{ airspeed: number }>('telemetry-airspeed', (event) => {
      telemetry.update((t) => ({
        ...t,
        airspeed: event.payload.airspeed,
        lastUpdate: Date.now(),
      }));
    })
  );
}

export function stopTelemetryListeners() {
  for (const unlisten of unlisteners) {
    unlisten();
  }
  unlisteners = [];
}
