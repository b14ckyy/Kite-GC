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
    wind_n: Option<f64>,
    wind_e: Option<f64>,
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
        airspeed_ms: None, // MSP2_INAV_AIR_SPEED not yet parsed in the raw-import path
        throttle_pct: None, // throttle not yet parsed in the raw-import path
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
        wind_n_ms: s.wind_n,
        wind_e_ms: s.wind_e,
        wind_d_ms: None, // no column is fed from WIND.speed_z live either
        rc_data_json: None,
        rc_command_json: None,
        nav_lat: None,
        nav_lon: None,
        nav_alt_m: None,
        mode_primary: s.mode_primary.clone(),
        mode_modifiers: s.mode_modifiers.clone(),
        link_snr: None,
        link_rssi_dbm: None,
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

/// Decode a MAVLink tlog: a sequence of `[u64 BE µs][raw frame]`. Returns (samples, identity).
fn decode_tlog(bytes: &[u8]) -> (Vec<Sample>, Identity) {
    let mut parser = MavParser::new();
    let mut snap = Snap::default();
    let mut armed = false;
    let mut samples = Vec::new();
    // Vehicle identity from the (continuously streamed) HEARTBEAT — defaults until the first one.
    let mut variant = "ArduCopter".to_string();
    let mut platform: u8 = 0;
    // The autopilot's (system, component) id, locked onto the first HEARTBEAT that identifies one.
    let mut fc_id: Option<(u8, u8)> = None;
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
                // A tlog carries every component on the link, not just the autopilot. Peripherals that
                // share the vehicle's system id — a camera on component 100, a gimbal on 154 — publish
                // their own HEARTBEATs with their own `custom_mode`, so consuming all of them makes the
                // replayed flight mode flap between senders once a second, and lets a peripheral's
                // `autopilot` field overwrite `variant`, which selects the mode table. Lock onto the
                // autopilot the way the live handshake does and ignore heartbeats from anything else.
                // Other message types stay unfiltered, matching the live handler.
                let mut consume = true;
                if let MavMessage::HEARTBEAT(hb) = &frame.message {
                    let id = (frame.header.system_id, frame.header.component_id);
                    let is_autopilot = hb.mavtype != MavType::MAV_TYPE_GCS
                        && (hb.autopilot != MavAutopilot::MAV_AUTOPILOT_INVALID || id.1 == 1);
                    if fc_id.is_none() && is_autopilot {
                        fc_id = Some(id);
                    }
                    consume = fc_id == Some(id);
                    if consume {
                        // Vehicle identity → drives the mode table + flight model.
                        let (v, p) =
                            crate::mavlink_proto::vehicle::identify(hb.autopilot, hb.mavtype);
                        variant = v;
                        platform = p;
                    }
                }
                if consume && update_from_mav(&frame.message, &mut snap, &mut armed, &variant) {
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
            // The caller already restricts heartbeats to the locked autopilot component; this keeps the
            // GCS guard as a second line (our own heartbeat would flap `armed` false).
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
            // `heading` is the course-over-ground column (see `TelemetryRecord`); the FC's own heading
            // is stored separately as `yaw` from ATTITUDE. `g.hdg` is the vehicle HEADING, not the
            // course — writing it here made a replayed flight report a course identical to its heading,
            // so the compass showed no crab angle while the map model, rotated by `yaw`, correctly drew
            // one. Derive the course from the fused velocity like the live handler does; below walking
            // pace that is just atan2 of velocity noise, so hold the previous value.
            let ground_speed = ((g.vx as f64).powi(2) + (g.vy as f64).powi(2)).sqrt() / 100.0;
            if ground_speed > 0.5 {
                s.heading = Some((g.vy as f64).atan2(g.vx as f64).to_degrees().rem_euclid(360.0));
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
        MavMessage::WIND(w) => {
            // ArduPilot's EKF wind estimate: `direction` is the bearing the wind blows FROM. Store the
            // vector it blows TOWARD as north/east components, matching what the live recorder writes
            // (`Recorder::on_wind`), so an imported flight drives the compass exactly like a recorded
            // one. `speed_z` has no column, same as live.
            let toward = (w.direction as f64 + 180.0).to_radians();
            s.wind_n = Some(w.speed as f64 * toward.cos());
            s.wind_e = Some(w.speed as f64 * toward.sin());
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

#[cfg(test)]
mod tests {
    use super::*;

    use ::mavlink::ardupilotmega::{
        MavState, ATTITUDE_DATA, GLOBAL_POSITION_INT_DATA, HEARTBEAT_DATA, WIND_DATA,
    };
    use ::mavlink::MavHeader;

    use crate::mavlink_proto::codec::{serialize_v2, MavSequence};

    fn heartbeat(mavtype: MavType, autopilot: MavAutopilot, custom_mode: u32) -> MavMessage {
        MavMessage::HEARTBEAT(HEARTBEAT_DATA {
            custom_mode,
            mavtype,
            autopilot,
            base_mode: MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED,
            system_status: MavState::MAV_STATE_ACTIVE,
            mavlink_version: 3,
        })
    }

    /// Wrap frames in the tlog container: `[u64 BE µs][frame]`, one second apart.
    fn tlog(frames: Vec<(u8, u8, MavMessage)>) -> Vec<u8> {
        let mut seq = MavSequence::new();
        let mut out = Vec::new();
        for (n, (system_id, component_id, msg)) in frames.into_iter().enumerate() {
            out.extend_from_slice(&((n as u64 + 1) * 1_000_000).to_be_bytes());
            let header = MavHeader { system_id, component_id, sequence: 0 };
            out.extend_from_slice(&serialize_v2(&header, &msg, &mut seq));
        }
        out
    }

    /// A SIYI-style link publishes HEARTBEATs from a camera and a gimbal under the vehicle's own
    /// system id. Only the autopilot's may drive the flight mode and the vehicle identity — otherwise
    /// the replayed mode flaps between all three senders once a second, and the peripherals'
    /// `MAV_AUTOPILOT_INVALID` overwrites the variant that selects the mode table.
    #[test]
    fn peripheral_heartbeats_do_not_drive_mode_or_identity() {
        let log = tlog(vec![
            // ArduPlane in GUIDED (plane custom_mode 15).
            (1, 1, heartbeat(MavType::MAV_TYPE_FIXED_WING, MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA, 15)),
            // Gimbal: custom_mode 0, which a copter table would read as "stabilize".
            (1, 154, heartbeat(MavType::MAV_TYPE_GIMBAL, MavAutopilot::MAV_AUTOPILOT_INVALID, 0)),
            // Camera: custom_mode 65535, which matches no mode at all.
            (1, 100, heartbeat(MavType::MAV_TYPE_CAMERA, MavAutopilot::MAV_AUTOPILOT_INVALID, 65535)),
            // Our own GCS heartbeat, echoed back into the log by the link.
            (255, 190, heartbeat(MavType::MAV_TYPE_GCS, MavAutopilot::MAV_AUTOPILOT_INVALID, 0)),
            // ATTITUDE emits the sample that carries the mode.
            (1, 1, MavMessage::ATTITUDE(ATTITUDE_DATA::default())),
        ]);

        let (samples, identity) = decode_tlog(&log);

        assert_eq!(identity.fc_variant.as_deref(), Some("ArduPlane"));
        assert_eq!(identity.platform_type, 1);
        let sample = samples.last().expect("ATTITUDE should emit a sample");
        assert_eq!(sample.rec.mode_primary.as_deref(), Some("guided"));
        assert!(sample.armed, "the autopilot heartbeat is armed");
    }

    /// The `heading` column is course over ground, not the vehicle heading — `GLOBAL_POSITION_INT.hdg`
    /// is the latter, so deriving the course from the fused velocity is what keeps a replayed crab
    /// angle honest. WIND rides along here because both are read off the same message stream.
    #[test]
    fn course_comes_from_velocity_and_wind_is_captured() {
        // Heading due east, but travelling due north: a 90° crab that `hdg` alone would hide.
        let gpi = MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
            time_boot_ms: 0,
            lat: 42_700_000,
            lon: 22_880_000,
            alt: 200_000,
            relative_alt: 100_000,
            vx: 1500, // cm/s north
            vy: 0,
            vz: 0,
            hdg: 9000, // 90.00° heading
        });
        let wind = MavMessage::WIND(WIND_DATA { direction: 60.0, speed: 6.0, speed_z: 0.0 });

        let log = tlog(vec![
            (1, 1, heartbeat(MavType::MAV_TYPE_FIXED_WING, MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA, 15)),
            (1, 1, wind),
            (1, 1, gpi),
        ]);

        let (samples, _) = decode_tlog(&log);
        let rec = &samples.last().expect("GLOBAL_POSITION_INT should emit a sample").rec;

        let course = rec.heading.expect("course recorded");
        assert!(course < 0.5 || course > 359.5, "course should follow the velocity (0°), got {course}");

        // Wind blows FROM 60°, so the vector points TOWARD 240°: north −3.0, east −5.196.
        let (n, e) = (rec.wind_n_ms.expect("wind north"), rec.wind_e_ms.expect("wind east"));
        assert!((n - -3.0).abs() < 0.01, "wind north {n}");
        assert!((e - -5.196).abs() < 0.01, "wind east {e}");
    }
}
