// ArduPilot DataFlash .bin decoder
//
// Native implementation — no external binary, no Python dependency.
//
// The DataFlash binary format is self-describing:
//   - FMT records (type=128) register message schemas at runtime.
//   - All other records are decoded against those registered schemas.
//
// Public API:
//   probe_message_types(data)           → Vec<FmtDef>       (inventory pass)
//   decode_to_normalized_csv(data, path) → Result<DecodeStats, String>
//   import_ardupilot_log_with_progress(...) → Result<BlackboxImportStatus, String>

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use super::db;
use super::types::{BlackboxImportStatus, Flight, TelemetryRecord};
use rusqlite::Connection;

// ─── DataFlash framing constants ─────────────────────────────────────────────

const HDR0: u8 = 0xA3;
const HDR1: u8 = 0x95;
/// FMT messages are always type 128 (0x80) and exactly 89 bytes long.
const FMT_TYPE_ID: u8 = 128;
const FMT_TOTAL_LEN: usize = 89;
// FMT data after the 3-byte header = 86 bytes:
//   type_id(1) + length(1) + name(4) + format(16) + labels(64)
const FMT_DATA_LEN: usize = 86;

// ─── Format character → byte width ───────────────────────────────────────────

fn fmt_char_width(c: u8) -> Option<usize> {
    match c as char {
        'b' | 'B' | 'M' => Some(1),
        'h' | 'H' | 'c' | 'C' => Some(2),
        'i' | 'I' | 'e' | 'E' | 'L' | 'f' => Some(4),
        'd' | 'q' | 'Q' => Some(8),
        'n' => Some(4),
        'N' => Some(16),
        'Z' => Some(64),
        'a' => Some(64), // i16[32] RC array
        _ => None,
    }
}

/// Expected total byte width of a format string (data only, no header).
fn fmt_string_width(format: &str) -> usize {
    format
        .bytes()
        .take_while(|&c| c != 0 && c != b'-')
        .filter_map(fmt_char_width)
        .sum()
}

// ─── FMT definition ───────────────────────────────────────────────────────────

/// Registered schema for one DataFlash message type.
#[derive(Debug, Clone)]
pub struct FmtDef {
    pub type_id: u8,
    /// Total record length in bytes including the 3-byte header.
    pub record_length: u8,
    pub name: String,
    /// Format characters (e.g. "QBHIBcLLefffff").
    pub format: String,
    /// Column names aligned with format characters.
    pub columns: Vec<String>,
}

impl FmtDef {
    /// Byte count of the data section (after the 3-byte header).
    pub fn data_len(&self) -> usize {
        (self.record_length as usize).saturating_sub(3)
    }
}

// ─── Decoded field value ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum DFValue {
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
}

impl DFValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            DFValue::Int(v) => Some(*v as f64),
            DFValue::UInt(v) => Some(*v as f64),
            DFValue::Float(v) => Some(*v),
            DFValue::Str(_) => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            DFValue::Int(v) => Some(*v as u64),
            DFValue::UInt(v) => Some(*v),
            DFValue::Float(v) => Some(*v as u64),
            DFValue::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let DFValue::Str(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
}

// ─── Parsed message ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParsedMsg {
    pub type_id: u8,
    pub type_name: String,
    pub fields: Vec<(String, DFValue)>,
}

impl ParsedMsg {
    pub fn get(&self, name: &str) -> Option<&DFValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name)?.as_f64()
    }

    pub fn get_u64(&self, name: &str) -> Option<u64> {
        self.get(name)?.as_u64()
    }

    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name)?.as_str()
    }
}

fn extract_armed_from_stat(msg: &ParsedMsg) -> Option<bool> {
    // Common ArduPilot STAT field names across versions.
    for key in ["Armed", "ARM", "IsArmed", "ArmedFlag", "ArmedFlg"] {
        if let Some(v) = msg.get_f64(key) {
            return Some(v >= 0.5);
        }
    }

    // Fallback: scan any field that looks like an arm flag.
    for (name, value) in &msg.fields {
        let lname = name.to_ascii_lowercase();
        if lname.contains("armed") || lname == "arm" {
            if let Some(v) = value.as_f64() {
                return Some(v >= 0.5);
            }
        }
    }

    None
}

// ─── Low-level binary scanner ─────────────────────────────────────────────────

/// Iterates DataFlash binary records, self-registering FMT schemas on the fly.
pub struct DataFlashScanner<'a> {
    data: &'a [u8],
    pos: usize,
    pub type_registry: HashMap<u8, FmtDef>,
}

impl<'a> DataFlashScanner<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            type_registry: HashMap::new(),
        }
    }

    /// Advance until the next complete message is found and return it.
    /// Returns `None` at EOF or when no more data is available.
    pub fn next_message(&mut self) -> Option<ParsedMsg> {
        loop {
            // Need at least 3 bytes for a header.
            if self.pos + 3 > self.data.len() {
                return None;
            }

            // Scan for 0xA3 0x95 sync bytes.
            if self.data[self.pos] != HDR0 || self.data[self.pos + 1] != HDR1 {
                self.pos += 1;
                continue;
            }

            let type_id = self.data[self.pos + 2];

            // ── FMT record (type 128, fixed 89-byte length) ──────────────────
            if type_id == FMT_TYPE_ID {
                let end = self.pos + FMT_TOTAL_LEN;
                if end > self.data.len() {
                    return None; // truncated file
                }
                let record_data = &self.data[self.pos + 3..end];
                self.pos = end;

                if let Some(fmt) = Self::parse_fmt_record(record_data) {
                    // Sanity-check: does the format string fit the declared length?
                    let declared_data = (fmt.record_length as usize).saturating_sub(3);
                    let computed = fmt_string_width(&fmt.format);
                    if computed > declared_data {
                        // Malformed FMT entry — skip it.
                        continue;
                    }

                    let msg = ParsedMsg {
                        type_id: FMT_TYPE_ID,
                        type_name: "FMT".to_string(),
                        fields: vec![
                            ("TypeID".to_string(), DFValue::UInt(fmt.type_id as u64)),
                            ("Name".to_string(), DFValue::Str(fmt.name.clone())),
                            ("Format".to_string(), DFValue::Str(fmt.format.clone())),
                        ],
                    };
                    self.type_registry.insert(fmt.type_id, fmt);
                    return Some(msg);
                }
                continue;
            }

            // ── Known type from registry ─────────────────────────────────────
            if let Some(fmt_def) = self.type_registry.get(&type_id).cloned() {
                let total_len = fmt_def.record_length as usize;
                let end = self.pos + total_len;

                if end > self.data.len() {
                    // Truncated record at end of file — stop.
                    return None;
                }

                let record_data = &self.data[self.pos + 3..end];
                self.pos = end;

                if let Some(msg) = Self::decode_record(&fmt_def, record_data) {
                    return Some(msg);
                }
                continue;
            }

            // Unknown type — advance one byte and resync.
            self.pos += 1;
        }
    }

    fn parse_fmt_record(data: &[u8]) -> Option<FmtDef> {
        // data layout (86 bytes):
        //   [0]     type_id
        //   [1]     record_length (total including 3-byte header)
        //   [2..6]  name (4 bytes, null-padded ASCII)
        //   [6..22] format (16 bytes)
        //   [22..86] labels (64 bytes, comma-separated)
        if data.len() < FMT_DATA_LEN {
            return None;
        }

        let type_id = data[0];
        let length = data[1];
        let name = bytes_to_str(&data[2..6]);
        let format = bytes_to_str(&data[6..22]);
        let labels_raw = bytes_to_str(&data[22..86]);

        // Reject obviously malformed entries.
        if name.is_empty() || length < 3 {
            return None;
        }

        let columns: Vec<String> = labels_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Some(FmtDef {
            type_id,
            record_length: length,
            name,
            format,
            columns,
        })
    }

    fn decode_record(fmt: &FmtDef, data: &[u8]) -> Option<ParsedMsg> {
        let mut fields: Vec<(String, DFValue)> = Vec::new();
        let mut offset = 0usize;

        for (col_idx, fmt_byte) in fmt.format.bytes().enumerate() {
            if fmt_byte == 0 || fmt_byte == b'-' {
                break; // null terminator or explicit padding
            }

            let width = match fmt_char_width(fmt_byte) {
                Some(w) => w,
                None => break, // unknown char — stop here
            };

            if offset + width > data.len() {
                break; // data shorter than expected
            }

            let col_name = fmt
                .columns
                .get(col_idx)
                .cloned()
                .unwrap_or_else(|| format!("col{}", col_idx));

            let value = decode_field_value(fmt_byte, &data[offset..offset + width]);
            fields.push((col_name, value));
            offset += width;
        }

        Some(ParsedMsg {
            type_id: fmt.type_id,
            type_name: fmt.name.clone(),
            fields,
        })
    }
}

fn decode_field_value(fmt_char: u8, data: &[u8]) -> DFValue {
    macro_rules! le_i16 { () => { i16::from_le_bytes([data[0], data[1]]) } }
    macro_rules! le_u16 { () => { u16::from_le_bytes([data[0], data[1]]) } }
    macro_rules! le_i32 { () => { i32::from_le_bytes([data[0], data[1], data[2], data[3]]) } }
    macro_rules! le_u32 { () => { u32::from_le_bytes([data[0], data[1], data[2], data[3]]) } }
    macro_rules! le_f32 { () => { f32::from_le_bytes([data[0], data[1], data[2], data[3]]) as f64 } }
    macro_rules! le_i64 { () => { i64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]) } }
    macro_rules! le_u64 { () => { u64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]) } }
    macro_rules! le_f64 { () => { f64::from_le_bytes([data[0],data[1],data[2],data[3],data[4],data[5],data[6],data[7]]) } }

    match fmt_char as char {
        'b'       => DFValue::Int(data[0] as i8 as i64),
        'B' | 'M' => DFValue::UInt(data[0] as u64),
        'h'       => DFValue::Int(le_i16!() as i64),
        'H'       => DFValue::UInt(le_u16!() as u64),
        'c'       => DFValue::Float(le_i16!() as f64 / 100.0),
        'C'       => DFValue::Float(le_u16!() as f64 / 100.0),
        'i'       => DFValue::Int(le_i32!() as i64),
        'I'       => DFValue::UInt(le_u32!() as u64),
        'e'       => DFValue::Float(le_i32!() as f64 / 100.0),
        'E'       => DFValue::Float(le_u32!() as f64 / 100.0),
        'L'       => DFValue::Float(le_i32!() as f64 / 1e7),
        'f'       => DFValue::Float(le_f32!()),
        'd'       => DFValue::Float(le_f64!()),
        'q'       => DFValue::Int(le_i64!()),
        'Q'       => DFValue::UInt(le_u64!()),
        'n'       => DFValue::Str(bytes_to_str(data)),
        'N'       => DFValue::Str(bytes_to_str(data)),
        'Z'       => DFValue::Str(bytes_to_str(data)),
        'a'       => DFValue::Str(String::new()), // skip i16 arrays
        _         => DFValue::Str(String::new()),
    }
}

fn bytes_to_str(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).trim().to_string()
}

// ─── Inventory pass (probe only) ──────────────────────────────────────────────

/// Scan a DataFlash file and return all registered FMT definitions.
/// Useful for inspecting which message types are present before importing.
pub fn probe_message_types(file_data: &[u8]) -> Vec<FmtDef> {
    let mut scanner = DataFlashScanner::new(file_data);
    while let Some(msg) = scanner.next_message() {
        // We only care that FMT records are processed (auto-registered).
        // Stop once all FMTs have been seen (first non-FMT after at least one FMT).
        if msg.type_name != "FMT" && !scanner.type_registry.is_empty() {
            // Keep going until we reach a message type that wasn't registered
            // after seeing plenty of FMTs — but DataFlash places all FMTs at
            // the start, so finish quickly in practice.
        }
    }
    let mut defs: Vec<FmtDef> = scanner.type_registry.into_values().collect();
    defs.sort_by_key(|d| d.type_id);
    defs
}

// ─── Normalized output record ─────────────────────────────────────────────────

/// A fully merged, per-GPS-tick telemetry snapshot.
/// All optional fields carry the latest-known value from their respective
/// message types at the time a GPS record arrived.
#[derive(Debug, Default, Clone)]
pub struct NormalizedRecord {
    // ── Timing ──────────────────────────────────────────────────────────────
    /// TimeUS (µs since boot) from the GPS record that triggered this row.
    pub timestamp_us: u64,
    /// Absolute UTC computed from GPS week/millisecond.
    pub utc_time: Option<DateTime<Utc>>,

    // ── GPS ─────────────────────────────────────────────────────────────────
    pub fix_type: Option<u8>,
    pub num_sat: Option<u8>,
    pub hdop: Option<f64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// GPS altitude (metres above MSL).
    pub gps_alt_m: Option<f64>,
    pub speed_ms: Option<f64>,
    pub ground_course_deg: Option<f64>,
    /// GPS vertical speed (m/s, negative = descending).
    pub gps_vz_ms: Option<f64>,

    // ── Attitude ────────────────────────────────────────────────────────────
    pub roll_deg: Option<f64>,
    pub pitch_deg: Option<f64>,
    pub yaw_deg: Option<f64>,

    // ── Battery ─────────────────────────────────────────────────────────────
    pub voltage_v: Option<f64>,
    pub current_a: Option<f64>,
    pub mah_drawn: Option<f64>,

    // ── Barometer ───────────────────────────────────────────────────────────
    pub baro_alt_m: Option<f64>,
    pub baro_climb_rate_ms: Option<f64>,
    pub baro_temp_c: Option<f64>,

    // ── EKF / Navigation altitude ────────────────────────────────────────────
    /// EKF-fused altitude relative to origin (m, positive = up).
    /// Sourced from POS.RelOriginAlt or -XKF1.PD.
    pub nav_alt_m: Option<f64>,

    // ── Wind estimate ────────────────────────────────────────────────────────
    pub wind_n_ms: Option<f64>,
    pub wind_e_ms: Option<f64>,

    // ── RC input (channels 1-4 only, µs PWM) ─────────────────────────────────
    pub rc_data: Option<[u16; 4]>,

    // ── Mission context ──────────────────────────────────────────────────────
    pub active_wp_number: Option<u16>,

    // ── Flight mode ──────────────────────────────────────────────────────────
    pub custom_mode: Option<u8>,

    // ── Arm state ────────────────────────────────────────────────────────────
    pub armed: bool,

    // ── Metadata (set once from MSG/VER) ─────────────────────────────────────
    pub vehicle_type: Option<String>,
    pub fw_version: Option<String>,
}

// ─── Decode statistics ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DecodeStats {
    pub total_records: usize,
    pub gps_rows: usize,
    pub vehicle_type: Option<String>,
    pub fw_version: Option<String>,
    pub first_fix_time: Option<DateTime<Utc>>,
    pub last_fix_time: Option<DateTime<Utc>>,
    pub arm_count: usize,
    pub disarm_count: usize,
    /// Count of parsed messages per message type name.
    pub message_type_counts: HashMap<String, usize>,
}

// ─── Internal decoder state ───────────────────────────────────────────────────

#[derive(Default)]
struct DecoderState {
    att: Option<AttState>,
    bat: Option<BatState>,
    baro: Option<BaroState>,
    nav_alt_m: Option<f64>,
    wind_n_ms: Option<f64>,
    wind_e_ms: Option<f64>,
    rc_channels: Option<[u16; 4]>,
    active_wp: Option<u16>,
    mode: Option<u8>,
    armed: bool,
    /// Set when we see any raw disarm indicator (ArmState=0, EV Id=11, STAT
    /// Armed=false), even if `state.armed` was already false.  Used to detect
    /// log-on-arm: the log began while armed and only contains the disarm.
    saw_raw_disarm: bool,
    /// TimeUS of the first detected disarm event (used for log-on-arm backfill).
    first_disarm_us: Option<u64>,
    vehicle_type: Option<String>,
    fw_version: Option<String>,
    /// GPS week of the most recent GPS time fix.
    gps_week: Option<u16>,
    /// GPS milliseconds-in-week of the most recent GPS time fix.
    gps_ms: Option<u32>,
    /// TimeUS at which gps_week/gps_ms were last updated.
    gps_ref_time_us: Option<u64>,
    /// TimeUS of the very first valid GPS position (used as origin for timestamp_ms).
    boot_ref_us: Option<u64>,
}

#[derive(Default, Clone)]
struct AttState {
    roll: f64,
    pitch: f64,
    yaw: f64,
}

#[derive(Default, Clone)]
struct BatState {
    voltage: f64,
    current: f64,
    mah: f64,
}

#[derive(Default, Clone)]
struct BaroState {
    alt_m: f64,
    climb_rate_ms: f64,
    temp_c: f64,
}

// ─── GPS week → UTC ───────────────────────────────────────────────────────────

/// Convert GPS week + milliseconds-in-week to UTC.
/// Leap seconds: 18 (valid since 2017-01-01).
fn gps_week_ms_to_utc(gps_week: u16, gps_ms: u32) -> Option<DateTime<Utc>> {
    // GPS epoch: 1980-01-06 00:00:00 UTC → Unix seconds 315964800
    const GPS_EPOCH_UNIX_SECS: i64 = 315964800;
    const LEAP_SECONDS: i64 = 18;

    let total_ms: i64 = gps_week as i64 * 7 * 24 * 3600 * 1000 + gps_ms as i64;
    let unix_ms: i64 = GPS_EPOCH_UNIX_SECS * 1000 + total_ms - LEAP_SECONDS * 1000;

    let secs = unix_ms / 1000;
    let nsecs = ((unix_ms % 1000).abs() * 1_000_000) as u32;

    DateTime::from_timestamp(secs, nsecs)
}

// ─── Main decode function ─────────────────────────────────────────────────────

/// Decode a DataFlash `.bin` file and write a normalized CSV to `out_path`.
///
/// The CSV has one row per valid GPS fix tick with all other sensor data
/// merged in as the latest-known value at that instant.
pub fn decode_to_normalized_csv(
    file_data: &[u8],
    out_path: &Path,
) -> Result<DecodeStats, String> {
    let mut scanner = DataFlashScanner::new(file_data);
    let mut state = DecoderState::default();
    let mut rows: Vec<NormalizedRecord> = Vec::new();
    let mut stats = DecodeStats::default();

    while let Some(msg) = scanner.next_message() {
        stats.total_records += 1;
        *stats
            .message_type_counts
            .entry(msg.type_name.clone())
            .or_insert(0) += 1;

        process_message(&msg, &mut state, &mut stats, &mut rows);
    }

    stats.vehicle_type = state.vehicle_type.clone();
    stats.fw_version = state.fw_version.clone();

    // ── Log-on-arm detection ──────────────────────────────────────────────
    // ArduPilot can be configured to start logging only when armed.  In
    // that case the arm event itself never appears in the log — only the
    // disarm at the end.  We detect this by checking:
    //   1. No arm event was ever counted, AND
    //   2. We saw a raw disarm indicator (ArmState=0, EV Id=11, or
    //      STAT Armed=false) — even if state.armed was already false.
    // When detected, backfill all rows before the first disarm as armed.
    //
    // Fallback: if there are GPS rows but no arm AND no disarm indicators
    // at all, the log was likely recorded entirely while armed (log-on-arm
    // without a clean disarm recorded).  Backfill everything.
    if stats.arm_count == 0 && !rows.is_empty() {
        if state.saw_raw_disarm || stats.disarm_count == 0 {
            stats.arm_count = 1;
            if state.saw_raw_disarm && stats.disarm_count == 0 {
                stats.disarm_count = 1;
            }
            let disarm_time = state.first_disarm_us.unwrap_or(u64::MAX);
            for r in rows.iter_mut() {
                if r.timestamp_us <= disarm_time {
                    r.armed = true;
                }
            }
        }
    }

    write_normalized_csv(&rows, out_path)?;

    Ok(stats)
}

// ─── DB import pipeline ───────────────────────────────────────────────────────

/// Haversine distance in metres between two GPS coordinates.
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

/// Decode a DataFlash file directly to DB records and import them.
///
/// This is the ArduPilot equivalent of `blackbox::import_blackbox_log_with_progress`.
/// It decodes the binary → NormalizedRecords → filters armed segments →
/// downsamples to 10 Hz → maps to TelemetryRecord → inserts into DB.
pub fn import_ardupilot_log_with_progress<F>(
    conn: &Connection,
    file_path: &Path,
    force_import: bool,
    mut report: F,
) -> Result<BlackboxImportStatus, String>
where
    F: FnMut(u8, &str, &str),
{
    report(5, "prepare", "Reading ArduPilot DataFlash file...");
    let file_data = std::fs::read(file_path)
        .map_err(|e| format!("Failed to read file '{}': {}", file_path.display(), e))?;

    report(10, "decode", "Decoding DataFlash binary...");

    // ── Full decode pass ─────────────────────────────────────────────
    let mut scanner = DataFlashScanner::new(&file_data);
    let mut state = DecoderState::default();
    let mut all_rows: Vec<NormalizedRecord> = Vec::new();
    let mut stats = DecodeStats::default();

    while let Some(msg) = scanner.next_message() {
        stats.total_records += 1;
        *stats
            .message_type_counts
            .entry(msg.type_name.clone())
            .or_insert(0) += 1;

        process_message(&msg, &mut state, &mut stats, &mut all_rows);
    }

    stats.vehicle_type = state.vehicle_type.clone();
    stats.fw_version = state.fw_version.clone();

    // ── Log-on-arm backfill ──────────────────────────────────────────
    if stats.arm_count == 0 && !all_rows.is_empty() {
        if state.saw_raw_disarm || stats.disarm_count == 0 {
            stats.arm_count = 1;
            if state.saw_raw_disarm && stats.disarm_count == 0 {
                stats.disarm_count = 1;
            }
            let disarm_time = state.first_disarm_us.unwrap_or(u64::MAX);
            for r in all_rows.iter_mut() {
                if r.timestamp_us <= disarm_time {
                    r.armed = true;
                }
            }
        }
    }

    if all_rows.is_empty() {
        return Err("ArduPilot import failed: no valid GPS rows found".into());
    }

    report(40, "filter", "Filtering armed segments...");

    // Determine craft name (ArduPilot logs have no craft_name field)
    let craft_name = file_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let fc_variant = state.vehicle_type.clone().unwrap_or_else(|| "ArduPilot".into());
    let fc_version = state.fw_version.clone().unwrap_or_default();

    // Use UTC time from GPS for start_time
    let start_time = all_rows
        .iter()
        .find(|r| r.armed && r.utc_time.is_some())
        .and_then(|r| r.utc_time)
        .or_else(|| all_rows.first().and_then(|r| r.utc_time))
        .unwrap_or_else(Utc::now);

    // Early duplicate check
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

    // ── Downsample armed rows to 10 Hz ───────────────────────────────
    let target_interval_us: u64 = 100_000; // 10 Hz
    let first_us = all_rows.first().map(|r| r.timestamp_us).unwrap_or(0);
    let mut last_kept_us: u64 = 0;
    let mut kept_count: usize = 0;

    let mut telemetry_rows: Vec<TelemetryRecord> = Vec::new();
    let mut start_lat: Option<f64> = None;
    let mut start_lon: Option<f64> = None;
    let mut max_alt_m: Option<f64> = None;
    let mut max_speed_ms: Option<f64> = None;
    let mut max_mah: Option<u32> = None;
    let mut total_distance_m: f64 = 0.0;
    let mut max_distance_m: f64 = 0.0;
    let mut prev_lat: Option<f64> = None;
    let mut prev_lon: Option<f64> = None;

    for r in &all_rows {
        // Only import armed rows
        if !r.armed {
            continue;
        }

        // Downsample: keep first armed row, then one per 100ms
        if kept_count > 0 && r.timestamp_us.saturating_sub(last_kept_us) < target_interval_us {
            continue;
        }
        last_kept_us = r.timestamp_us;
        kept_count += 1;

        // timestamp_ms relative to first row
        let timestamp_ms = ((r.timestamp_us - first_us) / 1000) as i64;

        // Best altitude: nav_alt > baro > GPS
        let best_alt = r.nav_alt_m.or(r.baro_alt_m).or(r.gps_alt_m);

        // Accumulate flight stats
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
        if let Some(mah) = r.mah_drawn {
            let mah_u32 = mah as u32;
            max_mah = Some(max_mah.map_or(mah_u32, |c: u32| c.max(mah_u32)));
        }

        // RC data as JSON array [ch1,ch2,ch3,ch4]
        let rc_data_json = r.rc_data.map(|[a, b, c, d]| {
            format!("[{},{},{},{}]", a, b, c, d)
        });

        telemetry_rows.push(TelemetryRecord {
            id: 0,
            flight_id: 0, // set after insert_flight
            timestamp_ms,
            lat: r.lat,
            lon: r.lon,
            alt_m: r.gps_alt_m,
            speed_ms: r.speed_ms,
            heading: r.ground_course_deg.map(|v| v as i16),
            // Prefer baro climb rate (positive = up, correct sign).
            // Fallback: GPS VZ is NED (positive = down) → negate.
            vario_ms: r.baro_climb_rate_ms.or(r.gps_vz_ms.map(|v| -v)),
            voltage: r.voltage_v,
            current_a: r.current_a,
            mah_drawn: r.mah_drawn.map(|v| v as u32),
            rssi: None,
            battery_percentage: None, // not present in ArduPilot blackbox logs
            roll: r.roll_deg,
            pitch: r.pitch_deg,
            yaw: r.yaw_deg.map(|v| v as i16),
            fix_type: r.fix_type,
            num_sat: r.num_sat,
            cpu_load: None,
            link_quality: None,
            baro_alt_m: r.baro_alt_m,
            gps_hdop: r.hdop,
            gps_eph: None,
            gps_epv: None,
            active_wp_number: r.active_wp_number.map(|v| v as i32),
            active_flight_mode_flags: r.custom_mode.map(|v| v as i64),
            state_flags: None,
            nav_state: None,
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
        });
    }

    if telemetry_rows.is_empty() {
        return Err("ArduPilot import failed: no armed GPS rows found".into());
    }

    let last_timestamp_ms = telemetry_rows.last().map(|r| r.timestamp_ms).unwrap_or(0);
    let duration_sec = Some(last_timestamp_ms / 1000);
    let end_time = start_time + Duration::milliseconds(last_timestamp_ms);

    report(70, "store-flight", "Creating logbook entry...");

    let flight = Flight {
        id: 0,
        start_time,
        end_time: Some(end_time),
        duration_sec,
        source: "blackbox".into(),
        craft_name: craft_name.clone(),
        fc_variant,
        fc_version,
        board_id: String::new(),
        platform_type: 0,
        protocol: "DATAFLASH".into(),
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
        battery_used_mah: max_mah,
        notes: Some(format!("Imported from {}", file_path.display())),
        linked_flight_id: None,
        pilot_name: None,
        pilot_id: None,
    };

    let flight_id = db::insert_flight(conn, &flight)
        .map_err(|e| format!("Failed to create ArduPilot flight: {}", e))?;

    for row in &mut telemetry_rows {
        row.flight_id = flight_id;
    }

    let rows_imported = telemetry_rows.len();

    report(82, "store-track", "Storing track data...");
    db::insert_telemetry_batch(conn, &telemetry_rows)
        .map_err(|e| format!("Failed to store ArduPilot telemetry: {}", e))?;

    report(92, "archive", "Archiving original DataFlash file...");
    db::insert_blackbox_file(
        conn,
        flight_id,
        &file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        0,
        &file_data,
    )
    .map_err(|e| format!("Failed to archive DataFlash file: {}", e))?;

    report(100, "done", "ArduPilot import complete.");
    Ok(BlackboxImportStatus::Success {
        flight_id,
        rows_imported,
    })
}

/// Process a single parsed message, updating state and optionally emitting GPS rows.
/// Extracted from `decode_to_normalized_csv` so both CSV and DB paths share the same logic.
fn process_message(
    msg: &ParsedMsg,
    state: &mut DecoderState,
    stats: &mut DecodeStats,
    rows: &mut Vec<NormalizedRecord>,
) {
    match msg.type_name.as_str() {
        // ── GPS ──────────────────────────────────────────────────────────
        "GPS" | "GPS2" => {
            let time_us = msg.get_u64("TimeUS").unwrap_or(0);
            let fix = msg.get_f64("Status").map(|v| v as u8);
            let lat = msg.get_f64("Lat");
            let lon = msg.get_f64("Lng").or_else(|| msg.get_f64("Lon"));
            let alt = msg.get_f64("Alt");
            let spd = msg.get_f64("Spd");
            let gcrs = msg.get_f64("GCrs");
            let vz = msg.get_f64("VZ");
            let nsats = msg.get_f64("NSats").map(|v| v as u8);
            let hdop = msg.get_f64("HDop");

            if let Some(gwk) = msg.get_f64("GWk").map(|v| v as u16) {
                if let Some(gms) = msg.get_f64("GMS").map(|v| v as u32) {
                    if gwk > 0 {
                        state.gps_week = Some(gwk);
                        state.gps_ms = Some(gms);
                        state.gps_ref_time_us = Some(time_us);
                    }
                }
            }

            let has_3d_fix = fix.map(|f| f >= 3).unwrap_or(false);
            let has_position = lat.map(|l| l.abs() > 1e-6).unwrap_or(false)
                && lon.map(|l| l.abs() > 1e-6).unwrap_or(false);

            if has_3d_fix && has_position {
                let utc = match (state.gps_week, state.gps_ms, state.gps_ref_time_us) {
                    (Some(gwk), Some(gms), Some(ref_us)) => {
                        let delta_us = time_us.saturating_sub(ref_us) as i64;
                        gps_week_ms_to_utc(gwk, gms).map(|base| {
                            base + chrono::Duration::microseconds(delta_us)
                        })
                    }
                    _ => None,
                };

                if stats.first_fix_time.is_none() {
                    stats.first_fix_time = utc;
                    state.boot_ref_us = Some(time_us);
                }
                stats.last_fix_time = utc;
                stats.gps_rows += 1;

                rows.push(NormalizedRecord {
                    timestamp_us: time_us,
                    utc_time: utc,
                    fix_type: fix,
                    num_sat: nsats,
                    hdop,
                    lat,
                    lon,
                    gps_alt_m: alt,
                    speed_ms: spd,
                    ground_course_deg: gcrs,
                    gps_vz_ms: vz,
                    roll_deg: state.att.as_ref().map(|a| a.roll),
                    // ArduPilot: positive = nose up (aviation std).
                    // DB/INAV convention: positive = nose down → negate.
                    pitch_deg: state.att.as_ref().map(|a| -a.pitch),
                    yaw_deg: state.att.as_ref().map(|a| a.yaw),
                    voltage_v: state.bat.as_ref().map(|b| b.voltage),
                    current_a: state.bat.as_ref().map(|b| b.current),
                    mah_drawn: state.bat.as_ref().map(|b| b.mah),
                    baro_alt_m: state.baro.as_ref().map(|b| b.alt_m),
                    baro_climb_rate_ms: state.baro.as_ref().map(|b| b.climb_rate_ms),
                    baro_temp_c: state.baro.as_ref().map(|b| b.temp_c),
                    nav_alt_m: state.nav_alt_m,
                    wind_n_ms: state.wind_n_ms,
                    wind_e_ms: state.wind_e_ms,
                    rc_data: state.rc_channels,
                    active_wp_number: state.active_wp,
                    custom_mode: state.mode,
                    armed: state.armed,
                    vehicle_type: state.vehicle_type.clone(),
                    fw_version: state.fw_version.clone(),
                });
            }
        }

        "ATT" => {
            if let (Some(r), Some(p), Some(y)) = (
                msg.get_f64("Roll"),
                msg.get_f64("Pitch"),
                msg.get_f64("Yaw"),
            ) {
                state.att = Some(AttState { roll: r, pitch: p, yaw: y });
            }
        }

        "BAT" | "CURR" => {
            let volt = msg.get_f64("Volt");
            let curr = msg.get_f64("Curr");
            let mah = msg.get_f64("CurrTot");
            if let Some(v) = volt {
                let bat = state.bat.get_or_insert_with(BatState::default);
                bat.voltage = v;
                if let Some(c) = curr { bat.current = c; }
                if let Some(m) = mah { bat.mah = m; }
            }
        }

        "BARO" => {
            if let Some(alt) = msg.get_f64("Alt") {
                let baro = state.baro.get_or_insert_with(BaroState::default);
                baro.alt_m = alt;
                if let Some(crt) = msg.get_f64("CRt") { baro.climb_rate_ms = crt; }
                if let Some(temp) = msg.get_f64("Temp") { baro.temp_c = temp; }
            }
        }

        "CTUN" => {
            if state.baro.is_none() {
                let alt = msg.get_f64("BAlt").or_else(|| msg.get_f64("Alt"));
                if let Some(a) = alt {
                    let baro = state.baro.get_or_insert_with(BaroState::default);
                    baro.alt_m = a;
                    if let Some(crt) = msg.get_f64("CRt") { baro.climb_rate_ms = crt; }
                }
            }
        }

        "MODE" => {
            let mode_val = msg
                .get_f64("Mode")
                .or_else(|| msg.get_f64("ModeNum"))
                .map(|v| v as u8);
            if let Some(m) = mode_val { state.mode = Some(m); }
        }

        "EV" => {
            if let Some(id) = msg.get_f64("Id").map(|v| v as u8) {
                match id {
                    10 => {
                        if !state.armed { stats.arm_count += 1; }
                        state.armed = true;
                    }
                    11 => {
                        if state.armed { stats.disarm_count += 1; }
                        state.saw_raw_disarm = true;
                        if state.first_disarm_us.is_none() {
                            state.first_disarm_us = Some(msg.get_u64("TimeUS").unwrap_or(0));
                        }
                        state.armed = false;
                    }
                    _ => {}
                }
            }
        }

        "ARM" => {
            if let Some(arm_val) = msg.get_f64("ArmState").map(|v| v as u8) {
                let armed = arm_val >= 1;
                if armed != state.armed {
                    if armed { stats.arm_count += 1; } else { stats.disarm_count += 1; }
                }
                if !armed {
                    state.saw_raw_disarm = true;
                    if state.first_disarm_us.is_none() {
                        state.first_disarm_us = Some(msg.get_u64("TimeUS").unwrap_or(0));
                    }
                }
                state.armed = armed;
            }
        }

        "STAT" => {
            if let Some(armed) = extract_armed_from_stat(msg) {
                if armed != state.armed {
                    if armed { stats.arm_count += 1; } else { stats.disarm_count += 1; }
                }
                if !armed {
                    state.saw_raw_disarm = true;
                    if state.first_disarm_us.is_none() {
                        state.first_disarm_us = Some(msg.get_u64("TimeUS").unwrap_or(0));
                    }
                }
                state.armed = armed;
            }
        }

        "MSG" => {
            if let Some(text) = msg.get_str("Message") {
                let vehicle_prefixes = [
                    "ArduCopter", "ArduPlane", "ArduRover",
                    "ArduSub", "ArduBlimp", "Copter", "Plane",
                ];
                for prefix in &vehicle_prefixes {
                    if text.starts_with(prefix) {
                        state.vehicle_type = Some(prefix.to_string());
                        if let Some(v_pos) = text.find(" V") {
                            let after = &text[v_pos + 2..];
                            let ver = after.split_whitespace().next().unwrap_or("");
                            if !ver.is_empty() {
                                state.fw_version = Some(ver.to_string());
                            }
                        }
                        break;
                    }
                }
            }
        }

        "VER" => {
            if let Some(fws) = msg.get_str("FWS") {
                if !fws.is_empty() { state.fw_version = Some(fws.to_string()); }
            }
        }

        "POS" => {
            if let Some(alt) = msg.get_f64("RelOriginAlt") {
                state.nav_alt_m = Some(alt);
            }
        }

        "XKF1" | "NKF1" => {
            if state.nav_alt_m.is_none() {
                if let Some(pd) = msg.get_f64("PD") {
                    state.nav_alt_m = Some(-pd);
                }
            }
        }

        "XKF2" | "NKF2" => {
            if let Some(vwn) = msg.get_f64("VWN") { state.wind_n_ms = Some(vwn); }
            if let Some(vwe) = msg.get_f64("VWE") { state.wind_e_ms = Some(vwe); }
        }

        "RCIN" => {
            let c1 = msg.get_f64("C1").map(|v| v as u16).unwrap_or(0);
            let c2 = msg.get_f64("C2").map(|v| v as u16).unwrap_or(0);
            let c3 = msg.get_f64("C3").map(|v| v as u16).unwrap_or(0);
            let c4 = msg.get_f64("C4").map(|v| v as u16).unwrap_or(0);
            state.rc_channels = Some([c1, c2, c3, c4]);
        }

        "MISE" | "CMD" => {
            if let Some(cnum) = msg.get_f64("CNum").map(|v| v as u16) {
                state.active_wp = Some(cnum);
            }
        }

        _ => {}
    }
}

// ─── CSV writer ───────────────────────────────────────────────────────────────

fn write_normalized_csv(rows: &[NormalizedRecord], out_path: &Path) -> Result<(), String> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(out_path)
        .map_err(|e| format!("Failed to create CSV '{}': {}", out_path.display(), e))?;
    let mut w = BufWriter::new(file);

    // Header row
    writeln!(
        w,
        "timestamp_us,utc_time,\
         fix_type,num_sat,hdop,\
         lat,lon,gps_alt_m,baro_alt_m,nav_alt_m,\
         speed_ms,ground_course_deg,gps_vz_ms,baro_climb_rate_ms,\
         roll_deg,pitch_deg,yaw_deg,\
         voltage_v,current_a,mah_drawn,baro_temp_c,\
         wind_n_ms,wind_e_ms,\
         rc1,rc2,rc3,rc4,\
         active_wp_number,\
         custom_mode,armed,vehicle_type,fw_version"
    )
    .map_err(|e| e.to_string())?;

    for r in rows {
        let (rc1, rc2, rc3, rc4) = match r.rc_data {
            Some([a, b, c, d]) => (
                a.to_string(), b.to_string(), c.to_string(), d.to_string()
            ),
            None => (String::new(), String::new(), String::new(), String::new()),
        };
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            r.timestamp_us,
            r.utc_time
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            fmt_opt_u8(r.fix_type),
            fmt_opt_u8(r.num_sat),
            fmt_opt_f(r.hdop, 3),
            fmt_opt_f(r.lat, 7),
            fmt_opt_f(r.lon, 7),
            fmt_opt_f(r.gps_alt_m, 2),
            fmt_opt_f(r.baro_alt_m, 2),
            fmt_opt_f(r.nav_alt_m, 2),
            fmt_opt_f(r.speed_ms, 3),
            fmt_opt_f(r.ground_course_deg, 2),
            fmt_opt_f(r.gps_vz_ms, 3),
            fmt_opt_f(r.baro_climb_rate_ms, 3),
            fmt_opt_f(r.roll_deg, 2),
            fmt_opt_f(r.pitch_deg, 2),
            fmt_opt_f(r.yaw_deg, 2),
            fmt_opt_f(r.voltage_v, 3),
            fmt_opt_f(r.current_a, 3),
            fmt_opt_f(r.mah_drawn, 1),
            fmt_opt_f(r.baro_temp_c, 1),
            fmt_opt_f(r.wind_n_ms, 3),
            fmt_opt_f(r.wind_e_ms, 3),
            rc1, rc2, rc3, rc4,
            r.active_wp_number.map(|v| v.to_string()).unwrap_or_default(),
            fmt_opt_u8(r.custom_mode),
            r.armed as u8,
            csv_escape(r.vehicle_type.as_deref().unwrap_or("")),
            csv_escape(r.fw_version.as_deref().unwrap_or("")),
        )
        .map_err(|e| e.to_string())?;
    }

    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

#[inline]
fn fmt_opt_u8(v: Option<u8>) -> String {
    v.map(|u| u.to_string()).unwrap_or_default()
}

#[inline]
fn fmt_opt_f(v: Option<f64>, decimals: usize) -> String {
    v.map(|f| format!("{:.prec$}", f, prec = decimals))
        .unwrap_or_default()
}

/// Wrap in quotes if the value contains a comma (minimal CSV escaping).
#[inline]
fn csv_escape(s: &str) -> String {
    if s.contains(',') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
