// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry state store
// Holds real-time telemetry data received from the flight controller.
// Listens to Tauri events emitted by the MSP scheduler thread.

import { writable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { FlightModeState } from '$lib/helpers/flightModeRegistry';

export interface TelemetryData {
  // GPS
  lat: number;
  lon: number;
  altMsl: number;
  groundSpeed: number;
  numSat: number;
  fixType: number;
  gpsHdop: number;
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

  // Flight mode (canonical, protocol-agnostic — see flightModeRegistry) & navigation
  flightMode: FlightModeState;
  navState: number;
  /** FC's current target waypoint (MSP_NAV_STATUS live / blackbox in replay). 0 = none. */
  activeWpNumber: number;

  // FC type (for mode classification)
  fcVariant: string;

  // Timestamps
  lastUpdate: number;
}

const defaultTelemetry: TelemetryData = {
  lat: 0, lon: 0, altMsl: 0, groundSpeed: 0, numSat: 0, fixType: 0, gpsHdop: 0, course: 0,
  roll: 0, pitch: 0, yaw: 0,
  altitude: 0, vario: 0,
  airspeed: 0,
  voltage: 0, current: 0, mAhDrawn: 0, rssi: 0, power: 0, batteryPercentage: 0, cellCount: 0,
  armingFlags: 0, cpuLoad: 0, sensorStatus: 0,
  sensorGyro: 0, sensorAcc: 0, sensorMag: 0, sensorBaro: 0,
  sensorGps: 0, sensorRangefinder: 0, sensorPitot: 0, sensorOpflow: 0,
  flightMode: { primary: '', modifiers: [] }, navState: 0, activeWpNumber: 0,
  fcVariant: 'INAV',
  lastUpdate: 0,
};

export const telemetry = writable<TelemetryData>({ ...defaultTelemetry });

// ── Dev GPS injection (debug only) ──────────────────────────────────
// Force the UAV position/altitude (+ a valid fix) regardless of the real telemetry, so conflict alerts
// and the map can be tested over busy airspace from the desk. Driven from the dev Debug Monitor.
export interface GpsInject {
  active: boolean;
  lat: number;
  lon: number;
  altMsl: number;
}

export const gpsInject = writable<GpsInject>({ active: false, lat: 0, lon: 0, altMsl: 0 });

/** Module-level mirror of the override so the live telemetry listeners can honour it (kept in sync by
 *  the subscription below). null = no injection. */
let gpsOverride: GpsInject | null = null;

gpsInject.subscribe((g) => {
  gpsOverride = g.active ? g : null;
  if (g.active) {
    telemetry.update((t) => ({
      ...t,
      lat: g.lat, lon: g.lon, altMsl: g.altMsl,
      fixType: 3, numSat: t.numSat || 12,
      lastUpdate: Date.now(),
    }));
  } else {
    // Drop the fake fix so alerts deactivate; a real connection re-fills on its next GPS packet.
    telemetry.update((t) => ({ ...t, fixType: 0, lastUpdate: Date.now() }));
  }
});

export function resetTelemetry() {
  telemetry.set({ ...defaultTelemetry });
  // Keep an active injection alive across a telemetry reset (e.g. disconnect while testing).
  if (gpsOverride) {
    const o = gpsOverride;
    telemetry.update((t) => ({ ...t, lat: o.lat, lon: o.lon, altMsl: o.altMsl, fixType: 3, numSat: 12 }));
  }
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
      gps_hdop?: number;
    }>('telemetry-gps', (event) => {
      const p = event.payload;
      const o = gpsOverride; // dev injection wins over real position/altitude when active
      telemetry.update((t) => ({
        ...t,
        fixType: o ? 3 : p.fix_type,
        numSat: p.num_sat,
        // Keep last valid HDOP when this packet omits it (or reports 0).
        gpsHdop: typeof p.gps_hdop === 'number' && p.gps_hdop > 0 ? p.gps_hdop : t.gpsHdop,
        lat: o ? o.lat : p.lat,
        lon: o ? o.lon : p.lon,
        altMsl: o ? o.altMsl : p.alt_msl,
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
      // flight_mode_flags is now forensic only — the canonical mode comes via telemetry-flightmode.
      telemetry.update((t) => ({
        ...t,
        armingFlags: event.payload.arming_flags,
        cpuLoad: event.payload.cpu_load,
        sensorStatus: event.payload.sensor_status,
        lastUpdate: Date.now(),
      }));
    })
  );

  unlisteners.push(
    await listen<FlightModeState>('telemetry-flightmode', (event) => {
      telemetry.update((t) => ({
        ...t,
        flightMode: { primary: event.payload.primary, modifiers: event.payload.modifiers ?? [] },
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
    await listen<{ active_wp_number: number; nav_state: number }>('telemetry-nav-status', (event) => {
      telemetry.update((t) => ({
        ...t,
        activeWpNumber: event.payload.active_wp_number,
        navState: event.payload.nav_state,
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

  unlisteners.push(
    await listen<{ hdop: number }>('telemetry-gps-stats', (event) => {
      telemetry.update((t) => ({
        ...t,
        // Ignore invalid values to avoid one-frame UI oscillation.
        gpsHdop: event.payload.hdop > 0 ? event.payload.hdop : t.gpsHdop,
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
