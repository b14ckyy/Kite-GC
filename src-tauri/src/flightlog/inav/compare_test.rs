// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Old path (blackbox_decode + Kite's CSV parser) vs. the native decoder on real logs — the
//! acceptance test of the native decoder (Dev-Docs active/NATIVE_BLACKBOX_DECODER.md).
//!
//! Opt-in: `cargo test --release --manifest-path src-tauri/Cargo.toml --lib inav_compare -- --ignored --nocapture`
//! - `KITE_INAV_TEST_LOGS=<folder>`: every `*.TXT` / `*.bbl` / `*.bfl` (any case) below it, every
//!   log in each file (blackbox_decode `--index`).
//! - `KITE_TEST_DB=<flights.db>`: additionally the archived logs of its INAV flights
//!   (`blackbox_files` of `protocol = 'BLACKBOX'`), opened read-only and immutable; each blob is
//!   decoded without an index, as the hi-res parse does.
//! - `KITE_BLACKBOX_DECODE=<binary>`: the oracle (default: the app's own lookup, `find_decoder`).
//! - `KITE_COMPARE_SCRATCH=<dir>`: where a blob is spilled for the external decoder (default: the
//!   temp dir); the spill is removed after each log.
//!
//! Per log: full rate (the hi-res shape) — rows matched by their `time`, every `TelemetryRecord`
//! field equal (floats bit-equal); 10 Hz (the importer shape) — the same rows kept, equal. Compat
//! mode must match exactly (the test fails otherwise); native mode is summarised: its extra rows
//! must all be ones the compat emulation drops. Also prints the old path's digest per log (to show
//! the shared mapping left the old path unchanged) and the decode times of both paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use sha1::{Digest, Sha1};

use super::super::blackbox::{
    find_decoder, log_start_offsets, parse_csv_rows, run_decoder_capture_stdout, ParsedRow,
};
use super::super::types::TelemetryRecord;
use super::telemetry::{decode_rows, Compat, DecodedLog, ToolUnknownEvent};

/// Where a log comes from.
enum Source {
    /// Log `index` of a file on disk (blackbox_decode `--index`).
    File(PathBuf, u32),
    /// An archived blob (`blackbox_files.id`) — one log, decoded without an index like the hi-res parse.
    Blob(i64),
}

struct Case {
    label: String,
    source: Source,
}

fn log_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            log_files(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "txt" | "bbl" | "bfl"))
        {
            out.push(p);
        }
    }
}

/// A read-only connection that never writes next to the fixture (`immutable`: no -wal/-shm).
fn open_fixture(path: &str) -> Connection {
    let uri = format!(
        "file:{}?mode=ro&immutable=1",
        path.replace('\\', "/").replace('%', "%25").replace(' ', "%20").replace('?', "%3f").replace('#', "%23")
    );
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open the fixture database read-only")
}

fn blob(conn: &Connection, id: i64) -> Vec<u8> {
    conn.query_row("SELECT file_data FROM blackbox_files WHERE id = ?1", [id], |r| r.get(0))
        .expect("blob")
}

fn cases(db: Option<&Connection>) -> Vec<Case> {
    let mut cases = Vec::new();
    if let Ok(dir) = std::env::var("KITE_INAV_TEST_LOGS") {
        let mut files = Vec::new();
        log_files(Path::new(&dir), &mut files);
        for file in files {
            let bytes = std::fs::read(&file).expect("read log file");
            let count = log_start_offsets(&bytes).len();
            if count == 0 {
                println!("CMP {}: no INAV log, skipped", file.display());
            }
            for index in 0..count {
                cases.push(Case {
                    label: format!("{} log {}/{count}", file.display(), index + 1),
                    source: Source::File(file.clone(), index as u32),
                });
            }
        }
    }
    if let Some(conn) = db {
        let mut stmt = conn
            .prepare(
                "SELECT b.id, b.original_filename, b.log_index FROM blackbox_files b \
                 JOIN flights f ON f.id = b.flight_id WHERE f.protocol = 'BLACKBOX' ORDER BY b.id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .unwrap();
        for row in rows {
            let (id, name, index) = row.unwrap();
            cases.push(Case {
                label: format!("db blob {id} ({name} log {})", index + 1),
                source: Source::Blob(id),
            });
        }
    }
    cases
}

/// The old path as the app runs it: blackbox_decode, then the CSV parser at full rate (the hi-res
/// shape) and at 10 Hz (the importer shape). The time covers process + full-rate parse.
struct OldPath {
    full: Vec<ParsedRow>,
    ten_hz: Vec<ParsedRow>,
    elapsed: Duration,
}

fn old_path(decoder: &Path, file: &Path, index: Option<u32>) -> Result<OldPath, String> {
    let started = Instant::now();
    let csv = run_decoder_capture_stdout(decoder, file, index)?;
    let full = parse_csv_rows(&csv, 0)?;
    let elapsed = started.elapsed();
    let ten_hz = parse_csv_rows(&csv, 100_000)?;
    Ok(OldPath { full, ten_hz, elapsed })
}

/// Digest of a row list (every field via `Debug`: a float that differs in any bit prints differently).
fn digest(rows: &[ParsedRow]) -> String {
    let mut h = Sha1::new();
    for r in rows {
        h.update(format!("{}|{:?}\n", r.timestamp_us, r.telemetry).as_bytes());
    }
    h.finalize().iter().take(10).map(|b| format!("{b:02x}")).collect()
}

/// Exact equality: floats by their bits.
trait Same {
    fn same(&self, other: &Self) -> bool;
}

impl Same for Option<f64> {
    fn same(&self, other: &Self) -> bool {
        self.map(f64::to_bits) == other.map(f64::to_bits)
    }
}

macro_rules! same_by_eq {
    ($($t:ty),*) => {$(
        impl Same for $t {
            fn same(&self, other: &Self) -> bool {
                self == other
            }
        }
    )*};
}
same_by_eq!(i64, Option<i64>, Option<i32>, Option<i16>, Option<i8>, Option<u32>, Option<u16>, Option<u8>, Option<String>);

/// Calls `diff(field, old, new)` for every differing field. The destructuring names every field,
/// so a field added to `TelemetryRecord` breaks the build here instead of escaping the comparison.
macro_rules! compare_records {
    ($a:expr, $b:expr, $diff:expr, [$($f:ident),* $(,)?]) => {{
        let TelemetryRecord { $($f: _),* } = $a;
        $(
            if !Same::same(&$a.$f, &$b.$f) {
                $diff(stringify!($f), format!("{:?}", $a.$f), format!("{:?}", $b.$f));
            }
        )*
    }};
}

fn compare_record(a: &TelemetryRecord, b: &TelemetryRecord, mut diff: impl FnMut(&'static str, String, String)) {
    compare_records!(a, b, diff, [
        id, flight_id, timestamp_ms, lat, lon, alt_m, speed_ms, airspeed_ms, throttle_pct, heading,
        vario_ms, voltage, current_a, mah_drawn, rssi, battery_percentage, roll, pitch, yaw, fix_type,
        num_sat, cpu_load, link_quality, baro_alt_m, gps_hdop, gps_eph, gps_epv, active_wp_number,
        active_flight_mode_flags, state_flags, nav_state, nav_flags, rx_signal_received, hw_health_status,
        baro_temperature, wind_n_ms, wind_e_ms, wind_d_ms, rc_data_json, rc_command_json, nav_lat, nav_lon,
        nav_alt_m, mode_primary, mode_modifiers, link_snr, link_rssi_dbm, wall_ms,
    ]);
}

/// Per field: differing rows and the first three `(time, old, new)`.
type FieldDiffs = BTreeMap<&'static str, (u64, Vec<(i64, String, String)>)>;

/// Old rows vs. new rows, matched by `time` (equal times in order).
#[derive(Default)]
struct RowDiff {
    compared: u64,
    old_only: Vec<i64>,
    new_only: Vec<i64>,
    fields: FieldDiffs,
}

impl RowDiff {
    fn new(old: &[ParsedRow], new: &[ParsedRow]) -> RowDiff {
        let mut d = RowDiff::default();
        let (mut i, mut j) = (0, 0);
        while i < old.len() || j < new.len() {
            let (a, b) = (old.get(i), new.get(j));
            match (a, b) {
                (Some(a), Some(b)) if a.source_time_us == b.source_time_us => {
                    d.compared += 1;
                    let t = a.source_time_us;
                    if a.timestamp_us != b.timestamp_us {
                        d.add("(row timestamp_us)", t, a.timestamp_us.to_string(), b.timestamp_us.to_string());
                    }
                    compare_record(&a.telemetry, &b.telemetry, |f, o, n| d.add(f, t, o, n));
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(b)) if a.source_time_us < b.source_time_us => {
                    d.old_only.push(a.source_time_us);
                    i += 1;
                }
                (Some(_), Some(b)) => {
                    d.new_only.push(b.source_time_us);
                    j += 1;
                }
                (Some(a), None) => {
                    d.old_only.push(a.source_time_us);
                    i += 1;
                }
                (None, Some(b)) => {
                    d.new_only.push(b.source_time_us);
                    j += 1;
                }
                (None, None) => unreachable!(),
            }
        }
        d
    }

    fn add(&mut self, field: &'static str, time: i64, old: String, new: String) {
        let entry = self.fields.entry(field).or_default();
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push((time, old, new));
        }
    }

    fn differing_fields(&self) -> u64 {
        self.fields.values().map(|f| f.0).sum()
    }

    fn exact(&self) -> bool {
        self.old_only.is_empty() && self.new_only.is_empty() && self.fields.is_empty()
    }

    fn print(&self, what: &str) {
        println!(
            "    {what}: compared {} | old-only {} | new-only {} | differing fields {}",
            self.compared,
            self.old_only.len(),
            self.new_only.len(),
            self.differing_fields()
        );
        for (field, (count, examples)) in &self.fields {
            let shown: Vec<String> = examples.iter().map(|(t, o, n)| format!("t={t} old={o} new={n}")).collect();
            println!("      {field}: {count} rows; {}", shown.join("; "));
        }
    }
}

/// The last event blackbox_decode does not know before `time`.
fn preceding_event(events: &[ToolUnknownEvent], time: i64) -> String {
    events
        .iter()
        .rev()
        .find(|e| e.after_time_us.is_some_and(|t| t < time))
        .map_or("none".into(), |e| format!("event {} after t={}", e.id, e.after_time_us.unwrap_or(-1)))
}

fn decode_native(
    bytes: &[u8],
    index: Option<u32>,
    compat: Compat,
    interval: i64,
) -> Result<(DecodedLog, Duration), String> {
    let started = Instant::now();
    let decoded = decode_rows(bytes, index, compat, interval)?;
    Ok((decoded, started.elapsed()))
}

#[derive(Default)]
struct Totals {
    logs: u64,
    compared: u64,
    old_only: u64,
    new_only: u64,
    differing_fields: u64,
    native_only: u64,
    native_unexplained: u64,
    native_differing: BTreeMap<&'static str, u64>,
    rule_exceptions: u64,
    old_ms: f64,
    new_ms: f64,
}

#[test]
#[ignore]
fn inav_compare() {
    let db_path = std::env::var("KITE_TEST_DB").ok();
    if std::env::var("KITE_INAV_TEST_LOGS").is_err() && db_path.is_none() {
        eprintln!("KITE_INAV_TEST_LOGS / KITE_TEST_DB not set; skipping");
        return;
    }
    let decoder = std::env::var("KITE_BLACKBOX_DECODE").map(PathBuf::from).ok().or_else(find_decoder);
    let Some(decoder) = decoder.filter(|d| d.is_file()) else {
        eprintln!("blackbox_decode not found (KITE_BLACKBOX_DECODE / find_decoder); skipping");
        return;
    };
    let scratch = std::env::var("KITE_COMPARE_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("kite-inav-compare"));
    std::fs::create_dir_all(&scratch).unwrap();
    let db = db_path.as_deref().map(open_fixture);
    let cases = cases(db.as_ref());
    assert!(!cases.is_empty(), "no INAV logs found");

    let mut totals = Totals::default();
    let mut failures = Vec::new();
    let mut timings = Vec::new();
    for case in &cases {
        let spill = scratch.join("blob.bbl");
        let (bytes, file, index) = match &case.source {
            Source::File(path, index) => (std::fs::read(path).unwrap(), path.clone(), Some(*index)),
            Source::Blob(id) => {
                let bytes = blob(db.as_ref().unwrap(), *id);
                std::fs::write(&spill, &bytes).unwrap();
                (bytes, spill.clone(), None)
            }
        };
        let old = old_path(&decoder, &file, index);
        let _ = std::fs::remove_file(&spill);
        let old = match old {
            Ok(old) => old,
            Err(e) => {
                println!("CMP {} | old path failed: {e}", case.label);
                failures.push(format!("{}: old path failed", case.label));
                continue;
            }
        };
        // A log the native decoder rejects (not INAV, a damaged header) is reported, not fatal — the
        // old path "succeeds" on such a file with whatever the tool printed.
        let natives = decode_native(&bytes, index, Compat::BlackboxDecode, 0).and_then(|full| {
            Ok((
                full,
                decode_native(&bytes, index, Compat::BlackboxDecode, 100_000)?,
                decode_native(&bytes, index, Compat::Native, 0)?,
            ))
        });
        let ((compat, compat_time), (compat_10, _), (native, native_time)) = match natives {
            Ok(n) => n,
            Err(e) => {
                println!("{}: native decode failed: {e}", case.label);
                failures.push(format!("{}: native decode failed: {e}", case.label));
                continue;
            }
        };

        let full = RowDiff::new(&old.full, &compat.rows);
        let ten = RowDiff::new(&old.ten_hz, &compat_10.rows);
        let vs_native = RowDiff::new(&old.full, &native.rows);
        let recovered: std::collections::BTreeSet<i64> = native.recovered_us.iter().copied().collect();
        let unexplained: Vec<i64> =
            vs_native.new_only.iter().copied().filter(|t| !recovered.contains(t)).collect();
        let mut event_ids: BTreeMap<u8, u64> = BTreeMap::new();
        for e in &compat.tool_unknown_events {
            *event_ids.entry(e.id).or_default() += 1;
        }

        println!(
            "CMP {} | old digest full {} 10hz {} | events unknown to the tool {:?} | I frames by rule {} / decoded {}",
            case.label,
            digest(&old.full),
            digest(&old.ten_hz),
            event_ids,
            compat.i_frames_by_rule,
            compat.i_frames_decoded
        );
        full.print("compat full rate");
        for t in full.old_only.iter().take(20) {
            println!("      old-only row t={t}, preceding: {}", preceding_event(&compat.tool_unknown_events, *t));
        }
        ten.print("compat 10 Hz");
        println!(
            "    native full rate: compared {} | old-only {} | native-only {} (recovered by the emulation {}, unexplained {}) | differing fields {}",
            vs_native.compared,
            vs_native.old_only.len(),
            vs_native.new_only.len(),
            native.recovered_us.len(),
            unexplained.len(),
            vs_native.differing_fields()
        );
        for (field, (count, examples)) in &vs_native.fields {
            let first = examples.first().map(|(t, o, n)| format!("t={t} old={o} new={n}")).unwrap_or_default();
            println!("      {field}: {count} rows; first {first}");
        }

        totals.logs += 1;
        totals.compared += full.compared;
        totals.old_only += full.old_only.len() as u64;
        totals.new_only += full.new_only.len() as u64;
        totals.differing_fields += full.differing_fields() + ten.differing_fields();
        totals.native_only += vs_native.new_only.len() as u64;
        totals.native_unexplained += unexplained.len() as u64;
        for (field, (count, _)) in &vs_native.fields {
            *totals.native_differing.entry(field).or_default() += count;
        }
        let (old_ms, new_ms) = (old.elapsed.as_secs_f64() * 1000.0, compat_time.as_secs_f64() * 1000.0);
        totals.old_ms += old_ms;
        totals.new_ms += new_ms;
        timings.push((case.label.clone(), old.full.len(), old_ms, new_ms, native_time.as_secs_f64() * 1000.0));

        if !full.exact() || !ten.exact() {
            failures.push(format!("{}: compat mode differs", case.label));
        }
        // Not a failure by itself: the rule only matters for a frame that ends an emulated window,
        // and a wrong one there shows up as rows only in the new path above.
        if compat.i_frames_by_rule != compat.i_frames_decoded {
            println!(
                "    note: {} main frames at an I-frame iteration were P frames (I-frame rule)",
                compat.i_frames_by_rule.abs_diff(compat.i_frames_decoded)
            );
            totals.rule_exceptions += compat.i_frames_by_rule.abs_diff(compat.i_frames_decoded);
        }
        if !vs_native.old_only.is_empty() || !unexplained.is_empty() {
            failures.push(format!("{}: native mode has unexplained rows", case.label));
        }
    }

    println!("TIMING log | rows | old path ms (blackbox_decode + CSV parse) | native compat ms | native ms | speed-up");
    for (label, rows, old_ms, new_ms, native_ms) in &timings {
        println!(
            "TIMING {label} | {rows} | {old_ms:.0} | {new_ms:.0} | {native_ms:.0} | {:.1}x",
            old_ms / new_ms.max(0.001)
        );
    }
    println!(
        "TOTAL logs {} | compat full rate: compared {}, old-only {}, new-only {}, differing fields (full + 10 Hz) {} | \
         native: native-only rows {} (unexplained {}), differing fields {:?} | I-frame rule exceptions {} | \
         time old {:.0} ms, native compat {:.0} ms",
        totals.logs,
        totals.compared,
        totals.old_only,
        totals.new_only,
        totals.differing_fields,
        totals.native_only,
        totals.native_unexplained,
        totals.native_differing,
        totals.rule_exceptions,
        totals.old_ms,
        totals.new_ms
    );
    assert!(failures.is_empty(), "differences: {failures:#?}");
}
