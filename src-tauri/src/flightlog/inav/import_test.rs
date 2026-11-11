// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The Blackbox importer end to end on the native decoder — no external binary: a synthetic INAV
//! log is written to a temp file, imported into a temp database, and the flight row, the stored
//! telemetry, the archived blob and the progress reports are checked; the hi-res parse of the
//! archived blob returns the full-rate rows.
//!
//! The log is built with fixed values (not the random ones of `coverage_test`), so every mapped
//! value can be predicted: I frames only (I interval 1, 50 Hz), one S and one H frame at the start,
//! a G frame every 10th iteration carrying the iteration's own time (so it merges into that row),
//! and INAV's `End of log` event.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::super::blackbox::{
    hires_telemetry_rows, import_blackbox_log_with_progress, legacy_decoder_requested,
};
use super::super::db;
use super::super::types::BlackboxImportStatus;
use super::stream::tests::{write_svb, write_uvb};

const HOME: [i64; 2] = [473_977_418, 85_455_939];
const FRAME_US: i64 = 20_000;
const START_US: i64 = 1_000_000;
/// 180.00 A: `mA * dt / 3.6e9` = 180 000 · 20 000 / 3.6e9 = exactly 1 mAh per frame.
const AMPERAGE: i64 = 18_000;

fn header_line(out: &mut Vec<u8>, key: &str, value: &str) {
    out.extend_from_slice(format!("H {key}:{value}\n").as_bytes());
}

/// One log: `frames` main frames from iteration 0; `vbat0` is the first frame's vbat (0.01 V).
fn write_log(craft: &str, firmware: &str, frames: i64, vbat0: i64) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in [
        ("Product", "Blackbox flight data recorder by Nicholas Sherlock"),
        ("Data version", "2"),
        ("I interval", "1"),
        ("Field I name", "loopIteration,time,vbat,amperage,attitude[0],attitude[1],attitude[2],BaroAlt,navPos[2]"),
        ("Field I signed", "0,0,0,1,1,1,1,1,1"),
        ("Field I predictor", "0,0,0,0,0,0,0,0,0"),
        ("Field I encoding", "1,1,1,0,0,0,0,0,0"),
        ("Field P predictor", "6,2,1,1,1,1,1,1,1"),
        ("Field P encoding", "9,0,0,0,0,0,0,0,0"),
        ("Field S name", "flightModeFlags,stateFlags,failsafePhase"),
        ("Field S signed", "0,0,0"),
        ("Field S predictor", "0,0,0"),
        ("Field S encoding", "1,1,1"),
        ("Field G name", "time,GPS_numSat,GPS_coord[0],GPS_coord[1],GPS_altitude,GPS_speed,GPS_ground_course"),
        ("Field G signed", "0,0,1,1,1,0,0"),
        ("Field G predictor", "10,0,7,7,0,0,0"),
        ("Field G encoding", "1,1,0,0,0,1,1"),
        ("Field H name", "GPS_home[0],GPS_home[1]"),
        ("Field H signed", "1,1"),
        ("Field H predictor", "0,0"),
        ("Field H encoding", "0,0"),
        ("Firmware type", "Cleanflight"),
        ("Firmware revision", firmware),
        ("Log start datetime", "2026-05-01T10:00:00.000+00:00"),
        ("Craft name", craft),
        ("P interval", "1/1"),
    ] {
        header_line(&mut out, key, value);
    }
    // S: flightModeFlags ARM|ANGLE, stateFlags GPS_FIX_HOME|GPS_FIX, failsafePhase IDLE.
    out.push(b'S');
    for v in [0b11, 0b11, 0] {
        write_uvb(&mut out, v);
    }
    for k in 0..frames {
        let time = START_US + k * FRAME_US;
        out.push(b'I');
        write_uvb(&mut out, k as u32);
        write_uvb(&mut out, time as u32);
        write_uvb(&mut out, (vbat0 - k) as u32);
        for v in [AMPERAGE, 100 + k, -50, 1800, 1234 + 10 * k, 500 + k] {
            write_svb(&mut out, v as i32);
        }
        if k == 0 {
            out.push(b'H');
            write_svb(&mut out, HOME[0] as i32);
            write_svb(&mut out, HOME[1] as i32);
        }
        if k % 10 == 0 {
            out.push(b'G');
            write_uvb(&mut out, 0); // time: the last main frame's
            write_uvb(&mut out, 12);
            write_svb(&mut out, (k * 100) as i32); // GPS_coord: from home
            write_svb(&mut out, (k * 50) as i32);
            write_svb(&mut out, (400 + k) as i32);
            write_uvb(&mut out, 1234);
            write_uvb(&mut out, 1800);
        }
    }
    out.extend_from_slice(b"E\xffEnd of log (disarm reason:3)\0");
    out
}

const INAV: &str = "INAV 9.1.0 (00000000) SYNTHETIC";

type Progress = (u8, String, String);

/// A temp folder with a fresh logbook database; removed on drop.
struct Scratch {
    dir: PathBuf,
    conn: Option<Connection>,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        assert!(
            !legacy_decoder_requested(),
            "these tests cover the native decoder — unset KITE_BLACKBOX_LEGACY"
        );
        let dir = std::env::temp_dir().join(format!("kite-bbx-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        Scratch { dir, conn: Some(conn) }
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().unwrap()
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn count(&self, table: &str) -> i64 {
        self.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// The import's result and every progress report (percent, stage, message).
    fn import(&self, path: &Path, log_index: Option<u32>) -> (Result<BlackboxImportStatus, String>, Vec<Progress>) {
        let mut progress = Vec::new();
        let result = import_blackbox_log_with_progress(self.conn(), path, log_index, false, |p, stage, msg| {
            progress.push((p, stage.to_string(), msg.to_string()))
        });
        (result, progress)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.conn.take());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn imported_id(result: Result<BlackboxImportStatus, String>) -> (i64, usize) {
    match result {
        Ok(BlackboxImportStatus::Success { flight_id, rows_imported }) => (flight_id, rows_imported),
        Ok(other) => panic!("expected a plain success, got {other:?}"),
        Err(e) => panic!("import failed: {e}"),
    }
}

/// Bytes before the first banner (a flash dump can start with erased flash) belong to no log.
const LEADING_JUNK: &[u8] = b"\xff\xff\xff\x00not a log\n";

#[test]
fn native_import_end_to_end() {
    let s = Scratch::new("single");
    let log = write_log("TESTCRAFT", INAV, 100, 1680);
    let path = s.write("LOG00001.TXT", &[LEADING_JUNK, &log].concat());

    let (result, progress) = s.import(&path, None);
    let (flight_id, rows_imported) = imported_id(result);

    // Progress: native stages only, rising, nothing about the external tool.
    let stages: Vec<(u8, &str)> = progress.iter().map(|(p, stage, _)| (*p, stage.as_str())).collect();
    assert_eq!(
        stages,
        [
            (5, "prepare"),
            (25, "decode"),
            (70, "check-dup"),
            (72, "store-flight"),
            (82, "store-track"),
            (96, "archive"),
            (100, "done"),
        ]
    );
    assert!(progress.iter().all(|(_, _, msg)| !msg.contains("blackbox_decode")));

    // 100 frames at 50 Hz, decimated to 10 Hz: every 5th frame, the first one included.
    assert_eq!(rows_imported, 20);
    let flight = db::get_flight(s.conn(), flight_id).unwrap().unwrap();
    assert_eq!(flight.craft_name, "TESTCRAFT");
    assert_eq!(flight.source, "blackbox");
    assert_eq!(flight.fc_variant, "INAV");
    assert_eq!(flight.fc_version, INAV);
    assert_eq!(flight.protocol, "BLACKBOX");
    assert_eq!(flight.duration_sec, Some(1)); // 1.0 s → 2.9 s, whole seconds
    assert_eq!(flight.start_lat, Some(HOME[0] as f64 / 1e7));
    assert_eq!(flight.start_lon, Some(HOME[1] as f64 / 1e7));
    assert_eq!(flight.battery_used_mah, Some(95)); // the last kept row, frame 95
    assert!(flight.total_distance_m.is_some_and(|d| d > 0.0));

    let track = db::get_flight_track(s.conn(), flight_id).unwrap();
    assert_eq!(track.len(), 20);
    for (j, row) in track.iter().enumerate() {
        let k = 5 * j as i64;
        // The latest G frame at or before this frame; it merged into its own iteration's row.
        let g = k / 10 * 10;
        assert_eq!(row.timestamp_ms, (START_US + k * FRAME_US) / 1000, "row {j}");
        assert_eq!(row.voltage, Some((1680 - k) as f64 / 100.0), "row {j}");
        assert_eq!(row.current_a, Some(180.0), "row {j}");
        assert_eq!(row.mah_drawn, Some(k as u32), "row {j}");
        assert_eq!(row.roll, Some((100 + k) as f64 / 10.0), "row {j}");
        assert_eq!(row.pitch, Some(-5.0), "row {j}");
        assert_eq!(row.yaw, Some(180.0), "row {j}");
        assert_eq!(row.baro_alt_m, Some((1234 + 10 * k) as f64 / 100.0), "row {j}");
        assert_eq!(row.nav_alt_m, Some((500 + k) as f64 / 100.0), "row {j}");
        assert_eq!(row.lat, Some((HOME[0] + g * 100) as f64 / 1e7), "row {j}");
        assert_eq!(row.lon, Some((HOME[1] + g * 50) as f64 / 1e7), "row {j}");
        assert_eq!(row.alt_m, Some((400 + g) as f64), "row {j}");
        assert_eq!(row.speed_ms, Some(12.34), "row {j}");
        assert_eq!(row.heading, Some(180.0), "row {j}");
        assert_eq!(row.num_sat, Some(12), "row {j}");
        assert_eq!(row.active_flight_mode_flags, Some(1), "row {j}: ANGLE");
        assert_eq!(row.state_flags, Some(0b11), "row {j}: GPS_FIX_HOME|GPS_FIX");
    }

    // The archive is the log itself, from its banner on — the bytes before it are not kept.
    let (name, blob) = db::get_blackbox_file(s.conn(), flight_id).unwrap().unwrap();
    assert_eq!(name, "LOG00001.TXT");
    assert_eq!(blob, log);

    // Hi-res: the archived blob at full rate, natively — the scratch path is never written.
    let scratch = s.dir.join("hires.src.tmp");
    let full = hires_telemetry_rows(&blob, &scratch).unwrap();
    assert!(!scratch.exists());
    assert_eq!(full.len(), 100);
    for (k, row) in full.iter().enumerate() {
        let k = k as i64;
        assert_eq!(row.timestamp_ms, (START_US + k * FRAME_US) / 1000);
        assert_eq!(row.mah_drawn, Some(k as u32));
        assert_eq!(row.lat, Some((HOME[0] + k / 10 * 1000) as f64 / 1e7));
        assert_eq!(row.voltage, Some((1680 - k) as f64 / 100.0));
    }
}

#[test]
fn native_import_picks_the_log_by_index() {
    let s = Scratch::new("multi");
    let first = write_log("FIRST", INAV, 50, 1680);
    let second = write_log("SECOND", INAV, 30, 1600);
    let path = s.write("FLASH.TXT", &[&first[..], &second[..]].concat());
    assert_eq!(super::super::blackbox::count_logs(&path), Ok(2));

    // A missing or out-of-range index fails before anything is written.
    let (result, _) = s.import(&path, None);
    assert!(result.unwrap_err().contains("a log number is required"));
    let (result, _) = s.import(&path, Some(2));
    assert!(result.unwrap_err().contains("log number 3 does not exist"));
    assert_eq!(s.count("flights"), 0);

    let (flight_id, rows_imported) = imported_id(s.import(&path, Some(1)).0);
    assert_eq!(rows_imported, 6); // 30 frames at 50 Hz → 10 Hz
    let flight = db::get_flight(s.conn(), flight_id).unwrap().unwrap();
    assert_eq!(flight.craft_name, "SECOND");
    assert_eq!(flight.battery_used_mah, Some(25));
    let track = db::get_flight_track(s.conn(), flight_id).unwrap();
    assert_eq!(track[0].voltage, Some(16.0));
    let (_, blob) = db::get_blackbox_file(s.conn(), flight_id).unwrap().unwrap();
    assert_eq!(blob, second);
    let index: i64 = s
        .conn()
        .query_row("SELECT log_index FROM blackbox_files WHERE flight_id = ?1", [flight_id], |r| r.get(0))
        .unwrap();
    assert_eq!(index, 1);
}

#[test]
fn native_import_refuses_other_firmware() {
    let s = Scratch::new("betaflight");
    let path = s.write(
        "btfl_001.bbl",
        &write_log("QUAD", "Betaflight 4.4.2 (025ae87) STM32F7X2", 20, 1680),
    );
    let err = s.import(&path, None).0.unwrap_err();
    assert!(err.starts_with("Not an INAV Blackbox log"), "{err}");
    assert!(err.contains("Betaflight 4.4.2"), "{err}");
    assert_eq!(s.count("flights"), 0);
    assert_eq!(s.count("telemetry_records"), 0);
    assert_eq!(s.count("blackbox_files"), 0);

    // No banner at all: refused as no Blackbox log, also before anything is written.
    let path = s.write("notes.txt", b"just some text\n");
    let err = s.import(&path, None).0.unwrap_err();
    assert!(err.starts_with("Not a Blackbox log"), "{err}");
    assert_eq!(s.count("flights"), 0);
}
