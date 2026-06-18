// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Raw-log parser → logbook (ADR-049). Imports recorded raw serial logs (.rawmsp = mwptools v2 MSP,
// .tlog = MAVLink) into the DB as LIVE flights.
//
//   - Decode the raw frames back through the existing parsers (MSP `MspParser` + `decode_telemetry`,
//     MAVLink `MavParser` + typed `MavMessage`) into a stream of telemetry samples + an armed flag.
//   - Split the stream into individual flights at arm/disarm, applying the same 5 s grace as live
//     recording (a re-arm within 5 s = one flight) — and, unlike live recording, FILL the grace gap
//     (the raw log has those bytes; they're real flight data).
//   - Store each flight as `source = "live"` (NOT blackbox → no live↔blackbox auto-linking), running a
//     duplicate check (craft + start_time window) so re-importing an already-recorded flight is skipped.

use std::path::Path;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;

use ::mavlink::ardupilotmega::{GpsFixType, MavAutopilot, MavMessage, MavModeFlag, MavType};

use super::db;
use super::timezone;
use super::types::{Flight, TelemetryRecord};
use crate::mavlink_proto::parser::MavParser;
use crate::msp::{
    MspParser, MSPV2_INAV_AIR_SPEED, MSPV2_INAV_ANALOG, MSPV2_INAV_MIXER, MSPV2_INAV_STATUS,
    MSP_ALTITUDE, MSP_ATTITUDE, MSP_BOARD_INFO, MSP_FC_VARIANT, MSP_FC_VERSION, MSP_GPSSTATISTICS,
    MSP_NAME, MSP_NAV_STATUS, MSP_RAW_GPS, MSP_SENSOR_STATUS,
};
use crate::scheduler::telemetry::{decode_telemetry, TelemetryPayload};

/// INAV-style re-arm grace (matches the live recorder, ADR-041): re-arm within this window = one flight.
const GRACE_MS: i64 = 5000;
/// Bit 2 of arming_flags = ARMED (MSP).
const ARMED_FLAG: u32 = 0x04;
/// Duplicate-detection window: a parsed flight matching an existing one by craft + start_time within
/// this many ms is treated as already present and skipped.
const DEDUP_WINDOW_MS: i64 = 15_000;

/// Result of a raw-log import.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub flight_ids: Vec<i64>,
}

/// Latest-known telemetry values, accumulated across messages (mirrors the recorder's snapshot).
#[derive(Default, Clone)]
struct Snap {
    roll: Option<f64>,
    pitch: Option<f64>,
    yaw: Option<f64>,
    lat: Option<f64>,
    lon: Option<f64>,
    alt_gps: Option<f64>,
    speed: Option<f64>,
    heading: Option<f64>,
    fix_type: Option<u8>,
    num_sat: Option<u8>,
    alt_baro: Option<f64>,
    vario: Option<f64>,
    voltage: Option<f64>,
    current: Option<f64>,
    mah: Option<u32>,
    rssi: Option<u16>,
    batt_pct: Option<u8>,
    cpu_load: Option<u16>,
    flight_mode_flags: Option<u32>,
    mode_primary: Option<String>,
    mode_modifiers: Option<String>,
    active_wp: Option<i32>,
    nav_state: Option<i32>,
    hdop: Option<f64>,
    eph: Option<f64>,
    epv: Option<f64>,
}

/// Vehicle identity recovered from the log (MSP handshake frames, if captured — continuous mode — or
/// the MAVLink HEARTBEAT). Written to the imported flight's metadata.
#[derive(Default)]
struct Identity {
    craft: Option<String>,
    fc_variant: Option<String>,
    fc_version: String,
    board: String,
    platform_type: u8,
}

/// One emitted telemetry sample at an absolute time, with the armed state at that instant.
struct Sample {
    t_ms: i64, // absolute epoch milliseconds
    rec: TelemetryRecord,
    armed: bool,
}

fn is_valid_gps(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && !(lat == 0.0 && lon == 0.0)
}

fn snap_to_record(s: &Snap, t_ms: i64) -> TelemetryRecord {
    TelemetryRecord {
        id: 0,
        flight_id: 0,
        timestamp_ms: t_ms,
        lat: s.lat,
        lon: s.lon,
        alt_m: s.alt_gps,
        speed_ms: s.speed,
        heading: s.heading,
        vario_ms: s.vario,
        voltage: s.voltage,
        current_a: s.current,
        mah_drawn: s.mah,
        rssi: s.rssi,
        battery_percentage: s.batt_pct,
        roll: s.roll,
        pitch: s.pitch,
        yaw: s.yaw,
        fix_type: s.fix_type,
        num_sat: s.num_sat,
        cpu_load: s.cpu_load,
        link_quality: None,
        baro_alt_m: s.alt_baro,
        gps_hdop: s.hdop,
        gps_eph: s.eph,
        gps_epv: s.epv,
        active_wp_number: s.active_wp,
        active_flight_mode_flags: s.flight_mode_flags.map(|f| f as i64),
        state_flags: None,
        nav_state: s.nav_state,
        nav_flags: None,
        rx_signal_received: None,
        hw_health_status: None,
        baro_temperature: None,
        wind_n_ms: None,
        wind_e_ms: None,
        wind_d_ms: None,
        rc_data_json: None,
        rc_command_json: None,
        nav_lat: None,
        nav_lon: None,
        nav_alt_m: None,
        mode_primary: s.mode_primary.clone(),
        mode_modifiers: s.mode_modifiers.clone(),
    }
}

// ── MSP (.rawmsp, mwptools v2) ───────────────────────────────────────────────

/// Decode an mwptools v2 raw log. Returns (samples, craft_name, fc_variant, platform_type). `base_ms`
/// anchors the relative per-record offsets to an absolute epoch (from the filename's local time).
/// MSP raw logs don't carry the vehicle/platform type, so `platform_type` is 0 (unknown).
fn decode_rawmsp(bytes: &[u8], base_ms: i64) -> (Vec<Sample>, Identity) {
    let mut i = if bytes.starts_with(b"v2\n") { 3 } else { 0 };
    let mut parser = MspParser::new();
    let mut snap = Snap::default();
    let mut armed = false;
    let mut samples = Vec::new();
    let mut id = Identity::default();

    while i + 11 <= bytes.len() {
        let offset = f64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        let size = u16::from_le_bytes([bytes[i + 8], bytes[i + 9]]) as usize;
        let dir = bytes[i + 10];
        i += 11;
        if i + size > bytes.len() {
            break;
        }
        let payload = &bytes[i..i + size];
        i += size;
        if dir != b'i' {
            continue; // only incoming (FC→GCS) reconstructs telemetry; outgoing requests are ignored
        }
        let t_ms = base_ms + (offset * 1000.0) as i64;
        for &b in payload {
            if let Some(msg) = parser.push(b) {
                update_from_msp(msg.code, &msg.payload, &mut snap, &mut armed, &mut id);
                if msg.code == MSP_ATTITUDE || msg.code == MSP_RAW_GPS {
                    samples.push(Sample { t_ms, rec: snap_to_record(&snap, t_ms), armed });
                }
            }
        }
    }
    (samples, id)
}

fn update_from_msp(code: u16, payload: &[u8], s: &mut Snap, armed: &mut bool, id: &mut Identity) {
    // Identity frames (no telemetry decoder) — present only when the handshake is in the log
    // (continuous mode). Best-effort, for the flight's metadata + dedup.
    if code == MSP_NAME {
        if id.craft.is_none() {
            let n = String::from_utf8_lossy(payload).trim_matches('\0').trim().to_string();
            if !n.is_empty() {
                id.craft = Some(n);
            }
        }
        return;
    }
    if code == MSP_FC_VARIANT {
        if id.fc_variant.is_none() && payload.len() >= 4 {
            id.fc_variant = Some(String::from_utf8_lossy(&payload[..4]).trim().to_string());
        }
        return;
    }
    if code == MSP_FC_VERSION {
        if id.fc_version.is_empty() && payload.len() >= 3 {
            id.fc_version = format!("{}.{}.{}", payload[0], payload[1], payload[2]);
        }
        return;
    }
    if code == MSP_BOARD_INFO {
        if id.board.is_empty() && payload.len() >= 4 {
            id.board = String::from_utf8_lossy(&payload[..4]).trim_matches('\0').trim().to_string();
        }
        return;
    }
    if code == MSPV2_INAV_MIXER {
        if id.platform_type == 0 && payload.len() >= 4 {
            id.platform_type = payload[3];
        }
        return;
    }
    // Only feed codes `decode_telemetry` actually handles (its fallback returns a zeroed Attitude,
    // which would corrupt the snapshot for unrelated codes).
    match code {
        MSP_ATTITUDE | MSP_RAW_GPS | MSP_ALTITUDE | MSPV2_INAV_ANALOG | MSPV2_INAV_STATUS
        | MSP_SENSOR_STATUS | MSPV2_INAV_AIR_SPEED | MSP_GPSSTATISTICS | MSP_NAV_STATUS => {}
        _ => return,
    }
    match decode_telemetry(code, payload, &[]) {
        TelemetryPayload::Attitude(a) => {
            s.roll = Some(a.roll);
            s.pitch = Some(a.pitch);
            s.yaw = Some(a.yaw);
        }
        TelemetryPayload::Gps(g) => {
            s.lat = Some(g.lat);
            s.lon = Some(g.lon);
            s.alt_gps = Some(g.alt_msl);
            s.speed = Some(g.ground_speed);
            s.heading = Some(g.course);
            s.fix_type = Some(g.fix_type);
            s.num_sat = Some(g.num_sat);
        }
        TelemetryPayload::Altitude(al) => {
            s.alt_baro = Some(al.altitude);
            s.vario = Some(al.vario);
        }
        TelemetryPayload::Analog(an) => {
            s.voltage = Some(an.voltage);
            s.current = Some(an.current);
            s.mah = Some(an.mah_drawn);
            s.rssi = Some(an.rssi);
            s.batt_pct = if an.battery_percentage > 0 { Some(an.battery_percentage) } else { None };
        }
        TelemetryPayload::Status(st) => {
            *armed = st.arming_flags & ARMED_FLAG != 0;
            s.cpu_load = Some(st.cpu_load);
            s.flight_mode_flags = Some(st.flight_mode_flags);
            let fm = crate::flightmode::classify_inav(st.flight_mode_flags);
            s.mode_primary = Some(fm.primary);
            s.mode_modifiers = if fm.modifiers.is_empty() { None } else { Some(fm.modifiers.join(",")) };
        }
        TelemetryPayload::GpsStats(gs) => {
            s.hdop = Some(gs.hdop);
            s.eph = gs.eph;
            s.epv = gs.epv;
        }
        TelemetryPayload::NavStatus(ns) => {
            s.active_wp = Some(ns.active_wp_number as i32);
            s.nav_state = Some(ns.nav_state as i32);
        }
        _ => {}
    }
}

// ── MAVLink (.tlog) ──────────────────────────────────────────────────────────

/// Map a vehicle's autopilot + type to (fc_variant, platform_type) — mirrors the live MAVLink handshake
/// (handshake.rs). The variant string drives the Plane-vs-Copter flight-mode table, so getting it right
/// is what makes the imported flight's mode resolve (and the replay model match).
fn ardu_type_info(autopilot: MavAutopilot, t: MavType) -> (String, u8) {
    let variant = match autopilot {
        MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA => match t {
            MavType::MAV_TYPE_FIXED_WING => "ArduPlane",
            MavType::MAV_TYPE_GROUND_ROVER => "ArduRover",
            MavType::MAV_TYPE_SUBMARINE => "ArduSub",
            _ => "ArduCopter",
        }
        .to_string(),
        MavAutopilot::MAV_AUTOPILOT_PX4 => "PX4".to_string(),
        other => format!("{:?}", other),
    };
    let platform = match t {
        MavType::MAV_TYPE_FIXED_WING => 1,
        MavType::MAV_TYPE_QUADROTOR
        | MavType::MAV_TYPE_HEXAROTOR
        | MavType::MAV_TYPE_OCTOROTOR
        | MavType::MAV_TYPE_TRICOPTER => 2,
        MavType::MAV_TYPE_HELICOPTER => 3,
        MavType::MAV_TYPE_GROUND_ROVER => 10,
        MavType::MAV_TYPE_SUBMARINE => 12,
        _ => 0,
    };
    (variant, platform)
}

/// Decode a MAVLink tlog: a sequence of `[u64 BE µs][raw frame]`. Returns (samples, identity).
fn decode_tlog(bytes: &[u8]) -> (Vec<Sample>, Identity) {
    let mut parser = MavParser::new();
    let mut snap = Snap::default();
    let mut armed = false;
    let mut samples = Vec::new();
    // Vehicle identity from the (continuously streamed) HEARTBEAT — defaults until the first one.
    let mut variant = "ArduCopter".to_string();
    let mut platform: u8 = 0;
    let mut i = 0usize;

    while i + 8 <= bytes.len() {
        let ts_us = u64::from_be_bytes(bytes[i..i + 8].try_into().unwrap());
        i += 8;
        let t_ms = (ts_us / 1000) as i64;
        // Feed bytes until exactly one frame completes (records are one frame each; well-formed logs
        // pass CRC so `push` returns at the frame boundary). A guard prevents runaway on corruption.
        let mut got = false;
        let mut guard = 0;
        while i < bytes.len() && !got && guard < 600 {
            let b = bytes[i];
            i += 1;
            guard += 1;
            if let Some(frame) = parser.push(b) {
                got = true;
                // Vehicle identity (skip our own GCS heartbeat) → drives the mode table + flight model.
                if let MavMessage::HEARTBEAT(hb) = &frame.message {
                    if hb.mavtype != MavType::MAV_TYPE_GCS {
                        let (v, p) = ardu_type_info(hb.autopilot, hb.mavtype);
                        variant = v;
                        platform = p;
                    }
                }
                if update_from_mav(&frame.message, &mut snap, &mut armed, &variant) {
                    samples.push(Sample { t_ms, rec: snap_to_record(&snap, t_ms), armed });
                }
            }
        }
        if !got {
            break;
        }
    }
    let fc_variant = if variant.is_empty() { None } else { Some(variant) };
    (samples, Identity { fc_variant, platform_type: platform, ..Default::default() })
}

/// Update the snapshot from a MAVLink message; returns true if this is a high-rate message that should
/// emit a sample (ATTITUDE / GLOBAL_POSITION_INT). `variant` selects the ArduPilot mode table.
fn update_from_mav(msg: &MavMessage, s: &mut Snap, armed: &mut bool, variant: &str) -> bool {
    match msg {
        MavMessage::HEARTBEAT(hb) => {
            // Ignore our own GCS heartbeat (it would flap `armed` false) — only the vehicle's counts.
            if hb.mavtype != MavType::MAV_TYPE_GCS {
                *armed = hb.base_mode.bits() & MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED.bits() != 0;
                s.flight_mode_flags = Some(hb.custom_mode);
                let fm = crate::flightmode::classify_mavlink(hb.custom_mode, variant);
                s.mode_primary = Some(fm.primary);
                s.mode_modifiers = if fm.modifiers.is_empty() { None } else { Some(fm.modifiers.join(",")) };
            }
            false
        }
        MavMessage::ATTITUDE(a) => {
            s.roll = Some(a.roll.to_degrees() as f64);
            s.pitch = Some(a.pitch.to_degrees() as f64);
            s.yaw = Some((a.yaw.to_degrees() as f64).rem_euclid(360.0));
            true
        }
        MavMessage::GLOBAL_POSITION_INT(g) => {
            s.lat = Some(g.lat as f64 / 1e7);
            s.lon = Some(g.lon as f64 / 1e7);
            s.alt_gps = Some(g.alt as f64 / 1000.0);
            s.alt_baro = Some(g.relative_alt as f64 / 1000.0);
            if g.hdg != u16::MAX {
                s.heading = Some(g.hdg as f64 / 100.0);
            }
            true
        }
        MavMessage::GPS_RAW_INT(gps) => {
            let fix = match gps.fix_type {
                GpsFixType::GPS_FIX_TYPE_NO_GPS | GpsFixType::GPS_FIX_TYPE_NO_FIX => 0,
                GpsFixType::GPS_FIX_TYPE_2D_FIX => 1,
                GpsFixType::GPS_FIX_TYPE_3D_FIX => 2,
                GpsFixType::GPS_FIX_TYPE_DGPS
                | GpsFixType::GPS_FIX_TYPE_RTK_FLOAT
                | GpsFixType::GPS_FIX_TYPE_RTK_FIXED => 3,
                _ => 0,
            };
            s.fix_type = Some(fix);
            s.num_sat = Some(gps.satellites_visible);
            if gps.eph != u16::MAX {
                s.hdop = Some(gps.eph as f64 / 100.0);
            }
            false
        }
        MavMessage::VFR_HUD(h) => {
            s.speed = Some(h.groundspeed as f64);
            s.vario = Some(h.climb as f64);
            false
        }
        MavMessage::SYS_STATUS(sys) => {
            s.voltage = Some(sys.voltage_battery as f64 / 1000.0);
            if sys.current_battery >= 0 {
                s.current = Some(sys.current_battery as f64 / 100.0);
            }
            false
        }
        MavMessage::BATTERY_STATUS(bat) => {
            if bat.current_consumed >= 0 {
                s.mah = Some(bat.current_consumed as u32);
            }
            false
        }
        _ => false,
    }
}

// ── Splitting + import ───────────────────────────────────────────────────────

/// Split the sample stream into per-flight index ranges by arm/disarm, merging runs whose disarmed
/// gap is ≤ 5 s (grace) into one flight — the in-between (disarmed) samples are kept (grace fill).
fn split_flights(samples: &[Sample]) -> Vec<std::ops::Range<usize>> {
    // Contiguous armed runs as (first_armed_idx, last_armed_idx).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<usize> = None;
    for (idx, s) in samples.iter().enumerate() {
        if s.armed {
            if cur.is_none() {
                cur = Some(idx);
            }
        } else if let Some(start) = cur.take() {
            runs.push((start, idx.saturating_sub(1)));
        }
    }
    if let Some(start) = cur {
        runs.push((start, samples.len() - 1));
    }

    // Merge consecutive runs with a ≤ grace disarmed gap (the merged range spans the gap → fill).
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in runs {
        if let Some(last) = merged.last_mut() {
            if samples[r.0].t_ms - samples[last.1].t_ms <= GRACE_MS {
                last.1 = r.1;
                continue;
            }
        }
        merged.push(r);
    }
    merged.into_iter().map(|(a, b)| a..b + 1).collect()
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().asin()
}

/// Anchor epoch (ms) for a `.rawmsp` file, parsed from its `YYYY-MM-DD_HHMMSS` filename prefix
/// (written in local time, ADR-048) → interpreted via the local timezone. Falls back to now.
fn rawmsp_base_ms(path: &Path) -> i64 {
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if stem.len() >= 17 {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&stem[..17], "%Y-%m-%d_%H%M%S") {
            if let Some(local) = Local.from_local_datetime(&naive).single() {
                return local.with_timezone(&Utc).timestamp_millis();
            }
        }
    }
    Utc::now().timestamp_millis()
}

/// Parse a raw log file and import its flights into the DB. `emit` reports progress (0–100).
pub fn import_raw_log_with_progress<F: Fn(u8, &str, &str)>(
    conn: &Connection,
    path: &Path,
    emit: F,
) -> Result<RawImportResult, String> {
    emit(2, "read", "Reading raw log...");
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read raw log: {}", e))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();

    emit(15, "decode", "Decoding frames...");
    let (samples, id, protocol) = if ext == "tlog" {
        let (s, id) = decode_tlog(&bytes);
        (s, id, "MAVLink")
    } else {
        // .rawmsp (or unknown → try MSP v2)
        let (s, id) = decode_rawmsp(&bytes, rawmsp_base_ms(path));
        (s, id, "MSP")
    };

    if samples.is_empty() {
        return Err("No telemetry decoded from the raw log".into());
    }

    let craft_name = id.craft.clone().unwrap_or_else(|| format!("Imported ({})", protocol));
    let fc_variant = id.fc_variant.clone().unwrap_or_default();
    let file_label = path.file_name().unwrap_or_default().to_string_lossy().to_string();

    emit(45, "split", "Splitting into flights...");
    let ranges = split_flights(&samples);
    if ranges.is_empty() {
        return Err("No armed flight segments found in the raw log".into());
    }

    // Existing flights for the duplicate check (craft + start_time window).
    let existing = db::list_flights(conn).unwrap_or_default();

    let mut result = RawImportResult { imported: 0, skipped: 0, flight_ids: Vec::new() };
    let total = ranges.len();
    for (n, range) in ranges.into_iter().enumerate() {
        emit(45 + (45 * n / total.max(1)) as u8, "store", "Storing flights...");
        let seg = &samples[range];
        let first_t = seg[0].t_ms;
        let last_t = seg[seg.len() - 1].t_ms;
        let start_time = Utc.timestamp_millis_opt(first_t).single().unwrap_or_else(Utc::now);
        let end_time = Utc.timestamp_millis_opt(last_t).single();

        // Duplicate check: same craft + start within the window → skip.
        let dup = existing.iter().any(|e| {
            e.craft_name == craft_name
                && (e.start_time.timestamp_millis() - first_t).abs() <= DEDUP_WINDOW_MS
        });
        if dup {
            result.skipped += 1;
            continue;
        }

        // Stats + start coordinates.
        let mut max_alt = 0.0f64;
        let mut max_speed = 0.0f64;
        let mut max_distance = 0.0f64;
        let mut total_distance = 0.0f64;
        let mut start_lat = None;
        let mut start_lon = None;
        let mut last_ll: Option<(f64, f64)> = None;
        let mut start_mah: Option<u32> = None;
        let mut end_mah: Option<u32> = None;

        for s in seg.iter() {
            if let Some(a) = s.rec.baro_alt_m.or(s.rec.alt_m) {
                if a > max_alt {
                    max_alt = a;
                }
            }
            if let Some(v) = s.rec.speed_ms {
                if v > max_speed {
                    max_speed = v;
                }
            }
            if let Some(m) = s.rec.mah_drawn {
                if start_mah.is_none() {
                    start_mah = Some(m);
                }
                end_mah = Some(m);
            }
            if let (Some(la), Some(lo)) = (s.rec.lat, s.rec.lon) {
                if is_valid_gps(la, lo) {
                    if start_lat.is_none() {
                        start_lat = Some(la);
                        start_lon = Some(lo);
                    }
                    if let Some((pla, plo)) = last_ll {
                        total_distance += haversine_m(pla, plo, la, lo);
                    }
                    if let (Some(sla), Some(slo)) = (start_lat, start_lon) {
                        let d = haversine_m(sla, slo, la, lo);
                        if d > max_distance {
                            max_distance = d;
                        }
                    }
                    last_ll = Some((la, lo));
                }
            }
        }

        let battery_used = match (start_mah, end_mah) {
            (Some(a), Some(b)) if b >= a => Some(b - a),
            _ => None,
        };
        let utc_offset_min = match (start_lat, start_lon) {
            (Some(la), Some(lo)) => timezone::offset_min_at(la, lo, start_time),
            _ => None,
        };

        let flight = Flight {
            id: 0,
            start_time,
            end_time,
            duration_sec: Some(((last_t - first_t) / 1000).max(0)),
            source: "live".into(),
            craft_name: craft_name.clone(),
            fc_variant: fc_variant.clone(),
            fc_version: id.fc_version.clone(),
            board_id: id.board.clone(),
            platform_type: id.platform_type,
            protocol: protocol.to_string(),
            start_lat,
            start_lon,
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            max_alt_m: Some(max_alt),
            max_speed_ms: Some(max_speed),
            max_distance_m: if max_distance > 0.0 { Some(max_distance) } else { None },
            total_distance_m: if total_distance > 0.0 { Some(total_distance) } else { None },
            battery_used_mah: battery_used,
            notes: Some(format!("Parsed from {}", file_label)),
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min,
        };

        let flight_id = db::insert_flight(conn, &flight)
            .map_err(|e| format!("Failed to insert parsed flight: {}", e))?;

        // Rebase telemetry timestamps to the flight start and write them.
        let rows: Vec<TelemetryRecord> = seg
            .iter()
            .map(|s| {
                let mut r = s.rec.clone();
                r.flight_id = flight_id;
                r.timestamp_ms = s.t_ms - first_t;
                r
            })
            .collect();
        db::insert_telemetry_batch(conn, &rows)
            .map_err(|e| format!("Failed to insert parsed telemetry: {}", e))?;

        result.imported += 1;
        result.flight_ids.push(flight_id);
    }

    emit(100, "done", "Raw log import complete.");
    Ok(result)
}
