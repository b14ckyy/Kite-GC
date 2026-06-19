// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry state store
// Holds real-time telemetry data received from the flight controller.
// Listens to Tauri events emitted by the MSP scheduler thread.

import { writable, get } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { connectionProtocol, fcLinkAlive } from '$lib/stores/connection';
import { arduVehicleClass } from '$lib/stores/missionArdupilot';
import type { FlightModeState } from '$lib/helpers/flightModeRegistry';

/** Unified RC-link statistics (RC Link widget). Each field is null when the active protocol can't
 *  provide it — the widget shows present fields and hides the rest. `rssiPercent` is normalized at the
 *  backend source (which knows its own raw RSSI scale). */
export interface LinkStats {
  rssiPercent: number | null; // 0–100
  rssiDbm: number | null;     // raw dBm (CRSF, INAV 9.1+)
  lq: number | null;          // link quality 0–100
  snrDb: number | null;       // dB (CRSF, INAV 9.1+)
}

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

  // RC link statistics (RSSI / LQ / SNR — protocol-dependent, see LinkStats)
  link: LinkStats;

  // Status
  armingFlags: number;
  cpuLoad: number;
  sensorStatus: number;
  /** Raw flight-mode flags. For MAVLink this is the FC's `custom_mode` (used by the vehicle-control
   *  panel to match/highlight the active mode); for MSP it is INAV's box flag bitfield (forensic). */
  flightModeFlags: number;

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

  // EKF estimator health (ArduPilot only; INAV never emits these so the tile stays hidden)
  // ekfStatus: 0=none/unknown, 1=OK (green), 2=warning (amber), 3=error (red)
  // ekfType: 0=unknown, 2=EKF2, 3=EKF3 (from the AHRS_EKF_TYPE parameter)
  ekfStatus: number;
  ekfType: number;

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
  link: { rssiPercent: null, rssiDbm: null, lq: null, snrDb: null },
  armingFlags: 0, cpuLoad: 0, sensorStatus: 0, flightModeFlags: 0,
  sensorGyro: 0, sensorAcc: 0, sensorMag: 0, sensorBaro: 0,
  sensorGps: 0, sensorRangefinder: 0, sensorPitot: 0, sensorOpflow: 0,
  ekfStatus: 0, ekfType: 0,
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

let gpsInjectWasActive = false;
gpsInject.subscribe((g) => {
  gpsOverride = g.active ? g : null;
  if (g.active) {
    gpsInjectWasActive = true;
    telemetry.update((t) => ({
      ...t,
      lat: g.lat, lon: g.lon, altMsl: g.altMsl,
      fixType: 3, numSat: t.numSat || 12,
      lastUpdate: Date.now(),
    }));
  } else if (gpsInjectWasActive) {
    // Drop the fake fix so alerts deactivate; a real connection re-fills on its next GPS packet.
    // Guarded by `gpsInjectWasActive`: the subscription also fires once on app start with active=false,
    // and bumping `lastUpdate` there made an idle store look "live" (lat/lon 0,0) → widgets such as HOME
    // showed a bogus distance with no connection. Only clear when an injection was actually on.
    gpsInjectWasActive = false;
    telemetry.update((t) => ({ ...t, fixType: 0, lastUpdate: Date.now() }));
  }
});

export function resetTelemetry() {
  telemetry.set({ ...defaultTelemetry });
  // Altitude reference defaults back to MSL (replay/idle treat altMsl as true MSL). The ground anchor
  // is intentionally NOT cleared — it persists across reconnects so a relative-only session (LTM) keeps
  // its reference until a real-MSL protocol supersedes it.
  altReference.set({ msl: true });
  // Keep an active injection alive across a telemetry reset (e.g. disconnect while testing).
  if (gpsOverride) {
    const o = gpsOverride;
    telemetry.update((t) => ({ ...t, lat: o.lat, lon: o.lon, altMsl: o.altMsl, fixType: 3, numSat: 12 }));
  }
}

// ── Altitude reference + ground MSL anchor (relative-only protocols: LTM / CRSF) ─────
// Some passive protocols (LTM, CRSF) send only arming-relative altitude, never true MSL. To place the
// UAV against terrain (Live AGL widget, 3D map) we anchor that relative altitude to a ground MSL
// captured at the ARMING EDGE — when the aircraft is on the ground (rel ≈ 0) at the home point. The
// anchor persists across reconnects until a real-MSL protocol supersedes it. With no anchor (late
// connect, or armed in the air) the true MSL is unknown → AGL is shown as N/A.

/** Whether the active protocol delivers true MSL GPS altitude. Default true; the backend's
 *  `telemetry-alt-ref` sets it false for relative-only protocols (LTM/CRSF). */
export const altReference = writable<{ msl: boolean }>({ msl: true });
/** Ground MSL (m) captured at the arming edge for relative-only protocols, or null if unknown. */
export const groundAnchor = writable<number | null>(null);

/** |relative altitude| (m) at the arm edge must be below this to trust it as the ground reference —
 *  this also excludes "armed in the air" (HITL / mid-air arm), where no ground ref is recoverable. */
const GROUND_ARM_MAX_REL = 5;

/** Resolve true MSL from a (possibly relative) altitude + the altitude reference + the ground anchor.
 *  Returns null when the protocol is relative-only and no ground anchor is known (→ AGL/3D show N/A). */
export function resolveTrueMsl(altMsl: number, ref: { msl: boolean }, anchor: number | null): number | null {
  if (ref.msl) return altMsl; // protocol already reports true MSL
  if (anchor != null) return anchor + altMsl; // relative + captured ground MSL
  return null; // relative-only, no reference yet
}

let prevArmed = false;
let anchorBusy = false;

/** Capture the ground MSL at the arming edge (relative-only protocols only, when on the ground). */
async function captureGroundAnchor(t: TelemetryData): Promise<void> {
  if (anchorBusy) return;
  if (get(altReference).msl) return; // protocol already in MSL → no anchor needed
  const validCoord = (t.lat !== 0 || t.lon !== 0) && Math.abs(t.lat) <= 90 && Math.abs(t.lon) <= 180;
  if (t.fixType < 2 || !validCoord) return; // need a GPS fix
  if (Math.abs(t.altMsl) > GROUND_ARM_MAX_REL) return; // armed in the air → no valid ground reference
  anchorBusy = true;
  try {
    const g = await invoke<number | null>('terrain_elevation', { lat: t.lat, lon: t.lon });
    if (g != null) {
      groundAnchor.set(g);
      console.log(`[altAnchor] ground MSL anchor = ${g.toFixed(1)} m (captured at arm, rel ${t.altMsl.toFixed(1)} m)`);
    }
  } catch (e) {
    console.warn('[altAnchor] terrain lookup failed', e);
  } finally {
    anchorBusy = false;
  }
}

// Detect the disarm→arm transition on the live telemetry stream (cheap; only acts on the edge).
telemetry.subscribe((t) => {
  const armed = (t.armingFlags & 0x04) !== 0;
  if (armed && !prevArmed) void captureGroundAnchor(t);
  prevArmed = armed;
});

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
    await listen<{ rssi_percent: number | null; rssi_dbm: number | null; lq: number | null; snr_db: number | null }>(
      'telemetry-linkstats',
      (event) => {
        const p = event.payload;
        telemetry.update((t) => ({
          ...t,
          link: { rssiPercent: p.rssi_percent, rssiDbm: p.rssi_dbm, lq: p.lq, snrDb: p.snr_db },
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
        flightModeFlags: event.payload.flight_mode_flags,
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

  // EKF estimator health (ArduPilot) — drives the header EKF indicator.
  unlisteners.push(
    await listen<{ status: number; max_variance: number; flags: number }>('telemetry-ekf-status', (event) => {
      telemetry.update((t) => ({ ...t, ekfStatus: event.payload.status, lastUpdate: Date.now() }));
    })
  );

  // EKF core version (AHRS_EKF_TYPE) — one-shot reply on connect, so no lastUpdate bump.
  unlisteners.push(
    await listen<{ ekf_type: number }>('telemetry-ekf-type', (event) => {
      telemetry.update((t) => ({ ...t, ekfType: event.payload.ekf_type }));
    })
  );

  // QuadPlane detection (ArduPilot Q_ENABLE reply) — a QuadPlane reports MAV_TYPE_FIXED_WING, so the
  // mission vehicle class can only be upgraded to quadplane from this one-shot param. Upgrade only.
  unlisteners.push(
    await listen<{ quadplane: boolean }>('telemetry-vehicle', (event) => {
      if (event.payload.quadplane) arduVehicleClass.set('quadplane');
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

  // Altitude reference: does this protocol deliver true MSL, or only arming-relative altitude?
  unlisteners.push(
    await listen<{ msl: boolean }>('telemetry-alt-ref', (event) => {
      altReference.set({ msl: event.payload.msl });
      // A real-MSL protocol supersedes any relative ground anchor.
      if (event.payload.msl) groundAnchor.set(null);
    })
  );

  // Passive-telemetry locked protocol (+ optional secondary) for the connection status box.
  unlisteners.push(
    await listen<{ primary: string; secondary: string | null }>('telemetry-protocol', (event) => {
      connectionProtocol.set({ primary: event.payload.primary, secondary: event.payload.secondary });
    })
  );

  // FC-link liveness (passive): fresh FC-origin frames, independent of the cached-state re-emit + RX noise.
  unlisteners.push(
    await listen<{ alive: boolean }>('telemetry-fc-link', (event) => {
      fcLinkAlive.set(event.payload.alive);
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
