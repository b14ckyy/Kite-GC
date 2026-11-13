// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Kite's frame sink on the ported decoder core: one log of an INAV Blackbox file → the rows the
//! importer stores ([`ParsedRow`]), mapped to `TelemetryRecord` by the same code as the
//! blackbox_decode CSV path (`blackbox::build_telemetry_record_indexed` through [`CellReader`]).
//!
//! The sink rebuilds the CSV `blackbox_decode --merge-gps --datetime --unit-height m
//! --unit-gps-speed mps --stdout` would print — not as text, but as a header of the same column
//! names plus a description of each cell, so the importer's column resolution runs unchanged and
//! every cell yields the `f64` Kite's CSV parser reads from the tool's text. Source of the rules:
//! blackbox-tools v9.0.0 (`1918a75`), `src/blackbox_decode.c` (BD) and `src/parser.c` (P); details
//! in Dev-Docs reference/BLACKBOX_DECODE_OUTPUT.md.
//!
//! The equality holds for what the importer reads, not for every column of the tool's CSV: where the
//! decoder core deliberately departs from the header (`accVib` is written as a delta although the
//! header declares AVERAGE_2, the extra motor byte of 7.x–8.0, the INAV 10 terrain field order — see
//! `schema.rs`) its values are the firmware's, and the tool's are not. The importer maps none of
//! those fields.
//!
//! [`Compat::BlackboxDecode`] also reproduces which frames the tool drops: it does not know INAV's
//! events 30 (FLIGHTMODE) and 40 (IMU_FAILURE) nor INAV's `LOG_END` text, and after such an event
//! its main stream stays invalid until the next I frame (P frames dropped, a home frame in that
//! window does not validate the GPS home, and the window's G frames take their time from the last
//! main frame the tool accepted, so they attach to its row). [`Compat::Native`] keeps every frame
//! the decoder accepts.
//!
//! Known limit of the emulation: the sink is not told whether a main frame was an I or a P frame
//! and takes "`loopIteration` is a multiple of the I interval" for I. Real logs carry the odd P
//! frame at such an iteration (one whose I frame is missing); one inside an emulated window would
//! end the window a frame interval early. `DecodedLog::i_frames_by_rule` vs. `i_frames_decoded`
//! shows whether a log has any.

use std::ops::Range;

use super::super::blackbox::{
    build_index_map, log_start_offsets, CellReader, ColumnIndices, ParsedRow, RowCollector,
};
use super::frames::{Event, EventValue, FrameDecoder, FrameKind, FrameSink, INDEX_ITERATION, INDEX_TIME};
use super::header::{parse_header, InavHeader};
use super::schema::{schema_for, EventEffect};

/// Largest plausible iteration / time jump between main frames (P:34-37, as in `frames.rs`).
const MAX_ITERATION_JUMP: u32 = 5000;
const MAX_TIME_JUMP_US: i64 = 10_000_000;
/// `RCMODES_TRANSITION_EPOCH` (P:42) and `VBAT_TRANSITION_EPOCH` (P:40).
const RCMODES_TRANSITION_EPOCH: i64 = 1_716_459_570;
const VBAT_TRANSITION_EPOCH: i64 = 1_531_004_400;

/// Which rows [`decode_rows`] produces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compat {
    /// Row for row and field for field what blackbox_decode + Kite's CSV parser produce, including
    /// the frames the tool drops. Logs without GPS field definitions are the exception: the tool's
    /// mAh integration has an off-by-one-frame bug on that path, which is not reproduced.
    // Only the comparison test decodes in this mode; the import and the hi-res parse use `Native`.
    #[cfg_attr(not(test), allow(dead_code))]
    BlackboxDecode,
    /// The same mapping, but every frame the native decoder accepts becomes a row.
    Native,
}

/// An event blackbox_decode does not know (`id` 30, 40, or 255 with INAV's text), with the time
/// of the last main frame before it.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct ToolUnknownEvent {
    pub id: u8,
    pub after_time_us: Option<i64>,
}

/// One decoded log. Besides `rows`, the fields are diagnostics the comparison test reads.
#[cfg_attr(not(test), allow(dead_code))]
pub struct DecodedLog {
    pub rows: Vec<ParsedRow>,
    /// Native mode: the `time` of every row blackbox_decode would not have produced (by the same
    /// emulation compat mode uses to drop them). Empty in compat mode.
    pub recovered_us: Vec<i64>,
    pub tool_unknown_events: Vec<ToolUnknownEvent>,
    /// Main frames whose `loopIteration` is a multiple of the I interval (the sink's I-frame rule)
    /// vs. I frames the decoder accepted — equal when the rule holds for the log.
    pub i_frames_by_rule: u64,
    pub i_frames_decoded: u64,
}

/// Byte range of log `index` (0-based) in a Blackbox file and the number of logs: from its banner to
/// the next one (or the end) — the boundaries blackbox_decode cuts at.
pub(super) fn log_slice(file: &[u8], index: usize) -> Result<(Range<usize>, usize), String> {
    let offsets = log_start_offsets(file);
    let start = *offsets.get(index).ok_or_else(|| {
        if offsets.is_empty() {
            "Not a Blackbox log: the file has no Blackbox header ('H Product:Blackbox flight data recorder')"
                .to_string()
        } else {
            format!(
                "The file holds {} Blackbox logs; log number {} does not exist",
                offsets.len(),
                index + 1
            )
        }
    })?;
    let end = offsets.get(index + 1).copied().unwrap_or(file.len());
    Ok((start..end, offsets.len()))
}

/// Byte range of the log a decode of `log_index` (0-based) reads. `None` takes the only log and
/// fails when the file holds several (blackbox_decode `--stdout` without `--index`). The importer
/// reads the header and archives the slice from this same range.
pub(crate) fn select_log(file: &[u8], log_index: Option<u32>) -> Result<Range<usize>, String> {
    match log_index {
        Some(index) => Ok(log_slice(file, index as usize)?.0),
        None => {
            let (range, count) = log_slice(file, 0)?;
            if count > 1 {
                return Err(format!(
                    "The file holds {count} Blackbox logs; a log number is required"
                ));
            }
            Ok(range)
        }
    }
}

/// Decode one log of a Blackbox file into importer rows. `log_index` is 0-based (see
/// [`select_log`]). `target_interval_us` is the importer's decimation (100 000 = 10 Hz, 0 = every
/// row).
pub fn decode_rows(
    file: &[u8],
    log_index: Option<u32>,
    compat: Compat,
    target_interval_us: i64,
) -> Result<DecodedLog, String> {
    let range = select_log(file, log_index)?;
    let base = range.start;
    let log = &file[range];
    let header = parse_header(log).map_err(|e| format!("Damaged or empty Blackbox log: {e}"))?;
    let tool = ToolConfig::from_header(&log[..header.data_start]);
    if !tool.inav {
        // Betaflight, Cleanflight, EmuFlight & co. write the same banner and frame format, but other
        // fields and predictors — say whose log it is rather than decode it wrongly.
        return Err(match header.firmware_revision.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            Some(revision) => format!(
                "Not an INAV Blackbox log: it was written by '{revision}'. Only INAV Blackbox logs can be imported."
            ),
            None => "Not an INAV Blackbox log: the header names no firmware ('Firmware revision' is missing). \
                     Only INAV Blackbox logs can be imported."
                .into(),
        });
    }
    if !tool.cleanflight {
        return Err(format!(
            "Damaged Blackbox header: 'Firmware type' is '{}', INAV writes 'Cleanflight'",
            header.firmware_type.as_deref().unwrap_or("")
        ));
    }
    check_header(&header)?;
    let (schema, known) = schema_for(header.firmware_version);
    if !known {
        log::warn!(
            "INAV Blackbox: firmware revision {} outside the known versions — decoding with the {} tables",
            header.firmware_revision.as_deref().unwrap_or("(none)"),
            schema.label
        );
    }
    let mut decoder = FrameDecoder::new(&header, schema, base)
        .map_err(|e| format!("Unsupported or damaged Blackbox header: {e}"))?;
    let mut sink = RowSink::new(&header, &tool, compat, target_interval_us);
    decoder.run(log, header.data_start, &mut sink);
    sink.flush();
    log::debug!(
        "INAV Blackbox native decode ({compat:?}): {} rows of {} offered, {} recovered, {} events blackbox_decode does not know",
        sink.collector.rows.len(),
        sink.collector.raw_rows,
        sink.recovered_us.len(),
        sink.tool_unknown_events.len()
    );
    Ok(DecodedLog {
        rows: sink.collector.rows,
        recovered_us: sink.recovered_us,
        tool_unknown_events: sink.tool_unknown_events,
        i_frames_by_rule: sink.i_frames_by_rule,
        i_frames_decoded: decoder.counters[FrameKind::I.index()].ok,
    })
}

/// Header predictor id of "predict from `motor[0]` of the frame being decoded".
const PREDICTOR_MOTOR0: i32 = 5;
/// INAV derives the I interval from a 16-bit rate denominator (next power of two).
const MAX_I_INTERVAL: u32 = 65_536;

/// Two damaged-header cases the decoder core does not reject itself (INAV writes neither; noted for
/// the core in Dev-Docs reference/DECODER_SYNC.md): a GPS or home field predicted from `motor[0]` —
/// the core checks that predictor for main frames only and would index past the GPS frame — and an
/// absurd I interval, which makes its skipped-frame arithmetic run for minutes.
fn check_header(header: &InavHeader) -> Result<(), String> {
    if header.g.predictor.contains(&PREDICTOR_MOTOR0) || header.h.predictor.contains(&PREDICTOR_MOTOR0) {
        return Err("Damaged Blackbox header: a GPS or home field predicts from motor[0]".into());
    }
    if header.i_interval > MAX_I_INTERVAL {
        return Err(format!("Damaged Blackbox header: I interval {}", header.i_interval));
    }
    Ok(())
}

// --- What blackbox_decode derives from the header. ----------------------------------------------

/// The parts of blackbox_decode's `sysConfig` that change what it prints, set by the header lines
/// in file order exactly as `parseHeaderLine` (P:411-617) does.
#[derive(Clone, Copy, Debug)]
struct ToolConfig {
    /// `Firmware type:Cleanflight` (INAV writes it); otherwise the flag columns are not printed.
    cleanflight: bool,
    /// `Firmware revision:INAV …`.
    inav: bool,
    /// `vbatType == INAV_V2`: vbat / amperage in 0.01 V / A, mAh from `amperage * 10`.
    vbat_v2: bool,
    /// `rcModes == INAV_V8`: the flight-mode table with 56-59 (gimbal) and bit 23 removed.
    rc_modes_v8: bool,
    vbatscale: i64,
    current_meter: (i64, i64),
}

#[derive(Clone, Copy, PartialEq)]
enum Version {
    Original,
    Transitional,
    New,
}

impl ToolConfig {
    fn from_header(header: &[u8]) -> ToolConfig {
        let mut cfg = ToolConfig {
            cleanflight: false,
            inav: false,
            vbat_v2: false,
            rc_modes_v8: false,
            vbatscale: 110,
            current_meter: (0, 400),
        };
        let (mut vbat, mut rc_modes) = (Version::Original, Version::Original);
        for line in header.split(|&b| b == b'\n') {
            let Some(line) = line.strip_prefix(b"H ") else {
                continue;
            };
            let line = String::from_utf8_lossy(line);
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key {
                "Firmware type" => cfg.cleanflight = value == "Cleanflight",
                "Firmware revision" => {
                    cfg.inav = value.starts_with("INAV");
                    if cfg.inav {
                        // `strtol` chain from "INAV " on, truncated to uint8 (P:563-569).
                        let rest = value.get(5..).unwrap_or("");
                        let (major, end) = strtol(rest);
                        let rest = rest.get(end + 1..).unwrap_or("");
                        let (minor, end) = strtol(rest);
                        let rest = rest.get(end + 1..).unwrap_or("");
                        let (micro, _) = strtol(rest);
                        let (major, minor, micro) = (major as u8, minor as u8, micro as u8);
                        if major > 2 {
                            vbat = Version::New;
                        } else if major == 2 {
                            vbat = if minor != 0 || micro != 0 {
                                Version::New
                            } else {
                                Version::Transitional
                            };
                        }
                        rc_modes = if major > 8 {
                            Version::New
                        } else if major < 8 {
                            Version::Original
                        } else if minor != 0 || micro != 0 {
                            Version::New
                        } else {
                            Version::Transitional
                        };
                    }
                }
                "Firmware date" => {
                    let built = parse_firmware_date(value);
                    if vbat == Version::Transitional {
                        vbat = if built < VBAT_TRANSITION_EPOCH { Version::Original } else { Version::New };
                    }
                    if rc_modes == Version::Transitional {
                        rc_modes = if built < RCMODES_TRANSITION_EPOCH {
                            Version::Original
                        } else {
                            Version::New
                        };
                    }
                }
                "vbatscale" => cfg.vbatscale = strtol(value).0,
                "currentMeter" => {
                    let mut parts = value.split(',').map(|p| strtol(p).0);
                    cfg.current_meter = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
                }
                _ => {}
            }
        }
        cfg.vbat_v2 = vbat == Version::New;
        cfg.rc_modes_v8 = rc_modes == Version::New;
        cfg
    }

    /// `flightLogVbatADCToMillivolts` (P:1025-1029).
    fn vbat_millivolts(&self, adc: u16) -> i64 {
        (i64::from(adc) * 33 * 10).wrapping_mul(self.vbatscale) / 0xFFF
    }

    /// `flightLogAmperageADCToMilliamps` (P:1031-1039); a zero scale yields 0 (the tool divides by it).
    fn amperage_milliamps(&self, adc: u16) -> i32 {
        let millivolts = ((u32::from(adc) * 33 * 100) / 4095) as i32;
        let millivolts = millivolts.wrapping_sub(self.current_meter.0 as i32);
        if self.current_meter.1 == 0 {
            return 0;
        }
        ((i64::from(millivolts) * 10_000) / self.current_meter.1) as i32
    }
}

/// C `strtol(s, &end, 10)`: the value and the byte offset after it (0 when nothing was parsed).
fn strtol(s: &str) -> (i64, usize) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let negative = matches!(bytes.get(i), Some(b'-'));
    if matches!(bytes.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let digits_from = i;
    let mut value: i64 = 0;
    while let Some(d) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        value = value.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    if i == digits_from {
        return (0, 0);
    }
    (if negative { -value } else { value }, i)
}

/// `parse_firmware_date` (P:387-409) under `TZ=UTC0`: `Mmm dd yyyy hh:mm:ss` at fixed offsets; the
/// seconds are read from the minutes' offset, as the tool does; out-of-range fields roll over as in
/// `mktime`.
fn parse_firmware_date(s: &str) -> i64 {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    // Clamped so a garbage header cannot overflow the arithmetic below.
    let at = |offset: usize| s.get(offset..).map_or(0, |rest| strtol(rest).0.clamp(-1_000_000, 1_000_000));
    let month = MONTHS.iter().position(|m| s.starts_with(m)).map_or(-1, |m| m as i64);
    let (day, year, hour, minute) = (at(4), at(7), at(12), at(15));
    let year = year + month.div_euclid(12);
    let month = month.rem_euclid(12) + 1;
    let days = days_from_civil(year, month, 1) + day - 1;
    days * 86_400 + hour * 3600 + minute * 60 + minute
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// blackbox_decode's own flag tables (blackbox_fielddefs.c `FLIGHT_LOG_FLIGHT_MODE_NAME_INAV`
/// FD:132-194, `…_INAV_LEGACY` FD:71-129, `FLIGHT_LOG_FLIGHT_STATE_NAME_INAV` FD:206-237,
/// `FLIGHT_LOG_FAILSAFE_PHASE_NAME` FD:240-249) — not the schema's per-version tables: the
/// importer's name → bit mapping has to see what the tool printed.
const TOOL_FLIGHT_MODES_INAV: &[&str] = &[
    "ARM", "ANGLE", "HORIZON", "NAVALTHOLD", "HEADINGHOLD", "HEADFREE", "HEADADJ", "CAMSTAB", "NAVRTH",
    "NAVPOSHOLD", "MANUAL", "BEEPERON", "LEDLOW", "LIGHTS", "NAVLAUNCH", "OSD", "TELEMETRY", "BLACKBOX",
    "FAILSAFE", "NAVWP", "AIRMODE", "HOMERESET", "GCSNAV", "**REMOVED**", "SURFACE", "FLAPERON",
    "TURNASSIST", "AUTOTRIM", "AUTOTUNE", "CAMERA1", "CAMERA2", "CAMERA3", "OSDALT1", "OSDALT2", "OSDALT3",
    "NAVCOURSEHOLD", "BRAKING", "USER1", "USER2", "FPVANGLEMIX", "LOITERDIRCHN", "MSPRCOVERRIDE", "PREARM",
    "TURTLE", "NAVCRUISE", "AUTOLEVEL", "PLANWPMISSION", "SOARING", "USER3", "USER4", "CHANGEMISSION",
    "BEEPERMUTE", "MULTIFUNCTION", "MIXERPROFILE", "MIXERTRANSITION", "ANGLEHOLD", "GIMBALTLOCK",
    "GIMBALRLOCK", "GIMBALCENTER", "GIMBALHTRK",
];
const TOOL_FLIGHT_MODES_INAV_LEGACY: &[&str] = &[
    "ARM", "ANGLE", "HORIZON", "NAVALTHOLD", "HEADINGHOLD", "HEADFREE", "HEADADJ", "CAMSTAB", "NAVRTH",
    "NAVPOSHOLD", "MANUAL", "BEEPERON", "LEDLOW", "LIGHTS", "NAVLAUNCH", "OSD", "TELEMETRY", "BLACKBOX",
    "FAILSAFE", "NAVWP", "AIRMODE", "HOMERESET", "GCSNAV", "KILLSWITCH", "SURFACE", "FLAPERON",
    "TURNASSIST", "AUTOTRIM", "AUTOTUNE", "CAMERA1", "CAMERA2", "CAMERA3", "OSDALT1", "OSDALT2", "OSDALT3",
    "NAVCOURSEHOLD", "BRAKING", "USER1", "USER2", "FPVANGLEMIX", "LOITERDIRCHN", "MSPRCOVERRIDE", "PREARM",
    "TURTLE", "NAVCRUISE", "AUTOLEVEL", "PLANWPMISSION", "SOARING", "USER3", "USER4", "CHANGEMISSION",
    "BEEPERMUTE", "MULTIFUNCTION", "MIXERPROFILE", "MIXERTRANSITION", "ANGLEHOLD",
];
const TOOL_STATE_FLAGS_INAV: &[&str] = &[
    "GPS_FIX_HOME", "GPS_FIX", "CALIBRATE_MAG", "SMALL_ANGLE", "FIXED_WING_LEGACY", "ANTI_WINDUP",
    "FLAPERON_AVAILABLE", "NAV_MOTOR_STOP_OR_IDLE", "COMPASS_CALIBRATED", "ACCELEROMETER_CALIBRATED",
    "PWM_DRIVER_AVAILABLE", "NAV_CRUISE_BRAKING", "NAV_CRUISE_BRAKING_BOOST", "NAV_CRUISE_BRAKING_LOCKED",
    "NAV_EXTRA_ARMING_SAFETY_BYPASSED", "AIRMODE_ACTIVE", "ESC_SENSOR_ENABLED", "AIRPLANE", "MULTIROTOR",
    "ROVER", "BOAT", "ALTITUDE_CONTROL", "MOVE_FORWARD_ONLY", "SET_REVERSIBLE_MOTORS_FORWARD",
    "FW_HEADING_USE_YAW", "ANTI_WINDUP_DEACTIVATED", "LANDING_DETECTED", "IN_FLIGHT_EMERG_REARM", "TAILSITTER",
];
const TOOL_FAILSAFE_PHASES: &[&str] = &[
    "IDLE", "RX_LOSS_DETECTED", "RX_LOSS_IDLE", "RETURN_TO_HOME", "LANDING", "LANDED", "RX_LOSS_MONITORING",
    "RX_LOSS_RECOVERED",
];

/// `flightlogDecodeFlagsToString` (P:1067-1109): the names of the set bits, bit 0 first, `|`-joined;
/// "0" when none of the table's bits is set.
fn flag_text(flags: u64, names: &[&str]) -> String {
    let mut text = String::new();
    for (bit, name) in names.iter().enumerate() {
        if flags & (1u64 << bit) != 0 {
            if !text.is_empty() {
                text.push('|');
            }
            text.push_str(name);
        }
    }
    if text.is_empty() {
        text.push('0');
    }
    text
}

/// `flightlogFailsafePhaseToString` (P:1157-1160): the name, or the value as `%u` from 8 on.
fn failsafe_text(value: i64) -> String {
    let phase = value as u8;
    TOOL_FAILSAFE_PHASES
        .get(usize::from(phase))
        .map_or_else(|| phase.to_string(), |name| (*name).to_string())
}

// --- The CSV layout. ------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GpsKind {
    /// `GPS_coord[0|1]`: `%.7f` of `v / 1e7`.
    Coordinate,
    /// `GPS_speed (m/s)`: `%lld.%02u` of `v / 100`, `|v| % 100`.
    Speed,
    /// `GPS_ground_course`: `%lld.%01u` of `v / 10`, `|v| % 10`.
    Course,
    /// Everything else (`GPS_altitude` included): `%PRId64`.
    Integer,
}

/// What one CSV column holds (BD:609-666 `outputMainFrameFields` / `outputMergeFrame`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    /// `time (us)`: the row's 64-bit main-frame time.
    Time,
    /// `dateTime`: not computed — Kite never reads it, and an ISO date never parses as a number.
    DateTime,
    /// A main field printed raw: `%d` / `%u` of its low 32 bits.
    Main { index: usize, signed: bool },
    /// `vbat (V)`: `%.3f`.
    Vbat { index: usize },
    /// `amperage (A)`: `%.3f`.
    Amperage { index: usize },
    /// `BaroAlt (m)`: `%.2f` of `(double) v / 100`.
    BaroAlt { index: usize },
    /// `energyCumulative (mAh)`: `%d` of `(int) round(E)`.
    Energy,
    /// `flightModeFlags (flags)` (`| flightModeFlags2 << 32`), `stateFlags (flags)`, `failsafePhase (flags)`.
    FlightModes,
    StateFlags,
    FailsafePhase,
    /// Any other slow field: `%PRId64` / `%PRIu64`.
    Slow { index: usize, signed: bool },
    Gps { index: usize, kind: GpsKind },
    HomeLat,
    HomeLon,
}

/// The tool's CSV header (BD:927-971) and what each column holds.
struct Layout {
    names: Vec<String>,
    cells: Vec<Cell>,
}

impl Layout {
    fn new(h: &InavHeader) -> Layout {
        let mut layout = Layout {
            names: Vec::new(),
            cells: Vec::new(),
        };
        let mut push = |name: String, cell: Cell| {
            layout.names.push(name);
            layout.cells.push(cell);
        };
        // `identifyMainFields`: the last field of each name wins.
        let find_last = |names: &[&str]| h.i.names.iter().rposition(|n| names.contains(&n.as_str()));
        let vbat = find_last(&["vbatLatest", "vbat"]);
        let amperage = find_last(&["amperageLatest", "amperage"]);
        let baro = find_last(&["BaroAlt"]);
        for (k, name) in h.i.names.iter().enumerate() {
            if k == INDEX_TIME {
                push(format!("{name} (us)"), Cell::Time);
                push("dateTime".into(), Cell::DateTime);
            } else if Some(k) == vbat {
                push(format!("{name} (V)"), Cell::Vbat { index: k });
            } else if Some(k) == amperage {
                push(format!("{name} (A)"), Cell::Amperage { index: k });
            } else if Some(k) == baro {
                push(format!("{name} (m)"), Cell::BaroAlt { index: k });
            } else {
                push(name.clone(), Cell::Main { index: k, signed: h.i.signed[k] });
            }
        }
        if amperage.is_some() {
            push("energyCumulative (mAh)".into(), Cell::Energy);
        }
        // `slowFieldIndexes` start at 0 (memset), so a missing name points at field 0 (P:315-332).
        let slow = SlowIndices::new(h);
        for (k, name) in h.s.names.iter().enumerate() {
            if Some(k) == slow.flight_modes2 {
                continue;
            }
            let cell = if k == slow.flight_modes {
                Cell::FlightModes
            } else if k == slow.state {
                Cell::StateFlags
            } else if k == slow.failsafe {
                Cell::FailsafePhase
            } else {
                Cell::Slow { index: k, signed: h.s.signed[k] }
            };
            let flags = k == slow.flight_modes || k == slow.state || k == slow.failsafe;
            push(if flags { format!("{name} (flags)") } else { name.clone() }, cell);
        }
        // `--merge-gps` adds the G columns only when the log defines G fields (BD:964-969).
        if h.g.count() > 0 {
            for (k, name) in h.g.names.iter().enumerate() {
                if name == "time" {
                    continue;
                }
                let (label, kind) = match name.as_str() {
                    "GPS_coord[0]" | "GPS_coord[1]" => (name.clone(), GpsKind::Coordinate),
                    "GPS_speed" => (format!("{name} (m/s)"), GpsKind::Speed),
                    "GPS_ground_course" => (name.clone(), GpsKind::Course),
                    _ => (name.clone(), GpsKind::Integer),
                };
                push(label, Cell::Gps { index: k, kind });
            }
            push("GPS_home_lat".into(), Cell::HomeLat);
            push("GPS_home_lon".into(), Cell::HomeLon);
        }
        layout
    }
}

struct SlowIndices {
    flight_modes: usize,
    flight_modes2: Option<usize>,
    state: usize,
    failsafe: usize,
}

impl SlowIndices {
    fn new(h: &InavHeader) -> SlowIndices {
        SlowIndices {
            flight_modes: h.s.index_of("flightModeFlags").unwrap_or(0),
            flight_modes2: h.s.index_of("flightModeFlags2"),
            state: h.s.index_of("stateFlags").unwrap_or(0),
            failsafe: h.s.index_of("failsafePhase").unwrap_or(0),
        }
    }
}

/// The flag columns as text — recomputed per S frame (they change only there), read per row.
struct SlowText {
    flight_modes: String,
    state: String,
    failsafe: String,
}

impl SlowText {
    fn new(slow: &[i64], idx: &SlowIndices, tool: &ToolConfig) -> SlowText {
        let value = |k: usize| slow.get(k).copied().unwrap_or(0);
        let mut modes = value(idx.flight_modes) as u64;
        if let Some(k2) = idx.flight_modes2 {
            modes |= (value(k2) as u64) << 32;
        }
        let mode_names = if tool.rc_modes_v8 {
            TOOL_FLIGHT_MODES_INAV
        } else {
            TOOL_FLIGHT_MODES_INAV_LEGACY
        };
        SlowText {
            flight_modes: flag_text(modes, mode_names),
            state: flag_text(value(idx.state) as u64, TOOL_STATE_FLAGS_INAV),
            failsafe: failsafe_text(value(idx.failsafe)),
        }
    }
}

/// `%lld.%0Nu` of `v / 10^N`, `|v| % 10^N` (BD:497 / 503): the sign is lost for `-10^N < v < 0`.
fn split_decimal_text(v: i64, scale: i64, digits: usize) -> String {
    format!("{}.{:0digits$}", v / scale, v.unsigned_abs() % scale.unsigned_abs())
}

/// The `f64` Kite's CSV parser reads from [`split_decimal_text`] — the text is the exact decimal
/// `±|v| / scale` (with the sign of the integer part), and a correctly rounded parse of that decimal
/// is the correctly rounded quotient of the two exactly representable integers.
fn split_decimal_value(v: i64, scale: i64) -> f64 {
    if v < 0 && v > -scale {
        v.unsigned_abs() as f64 / scale as f64
    } else {
        v as f64 / scale as f64
    }
}

// --- One output row. ------------------------------------------------------------------------------

/// One CSV row as the field mapping reads it: each cell is computed as the `f64` Kite's
/// `parse_loose_f64` gets from the tool's text, without the text (see [`NativeRow::text`] for the
/// text itself, which the tests use to check every fast path against).
struct NativeRow<'a> {
    cells: &'a [Cell],
    tool: &'a ToolConfig,
    time: i64,
    main: &'a [i64],
    slow: &'a [i64],
    slow_text: &'a SlowText,
    gps: &'a [i64],
    energy_mah: f64,
    /// `None`: no valid home, printed as `0.0000000`.
    home: Option<[i64; 2]>,
}

impl NativeRow<'_> {
    fn energy(&self) -> i32 {
        self.energy_mah.round() as i32
    }

    fn vbat(&self, v: i64) -> f64 {
        if self.tool.vbat_v2 {
            f64::from(v as u16) / 100.0
        } else {
            self.tool.vbat_millivolts(v as u16) as f64 / 1000.0
        }
    }

    fn amperage(&self, v: i64) -> f64 {
        if self.tool.vbat_v2 {
            f64::from(v as u16) / 100.0
        } else {
            f64::from(self.tool.amperage_milliamps(v as u16)) / 1000.0
        }
    }

    fn home(&self, k: usize) -> f64 {
        self.home.map_or(0.0, |h| h[k] as f64 / 1e7)
    }

    /// The cell exactly as blackbox_decode prints it.
    fn text(&self, index: usize) -> String {
        match self.cells[index] {
            Cell::Time => self.time.to_string(),
            Cell::DateTime => "0000-00-00T00:00:00.000000Z".into(),
            Cell::Main { index, signed: true } => (self.main[index] as i32).to_string(),
            Cell::Main { index, signed: false } => (self.main[index] as u32).to_string(),
            Cell::Vbat { index } => format!("{:.3}", self.vbat(self.main[index])),
            Cell::Amperage { index } => format!("{:.3}", self.amperage(self.main[index])),
            Cell::BaroAlt { index } => format!("{:.2}", self.main[index] as f64 / 100.0),
            Cell::Energy => self.energy().to_string(),
            Cell::FlightModes => self.slow_text.flight_modes.clone(),
            Cell::StateFlags => self.slow_text.state.clone(),
            Cell::FailsafePhase => self.slow_text.failsafe.clone(),
            Cell::Slow { index, signed: true } => self.slow[index].to_string(),
            Cell::Slow { index, signed: false } => (self.slow[index] as u64).to_string(),
            Cell::Gps { index, kind } => {
                let v = self.gps[index];
                match kind {
                    GpsKind::Coordinate => format!("{:.7}", v as f64 / 1e7),
                    GpsKind::Speed => split_decimal_text(v, 100, 2),
                    GpsKind::Course => split_decimal_text(v, 10, 1),
                    GpsKind::Integer => v.to_string(),
                }
            }
            Cell::HomeLat => format!("{:.7}", self.home(0)),
            Cell::HomeLon => format!("{:.7}", self.home(1)),
        }
    }
}

impl CellReader for NativeRow<'_> {
    fn f64_at(&self, index: usize) -> Option<f64> {
        // Each arm is the value of `parse_loose_f64(&self.text(index))` (checked by the tests):
        // integers below 2^53 print and parse exactly; `%.Nf` of `n / 10^N` prints the exact decimal
        // (the double's error is far below half a unit of the last printed digit), which parses back
        // to the correctly rounded `n as f64 / 10^N` — the same double the division yields.
        Some(match self.cells.get(index)? {
            Cell::Time => self.time as f64,
            Cell::DateTime => return None,
            Cell::Main { index, signed: true } => f64::from(self.main[*index] as i32),
            Cell::Main { index, signed: false } => f64::from(self.main[*index] as u32),
            Cell::Vbat { index } => self.vbat(self.main[*index]),
            Cell::Amperage { index } => self.amperage(self.main[*index]),
            Cell::BaroAlt { index } => self.main[*index] as f64 / 100.0,
            Cell::Energy => f64::from(self.energy()),
            // A names cell parses as a number only when it is the "0" of no flags (or, for the
            // failsafe phase, a value from 8 on printed as `%u`).
            Cell::FlightModes => return (self.slow_text.flight_modes == "0").then_some(0.0),
            Cell::StateFlags => return (self.slow_text.state == "0").then_some(0.0),
            Cell::FailsafePhase => return self.slow_text.failsafe.parse::<u8>().ok().map(f64::from),
            Cell::Slow { index, signed: true } => self.slow[*index] as f64,
            Cell::Slow { index, signed: false } => self.slow[*index] as u64 as f64,
            Cell::Gps { index, kind } => {
                let v = self.gps[*index];
                match kind {
                    GpsKind::Coordinate => v as f64 / 1e7,
                    GpsKind::Speed => split_decimal_value(v, 100),
                    GpsKind::Course => split_decimal_value(v, 10),
                    GpsKind::Integer => v as f64,
                }
            }
            Cell::HomeLat => self.home(0),
            Cell::HomeLon => self.home(1),
        })
    }

    fn i64_at(&self, index: usize) -> Option<i64> {
        match self.cells.get(index)? {
            Cell::Time => Some(self.time),
            _ => super::super::blackbox::parse_loose_i64(&self.text(index)),
        }
    }

    fn flag_bits_at(&self, index: usize, map: fn(&str) -> u32) -> Option<u32> {
        let text = match self.cells.get(index)? {
            Cell::FlightModes => return Some(map(&self.slow_text.flight_modes)),
            Cell::StateFlags => return Some(map(&self.slow_text.state)),
            Cell::FailsafePhase => return Some(map(&self.slow_text.failsafe)),
            _ => self.text(index),
        };
        (!text.trim().is_empty()).then(|| map(&text))
    }
}

// --- The sink. ----------------------------------------------------------------------------------

struct RowSink {
    compat: Compat,
    tool: ToolConfig,
    layout: Layout,
    cols: ColumnIndices,
    collector: RowCollector,
    i_interval: u32,
    amperage: Option<usize>,
    g_has_time: bool,
    home_idx: [Option<usize>; 2],
    slow_idx: SlowIndices,

    /// `bufferedMainFrame` / `haveBufferedMainFrame` / `bufferedFrameTime` (BD:104-111).
    main: Vec<i64>,
    buffered: bool,
    buffered_time: i64,
    /// `bufferedSlowFrame` / `bufferedGPSFrame`: zeros until the first S / accepted G frame.
    slow: Vec<i64>,
    slow_text: SlowText,
    gps: Vec<i64>,
    /// The last decoded `GPS_home` (valid or not) and whether it counts: blackbox_decode's
    /// `gpsHomeIsValid` in compat mode, the decoder's in native mode.
    home: [i64; 2],
    home_valid: bool,
    /// `lastFrameTime` (BD:87): the time of the last main frame that produced a row.
    last_frame_time: i64,
    /// `currentMeterMeasured` (battery.c): mAh, last time (0 = none).
    energy_mah: f64,
    energy_last_time: u64,

    /// blackbox_decode's main-stream state, shadowed: `mainStreamIsValid`, `lastMainFrameIteration`,
    /// `lastMainFrameTime` (P:1238-1377), and `gpsHomeIsValid`.
    tool_valid: bool,
    tool_last_iteration: Option<u32>,
    tool_last_time: i64,
    tool_home_valid: bool,
    /// The time of `mainHistory[1]` — the last main frame the tool accepted, `None` after it
    /// invalidated its history (P:1250-1255) — and of the decoder's last main frame: a G frame's
    /// `time` is predicted from these (LAST_MAIN_FRAME_TIME, P:687-690).
    tool_history_time: Option<i64>,
    last_main_time: Option<i64>,

    recovered_us: Vec<i64>,
    tool_unknown_events: Vec<ToolUnknownEvent>,
    i_frames_by_rule: u64,
}

impl RowSink {
    fn new(h: &InavHeader, tool: &ToolConfig, compat: Compat, target_interval_us: i64) -> RowSink {
        let layout = Layout::new(h);
        let cols = ColumnIndices::from_map(&build_index_map(layout.names.iter().map(String::as_str)));
        let slow_idx = SlowIndices::new(h);
        let slow = vec![0; h.s.count()];
        let slow_text = SlowText::new(&slow, &slow_idx, tool);
        RowSink {
            compat,
            tool: *tool,
            amperage: layout.cells.iter().find_map(|c| match c {
                Cell::Amperage { index } => Some(*index),
                _ => None,
            }),
            layout,
            cols,
            collector: RowCollector::new(target_interval_us),
            i_interval: h.i_interval.max(1),
            g_has_time: h.g.index_of("time").is_some(),
            home_idx: [h.h.index_of("GPS_home[0]"), h.h.index_of("GPS_home[1]")],
            slow_idx,
            main: vec![0; h.i.count()],
            buffered: false,
            buffered_time: -1,
            slow,
            slow_text,
            gps: vec![0; h.g.count()],
            home: [0; 2],
            home_valid: false,
            last_frame_time: -1,
            energy_mah: 0.0,
            energy_last_time: 0,
            tool_valid: false,
            tool_last_iteration: None,
            tool_last_time: -1,
            tool_home_valid: false,
            tool_history_time: None,
            last_main_time: None,
            recovered_us: Vec::new(),
            tool_unknown_events: Vec::new(),
            i_frames_by_rule: 0,
        }
    }

    /// `outputMergeFrame` (BD:658-666): the buffered main frame with the current S / G / home state
    /// and mAh becomes a row.
    fn flush(&mut self) {
        if !self.buffered {
            return;
        }
        self.buffered = false;
        let row = NativeRow {
            cells: &self.layout.cells,
            tool: &self.tool,
            time: self.buffered_time,
            main: &self.main,
            slow: &self.slow,
            slow_text: &self.slow_text,
            gps: &self.gps,
            energy_mah: self.energy_mah,
            home: self.home_valid.then_some(self.home),
        };
        self.collector.offer(&self.cols, &row);
    }

    /// Would blackbox_decode accept this main frame (one the decoder accepted)? Updates the shadow
    /// state as `completeIntraframe` / `completeInterframe` do. An I frame is a main frame at an
    /// iteration that is a multiple of the I interval (INAV writes I frames exactly there).
    fn tool_accepts(&mut self, iteration: u32, time: i64) -> bool {
        let plausible = |last_iteration: u32, last_time: i64| {
            iteration >= last_iteration
                && iteration < last_iteration.wrapping_add(MAX_ITERATION_JUMP)
                && time >= last_time
                && time < last_time + MAX_TIME_JUMP_US
        };
        // `flightLogInvalidateStream` (an implausible frame) also drops the tool's history; the
        // corrupt-frame path (and so an unknown event) only clears the valid flag.
        if iteration.is_multiple_of(self.i_interval) {
            self.i_frames_by_rule += 1;
            self.tool_valid = match self.tool_last_iteration {
                Some(last) => plausible(last, self.tool_last_time),
                None => true,
            };
            if !self.tool_valid {
                self.tool_history_time = None;
            }
        } else if self.tool_valid {
            let last = self.tool_last_iteration.unwrap_or(u32::MAX);
            self.tool_valid = plausible(last, self.tool_last_time);
            if !self.tool_valid {
                self.tool_history_time = None;
            }
        }
        if self.tool_valid {
            self.tool_last_iteration = Some(iteration);
            self.tool_last_time = time;
            self.tool_history_time = Some(time);
        }
        self.tool_valid
    }

    /// `updateSimulations` → `currentMeterUpdateMeasured` (BD:449-461, battery.c:46-54).
    fn integrate_energy(&mut self, values: &[i64], time: i64) {
        let Some(k) = self.amperage else {
            return;
        };
        let milliamps = if self.tool.vbat_v2 {
            values[k].wrapping_mul(10) as i32
        } else {
            self.tool.amperage_milliamps(values[k] as u16)
        };
        let time = time as u64;
        if self.energy_last_time != 0 {
            self.energy_mah +=
                (f64::from(milliamps) * time.wrapping_sub(self.energy_last_time) as f64) / 3_600_000_000.0;
        }
        self.energy_last_time = time;
    }
}

impl FrameSink for RowSink {
    fn main_frame(&mut self, values: &[i64]) {
        let time = values[INDEX_TIME];
        self.last_main_time = Some(time);
        let accepted = self.tool_accepts(values[INDEX_ITERATION] as u32, time);
        if !accepted {
            match self.compat {
                Compat::BlackboxDecode => return,
                Compat::Native => self.recovered_us.push(time),
            }
        }
        // `onFrameReadyMerge` 'I'/'P' (BD:739-769): flush, then the meter, then buffer.
        self.flush();
        self.last_frame_time = time;
        self.integrate_energy(values, time);
        self.main.copy_from_slice(values);
        self.buffered = true;
        self.buffered_time = time;
    }

    fn slow_frame(&mut self, values: &[i64]) {
        // BD:730-737: the buffered row keeps the S state it was logged under.
        self.flush();
        self.slow.copy_from_slice(values);
        self.slow_text = SlowText::new(&self.slow, &self.slow_idx, &self.tool);
    }

    fn gps_frame(&mut self, values: &[i64], time_us: Option<i64>, home_valid: bool) {
        let valid = match self.compat {
            Compat::BlackboxDecode => self.tool_home_valid,
            Compat::Native => home_valid,
        };
        if !valid {
            return;
        }
        // The tool predicts the G `time` from ITS last main frame: after an event it does not know,
        // that is the frame before the event, so the G frames of the dropped iterations come out
        // with that frame's time and attach to its row.
        let time_us = match self.compat {
            Compat::BlackboxDecode => time_us.map(|t| {
                t - self.last_main_time.unwrap_or(0) + self.tool_history_time.unwrap_or(0)
            }),
            Compat::Native => time_us,
        };
        // BD:694-715: a G frame of the buffered row's iteration attaches to it; a later one first
        // flushes that row with the previous GPS state.
        if self.g_has_time && time_us != Some(self.last_frame_time) {
            self.flush();
        }
        self.gps.copy_from_slice(values);
    }

    fn home_frame(&mut self, values: &[i64], _time_us: Option<i64>, home_valid: bool) {
        for (k, idx) in self.home_idx.iter().enumerate() {
            if let Some(i) = idx {
                self.home[k] = values[*i];
            }
        }
        // `completeGPSHomeFrame` (P:1429): valid if the main stream is valid right now.
        self.tool_home_valid = home_valid && self.tool_valid;
        self.home_valid = match self.compat {
            Compat::BlackboxDecode => self.tool_home_valid,
            Compat::Native => home_valid,
        };
    }

    fn event(&mut self, event: &Event) {
        let id = event.layout.id;
        if event.layout.effect == EventEffect::LoggingResume {
            if let [Some(EventValue::U32(iteration)), Some(EventValue::I64(time)), ..] = event.values {
                self.tool_last_iteration = Some(iteration);
                self.tool_last_time = time;
            }
            return;
        }
        // The tool knows 0, 13, 14 and a LOG_END whose text is exactly "End of log" (P:942-999);
        // INAV's "End of log (disarm reason:N)" fails its 11-byte compare.
        let known = match id {
            0 | 13 => true,
            255 => event.values[0].is_none(),
            _ => false,
        };
        if !known {
            self.tool_valid = false;
            self.tool_unknown_events.push(ToolUnknownEvent {
                id,
                after_time_us: self.last_main_time,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::blackbox::parse_loose_f64;
    use super::*;

    /// Deterministic pseudo-random numbers (no fixtures, no flakes).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 11
        }
        /// Mostly small magnitudes, sometimes anything in the 32-bit range, both signs.
        fn value(&mut self) -> i64 {
            let v = self.next();
            let magnitude = match v % 4 {
                0 => (v >> 8) % 1_000,
                1 => (v >> 8) % 100_000,
                2 => (v >> 8) % 2_000_000_000,
                _ => (v >> 8) % (1 << 32),
            } as i64;
            if v & 0x80 != 0 {
                -magnitude
            } else {
                magnitude
            }
        }
    }

    /// Every fast path of `f64_at` equals parsing the tool's text with the CSV path's parser,
    /// bit for bit, over edge values and random ones.
    #[test]
    fn cell_values_equal_parsed_text() {
        let cells = [
            Cell::Time,
            Cell::DateTime,
            Cell::Main { index: 0, signed: true },
            Cell::Main { index: 0, signed: false },
            Cell::Vbat { index: 0 },
            Cell::Amperage { index: 0 },
            Cell::BaroAlt { index: 0 },
            Cell::Energy,
            Cell::Slow { index: 0, signed: true },
            Cell::Slow { index: 0, signed: false },
            Cell::Gps { index: 0, kind: GpsKind::Coordinate },
            Cell::Gps { index: 0, kind: GpsKind::Speed },
            Cell::Gps { index: 0, kind: GpsKind::Course },
            Cell::Gps { index: 0, kind: GpsKind::Integer },
            Cell::HomeLat,
            Cell::HomeLon,
        ];
        let edges = [
            0i64, 1, -1, 5, -5, 9, -9, 10, -10, 50, -50, 99, -99, 100, -100, 101, -101, 999, -999, 1000,
            12_345, -12_345, 65_535, 65_536, -65_536, 473_977_418, -1_223_456_789, 1_800_000_000,
            -1_800_000_000, i64::from(i32::MAX), i64::from(i32::MIN), i64::from(u32::MAX),
        ];
        let mut rng = Rng(7);
        let values: Vec<i64> = edges.iter().copied().chain((0..20_000).map(|_| rng.value())).collect();
        let slow_idx = SlowIndices {
            flight_modes: 1,
            flight_modes2: None,
            state: 2,
            failsafe: 3,
        };
        for (n, &v) in values.iter().enumerate() {
            for vbat_v2 in [true, false] {
                let tool = ToolConfig {
                    cleanflight: true,
                    inav: true,
                    vbat_v2,
                    rc_modes_v8: true,
                    vbatscale: 110,
                    current_meter: (0, 400),
                };
                let frame = [v, 0, 0, 0];
                let slow_text = SlowText::new(&frame, &slow_idx, &tool);
                let row = NativeRow {
                    cells: &cells,
                    tool: &tool,
                    time: v.abs() * 1000,
                    main: &frame,
                    slow: &frame,
                    slow_text: &slow_text,
                    gps: &frame,
                    energy_mah: v as f64 / 7.0,
                    home: (n % 2 == 0).then_some([v, -v]),
                };
                for (k, cell) in cells.iter().enumerate() {
                    let text = row.text(k);
                    let parsed = parse_loose_f64(&text);
                    let fast = row.f64_at(k);
                    assert_eq!(
                        fast.map(f64::to_bits),
                        parsed.map(f64::to_bits),
                        "{cell:?} of {v}: text {text:?} parses to {parsed:?}, fast path {fast:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn flag_columns_parse_like_the_csv() {
        let tool = ToolConfig {
            cleanflight: true,
            inav: true,
            vbat_v2: true,
            rc_modes_v8: false,
            vbatscale: 110,
            current_meter: (0, 400),
        };
        let idx = SlowIndices {
            flight_modes: 0,
            flight_modes2: Some(1),
            state: 2,
            failsafe: 3,
        };
        let cells = [Cell::FlightModes, Cell::StateFlags, Cell::FailsafePhase];
        for slow in [[0i64, 0, 0, 0], [0b11, 1, 0b100_0000_0011, 2], [1 << 23, 0, 1 << 10, 9]] {
            let text = SlowText::new(&slow, &idx, &tool);
            let row = NativeRow {
                cells: &cells,
                tool: &tool,
                time: 0,
                main: &[],
                slow: &slow,
                slow_text: &text,
                gps: &[],
                energy_mah: 0.0,
                home: None,
            };
            for k in 0..cells.len() {
                assert_eq!(
                    row.f64_at(k).map(f64::to_bits),
                    parse_loose_f64(&row.text(k)).map(f64::to_bits)
                );
            }
        }
        // Merged 64-bit mask, legacy table: bit 23 is KILLSWITCH, bit 32 (flightModeFlags2 bit 0) OSDALT1.
        let text = SlowText::new(&[(1 << 23) | 0b10, 1, 0, 0], &idx, &tool);
        assert_eq!(text.flight_modes, "ANGLE|KILLSWITCH|OSDALT1");
        assert_eq!(text.state, "0");
        assert_eq!(text.failsafe, "IDLE");
        assert_eq!(failsafe_text(8), "8");
        assert_eq!(failsafe_text(256 + 3), "RETURN_TO_HOME"); // cast to uint8 first
    }

    /// A log that decodes: loopIteration, time and one field; G = time + one field; H = one field;
    /// then one I frame (iteration 0, time 0, a = 0).
    fn small_log(i_interval: &str, g_predictor: &str, h_predictor: &str) -> Vec<u8> {
        let mut log = format!(
            "H Product:Blackbox flight data recorder by Nicholas Sherlock\n\
             H Data version:2\n\
             H I interval:{i_interval}\n\
             H Field I name:loopIteration,time,a\n\
             H Field I signed:0,0,1\n\
             H Field I predictor:0,0,0\n\
             H Field I encoding:1,1,0\n\
             H Field P predictor:6,2,1\n\
             H Field P encoding:9,0,0\n\
             H Field G name:time,GPS_numSat\n\
             H Field G signed:0,0\n\
             H Field G predictor:{g_predictor}\n\
             H Field G encoding:1,1\n\
             H Field H name:GPS_home[0]\n\
             H Field H signed:1\n\
             H Field H predictor:{h_predictor}\n\
             H Field H encoding:0\n\
             H Firmware type:Cleanflight\n\
             H Firmware revision:INAV 9.1.0 (d17f2b3e) TEST\n\
             H P interval:1/2\n"
        )
        .into_bytes();
        log.extend_from_slice(b"I\x00\x00\x00");
        log
    }

    /// Damaged headers the decoder core would panic or hang on end in an error instead.
    #[test]
    fn damaged_headers_are_rejected() {
        let decode = |log: Vec<u8>| decode_rows(&log, Some(0), Compat::Native, 0).map(|d| d.rows.len());
        assert_eq!(decode(small_log("4", "10,0", "0")), Ok(1), "the sound log decodes its one frame");
        let err = |log: Vec<u8>| decode(log).expect_err("must be rejected");
        assert!(err(small_log("4", "10,5", "0")).contains("motor[0]"));
        assert!(err(small_log("4", "10,0", "5")).contains("motor[0]"));
        assert!(err(small_log("2147483647", "10,0", "0")).contains("I interval"));
    }

    #[test]
    fn split_decimal_loses_the_sign_like_the_tool() {
        assert_eq!(split_decimal_text(-50, 100, 2), "0.50");
        assert_eq!(split_decimal_text(-150, 100, 2), "-1.50");
        assert_eq!(split_decimal_text(-5, 10, 1), "0.5");
        assert_eq!(split_decimal_text(3599, 10, 1), "359.9");
    }

    #[test]
    fn tool_config_follows_the_header_rules() {
        let cfg = |lines: &str| ToolConfig::from_header(lines.as_bytes());
        let v9 = cfg("H Firmware type:Cleanflight\nH Firmware revision:INAV 9.1.0 (d17f2b3e) MATEKF765\nH Firmware date:Aug 26 2026 10:33:45\n");
        assert!(v9.cleanflight && v9.inav && v9.vbat_v2 && v9.rc_modes_v8);
        // 8.0.0: the firmware date decides the mode table (2024-05-23T10:19:30Z).
        let early = cfg("H Firmware revision:INAV 8.0.0 (81e18244) X\nH Firmware date:Apr 17 2024 14:48:29\n");
        assert!(!early.rc_modes_v8 && early.vbat_v2 && !early.cleanflight);
        let late = cfg("H Firmware revision:INAV 8.0.0 (ec2106af) X\nH Firmware date:Jan 21 2025 16:16:30\n");
        assert!(late.rc_modes_v8);
        // Without a date 8.0.0 stays transitional = legacy; a date line before the revision is ignored.
        assert!(!cfg("H Firmware revision:INAV 8.0.0 (x) X\n").rc_modes_v8);
        assert!(!cfg("H Firmware date:Jan 21 2025 16:16:30\nH Firmware revision:INAV 8.0.0 (x) X\n").rc_modes_v8);
        assert!(cfg("H Firmware revision:INAV 8.0.1 (x) X\n").rc_modes_v8);
        let v7 = cfg("H Firmware revision:INAV 7.1.2 (4e1e59eb) MATEKF765\n");
        assert!(!v7.rc_modes_v8 && v7.vbat_v2);
        let v2 = cfg("H Firmware revision:INAV 2.0.0 (x) X\nH Firmware date:Jul  1 2018 10:00:00\n");
        assert!(!v2.vbat_v2);
        assert!(!cfg("H Firmware revision:Betaflight 4.4.0\n").inav);
        // The tool reads the seconds from the minutes' offset.
        assert_eq!(parse_firmware_date("Jan  1 1970 00:01:30"), 60 + 1);
        assert_eq!(parse_firmware_date("May 23 2024 10:19:30"), RCMODES_TRANSITION_EPOCH - 30 + 19);
    }
}
