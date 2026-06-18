// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry decoding and configuration
// Decodes raw MSP payloads into structured telemetry events.

use serde::{Deserialize, Serialize};

use crate::msp::*;

use crate::flightlog::recorder::FlightRecorder;

/// Telemetry poll group identifiers
#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryGroup {
    Attitude,
    Analog,
    PositionPrimary,
    PositionSecondary,
    Status,
    LinkStats,
}

/// User-configurable telemetry polling rates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Attitude poll rate in Hz (1.0–5.0), default 5.0
    pub attitude_rate_hz: f64,
    /// Position primary poll rate in Hz (1.0–5.0), default 2.0
    pub position_rate_hz: f64,
    /// Enable airspeed polling (requires pitot sensor)
    pub airspeed_enabled: bool,
    /// Poll MSP2_INAV_GET_LINK_STATS (RC link RSSI/LQ/SNR) — INAV 9.1+ only. When on, the RSSI-only
    /// link derived from MSPV2_INAV_ANALOG is suppressed so this richer source is authoritative.
    #[serde(default)]
    pub link_stats_enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            attitude_rate_hz: 5.0,
            position_rate_hz: 2.0,
            airspeed_enabled: false,
            link_stats_enabled: false,
        }
    }
}

// ── Telemetry event payloads ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttitudeData {
    pub roll: f64,  // degrees, ±180
    pub pitch: f64, // degrees, ±90
    pub yaw: f64,   // heading 0–360 (degrees, decimals preserved)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpsData {
    pub fix_type: u8,
    pub num_sat: u8,
    pub lat: f64,         // decimal degrees
    pub lon: f64,         // decimal degrees
    pub alt_msl: f64,     // meters
    pub ground_speed: f64, // cm/s → m/s
    pub course: f64,      // degrees * 10 → degrees
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AltitudeData {
    pub altitude: f64, // cm → meters
    pub vario: f64,    // cm/s → m/s
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalogData {
    pub voltage: f64,     // V (0.01V units → V)
    pub mah_drawn: u32,
    pub rssi: u16,
    pub current: f64,     // A (0.01A units → A)
    pub power: f64,       // W (0.01W units → W)
    pub battery_percentage: u8,
    pub cell_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    pub arming_flags: u32,
    pub flight_mode_flags: u32,
    pub cpu_load: u16,
    pub sensor_status: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorStatusData {
    pub gyro: u8,        // hardwareSensorStatus_e: 0=NONE, 1=OK, 2=UNAVAILABLE, 3=UNHEALTHY
    pub acc: u8,
    pub mag: u8,
    pub baro: u8,
    pub gps: u8,
    pub rangefinder: u8,
    pub pitot: u8,
    pub opflow: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirspeedData {
    pub airspeed: f64, // cm/s → m/s
}

/// Unified RC-link statistics (RC Link widget). Every field is optional: each protocol fills only what
/// it can (LTM/MAVLink/INAV-pre-9.1 = RSSI only; SmartPort = RSSI + LQ; CRSF = RSSI dBm + LQ + SNR), and
/// the widget shows present fields and hides the rest. `rssi_percent` is normalized at the source (which
/// knows its own raw scale) so the frontend never has to guess the protocol's RSSI range.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkStatsData {
    pub rssi_percent: Option<f32>, // 0–100, normalized
    pub rssi_dbm: Option<i16>,     // raw dBm (negative), when the protocol reports it (CRSF, INAV 9.1+)
    pub lq: Option<u8>,            // link quality 0–100
    pub snr_db: Option<i8>,        // signal-to-noise ratio, dB (CRSF, INAV 9.1+)
}

impl LinkStatsData {
    /// RSSI-only link from a 0–1023 INAV-scale value (INAV `MSPV2_INAV_ANALOG`; LTM after its
    /// 0–254→0–1023 mapping). Out-of-range values are clamped.
    pub fn from_rssi_1023(rssi: u16) -> Self {
        Self {
            rssi_percent: Some((rssi.min(1023) as f32) / 1023.0 * 100.0),
            ..Default::default()
        }
    }

    /// Map a CRSF/ELRS uplink RSSI in dBm to a 0–100 percentage. Linear over the usable window
    /// −50 dBm (close, 100 %) … −120 dBm (sensitivity floor, 0 %) — the convention OpenTX/Yaapu use.
    pub fn dbm_to_percent(dbm: i16) -> f32 {
        (((dbm as f32) + 120.0) / 70.0 * 100.0).clamp(0.0, 100.0)
    }
}

/// GPS quality statistics (from MSP_GPSSTATISTICS 166)
#[derive(Debug, Clone, Serialize)]
pub struct GpsStatsData {
    pub hdop: f64,
    /// Estimated horizontal/vertical position error (raw INAV units, cm). `None` if the firmware's
    /// payload is too short to contain them.
    pub eph: Option<f64>,
    pub epv: Option<f64>,
}

/// Navigation status (from MSP_NAV_STATUS 121) — the currently targeted waypoint.
#[derive(Debug, Clone, Serialize)]
pub struct NavStatusData {
    pub active_wp_number: u8, // FC's current target WP (0 = not navigating a mission)
    pub nav_state: u8,
}

/// Generic telemetry payload wrapper for Tauri events
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum TelemetryPayload {
    Attitude(AttitudeData),
    Gps(GpsData),
    Altitude(AltitudeData),
    Analog(AnalogData),
    Status(StatusData),
    SensorStatus(SensorStatusData),
    Airspeed(AirspeedData),
    GpsStats(GpsStatsData),
    NavStatus(NavStatusData),
    LinkStats(LinkStatsData),
}

/// Map MSP code to Tauri event name
pub fn event_name_for_code(code: u16) -> String {
    match code {
        MSP_ATTITUDE => "telemetry-attitude".into(),
        MSP_RAW_GPS => "telemetry-gps".into(),
        MSP_ALTITUDE => "telemetry-altitude".into(),
        MSPV2_INAV_ANALOG => "telemetry-analog".into(),
        MSPV2_INAV_STATUS => "telemetry-status".into(),
        MSP_SENSOR_STATUS => "telemetry-sensor-status".into(),
        MSPV2_INAV_AIR_SPEED => "telemetry-airspeed".into(),
        MSP_GPSSTATISTICS => "telemetry-gps-stats".into(),
        MSP_NAV_STATUS => "telemetry-nav-status".into(),
        MSP2_INAV_GET_LINK_STATS => "telemetry-linkstats".into(),
        _ => format!("telemetry-0x{:04X}", code),
    }
}

/// Decode a raw MSP payload into a structured telemetry event
pub fn decode_telemetry(code: u16, payload: &[u8], box_ids: &[u8]) -> TelemetryPayload {
    match code {
        MSP_ATTITUDE => decode_attitude(payload),
        MSP_RAW_GPS => decode_gps(payload),
        MSP_ALTITUDE => decode_altitude(payload),
        MSPV2_INAV_ANALOG => decode_analog(payload),
        MSPV2_INAV_STATUS => decode_status(payload, box_ids),
        MSP_SENSOR_STATUS => decode_sensor_status(payload),
        MSPV2_INAV_AIR_SPEED => decode_airspeed(payload),
        MSP_GPSSTATISTICS => decode_gps_statistics(payload),
        MSP_NAV_STATUS => decode_nav_status_event(payload),
        MSP2_INAV_GET_LINK_STATS => decode_link_stats(payload),
        _ => {
            log::warn!("No decoder for MSP 0x{:04X}", code);
            TelemetryPayload::Attitude(AttitudeData {
                roll: 0.0,
                pitch: 0.0,
                yaw: 0.0,
            })
        }
    }
}

// ── Decoders ─────────────────────────────────────────────────────────

/// MSP_NAV_STATUS (121): the FC's current navigation/target-waypoint state. Reuses the
/// mission codec's decoder; on a malformed payload, reports "not navigating" (0).
fn decode_nav_status_event(payload: &[u8]) -> TelemetryPayload {
    match crate::mission::codec::decode_nav_status(payload) {
        Ok(s) => TelemetryPayload::NavStatus(NavStatusData {
            active_wp_number: s.active_wp_number,
            nav_state: s.nav_state,
        }),
        Err(_) => TelemetryPayload::NavStatus(NavStatusData { active_wp_number: 0, nav_state: 0 }),
    }
}

/// MSP2_INAV_GET_LINK_STATS (0x2103, INAV 9.1+): [uplinkRSSI:u8 (=−dBm), uplinkLQ:u8 (%), uplinkSNR:i8 (dB)].
/// INAV sends the RSSI as the negated magnitude (`(uint8_t)-uplinkRSSI`), so the true dBm is −byte0.
fn decode_link_stats(payload: &[u8]) -> TelemetryPayload {
    if payload.len() < 3 {
        return TelemetryPayload::LinkStats(LinkStatsData::default());
    }
    let rssi_dbm = -(payload[0] as i16);
    let lq = payload[1];
    let snr = payload[2] as i8;
    TelemetryPayload::LinkStats(LinkStatsData {
        rssi_percent: Some(LinkStatsData::dbm_to_percent(rssi_dbm)),
        rssi_dbm: Some(rssi_dbm),
        lq: Some(lq),
        snr_db: Some(snr),
    })
}

fn read_i16(buf: &[u8], offset: usize) -> i16 {
    if offset + 1 < buf.len() {
        i16::from_le_bytes([buf[offset], buf[offset + 1]])
    } else {
        0
    }
}

fn read_u16(buf: &[u8], offset: usize) -> u16 {
    if offset + 1 < buf.len() {
        u16::from_le_bytes([buf[offset], buf[offset + 1]])
    } else {
        0
    }
}

fn read_i32(buf: &[u8], offset: usize) -> i32 {
    if offset + 3 < buf.len() {
        i32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
    } else {
        0
    }
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    if offset + 3 < buf.len() {
        u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
    } else {
        0
    }
}

/// MSP_ATTITUDE (108): [roll:i16, pitch:i16, yaw:i16] — tenths of degrees
fn decode_attitude(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Attitude(AttitudeData {
        roll: read_i16(payload, 0) as f64 / 10.0,
        pitch: read_i16(payload, 2) as f64 / 10.0,
        yaw: read_i16(payload, 4) as f64, // MSP yaw is whole degrees
    })
}

/// MSP_RAW_GPS (106): [fixType:u8, numSat:u8, lat:i32, lon:i32, alt:i16, speed:u16, cog:u16]
fn decode_gps(payload: &[u8]) -> TelemetryPayload {
    let fix_type = if !payload.is_empty() { payload[0] } else { 0 };
    let num_sat = if payload.len() > 1 { payload[1] } else { 0 };

    TelemetryPayload::Gps(GpsData {
        fix_type,
        num_sat,
        lat: read_i32(payload, 2) as f64 / 1e7,
        lon: read_i32(payload, 6) as f64 / 1e7,
        alt_msl: read_i16(payload, 10) as f64, // meters
        ground_speed: read_u16(payload, 12) as f64 / 100.0, // cm/s → m/s
        course: read_u16(payload, 14) as f64 / 10.0, // decidegrees → degrees
    })
}

/// MSP_ALTITUDE (109): [altitude:i32, vario:i16] — cm, cm/s
fn decode_altitude(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Altitude(AltitudeData {
        altitude: read_i32(payload, 0) as f64 / 100.0, // cm → m
        vario: read_i16(payload, 4) as f64 / 100.0,    // cm/s → m/s
    })
}

/// MSPV2_INAV_ANALOG (0x2002): Extended battery/power data
/// Layout: [batteryFlags:u8, vbat:u16, amperage:i16, powerDraw:u32,
///          mAhDrawn:u32, mWhDrawn:u32, remainingCapacity:u32,
///          percentageRemaining:u8, rssi:u16]
fn decode_analog(payload: &[u8]) -> TelemetryPayload {
    let battery_flags = if !payload.is_empty() { payload[0] } else { 0 };
    let cell_count = (battery_flags >> 4) & 0x0F;

    TelemetryPayload::Analog(AnalogData {
        voltage: read_u16(payload, 1) as f64 / 100.0,     // 0.01V → V
        current: read_i16(payload, 3) as f64 / 100.0,     // 0.01A → A
        power: read_u32(payload, 5) as f64 / 100.0,       // 0.01W → W
        mah_drawn: read_u32(payload, 9),                    // mAh
        battery_percentage: if payload.len() > 21 { payload[21] } else { 0 },
        rssi: read_u16(payload, 22),
        cell_count,
    })
}

/// Map an INAV **permanent box ID** to our FLIGHT_MODE bitmask position.
///
/// CRITICAL: MSP_BOXIDS (and therefore the active-modes bitmask) reports the **`permanentId`**
/// field from INAV `fc/fc_msp_box.c` `boxes[]` — NOT the `boxId_e` enum ordinal. The two differ
/// for most boxes (e.g. BOXNAVWP: enum 19 but permanentId 28; BOXTURNASSIST: enum 26 but
/// permanentId 35). The permanentId is STABLE across releases (that is its entire purpose), so a
/// permanentId table is version-robust. Output bits match `runtime_config.h` flightModeFlags_e and
/// `trackColors.ts` FLIGHT_MODE. (An earlier version used the enum ordinals → NAV_WP decoded as
/// AUTO_TUNE and TURN_ASSIST as NAV_COURSE_HOLD, so a flown mission showed "Cruise" not "Mission".)
fn box_id_to_flight_mode_bit(box_id: usize) -> Option<u32> {
    match box_id {
        1  => Some(1 << 0),   // BOXANGLE         → ANGLE_MODE
        2  => Some(1 << 1),   // BOXHORIZON       → HORIZON_MODE
        5  => Some(1 << 2),   // BOXHEADINGHOLD   → HEADING_MODE
        3  => Some(1 << 3),   // BOXNAVALTHOLD    → NAV_ALTHOLD_MODE
        10 => Some(1 << 4),   // BOXNAVRTH        → NAV_RTH_MODE
        11 => Some(1 << 5),   // BOXNAVPOSHOLD    → NAV_POSHOLD_MODE
        6  => Some(1 << 6),   // BOXHEADFREE      → HEADFREE_MODE
        36 => Some(1 << 7),   // BOXNAVLAUNCH     → NAV_LAUNCH_MODE
        12 => Some(1 << 8),   // BOXMANUAL        → MANUAL_MODE
        27 => Some(1 << 9),   // BOXFAILSAFE      → FAILSAFE_MODE
        21 => Some(1 << 10),  // BOXAUTOTUNE      → AUTO_TUNE
        28 => Some(1 << 11),  // BOXNAVWP         → NAV_WP_MODE
        45 => Some(1 << 12),  // BOXNAVCOURSEHOLD → NAV_COURSE_HOLD_MODE
        53 => Some(1 << 12),  // BOXNAVCRUISE     → NAV_COURSE_HOLD_MODE (cruise = course hold + alt; shares display)
        34 => Some(1 << 13),  // BOXFLAPERON      → FLAPERON
        35 => Some(1 << 14),  // BOXTURNASSIST    → TURN_ASSISTANT
        52 => Some(1 << 15),  // BOXTURTLE        → TURTLE_MODE
        56 => Some(1 << 16),  // BOXSOARING       → SOARING_MODE
        64 => Some(1 << 17),  // BOXANGLEHOLD     → ANGLEHOLD_MODE
        // NAV_FW_AUTOLAND (1<<18) has no box → not detectable live; appears only in logged flags (replay).
        _  => None,
    }
}

/// Parse the activeModes packed bitmask from MSPV2_INAV_STATUS payload
/// and convert INAV box IDs to our FLIGHT_MODE bitmask.
///
/// IMPORTANT: INAV packs activeModes by INDEX into the configured box list,
/// NOT by permanent box ID. We need the box_ids mapping from MSP_BOXIDS (119)
/// to correctly translate bit positions → permanent IDs → flight mode flags.
///
/// Payload layout: [...13 bytes header...][activeModes bytes][mixerProfile:u8]
fn parse_active_modes(payload: &[u8], box_ids: &[u8]) -> u32 {
    if payload.len() < 15 {
        return 0; // Payload too short to contain activeModes
    }
    let modes_start = 13;
    let modes_end = payload.len() - 1; // last byte is mixerProfile
    let modes_bytes = &payload[modes_start..modes_end];

    let mut flags: u32 = 0;
    let mut active_box_ids: Vec<usize> = Vec::new();
    for (byte_idx, &byte) in modes_bytes.iter().enumerate() {
        for bit in 0..8usize {
            if byte & (1 << bit) != 0 {
                let bit_index = byte_idx * 8 + bit;
                // Translate bit index → permanent box ID using the MSP_BOXIDS mapping
                let permanent_id = if bit_index < box_ids.len() {
                    box_ids[bit_index] as usize
                } else {
                    // Fallback: no mapping available, treat index as permanent ID
                    bit_index
                };
                active_box_ids.push(permanent_id);
                if let Some(flag) = box_id_to_flight_mode_bit(permanent_id) {
                    flags |= flag;
                }
            }
        }
    }
    eprintln!("[MSP-MODES] payload_len={}, modes_bytes={}, active_box_ids={:?}, flags=0x{:05X}",
        payload.len(), modes_bytes.len(), active_box_ids, flags);

    // Mirror INAV firmware logic from processRcModes():
    // When a NAV mode (ALTHOLD, POSHOLD, RTH, WP, LAUNCH) is active but no explicit
    // stabilization mode (ANGLE, HORIZON, MANUAL) was selected via RC switch,
    // INAV implicitly enables ANGLE_MODE for safety.
    const ANGLE: u32      = 1 << 0;
    const HORIZON: u32    = 1 << 1;
    const NAV_ALTHOLD: u32 = 1 << 3;
    const NAV_RTH: u32    = 1 << 4;
    const NAV_POSHOLD: u32 = 1 << 5;
    const NAV_LAUNCH: u32 = 1 << 7;
    const MANUAL: u32     = 1 << 8;
    const NAV_WP: u32     = 1 << 11;

    let has_nav = flags & (NAV_ALTHOLD | NAV_RTH | NAV_POSHOLD | NAV_LAUNCH | NAV_WP) != 0;
    let has_stab = flags & (ANGLE | HORIZON | MANUAL) != 0;
    if has_nav && !has_stab {
        flags |= ANGLE;
        eprintln!("[MSP-MODES] Implicit ANGLE from NAV mode, flags=0x{:05X}", flags);
    }

    flags
}

/// MSPV2_INAV_STATUS (0x2000): comprehensive FC status
/// Layout: [cycleTime:u16, i2cErrors:u16, sensorStatus:u16, cpuLoad:u16,
///          profileAndBattProfile:u8, armingFlags:u32, activeModes:..., mixerProfile:u8]
fn decode_status(payload: &[u8], box_ids: &[u8]) -> TelemetryPayload {
    let sensor_status = read_u16(payload, 4);
    let cpu_load = read_u16(payload, 6);
    let arming_flags = read_u32(payload, 9);
    let flight_mode_flags = parse_active_modes(payload, box_ids);

    TelemetryPayload::Status(StatusData {
        arming_flags,
        flight_mode_flags,
        cpu_load,
        sensor_status,
    })
}

/// MSP_SENSOR_STATUS (151): per-sensor hardware health
/// Layout: [overallHealth:u8, gyro:u8, acc:u8, mag:u8, baro:u8,
///          gps:u8, rangefinder:u8, pitot:u8, opflow:u8]
/// Values: 0=NONE, 1=OK, 2=UNAVAILABLE, 3=UNHEALTHY
fn decode_sensor_status(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::SensorStatus(SensorStatusData {
        gyro: if payload.len() > 1 { payload[1] } else { 0 },
        acc: if payload.len() > 2 { payload[2] } else { 0 },
        mag: if payload.len() > 3 { payload[3] } else { 0 },
        baro: if payload.len() > 4 { payload[4] } else { 0 },
        gps: if payload.len() > 5 { payload[5] } else { 0 },
        rangefinder: if payload.len() > 6 { payload[6] } else { 0 },
        pitot: if payload.len() > 7 { payload[7] } else { 0 },
        opflow: if payload.len() > 8 { payload[8] } else { 0 },
    })
}

/// MSPV2_INAV_AIR_SPEED (0x2009): [airspeed:i32] — cm/s
fn decode_airspeed(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Airspeed(AirspeedData {
        airspeed: read_i32(payload, 0) as f64 / 100.0, // cm/s → m/s
    })
}

/// MSP_GPSSTATISTICS (166): [lastDt:u32, errors:u32, timeouts:u32, packetCount:u32,
///                            hdop:u16, eph:u16, epv:u16]
/// HDOP is a raw u16, scaled * 100 by INAV (e.g. 100 = HDOP 1.00).
fn decode_gps_statistics(payload: &[u8]) -> TelemetryPayload {
    let hdop_raw = read_u16(payload, 16); // bytes 16–17
    let hdop = hdop_raw as f64 / 100.0;
    // eph (18–19) / epv (20–21) ride along in the same message — captured for the recorder.
    let eph = if payload.len() >= 20 { Some(read_u16(payload, 18) as f64) } else { None };
    let epv = if payload.len() >= 22 { Some(read_u16(payload, 20) as f64) } else { None };
    eprintln!("[GPS-STATS] hdop_raw={} hdop={:.2} eph={:?} epv={:?}", hdop_raw, hdop, eph, epv);
    TelemetryPayload::GpsStats(GpsStatsData { hdop, eph, epv })
}

/// Feed decoded telemetry data to the flight recorder.
/// Decodes the raw payload again (cheap) to pass typed data to the recorder.
pub fn feed_recorder(code: u16, payload: &[u8], recorder: &mut FlightRecorder, box_ids: &[u8]) {
    match code {
        MSP_ATTITUDE => {
            let data = AttitudeData {
                roll: read_i16(payload, 0) as f64 / 10.0,
                pitch: read_i16(payload, 2) as f64 / 10.0,
                yaw: read_i16(payload, 4) as f64, // MSP yaw is whole degrees
            };
            recorder.on_attitude(&data);
        }
        MSP_RAW_GPS => {
            let data = GpsData {
                fix_type: if !payload.is_empty() { payload[0] } else { 0 },
                num_sat: if payload.len() > 1 { payload[1] } else { 0 },
                lat: read_i32(payload, 2) as f64 / 1e7,
                lon: read_i32(payload, 6) as f64 / 1e7,
                alt_msl: read_i16(payload, 10) as f64,
                ground_speed: read_u16(payload, 12) as f64 / 100.0,
                course: read_u16(payload, 14) as f64 / 10.0,
            };
            recorder.on_gps(&data);
        }
        MSP_ALTITUDE => {
            let data = AltitudeData {
                altitude: read_i32(payload, 0) as f64 / 100.0,
                vario: read_i16(payload, 4) as f64 / 100.0,
            };
            recorder.on_altitude(&data);
        }
        MSPV2_INAV_ANALOG => {
            let data = AnalogData {
                voltage: read_u16(payload, 1) as f64 / 100.0,
                current: read_i16(payload, 3) as f64 / 100.0,
                power: read_u32(payload, 5) as f64 / 100.0,
                mah_drawn: read_u32(payload, 9),
                battery_percentage: if payload.len() > 21 { payload[21] } else { 0 },
                rssi: read_u16(payload, 22),
                cell_count: if !payload.is_empty() { (payload[0] >> 4) & 0x0F } else { 0 },
            };
            recorder.on_analog(&data);
        }
        MSPV2_INAV_STATUS => {
            let data = StatusData {
                arming_flags: read_u32(payload, 9),
                flight_mode_flags: parse_active_modes(payload, box_ids),
                cpu_load: read_u16(payload, 6),
                sensor_status: read_u16(payload, 4),
            };
            recorder.on_status(&data);
        }
        MSPV2_INAV_AIR_SPEED => {
            let data = AirspeedData {
                airspeed: read_i32(payload, 0) as f64 / 100.0,
            };
            recorder.on_airspeed(&data);
        }
        MSP_NAV_STATUS => {
            // Mission context (target WP + nav state) — recorded so replay matches the live map.
            if let Ok(s) = crate::mission::codec::decode_nav_status(payload) {
                recorder.on_nav_status(&NavStatusData {
                    active_wp_number: s.active_wp_number,
                    nav_state: s.nav_state,
                });
            }
        }
        MSP_GPSSTATISTICS => {
            if let TelemetryPayload::GpsStats(data) = decode_gps_statistics(payload) {
                recorder.on_gps_stats(&data);
            }
        }
        MSP_SENSOR_STATUS => {
            if let TelemetryPayload::SensorStatus(data) = decode_sensor_status(payload) {
                recorder.on_sensor_status(&data);
            }
        }
        MSP2_INAV_GET_LINK_STATS => {
            // INAV 9.1+ real RC link stats (dBm / LQ / SNR) → recorded for replay.
            if let TelemetryPayload::LinkStats(data) = decode_link_stats(payload) {
                recorder.on_linkstats(&data);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_attitude() {
        // roll=45.0° (450), pitch=-10.5° (-105), yaw=270
        let payload = [
            0xC2, 0x01, // 450 → 45.0°
            0x97, 0xFF, // -105 → -10.5°
            0x0E, 0x01, // 270
        ];
        match decode_telemetry(MSP_ATTITUDE, &payload, &[]) {
            TelemetryPayload::Attitude(a) => {
                assert!((a.roll - 45.0).abs() < 0.01);
                assert!((a.pitch - (-10.5)).abs() < 0.01);
                assert_eq!(a.yaw, 270.0);
            }
            _ => panic!("Expected Attitude"),
        }
    }

    #[test]
    fn test_decode_gps() {
        // fixType=2, numSat=12, lat=51.505*1e7, lon=-0.09*1e7, alt=100m, speed=500cm/s, cog=1800
        let mut payload = vec![2u8, 12];
        payload.extend_from_slice(&515050000i32.to_le_bytes()); // lat
        payload.extend_from_slice(&(-900000i32).to_le_bytes()); // lon
        payload.extend_from_slice(&100i16.to_le_bytes());       // alt
        payload.extend_from_slice(&500u16.to_le_bytes());       // speed
        payload.extend_from_slice(&1800u16.to_le_bytes());      // cog

        match decode_telemetry(MSP_RAW_GPS, &payload, &[]) {
            TelemetryPayload::Gps(g) => {
                assert_eq!(g.fix_type, 2);
                assert_eq!(g.num_sat, 12);
                assert!((g.lat - 51.505).abs() < 0.001);
                assert!((g.lon - (-0.09)).abs() < 0.001);
                assert_eq!(g.alt_msl as i16, 100);
                assert!((g.ground_speed - 5.0).abs() < 0.01);
                assert!((g.course - 180.0).abs() < 0.01);
            }
            _ => panic!("Expected Gps"),
        }
    }

    #[test]
    fn test_decode_altitude() {
        // altitude=15000cm (150m), vario=250cm/s (2.5m/s)
        let mut payload = Vec::new();
        payload.extend_from_slice(&15000i32.to_le_bytes());
        payload.extend_from_slice(&250i16.to_le_bytes());

        match decode_telemetry(MSP_ALTITUDE, &payload, &[]) {
            TelemetryPayload::Altitude(a) => {
                assert!((a.altitude - 150.0).abs() < 0.01);
                assert!((a.vario - 2.5).abs() < 0.01);
            }
            _ => panic!("Expected Altitude"),
        }
    }

    #[test]
    fn test_event_names() {
        assert_eq!(event_name_for_code(MSP_ATTITUDE), "telemetry-attitude");
        assert_eq!(event_name_for_code(MSP_RAW_GPS), "telemetry-gps");
        assert_eq!(event_name_for_code(MSPV2_INAV_ANALOG), "telemetry-analog");
        assert_eq!(event_name_for_code(0xFFFF), "telemetry-0xFFFF");
    }

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert_eq!(config.attitude_rate_hz, 5.0);
        assert_eq!(config.position_rate_hz, 2.0);
        assert!(!config.airspeed_enabled);
    }

    #[test]
    fn test_decode_analog_v2() {
        // MSP2_INAV_ANALOG (0x2002) — correct format with batteryFlags at offset 0
        // batteryFlags=0x31 (cellCount=3 in bits 4-7, state in bits 2-3)
        // vbat=1164 (11.64V), amperage=1994 (19.94A), powerDraw=23200 (232.00W)
        // mAhDrawn=500, mWhDrawn=5820, remainingCapacity=4500
        // percentageRemaining=90, rssi=512
        let mut payload = vec![0x31u8]; // batteryFlags: 3 cells, state bits
        payload.extend_from_slice(&1164u16.to_le_bytes());    // vbat = 11.64V
        payload.extend_from_slice(&1994i16.to_le_bytes());    // amperage = 19.94A
        payload.extend_from_slice(&23200u32.to_le_bytes());   // powerDraw = 232.00W
        payload.extend_from_slice(&500u32.to_le_bytes());     // mAhDrawn
        payload.extend_from_slice(&5820u32.to_le_bytes());    // mWhDrawn
        payload.extend_from_slice(&4500u32.to_le_bytes());    // remainingCapacity
        payload.push(90);                                       // percentageRemaining
        payload.extend_from_slice(&512u16.to_le_bytes());     // rssi

        match decode_telemetry(MSPV2_INAV_ANALOG, &payload, &[]) {
            TelemetryPayload::Analog(a) => {
                assert!((a.voltage - 11.64).abs() < 0.01, "voltage: {}", a.voltage);
                assert!((a.current - 19.94).abs() < 0.01, "current: {}", a.current);
                assert!((a.power - 232.0).abs() < 0.01, "power: {}", a.power);
                assert_eq!(a.mah_drawn, 500);
                assert_eq!(a.battery_percentage, 90);
                assert_eq!(a.rssi, 512);
                assert_eq!(a.cell_count, 3);
            }
            _ => panic!("Expected Analog"),
        }
    }

    #[test]
    fn test_decode_status_v2() {
        // MSP2_INAV_STATUS (0x2000) — correct offsets
        let mut payload = Vec::new();
        payload.extend_from_slice(&500u16.to_le_bytes());    // cycleTime
        payload.extend_from_slice(&0u16.to_le_bytes());      // i2cErrors
        payload.extend_from_slice(&0x000Fu16.to_le_bytes()); // sensorStatus (ACC+BARO+MAG+GPS)
        payload.extend_from_slice(&25u16.to_le_bytes());     // cpuLoad
        payload.push(0x00);                                    // profileAndBattProfile
        payload.extend_from_slice(&4u32.to_le_bytes());      // armingFlags (bit 2 = ARMED)

        match decode_telemetry(MSPV2_INAV_STATUS, &payload, &[]) {
            TelemetryPayload::Status(s) => {
                assert_eq!(s.cpu_load, 25);
                assert_eq!(s.arming_flags, 4); // ARMED
                assert_eq!(s.sensor_status, 0x000F);
            }
            _ => panic!("Expected Status"),
        }
    }

    #[test]
    fn test_decode_sensor_status() {
        // MSP_SENSOR_STATUS (151)
        let payload = [
            1, // overallHealth = OK
            1, // gyro = OK
            1, // acc = OK
            1, // mag = OK
            1, // baro = OK
            1, // gps = OK
            0, // rangefinder = NONE
            0, // pitot = NONE
            0, // opflow = NONE
        ];
        match decode_telemetry(MSP_SENSOR_STATUS, &payload, &[]) {
            TelemetryPayload::SensorStatus(s) => {
                assert_eq!(s.gyro, 1);
                assert_eq!(s.acc, 1);
                assert_eq!(s.mag, 1);
                assert_eq!(s.baro, 1);
                assert_eq!(s.gps, 1);
                assert_eq!(s.rangefinder, 0);
                assert_eq!(s.pitot, 0);
                assert_eq!(s.opflow, 0);
            }
            _ => panic!("Expected SensorStatus"),
        }
    }
}
