// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP raw serial logger — mwptools-compatible "v2" raw format (ADR-049).
//
// File layout (matches mwptools mwp-serial-cap / mwp-log-replay, so our `.rawmsp` files replay in
// mwp's "Replay mwp RAW log" and any MSP decoder built on that format):
//   - Header (once): ASCII `v2\n`.
//   - Per record (little-endian, 11-byte header + payload):
//       offset : f64  — seconds since log start
//       size   : u16  — payload length
//       dirn   : u8   — b'i' incoming (FC→GCS) / b'o' outgoing (GCS→FC)
//       payload: `size` raw serial bytes
//
// Absolute wall-clock time is carried by the filename (local time, ADR-048) + the DB flight; the
// in-file offset is relative, exactly like mwp.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Local, Utc};

/// Direction markers (mwptools v2): incoming = from the FC, outgoing = to the FC.
pub const DIR_IN: u8 = b'i';
pub const DIR_OUT: u8 = b'o';

/// Shared slot for the active MSP raw logger. The `FlightRecorder` owns the lifecycle (opens on
/// arm / continuous, drops on disarm / disconnect); the `MspTransport` writes into it while `Some`.
/// Mirrors how the `FlightRecorderHandle` is shared between the scheduler and command layer.
pub type MspRawSink = Arc<Mutex<Option<MspRawLogger>>>;

pub struct MspRawLogger {
    writer: BufWriter<File>,
    start: Instant,
    path: String,
}

impl MspRawLogger {
    /// Create a new raw MSP log: `raw_logs/<localtime>_flight_N.rawmsp` with the mwp `v2\n` header.
    pub fn new(
        base_dir: &Path,
        flight_id: i64,
        start_time: &DateTime<Utc>,
    ) -> Result<Self, String> {
        let log_dir = base_dir.join("raw_logs");
        fs::create_dir_all(&log_dir).map_err(|e| format!("Cannot create raw_logs dir: {}", e))?;

        // Name in the GCS's local time (it sits at the flight location) so the title matches the
        // logbook clock (ADR-048).
        let filename = format!(
            "{}_flight_{}.rawmsp",
            start_time.with_timezone(&Local).format("%Y-%m-%d_%H%M%S"),
            flight_id
        );
        let path = log_dir.join(&filename);
        let path_str = path.display().to_string();

        let file =
            File::create(&path).map_err(|e| format!("Cannot create rawmsp {}: {}", filename, e))?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(b"v2\n")
            .map_err(|e| format!("Cannot write rawmsp header: {}", e))?;

        log::info!("MSP raw log: {}", path_str);

        Ok(Self {
            writer,
            start: Instant::now(),
            path: path_str,
        })
    }

    /// Append one record: the timed, directional raw serial bytes (mwp v2). Frames over `u16::MAX`
    /// can't be expressed by the format's `size` field — skipped (never happens for real MSP frames).
    pub fn write_frame(&mut self, dir: u8, bytes: &[u8]) {
        if bytes.len() > u16::MAX as usize {
            return;
        }
        let offset = self.start.elapsed().as_secs_f64();
        if self.writer.write_all(&offset.to_le_bytes()).is_err() {
            return;
        }
        if self.writer.write_all(&(bytes.len() as u16).to_le_bytes()).is_err() {
            return;
        }
        if self.writer.write_all(&[dir]).is_err() {
            return;
        }
        self.writer.write_all(bytes).ok();
    }

    /// Flush and close the log file.
    pub fn close(&mut self) {
        self.writer.flush().ok();
        log::info!("MSP raw log closed: {}", self.path);
    }
}

/// Write a record into a shared sink, if a logger is currently active. Used by the transport tap.
pub fn log_to_sink(sink: &MspRawSink, dir: u8, bytes: &[u8]) {
    if let Ok(mut guard) = sink.lock() {
        if let Some(logger) = guard.as_mut() {
            logger.write_frame(dir, bytes);
        }
    }
}
