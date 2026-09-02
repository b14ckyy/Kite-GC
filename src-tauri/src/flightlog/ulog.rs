// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// PX4 ULog (.ulg) decoder
//
// Native implementation — no external binary, no Python dependency. The PX4 equivalent of
// `ardupilot.rs` (DataFlash): both formats are self-describing, so one adapter covers every
// PX4 vehicle.
//
// ULog framing (https://docs.px4.io/main/en/dev_log/ulog_file_format):
//   - 16-byte header: 7-byte magic "ULog\x01\x12\x35" + version byte + u64 µs timestamp.
//   - Then a stream of messages, each `u16 size (LE) + u8 type + payload`. There are NO
//     per-message sync bytes (unlike DataFlash's 0xA3 0x95) — framing is purely size-driven.
//   - 'F' messages register topic schemas as C-style typed field lists
//     ("uint64_t timestamp;double latitude_deg;..."); 'A' messages subscribe a topic under a
//     u16 msg_id (+ multi_id instance); 'D' messages carry packed little-endian data for one
//     msg_id. All fields are packed (no alignment padding — PX4 pads explicitly via _padding
//     fields inside the format string).
//
// Format versioning: the spec's "File Format Version History" lists v2 = flag-bits ('B') +
// multi-info ('M') messages + the ability to append data (crash dumps). Real-world PX4 logs
// (verified against v1.16) still stamp version byte 1 while already containing 'B' messages,
// so the byte is NOT a reliable feature gate. Like pyulog we therefore accept any version
// byte (warn if unexpected), always handle 'B'/'M', honour the DATA_APPENDED offsets, and
// ignore unknown message types — which is exactly what the spec prescribes for forward
// compatibility. Unknown *incompatible* flag bits are the one hard refusal.
//
// Public API:
//   decode_ulog(data)                    → (Vec<NormalizedRow>, UlogMeta)
//   import_ulog_log_with_progress(...)   → Result<BlackboxImportStatus, String>

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use super::db;
use super::types::{BatteryRecord, BlackboxImportStatus, Flight, TelemetryRecord};

// ─── ULog framing constants ──────────────────────────────────────────────────

const ULOG_MAGIC: [u8; 7] = *b"ULog\x01\x12\x35";
const HEADER_LEN: usize = 16;

const MSG_FLAG_BITS: u8 = b'B';
const MSG_FORMAT: u8 = b'F';
const MSG_INFO: u8 = b'I';
const MSG_ADD_SUB: u8 = b'A';
const MSG_DATA: u8 = b'D';
const MSG_DROPOUT: u8 = b'O';

// ─── Field types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Bool,
    Char,
}

fn base_type(name: &str) -> Option<BaseType> {
    Some(match name {
        "int8_t" => BaseType::I8,
        "uint8_t" => BaseType::U8,
        "int16_t" => BaseType::I16,
        "uint16_t" => BaseType::U16,
        "int32_t" => BaseType::I32,
        "uint32_t" => BaseType::U32,
        "int64_t" => BaseType::I64,
        "uint64_t" => BaseType::U64,
        "float" => BaseType::F32,
        "double" => BaseType::F64,
        "bool" => BaseType::Bool,
        "char" => BaseType::Char,
        _ => return None,
    })
}

fn type_width(t: BaseType) -> usize {
    match t {
        BaseType::I8 | BaseType::U8 | BaseType::Bool | BaseType::Char => 1,
        BaseType::I16 | BaseType::U16 => 2,
        BaseType::I32 | BaseType::U32 | BaseType::F32 => 4,
        BaseType::I64 | BaseType::U64 | BaseType::F64 => 8,
    }
}

/// One addressable field in a topic's packed layout.
#[derive(Debug, Clone)]
struct FieldDef {
    name: String,
    ty: BaseType,
    count: usize,
    offset: usize,
}

/// Compute the packed field layout (+ total byte size) for a topic from the registered format
/// strings. Nested message types are resolved recursively for correct sizing, but only flat
/// basic fields become addressable — none of the topics we read use nesting.
fn compute_layout(
    formats: &HashMap<String, String>,
    topic: &str,
    depth: usize,
) -> Option<(Vec<FieldDef>, usize)> {
    if depth > 4 {
        return None; // nesting runaway guard
    }
    let def = formats.get(topic)?;
    let mut fields = Vec::new();
    let mut off = 0usize;
    for f in def.split(';').filter(|s| !s.is_empty()) {
        let (tname, fname) = f.split_once(' ')?;
        let (tbase, count) = match tname.split_once('[') {
            Some((b, rest)) => (b, rest.trim_end_matches(']').parse::<usize>().ok()?),
            None => (tname, 1),
        };
        match base_type(tbase) {
            Some(ty) => {
                fields.push(FieldDef { name: fname.to_string(), ty, count, offset: off });
                off += type_width(ty) * count;
            }
            None => {
                // Nested message type — recurse for its size, skip addressability.
                let (_, size) = compute_layout(formats, tbase, depth + 1)?;
                off += size * count;
            }
        }
    }
    Some((fields, off))
}

/// Read-only view over one data payload, addressed through a topic layout.
struct FieldView<'a> {
    layout: &'a [FieldDef],
    data: &'a [u8],
}

impl<'a> FieldView<'a> {
    fn find(&self, name: &str) -> Option<&'a FieldDef> {
        self.layout.iter().find(|f| f.name == name)
    }

    /// Element `idx` of field `name` as f64 (scalars: idx 0). None when absent, out of
    /// bounds, or non-numeric. NaN passes through — callers filter with `is_finite()`.
    fn elem(&self, name: &str, idx: usize) -> Option<f64> {
        let f = self.find(name)?;
        if idx >= f.count {
            return None;
        }
        let w = type_width(f.ty);
        let start = f.offset + idx * w;
        let d = self.data.get(start..start + w)?;
        Some(match f.ty {
            BaseType::I8 => d[0] as i8 as f64,
            BaseType::U8 | BaseType::Bool => d[0] as f64,
            BaseType::I16 => i16::from_le_bytes([d[0], d[1]]) as f64,
            BaseType::U16 => u16::from_le_bytes([d[0], d[1]]) as f64,
            BaseType::I32 => i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64,
            BaseType::U32 => u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64,
            BaseType::I64 => i64::from_le_bytes(d.try_into().ok()?) as f64,
            BaseType::U64 => u64::from_le_bytes(d.try_into().ok()?) as f64,
            BaseType::F32 => f32::from_le_bytes([d[0], d[1], d[2], d[3]]) as f64,
            BaseType::F64 => f64::from_le_bytes(d.try_into().ok()?),
            BaseType::Char => return None,
        })
    }

    fn f64(&self, name: &str) -> Option<f64> {
        self.elem(name, 0)
    }

    /// Finite-only variant — the common case for physical quantities (PX4 logs NaN for
    /// "unknown" in many float fields).
    fn finite(&self, name: &str) -> Option<f64> {
        self.f64(name).filter(|v| v.is_finite())
    }

    fn u64(&self, name: &str) -> Option<u64> {
        self.f64(name).map(|v| v as u64)
    }

    fn boolean(&self, name: &str) -> Option<bool> {
        self.f64(name).map(|v| v >= 0.5)
    }
}

// ─── Topics of interest ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Topic {
    Gps,
    Attitude,
    AirData,
    LocalPos,
    Battery,
    Status,
    ActuatorArmed,
    InputRc,
    MissionResult,
    Wind,
    Airspeed,
    AirspeedValidated,
    ThrustSetpoint,
    ActuatorControls0,
}

fn topic_of(name: &str) -> Option<Topic> {
    Some(match name {
        "vehicle_gps_position" => Topic::Gps,
        "vehicle_attitude" => Topic::Attitude,
        "vehicle_air_data" => Topic::AirData,
        "vehicle_local_position" => Topic::LocalPos,
        "battery_status" => Topic::Battery,
        "vehicle_status" => Topic::Status,
        "actuator_armed" => Topic::ActuatorArmed,
        "input_rc" => Topic::InputRc,
        "mission_result" => Topic::MissionResult,
        "wind" | "wind_estimate" => Topic::Wind, // renamed wind_estimate→wind in PX4 v1.12
        "airspeed" => Topic::Airspeed,
        "airspeed_validated" => Topic::AirspeedValidated,
        "vehicle_thrust_setpoint" => Topic::ThrustSetpoint,
        "actuator_controls_0" => Topic::ActuatorControls0,
        _ => return None,
    })
}

/// One subscription (msg_id) we actually read.
struct Sub {
    topic: Topic,
    multi_id: u8,
    layout_idx: usize,
}

// ─── Normalized output row ───────────────────────────────────────────────────

/// A fully merged, per-GPS-tick telemetry snapshot (latest-known values from all other
/// topics at the moment a valid GPS fix arrived). Mirrors ardupilot::NormalizedRecord.
#[derive(Debug, Default, Clone)]
pub struct NormalizedRow {
    pub timestamp_us: u64,
    pub utc_time: Option<DateTime<Utc>>,
    pub fix_type: Option<u8>,
    pub num_sat: Option<u8>,
    pub hdop: Option<f64>,
    pub eph: Option<f64>,
    pub epv: Option<f64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub gps_alt_m: Option<f64>,
    pub speed_ms: Option<f64>,
    pub course_deg: Option<f64>,
    pub roll_deg: Option<f64>,
    pub pitch_deg: Option<f64>,
    pub yaw_deg: Option<f64>,
    pub voltage_v: Option<f64>,
    pub current_a: Option<f64>,
    pub mah_drawn: Option<f64>,
    pub battery_pct: Option<f64>,
    batteries: Vec<UlogBattery>,
    pub baro_alt_m: Option<f64>,
    pub baro_temp_c: Option<f64>,
    pub nav_alt_m: Option<f64>,
    pub vario_ms: Option<f64>,
    pub wind_n_ms: Option<f64>,
    pub wind_e_ms: Option<f64>,
    pub airspeed_ms: Option<f64>,
    /// Thrust SETPOINT magnitude in percent — PX4 logs no throttle output; documented as such.
    pub throttle_pct: Option<f64>,
    pub rc_data: Option<[u16; 4]>,
    pub link_quality: Option<u8>,
    pub active_wp_number: Option<u16>,
    pub nav_state: Option<u8>,
    pub armed: bool,
}

/// One battery monitor instance captured at a row (PX4 multi-battery).
#[derive(Debug, Default, Clone)]
struct UlogBattery {
    instance: u8,
    voltage: f64,
    current: f64,
    mah: f64,
    pct: Option<f64>,
    temp: Option<f64>,
    cells: Option<u8>,
}

/// Log-level metadata recovered from the info messages + vehicle_status.
#[derive(Debug, Default)]
pub struct UlogMeta {
    /// Decoded from `ver_sw_release` (e.g. "1.16.0"); empty when absent.
    pub fw_version: String,
    /// Git hash from `ver_sw` (informational).
    pub fw_hash: String,
    /// INAV mixer platform enum (0=multirotor, 1=airplane, 4=rover, 6=other) — display only.
    pub platform_type: u8,
    pub arm_count: usize,
    pub disarm_count: usize,
    pub dropout_count: usize,
    pub total_messages: usize,
    pub first_fix_time: Option<DateTime<Utc>>,
    pub last_fix_time: Option<DateTime<Utc>>,
}

// ─── Internal decoder state ──────────────────────────────────────────────────

#[derive(Default)]
struct DecoderState {
    att: Option<(f64, f64, f64)>, // roll, pitch, yaw (deg, DB conventions applied)
    /// Battery monitors by multi_id instance. Instance 0 = primary.
    bat_instances: BTreeMap<u8, UlogBattery>,
    baro_alt_m: Option<f64>,
    baro_temp_c: Option<f64>,
    nav_alt_m: Option<f64>,
    vario_ms: Option<f64>,
    wind_n_ms: Option<f64>,
    wind_e_ms: Option<f64>,
    airspeed_ms: Option<f64>,
    throttle_pct: Option<f64>,
    rc_data: Option<[u16; 4]>,
    link_quality: Option<u8>,
    active_wp: Option<u16>,
    nav_state: Option<u8>,
    armed: bool,
    armed_prev: Option<bool>,
    /// Once actuator_armed has been seen it owns the armed flag; vehicle_status.arming_state
    /// is only the fallback for logs missing the topic.
    seen_actuator_armed: bool,
    /// vehicle_type + is_vtol from vehicle_status.
    vehicle_type: Option<u8>,
    is_vtol: bool,
    /// UTC anchor: `time_utc_usec − timestamp` of the first GPS fix carrying real UTC.
    utc_anchor_us: Option<i64>,
}

impl DecoderState {
    fn track_armed(&mut self, armed: bool, meta: &mut UlogMeta) {
        match self.armed_prev {
            None => {
                // A log that starts while armed (PX4's default log-from-arm) counts as an arm.
                if armed {
                    meta.arm_count += 1;
                }
            }
            Some(prev) => {
                if armed && !prev {
                    meta.arm_count += 1;
                } else if !armed && prev {
                    meta.disarm_count += 1;
                }
            }
        }
        self.armed_prev = Some(armed);
        self.armed = armed;
    }
}

/// Decode `ver_sw_release`: major<<24 | minor<<16 | patch<<8 | type → "major.minor.patch".
fn decode_ver_release(v: u32) -> String {
    format!("{}.{}.{}", (v >> 24) & 0xFF, (v >> 16) & 0xFF, (v >> 8) & 0xFF)
}

/// PX4 vehicle_status.vehicle_type (0 unknown, 1 rotary wing, 2 fixed wing, 3 rover,
/// 4 airship) + is_vtol → INAV mixer platform enum (display only, drives the map symbol).
fn platform_from_vehicle_type(vt: Option<u8>, is_vtol: bool) -> u8 {
    if is_vtol {
        return 1; // shown as airplane
    }
    match vt {
        Some(1) => 0, // multirotor
        Some(2) => 1, // airplane
        Some(3) => 4, // rover
        Some(4) => 6, // airship → other
        _ => 0,
    }
}

/// PX4 quaternion `q = [w, x, y, z]` → (roll, pitch, yaw) in degrees, with the DB's sign
/// conventions applied: pitch positive = nose DOWN (INAV convention, ArduPilot import negates
/// the same way), yaw normalised to 0..360.
fn quat_to_euler_deg(w: f64, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    (
        roll.to_degrees(),
        -pitch.to_degrees(),
        yaw.to_degrees().rem_euclid(360.0),
    )
}

// ─── Main decode ─────────────────────────────────────────────────────────────

/// Decode a ULog file into merged per-GPS-tick rows + metadata.
pub fn decode_ulog(data: &[u8]) -> Result<(Vec<NormalizedRow>, UlogMeta), String> {
    if data.len() < HEADER_LEN || data[..7] != ULOG_MAGIC {
        return Err("Not a ULog file (bad magic)".into());
    }
    let version = data[7];
    if version > 1 {
        // The spec promises forward compatibility when unknown messages are ignored; the
        // version byte has not been bumped in practice (v2 features ship under byte 1).
        log::warn!("ULog header version {} (expected 1) — attempting to parse anyway", version);
    }

    let mut pos = HEADER_LEN;
    let mut formats: HashMap<String, String> = HashMap::new();
    let mut layouts: Vec<Vec<FieldDef>> = Vec::new();
    let mut layout_by_topic: HashMap<String, usize> = HashMap::new();
    let mut subs: HashMap<u16, Sub> = HashMap::new();
    let mut state = DecoderState::default();
    let mut meta = UlogMeta::default();
    let mut rows: Vec<NormalizedRow> = Vec::new();
    // DATA_APPENDED recovery points (ascending, 0 = unused).
    let mut appended: Vec<usize> = Vec::new();

    'outer: loop {
        // A message that would run past EOF (or past an appended-data boundary) is truncated:
        // resume at the next appended-data offset if one exists, else stop.
        if pos + 3 > data.len() {
            break;
        }
        let size = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        let mtype = data[pos + 2];
        let end = pos + 3 + size;
        if end > data.len() {
            log::warn!("ULog: truncated message at byte {} — stopping", pos);
            break;
        }
        // Jump over a message cut by the appended-data boundary (spec: appended data may start
        // mid-message; a version-1-style linear parse would derail there).
        while let Some(&off) = appended.first() {
            if pos >= off {
                appended.remove(0);
                continue;
            }
            if end > off {
                log::debug!("ULog: message at {} crosses appended-data offset {} — resyncing", pos, off);
                appended.remove(0);
                pos = off;
                continue 'outer;
            }
            break;
        }
        let payload = &data[pos + 3..end];
        pos = end;
        meta.total_messages += 1;

        match mtype {
            MSG_FLAG_BITS => {
                if payload.len() >= 40 {
                    let incompat = &payload[8..16];
                    if incompat[0] & !0x01 != 0 || incompat[1..].iter().any(|&b| b != 0) {
                        return Err(
                            "ULog file sets unknown incompatible flag bits — cannot parse".into()
                        );
                    }
                    if incompat[0] & 0x01 != 0 {
                        // DATA_APPENDED: up to 3 file offsets where appended data begins.
                        for i in 0..3 {
                            let off = u64::from_le_bytes(
                                payload[16 + i * 8..24 + i * 8].try_into().unwrap(),
                            ) as usize;
                            if off > 0 {
                                appended.push(off);
                            }
                        }
                        appended.sort_unstable();
                        log::info!("ULog: DATA_APPENDED set, offsets {:?}", appended);
                    }
                }
            }
            MSG_FORMAT => {
                let s = String::from_utf8_lossy(payload);
                if let Some((name, fields)) = s.split_once(':') {
                    formats.insert(name.to_string(), fields.to_string());
                }
            }
            MSG_INFO => {
                if payload.is_empty() {
                    continue;
                }
                let klen = payload[0] as usize;
                if payload.len() < 1 + klen {
                    continue;
                }
                let key = String::from_utf8_lossy(&payload[1..1 + klen]).to_string();
                let value = &payload[1 + klen..];
                if key.starts_with("char[") && key.ends_with(" ver_sw") {
                    meta.fw_hash = String::from_utf8_lossy(value).trim().to_string();
                } else if key == "uint32_t ver_sw_release" && value.len() >= 4 {
                    let v = u32::from_le_bytes(value[..4].try_into().unwrap());
                    if v > 0 {
                        meta.fw_version = decode_ver_release(v);
                    }
                }
            }
            MSG_ADD_SUB => {
                if payload.len() < 4 {
                    continue;
                }
                let multi_id = payload[0];
                let msg_id = u16::from_le_bytes([payload[1], payload[2]]);
                let name = String::from_utf8_lossy(&payload[3..]).to_string();
                let Some(topic) = topic_of(&name) else { continue };
                // Instance 0 only — except battery_status, where every monitor is read.
                if multi_id != 0 && topic != Topic::Battery {
                    continue;
                }
                let layout_idx = match layout_by_topic.get(&name) {
                    Some(&idx) => idx,
                    None => match compute_layout(&formats, &name, 0) {
                        Some((fields, _)) => {
                            layouts.push(fields);
                            let idx = layouts.len() - 1;
                            layout_by_topic.insert(name.clone(), idx);
                            idx
                        }
                        None => {
                            log::warn!("ULog: no/unresolvable format for subscribed topic '{}'", name);
                            continue;
                        }
                    },
                };
                subs.insert(msg_id, Sub { topic, multi_id, layout_idx });
            }
            MSG_DATA => {
                if payload.len() < 2 {
                    continue;
                }
                let msg_id = u16::from_le_bytes([payload[0], payload[1]]);
                let Some(sub) = subs.get(&msg_id) else { continue };
                let view = FieldView { layout: &layouts[sub.layout_idx], data: &payload[2..] };
                process_topic(sub.topic, sub.multi_id, &view, &mut state, &mut meta, &mut rows);
            }
            MSG_DROPOUT => {
                meta.dropout_count += 1;
            }
            // 'M', 'P', 'Q', 'R', 'L', 'C', 'S' and anything future: skipped by size.
            _ => {}
        }
    }

    meta.platform_type = platform_from_vehicle_type(state.vehicle_type, state.is_vtol);

    // Fallback: rows exist but no arm indicator was ever seen (neither actuator_armed nor
    // vehicle_status) — treat the whole log as armed rather than importing nothing.
    if meta.arm_count == 0 && !rows.is_empty() && state.armed_prev.is_none() {
        log::warn!("ULog: no arming information found — treating the whole log as armed");
        meta.arm_count = 1;
        for r in rows.iter_mut() {
            r.armed = true;
        }
    }

    Ok((rows, meta))
}

/// Update decoder state from one data message; GPS messages with a valid 3D fix emit a row.
fn process_topic(
    topic: Topic,
    multi_id: u8,
    view: &FieldView,
    state: &mut DecoderState,
    meta: &mut UlogMeta,
    rows: &mut Vec<NormalizedRow>,
) {
    match topic {
        Topic::Gps => {
            let timestamp_us = view.u64("timestamp").unwrap_or(0);
            // Modern (≥ v1.14) logs: double degrees + double MSL metres. Older logs: int32
            // 1e7 degrees (`lat`/`lon`) + int32 millimetres (`alt`). Field-name fallback
            // covers both — the layout is self-describing either way.
            let lat = view
                .finite("latitude_deg")
                .or_else(|| view.f64("lat").map(|v| v / 1e7));
            let lon = view
                .finite("longitude_deg")
                .or_else(|| view.f64("lon").map(|v| v / 1e7));
            let alt = view
                .finite("altitude_msl_m")
                .or_else(|| view.f64("alt").map(|v| v / 1000.0));
            let fix = view.u64("fix_type").map(|v| v as u8);
            let sats = view.u64("satellites_used").map(|v| v as u8);
            let hdop = view.finite("hdop");
            let eph = view.finite("eph");
            let epv = view.finite("epv");
            // Ground speed from the NE velocity components; fall back to the scalar field.
            let speed = match (view.finite("vel_n_m_s"), view.finite("vel_e_m_s")) {
                (Some(n), Some(e)) => Some((n * n + e * e).sqrt()),
                _ => view.finite("vel_m_s"),
            };
            let course = view.finite("cog_rad").map(|c| c.to_degrees().rem_euclid(360.0));

            let has_fix = fix.map(|f| f >= 3).unwrap_or(false);
            let has_pos = matches!((lat, lon), (Some(la), Some(lo)) if is_valid_gps_coord(la, lo));
            if !(has_fix && has_pos) {
                return;
            }

            // Absolute UTC: `time_utc_usec` per fix (0 when the GPS has no time yet). Anchor
            // boot-time → UTC on the first real value so every row gets an absolute time.
            let utc_us = view.u64("time_utc_usec").unwrap_or(0);
            const MIN_PLAUSIBLE_UTC_US: u64 = 946_684_800_000_000; // 2000-01-01
            let utc = if utc_us > MIN_PLAUSIBLE_UTC_US {
                if state.utc_anchor_us.is_none() {
                    state.utc_anchor_us = Some(utc_us as i64 - timestamp_us as i64);
                }
                DateTime::from_timestamp_micros(utc_us as i64)
            } else {
                state
                    .utc_anchor_us
                    .and_then(|a| DateTime::from_timestamp_micros(a + timestamp_us as i64))
            };
            if meta.first_fix_time.is_none() {
                meta.first_fix_time = utc;
            }
            meta.last_fix_time = utc;

            // Primary battery = instance 0, else the lowest instance present.
            let primary_bat = state
                .bat_instances
                .get(&0)
                .or_else(|| state.bat_instances.values().next());
            // Per-instance snapshot only when >1 monitor (single battery replays from the
            // denormalised primary columns, like the ArduPilot import).
            let batteries: Vec<UlogBattery> = if state.bat_instances.len() >= 2 {
                state.bat_instances.values().cloned().collect()
            } else {
                Vec::new()
            };

            rows.push(NormalizedRow {
                timestamp_us,
                utc_time: utc,
                fix_type: fix,
                num_sat: sats,
                hdop,
                eph,
                epv,
                lat,
                lon,
                gps_alt_m: alt,
                speed_ms: speed,
                course_deg: course,
                roll_deg: state.att.map(|a| a.0),
                pitch_deg: state.att.map(|a| a.1),
                yaw_deg: state.att.map(|a| a.2),
                voltage_v: primary_bat.map(|b| b.voltage),
                current_a: primary_bat.map(|b| b.current),
                mah_drawn: primary_bat.map(|b| b.mah),
                battery_pct: primary_bat.and_then(|b| b.pct),
                batteries,
                baro_alt_m: state.baro_alt_m,
                baro_temp_c: state.baro_temp_c,
                nav_alt_m: state.nav_alt_m,
                vario_ms: state.vario_ms,
                wind_n_ms: state.wind_n_ms,
                wind_e_ms: state.wind_e_ms,
                airspeed_ms: state.airspeed_ms,
                throttle_pct: state.throttle_pct,
                rc_data: state.rc_data,
                link_quality: state.link_quality,
                active_wp_number: state.active_wp,
                nav_state: state.nav_state,
                armed: state.armed,
            });
        }

        Topic::Attitude => {
            if let (Some(w), Some(x), Some(y), Some(z)) = (
                view.elem("q", 0),
                view.elem("q", 1),
                view.elem("q", 2),
                view.elem("q", 3),
            ) {
                if [w, x, y, z].iter().all(|v| v.is_finite()) {
                    state.att = Some(quat_to_euler_deg(w, x, y, z));
                }
            }
        }

        Topic::AirData => {
            if let Some(alt) = view.finite("baro_alt_meter") {
                state.baro_alt_m = Some(alt);
            }
            if let Some(t) = view.finite("ambient_temperature") {
                state.baro_temp_c = Some(t);
            }
        }

        Topic::LocalPos => {
            // NED: z down → altitude above EKF origin; vz down → climb rate.
            if view.boolean("z_valid").unwrap_or(true) {
                if let Some(z) = view.finite("z") {
                    state.nav_alt_m = Some(-z);
                }
            }
            if view.boolean("v_z_valid").unwrap_or(true) {
                if let Some(vz) = view.finite("vz") {
                    state.vario_ms = Some(-vz);
                }
            }
        }

        Topic::Battery => {
            let voltage = view
                .finite("voltage_v")
                .or_else(|| view.finite("voltage_filtered_v"));
            let Some(voltage) = voltage else { return };
            if voltage <= 0.0 {
                return; // disconnected monitor
            }
            let b = state.bat_instances.entry(multi_id).or_default();
            b.instance = multi_id;
            b.voltage = voltage;
            if let Some(c) = view
                .finite("current_a")
                .or_else(|| view.finite("current_filtered_a"))
            {
                b.current = c;
            }
            if let Some(m) = view.finite("discharged_mah") {
                b.mah = m.max(0.0);
            }
            // `remaining` is 0..1 (NaN/negative = unknown).
            b.pct = view
                .finite("remaining")
                .filter(|r| (0.0..=1.0).contains(r))
                .map(|r| r * 100.0);
            b.temp = view.finite("temperature");
            b.cells = view
                .u64("cell_count")
                .map(|c| c as u8)
                .filter(|&c| c > 0);
        }

        Topic::Status => {
            if let Some(ns) = view.u64("nav_state") {
                state.nav_state = Some(ns as u8);
            }
            if let Some(vt) = view.u64("vehicle_type") {
                state.vehicle_type = Some(vt as u8);
            }
            if let Some(v) = view.boolean("is_vtol") {
                state.is_vtol = v;
            }
            // Fallback armed source only: ARMING_STATE_ARMED == 2 across PX4 versions.
            if !state.seen_actuator_armed {
                if let Some(a) = view.u64("arming_state") {
                    state.track_armed(a == 2, meta);
                }
            }
        }

        Topic::ActuatorArmed => {
            if let Some(armed) = view.boolean("armed") {
                if !state.seen_actuator_armed {
                    // Take over from any vehicle_status-derived value cleanly.
                    state.seen_actuator_armed = true;
                    state.armed_prev = None;
                    meta.arm_count = 0;
                    meta.disarm_count = 0;
                }
                state.track_armed(armed, meta);
            }
        }

        Topic::InputRc => {
            let mut ch = [0u16; 4];
            let mut any = false;
            for (i, slot) in ch.iter_mut().enumerate() {
                if let Some(v) = view.elem("values", i) {
                    *slot = v as u16;
                    any = any || v > 0.0;
                }
            }
            if any {
                state.rc_data = Some(ch);
            }
            state.link_quality = view
                .f64("link_quality")
                .filter(|&q| (0.0..=100.0).contains(&q))
                .map(|q| q as u8);
        }

        Topic::MissionResult => {
            if let Some(seq) = view.u64("seq_current") {
                state.active_wp = Some(seq as u16);
            }
        }

        Topic::Wind => {
            if let Some(n) = view.finite("windspeed_north") {
                state.wind_n_ms = Some(n);
            }
            if let Some(e) = view.finite("windspeed_east") {
                state.wind_e_ms = Some(e);
            }
        }

        // airspeed_validated (newer) wins over the raw airspeed topic when both stream.
        Topic::AirspeedValidated => {
            let v = view
                .finite("indicated_airspeed_m_s")
                .or_else(|| view.finite("calibrated_airspeed_m_s"))
                .or_else(|| view.finite("true_airspeed_m_s"));
            if let Some(v) = v.filter(|v| *v >= 0.0) {
                state.airspeed_ms = Some(v);
            }
        }
        Topic::Airspeed => {
            if state.airspeed_ms.is_none() {
                let v = view
                    .finite("indicated_airspeed_m_s")
                    .or_else(|| view.finite("true_airspeed_m_s"));
                if let Some(v) = v.filter(|v| *v >= 0.0) {
                    state.airspeed_ms = Some(v);
                }
            }
        }

        // PX4 logs no throttle OUTPUT — the thrust SETPOINT is the closest stand-in and is
        // documented as such (user docs). Magnitude covers MC (−z) and FW (x) alike.
        Topic::ThrustSetpoint => {
            if let (Some(x), Some(y), Some(z)) = (
                view.elem("xyz", 0),
                view.elem("xyz", 1),
                view.elem("xyz", 2),
            ) {
                if [x, y, z].iter().all(|v| v.is_finite()) {
                    state.throttle_pct = Some(((x * x + y * y + z * z).sqrt() * 100.0).clamp(0.0, 100.0));
                }
            }
        }
        // Pre-1.14 fallback: actuator_controls_0.control[3] = throttle 0..1.
        Topic::ActuatorControls0 => {
            if state.throttle_pct.is_none() {
                if let Some(t) = view.elem("control", 3).filter(|v| v.is_finite()) {
                    state.throttle_pct = Some((t * 100.0).clamp(0.0, 100.0));
                }
            }
        }
    }
}

// ─── DB import pipeline ──────────────────────────────────────────────────────

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().asin()
}

#[inline]
fn is_valid_gps_coord(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// Decode a ULog file directly to DB records and import it as one flight.
///
/// The PX4 equivalent of `ardupilot::import_ardupilot_log_with_progress`: decode →
/// filter armed rows → downsample to 10 Hz → map to TelemetryRecord → insert into DB →
/// archive the original file. PX4's default is one log file per arm session, so a file
/// maps to one flight (multiple arm cycles in a log-from-boot file merge into one entry,
/// matching the ArduPilot importer's behaviour).
pub fn import_ulog_log_with_progress<F>(
    conn: &Connection,
    file_path: &Path,
    force_import: bool,
    mut report: F,
) -> Result<BlackboxImportStatus, String>
where
    F: FnMut(u8, &str, &str),
{
    report(5, "prepare", "Reading PX4 ULog file...");
    let file_data = std::fs::read(file_path)
        .map_err(|e| format!("Failed to read file '{}': {}", file_path.display(), e))?;

    report(10, "decode", "Decoding ULog...");
    let (all_rows, meta) = decode_ulog(&file_data)?;
    log::info!(
        "ULog decoded: {} messages, {} GPS rows, fw {} ({}), arms {}, dropouts {}",
        meta.total_messages,
        all_rows.len(),
        if meta.fw_version.is_empty() { "?" } else { &meta.fw_version },
        meta.fw_hash,
        meta.arm_count,
        meta.dropout_count,
    );

    if all_rows.is_empty() {
        return Err("ULog import failed: no valid GPS rows found".into());
    }

    report(40, "filter", "Filtering armed segments...");

    // ULog files carry no craft name — use the file stem (like the ArduPilot import).
    let craft_name = file_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let fc_variant = "PX4".to_string();

    // Anchor flight time at the first ARMED row (PX4 logs often begin well before arming).
    let start_time = all_rows
        .iter()
        .find(|r| r.armed && r.utc_time.is_some())
        .and_then(|r| r.utc_time)
        .or_else(|| all_rows.first().and_then(|r| r.utc_time))
        .unwrap_or_else(Utc::now);

    if !force_import {
        report(42, "check-dup", "Checking for duplicate flights...");
        if let Ok(Some(existing_flight)) = db::find_duplicate_flight(conn, &craft_name, start_time) {
            return Ok(BlackboxImportStatus::DuplicateDetected {
                existing_flight,
                duplicate_craft_name: craft_name,
                duplicate_start_time: start_time,
                duplicate_duration_sec: None,
                duplicate_lat: None,
                duplicate_lon: None,
            });
        }
    }

    report(45, "downsample", "Downsampling to 10 Hz...");

    // Downsample armed rows to at most 10 Hz. Timestamps are rebased to the FIRST ARMED row,
    // so a log that starts minutes before arming replays from t=0 at arm.
    let target_interval_us: u64 = 100_000;
    let first_armed_us = all_rows
        .iter()
        .find(|r| r.armed)
        .map(|r| r.timestamp_us)
        .ok_or_else(|| "ULog import failed: no armed GPS rows found".to_string())?;
    let mut last_kept_us: u64 = 0;
    let mut kept_count: usize = 0;

    let mut telemetry_rows: Vec<TelemetryRecord> = Vec::new();
    let mut battery_rows: Vec<BatteryRecord> = Vec::new();
    let mut start_lat: Option<f64> = None;
    let mut start_lon: Option<f64> = None;
    let mut max_alt_m: Option<f64> = None;
    let mut max_speed_ms: Option<f64> = None;
    let mut first_mah: Option<f64> = None;
    let mut last_mah: Option<f64> = None;
    let mut total_distance_m: f64 = 0.0;
    let mut max_distance_m: f64 = 0.0;
    let mut prev_lat: Option<f64> = None;
    let mut prev_lon: Option<f64> = None;

    for r in &all_rows {
        if !r.armed {
            continue;
        }
        if kept_count > 0 && r.timestamp_us.saturating_sub(last_kept_us) < target_interval_us {
            continue;
        }
        last_kept_us = r.timestamp_us;
        kept_count += 1;

        let timestamp_ms = (r.timestamp_us.saturating_sub(first_armed_us) / 1000) as i64;
        let best_alt = r.nav_alt_m.or(r.baro_alt_m).or(r.gps_alt_m);

        if let (Some(lat), Some(lon)) = (r.lat, r.lon) {
            if is_valid_gps_coord(lat, lon) {
                if start_lat.is_none() {
                    start_lat = Some(lat);
                    start_lon = Some(lon);
                }
                if let (Some(plat), Some(plon)) = (prev_lat, prev_lon) {
                    total_distance_m += haversine_m(plat, plon, lat, lon);
                }
                if let (Some(slat), Some(slon)) = (start_lat, start_lon) {
                    let dist = haversine_m(slat, slon, lat, lon);
                    if dist > max_distance_m {
                        max_distance_m = dist;
                    }
                }
                prev_lat = Some(lat);
                prev_lon = Some(lon);
            }
        }
        if let Some(alt) = best_alt {
            max_alt_m = Some(max_alt_m.map_or(alt, |c: f64| c.max(alt)));
        }
        if let Some(spd) = r.speed_ms {
            max_speed_ms = Some(max_speed_ms.map_or(spd, |c: f64| c.max(spd)));
        }
        // `discharged_mah` counts since BOOT — used-in-flight = last − first armed value.
        if let Some(mah) = r.mah_drawn {
            if first_mah.is_none() {
                first_mah = Some(mah);
            }
            last_mah = Some(mah);
        }

        let rc_data_json = r
            .rc_data
            .map(|[a, b, c, d]| format!("[{},{},{},{}]", a, b, c, d));

        let (mode_primary, mode_modifiers) = match r.nav_state {
            Some(ns) => (
                Some(crate::flightmode::classify_px4_nav_state(ns).primary),
                None,
            ),
            None => (None, None),
        };

        telemetry_rows.push(TelemetryRecord {
            id: 0,
            flight_id: 0, // set after insert_flight
            timestamp_ms,
            lat: r.lat,
            lon: r.lon,
            alt_m: r.gps_alt_m,
            speed_ms: r.speed_ms,
            heading: r.course_deg,
            vario_ms: r.vario_ms,
            voltage: r.voltage_v,
            current_a: r.current_a,
            mah_drawn: r.mah_drawn.map(|v| v.max(0.0) as u32),
            rssi: None,
            battery_percentage: r.battery_pct.map(|p| p.clamp(0.0, 100.0) as u8),
            roll: r.roll_deg,
            pitch: r.pitch_deg,
            yaw: r.yaw_deg,
            fix_type: r.fix_type,
            num_sat: r.num_sat,
            cpu_load: None,
            link_quality: r.link_quality,
            baro_alt_m: r.baro_alt_m,
            gps_hdop: r.hdop,
            gps_eph: r.eph,
            gps_epv: r.epv,
            active_wp_number: r.active_wp_number.map(|v| v as i32),
            // Forensic raw value = the logged NAVIGATION_STATE (also mirrored in nav_state).
            active_flight_mode_flags: r.nav_state.map(|v| v as i64),
            state_flags: None,
            nav_state: r.nav_state.map(|v| v as i32),
            nav_flags: None,
            rx_signal_received: None,
            hw_health_status: None,
            baro_temperature: r.baro_temp_c,
            wind_n_ms: r.wind_n_ms,
            wind_e_ms: r.wind_e_ms,
            wind_d_ms: None,
            rc_data_json,
            rc_command_json: None,
            nav_lat: None,
            nav_lon: None,
            nav_alt_m: r.nav_alt_m,
            mode_primary,
            mode_modifiers,
            link_snr: None,
            link_rssi_dbm: None,
            airspeed_ms: r.airspeed_ms,
            throttle_pct: r.throttle_pct,
        });

        for b in &r.batteries {
            battery_rows.push(BatteryRecord {
                id: 0,
                flight_id: 0,
                timestamp_ms,
                instance: b.instance,
                voltage: Some(b.voltage),
                current_a: Some(b.current),
                mah_drawn: Some(b.mah.max(0.0) as u32),
                battery_percentage: b.pct.map(|p| p.clamp(0.0, 100.0) as u8),
                cell_count: b.cells.or_else(|| {
                    if b.voltage > 0.5 {
                        Some(((b.voltage / 3.7).round() as u8).max(1))
                    } else {
                        None
                    }
                }),
                temperature: b.temp,
            });
        }
    }

    if telemetry_rows.is_empty() {
        return Err("ULog import failed: no armed GPS rows found".into());
    }

    let last_timestamp_ms = telemetry_rows.last().map(|r| r.timestamp_ms).unwrap_or(0);
    let duration_sec = Some(last_timestamp_ms / 1000);
    let end_time = start_time + Duration::milliseconds(last_timestamp_ms);
    let battery_used = match (first_mah, last_mah) {
        (Some(a), Some(b)) if b >= a => Some((b - a) as u32),
        _ => None,
    };

    report(70, "store-flight", "Creating logbook entry...");

    let flight = Flight {
        id: 0,
        start_time,
        end_time: Some(end_time),
        duration_sec,
        source: "blackbox".into(),
        craft_name: craft_name.clone(),
        fc_variant,
        fc_version: meta.fw_version.clone(),
        board_id: String::new(),
        platform_type: meta.platform_type,
        protocol: "ULOG".into(),
        start_lat,
        start_lon,
        location_name: None,
        weather_temp_c: None,
        weather_wind_ms: None,
        weather_wind_deg: None,
        weather_desc: None,
        max_alt_m,
        max_speed_ms,
        max_distance_m: if max_distance_m > 0.0 { Some(max_distance_m) } else { None },
        total_distance_m: if total_distance_m > 0.0 { Some(total_distance_m) } else { None },
        battery_used_mah: battery_used,
        notes: Some(format!("Imported from {}", file_path.display())),
        linked_flight_id: None,
        pilot_name: None,
        pilot_id: None,
        battery_serial: None,
        // `start_time` is true UTC (GPS `time_utc_usec`); resolve the flight-local offset
        // from the start coordinates for display (ADR-048).
        utc_offset_min: match (start_lat, start_lon) {
            (Some(la), Some(lo)) => super::timezone::offset_min_at(la, lo, start_time),
            _ => None,
        },
    };

    let flight_id = db::insert_flight(conn, &flight)
        .map_err(|e| format!("Failed to create ULog flight: {}", e))?;

    for row in &mut telemetry_rows {
        row.flight_id = flight_id;
    }
    let rows_imported = telemetry_rows.len();

    report(82, "store-track", "Storing track data...");
    db::insert_telemetry_batch(conn, &telemetry_rows)
        .map_err(|e| format!("Failed to store ULog telemetry: {}", e))?;

    if !battery_rows.is_empty() {
        for row in &mut battery_rows {
            row.flight_id = flight_id;
        }
        db::insert_battery_records_batch(conn, &battery_rows)
            .map_err(|e| format!("Failed to store ULog battery records: {}", e))?;
    }

    report(92, "archive", "Archiving original ULog file...");
    db::insert_blackbox_file(
        conn,
        flight_id,
        &file_path.file_name().unwrap_or_default().to_string_lossy(),
        0,
        &file_data,
    )
    .map_err(|e| format!("Failed to archive ULog file: {}", e))?;

    report(100, "done", "PX4 ULog import complete.");
    Ok(BlackboxImportStatus::Success { flight_id, rows_imported })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Synthetic ULog builder ──────────────────────────────────────────────

    fn header() -> Vec<u8> {
        let mut out = ULOG_MAGIC.to_vec();
        out.push(1); // version
        out.extend_from_slice(&1_000_000u64.to_le_bytes());
        out
    }

    fn msg(out: &mut Vec<u8>, mtype: u8, payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        out.push(mtype);
        out.extend_from_slice(payload);
    }

    fn format_msg(out: &mut Vec<u8>, def: &str) {
        msg(out, MSG_FORMAT, def.as_bytes());
    }

    fn subscribe(out: &mut Vec<u8>, multi_id: u8, msg_id: u16, name: &str) {
        let mut p = vec![multi_id];
        p.extend_from_slice(&msg_id.to_le_bytes());
        p.extend_from_slice(name.as_bytes());
        msg(out, MSG_ADD_SUB, &p);
    }

    fn data_msg(out: &mut Vec<u8>, msg_id: u16, fields: &[u8]) {
        let mut p = msg_id.to_le_bytes().to_vec();
        p.extend_from_slice(fields);
        msg(out, MSG_DATA, &p);
    }

    struct P(Vec<u8>);
    impl P {
        fn new() -> Self {
            P(Vec::new())
        }
        fn u64(mut self, v: u64) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f64(mut self, v: f64) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f32(mut self, v: f32) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn i32(mut self, v: i32) -> Self {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn u8(mut self, v: u8) -> Self {
            self.0.push(v);
            self
        }
    }

    const GPS_FMT: &str = "vehicle_gps_position:uint64_t timestamp;double latitude_deg;double longitude_deg;double altitude_msl_m;uint64_t time_utc_usec;float vel_n_m_s;float vel_e_m_s;float cog_rad;uint8_t fix_type;uint8_t satellites_used;";
    const ATT_FMT: &str = "vehicle_attitude:uint64_t timestamp;float[4] q;";
    const ARM_FMT: &str = "actuator_armed:uint64_t timestamp;bool armed;";
    const BAT_FMT: &str = "battery_status:uint64_t timestamp;float voltage_v;float current_a;float discharged_mah;float remaining;";
    const STATUS_FMT: &str = "vehicle_status:uint64_t timestamp;uint8_t nav_state;uint8_t vehicle_type;bool is_vtol;";

    fn gps_payload(ts_us: u64, lat: f64, lon: f64, alt: f64, utc_us: u64) -> Vec<u8> {
        P::new()
            .u64(ts_us)
            .f64(lat)
            .f64(lon)
            .f64(alt)
            .u64(utc_us)
            .f32(3.0) // vel_n
            .f32(4.0) // vel_e
            .f32(0.5) // cog_rad
            .u8(4) // fix
            .u8(18) // sats
            .0
    }

    /// End-to-end synthetic log: arm → attitude → battery → status → 2 GPS fixes.
    #[test]
    fn decodes_a_synthetic_log() {
        let mut log = header();
        for f in [GPS_FMT, ATT_FMT, ARM_FMT, BAT_FMT, STATUS_FMT] {
            format_msg(&mut log, f);
        }
        subscribe(&mut log, 0, 1, "vehicle_gps_position");
        subscribe(&mut log, 0, 2, "vehicle_attitude");
        subscribe(&mut log, 0, 3, "actuator_armed");
        subscribe(&mut log, 0, 4, "battery_status");
        subscribe(&mut log, 0, 5, "vehicle_status");

        data_msg(&mut log, 3, &P::new().u64(1_000_000).u8(1).0); // armed
        // 90° yaw: q = [cos(45°), 0, 0, sin(45°)]
        let s = std::f32::consts::FRAC_1_SQRT_2;
        data_msg(&mut log, 2, &P::new().u64(1_000_000).f32(s).f32(0.0).f32(0.0).f32(s).0);
        data_msg(&mut log, 4, &P::new().u64(1_000_000).f32(16.8).f32(5.5).f32(100.0).f32(0.9).0);
        data_msg(&mut log, 5, &P::new().u64(1_000_000).u8(3).u8(1).u8(0).0); // AUTO_MISSION, rotary
        let utc0: u64 = 1_754_500_000_000_000;
        data_msg(&mut log, 1, &gps_payload(2_000_000, 52.25, 6.85, 27.5, utc0));
        data_msg(&mut log, 1, &gps_payload(3_000_000, 52.26, 6.86, 30.0, 0)); // no UTC → anchor

        let (rows, meta) = decode_ulog(&log).expect("decode");
        assert_eq!(rows.len(), 2);
        assert_eq!(meta.arm_count, 1, "log starting armed counts as one arm");
        assert_eq!(meta.platform_type, 0, "rotary wing → multirotor");

        let r = &rows[0];
        assert!(r.armed);
        assert!((r.lat.unwrap() - 52.25).abs() < 1e-9);
        assert!((r.lon.unwrap() - 6.85).abs() < 1e-9);
        assert!((r.gps_alt_m.unwrap() - 27.5).abs() < 1e-9);
        assert!((r.speed_ms.unwrap() - 5.0).abs() < 1e-3, "speed from NE velocity");
        assert!((r.yaw_deg.unwrap() - 90.0).abs() < 0.1, "quaternion yaw");
        assert!(r.pitch_deg.unwrap().abs() < 0.1);
        assert!((r.voltage_v.unwrap() - 16.8).abs() < 1e-3);
        assert!((r.battery_pct.unwrap() - 90.0).abs() < 0.01);
        assert_eq!(r.nav_state, Some(3));
        assert_eq!(r.utc_time.unwrap().timestamp_micros() as u64, utc0);

        // Second fix has no GPS UTC → derived from the anchor (+1 s of boot time).
        let r2 = &rows[1];
        assert_eq!(r2.utc_time.unwrap().timestamp_micros() as u64, utc0 + 1_000_000);
    }

    /// Pre-v1.14 logs name the position fields `lat`/`lon`/`alt` as int32 1e7-degrees / mm.
    #[test]
    fn old_gps_field_names_fall_back() {
        let mut log = header();
        format_msg(
            &mut log,
            "vehicle_gps_position:uint64_t timestamp;int32_t lat;int32_t lon;int32_t alt;uint64_t time_utc_usec;float vel_m_s;uint8_t fix_type;uint8_t satellites_used;",
        );
        format_msg(&mut log, ARM_FMT);
        subscribe(&mut log, 0, 1, "vehicle_gps_position");
        subscribe(&mut log, 0, 2, "actuator_armed");
        data_msg(&mut log, 2, &P::new().u64(500_000).u8(1).0);
        data_msg(
            &mut log,
            1,
            &P::new()
                .u64(1_000_000)
                .i32(522_510_640) // 52.251064°
                .i32(68_526_330) // 6.852633°
                .i32(27_700) // 27.7 m
                .u64(1_754_500_000_000_000)
                .f32(7.5)
                .u8(3)
                .u8(12)
                .0,
        );

        let (rows, _) = decode_ulog(&log).expect("decode");
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!((r.lat.unwrap() - 52.251064).abs() < 1e-6);
        assert!((r.lon.unwrap() - 6.852633).abs() < 1e-6);
        assert!((r.gps_alt_m.unwrap() - 27.7).abs() < 1e-3);
        assert!((r.speed_ms.unwrap() - 7.5).abs() < 1e-3, "scalar vel_m_s fallback");
    }

    /// Rows before arming are emitted but flagged disarmed; fixes without 3D fix are dropped.
    #[test]
    fn armed_flag_and_fix_gating() {
        let mut log = header();
        format_msg(&mut log, GPS_FMT);
        format_msg(&mut log, ARM_FMT);
        subscribe(&mut log, 0, 1, "vehicle_gps_position");
        subscribe(&mut log, 0, 2, "actuator_armed");

        data_msg(&mut log, 2, &P::new().u64(500_000).u8(0).0); // disarmed
        data_msg(&mut log, 1, &gps_payload(1_000_000, 52.0, 6.0, 10.0, 0));
        // fix_type 1 → dropped
        let mut nofix = gps_payload(1_500_000, 52.0, 6.0, 10.0, 0);
        let fix_off = nofix.len() - 2;
        nofix[fix_off] = 1;
        data_msg(&mut log, 1, &nofix);
        data_msg(&mut log, 2, &P::new().u64(2_000_000).u8(1).0); // arm
        data_msg(&mut log, 1, &gps_payload(3_000_000, 52.1, 6.1, 20.0, 0));

        let (rows, meta) = decode_ulog(&log).expect("decode");
        assert_eq!(rows.len(), 2, "no-fix row dropped");
        assert!(!rows[0].armed);
        assert!(rows[1].armed);
        assert_eq!(meta.arm_count, 1);
        assert_eq!(meta.disarm_count, 0);
    }

    /// Unknown message types are skipped by size; unknown incompatible flag bits refuse.
    #[test]
    fn unknown_messages_skip_and_incompat_flags_refuse() {
        let mut log = header();
        msg(&mut log, b'Z', &[0xAA; 17]); // unknown type — must be skipped cleanly
        format_msg(&mut log, GPS_FMT);
        format_msg(&mut log, ARM_FMT);
        subscribe(&mut log, 0, 1, "vehicle_gps_position");
        subscribe(&mut log, 0, 2, "actuator_armed");
        data_msg(&mut log, 2, &P::new().u64(500_000).u8(1).0);
        data_msg(&mut log, 1, &gps_payload(1_000_000, 52.0, 6.0, 10.0, 0));
        let (rows, _) = decode_ulog(&log).expect("decode");
        assert_eq!(rows.len(), 1);

        let mut bad = header();
        let mut flags = [0u8; 40];
        flags[8] = 0x02; // unknown incompatible bit
        msg(&mut bad, MSG_FLAG_BITS, &flags);
        assert!(decode_ulog(&bad).is_err());
    }

    #[test]
    fn ver_release_decodes() {
        assert_eq!(decode_ver_release(0x0110_0040), "1.16.0");
        assert_eq!(decode_ver_release(0x010E_0200), "1.14.2");
    }

    /// Optional smoke test against a real PX4 log: set KITE_ULOG_SAMPLE to a .ulg path.
    /// `cargo test -p kite-gc ulog -- --ignored` (skips silently when the env var is unset).
    #[test]
    #[ignore]
    fn decodes_a_real_log_when_provided() {
        let Ok(path) = std::env::var("KITE_ULOG_SAMPLE") else {
            eprintln!("KITE_ULOG_SAMPLE not set — skipping");
            return;
        };
        let data = std::fs::read(&path).expect("read sample");
        let (rows, meta) = decode_ulog(&data).expect("decode");
        eprintln!(
            "rows={} fw={} hash={} platform={} arms={}/{} msgs={} first={:?} last={:?}",
            rows.len(),
            meta.fw_version,
            meta.fw_hash,
            meta.platform_type,
            meta.arm_count,
            meta.disarm_count,
            meta.total_messages,
            meta.first_fix_time,
            meta.last_fix_time,
        );
        assert!(!rows.is_empty());
        assert!(meta.arm_count >= 1);
        let armed: Vec<_> = rows.iter().filter(|r| r.armed).collect();
        assert!(!armed.is_empty(), "expected armed rows");
        let with_att = armed.iter().filter(|r| r.yaw_deg.is_some()).count();
        assert!(with_att > 0, "expected attitude-merged rows");
    }
}
