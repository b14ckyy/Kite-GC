// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Telemetry Adapter — converts DB TelemetryRecord → TelemetryData
 *
 * This is the single replay adapter: every protocol that records to the
 * unified DB schema goes through this mapper for playback into widgets.
 * NULL fields (protocol didn't provide them) default to 0.
 */

import type { TelemetryData } from '$lib/stores/telemetry';
import type { TelemetryRecord } from '$lib/stores/flightlog';

/** Normalize a recorded raw RSSI to 0–100 %. The scale isn't stored, so infer it from the magnitude:
 *  ≤100 already a percentage, ≤255 the MAVLink 0–254 range, otherwise the INAV 0–1023 range. */
function rssiRawToPercent(raw: number): number {
  if (raw <= 100) return raw;
  if (raw <= 255) return (raw / 254) * 100;
  return (raw / 1023) * 100;
}

/** Map a CRSF/ELRS uplink RSSI in dBm to 0–100 % — mirrors the backend `LinkStatsData::dbm_to_percent`
 *  (−50 dBm = 100 %, −120 dBm = 0 %). */
function dbmToPercent(dbm: number): number {
  return Math.max(0, Math.min(100, ((dbm + 120) / 70) * 100));
}

/** Convert a DB telemetry row to the widget-consumable TelemetryData format. */
export function toTelemetryData(r: TelemetryRecord, fcVariant = 'INAV'): TelemetryData {
  return {
    // GPS — always use raw GPS for position (nav fused local offsets are inaccurate for geo coords)
    lat: r.lat ?? 0,
    lon: r.lon ?? 0,
    altMsl: r.alt_m ?? 0,
    groundSpeed: r.speed_ms ?? 0,
    numSat: r.num_sat ?? 0,
    fixType: r.fix_type ?? 0,
    gpsHdop: r.gps_hdop ?? 0,
    // `heading` column = course over ground (direction of travel); `yaw` column = FC fused heading.
    course: r.heading ?? 0,

    // Attitude — yaw is the FC fused heading (body orientation / icon). Fall back to COG only if the
    // log has no heading at all, so the icon still points sensibly.
    roll: r.roll ?? 0,
    pitch: r.pitch ?? 0,
    yaw: r.yaw ?? r.heading ?? 0,

    // Altitude — prefer nav filter fused altitude (smooth), fallback to baro, then GPS
    altitude: r.nav_alt_m ?? r.baro_alt_m ?? r.alt_m ?? 0,
    vario: r.vario_ms ?? 0,

    // Airspeed (m/s) from the record when present (live recording, ArduPilot ARSP, INAV blackbox).
    airspeed: r.airspeed_ms ?? 0,

    // Wind from the stored NED velocity vector (direction the air moves TOWARD) → "from" bearing.
    windSpeedMs: Math.hypot(r.wind_n_ms ?? 0, r.wind_e_ms ?? 0),
    windDirFrom:
      (r.wind_n_ms || r.wind_e_ms)
        ? ((Math.atan2(r.wind_e_ms ?? 0, r.wind_n_ms ?? 0) * 180) / Math.PI + 180 + 360) % 360
        : 0,

    // Battery / Analog
    voltage: r.voltage ?? 0,
    current: r.current_a ?? 0,
    mAhDrawn: r.mah_drawn ?? 0,
    rssi: r.rssi ?? 0,
    power: (r.voltage ?? 0) * (r.current_a ?? 0),
    batteryPercentage: r.battery_percentage ?? 0,
    cellCount: 0,         // not recorded in DB yet

    // RC link — from the recorded link-stats fields. dBm (CRSF / INAV 9.1) → % via the same curve as
    // the backend; otherwise normalize the raw RSSI heuristically (≤100 = %, ≤255 = 0–254, else 0–1023).
    link: {
      rssiPercent: r.link_rssi_dbm != null
        ? dbmToPercent(r.link_rssi_dbm)
        : (r.rssi != null ? rssiRawToPercent(r.rssi) : null),
      rssiDbm: r.link_rssi_dbm ?? null,
      lq: r.link_quality != null && r.link_quality > 0 ? Math.min(100, r.link_quality) : null,
      snrDb: r.link_snr ?? null,
    },

    // Status
    armingFlags: r.state_flags ?? 0,
    cpuLoad: r.cpu_load ?? 0,
    sensorStatus: r.hw_health_status ?? 0,
    flightModeFlags: 0, // raw custom_mode is live-only (vehicle control); not replayed from the DB
    mspRcOverride: false, // live-only (RC control engage); not replayed from the DB

    // Sensor hardware status — not individually recorded in DB
    sensorGyro: 0,
    sensorAcc: 0,
    sensorMag: 0,
    sensorBaro: 0,
    sensorGps: 0,
    sensorRangefinder: 0,
    sensorPitot: 0,
    sensorOpflow: 0,
    prearmHealthy: 0,

    // EKF estimator — live-only (not recorded), default to hidden on replay
    ekfStatus: 0,
    ekfType: 0,

    // Flight mode (canonical) & navigation state
    flightMode: {
      primary: r.mode_primary ?? '',
      modifiers: r.mode_modifiers ? r.mode_modifiers.split(',').filter(Boolean) : [],
    },
    navState: r.nav_state ?? 0,
    activeWpNumber: r.active_wp_number ?? 0,

    // FC type
    fcVariant,

    // Timestamp
    lastUpdate: r.timestamp_ms,
  };
}
