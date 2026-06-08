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
    course: r.heading ?? 0,

    // Attitude — yaw: prefer GPS COG for replay (heading), fall back to attitude yaw
    roll: r.roll ?? 0,
    pitch: r.pitch ?? 0,
    yaw: r.heading ?? r.yaw ?? 0,

    // Altitude — prefer nav filter fused altitude (smooth), fallback to baro, then GPS
    altitude: r.nav_alt_m ?? r.baro_alt_m ?? r.alt_m ?? 0,
    vario: r.vario_ms ?? 0,

    // Airspeed — not in current DB schema, default 0
    airspeed: 0,

    // Battery / Analog
    voltage: r.voltage ?? 0,
    current: r.current_a ?? 0,
    mAhDrawn: r.mah_drawn ?? 0,
    rssi: r.rssi ?? 0,
    power: (r.voltage ?? 0) * (r.current_a ?? 0),
    batteryPercentage: r.battery_percentage ?? 0,
    cellCount: 0,         // not recorded in DB yet

    // Status
    armingFlags: r.state_flags ?? 0,
    cpuLoad: r.cpu_load ?? 0,
    sensorStatus: r.hw_health_status ?? 0,

    // Sensor hardware status — not individually recorded in DB
    sensorGyro: 0,
    sensorAcc: 0,
    sensorMag: 0,
    sensorBaro: 0,
    sensorGps: 0,
    sensorRangefinder: 0,
    sensorPitot: 0,
    sensorOpflow: 0,

    // Flight mode & navigation state
    activeFlightModeFlags: r.active_flight_mode_flags ?? 0,
    navState: r.nav_state ?? 0,
    activeWpNumber: r.active_wp_number ?? 0,

    // FC type
    fcVariant,

    // Timestamp
    lastUpdate: r.timestamp_ms,
  };
}
