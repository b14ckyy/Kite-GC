// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink .tlog binary logger — crash-safe raw frame recording.
//
// Format: Standard MAVLink tlog (compatible with Mission Planner / QGroundControl).
// Each entry: [u64 microseconds since epoch (big-endian)] [raw MAVLink frame bytes]
//
// On disarm the file is flushed and closed. Can be replayed or imported later.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};

pub struct TlogLogger {
    writer: BufWriter<File>,
    path: String,
}

impl TlogLogger {
    /// Create a new tlog file: `raw_logs/YYYY-MM-DD_HHMMSS_flight_N.tlog`
    pub fn new(
        base_dir: &Path,
        flight_id: i64,
        start_time: &DateTime<Utc>,
    ) -> Result<Self, String> {
        let log_dir = base_dir.join("raw_logs");
        fs::create_dir_all(&log_dir).map_err(|e| format!("Cannot create raw_logs dir: {}", e))?;

        let filename = format!(
            "{}_flight_{}.tlog",
            start_time.format("%Y-%m-%d_%H%M%S"),
            flight_id
        );
        let path = log_dir.join(&filename);
        let path_str = path.display().to_string();

        let file =
            File::create(&path).map_err(|e| format!("Cannot create tlog {}: {}", filename, e))?;
        let writer = BufWriter::new(file);

        log::info!("Tlog: {}", path_str);

        Ok(Self {
            writer,
            path: path_str,
        })
    }

    /// Write a raw MAVLink frame with a microsecond timestamp.
    /// `raw_frame` must be the complete MAVLink frame (STX through checksum).
    pub fn write_frame(&mut self, raw_frame: &[u8]) {
        let timestamp_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;

        // tlog format: 8-byte big-endian timestamp + raw frame
        if self.writer.write_all(&timestamp_us.to_be_bytes()).is_err() {
            return;
        }
        self.writer.write_all(raw_frame).ok();
    }

    /// Flush and close the log file
    pub fn close(&mut self) {
        self.writer.flush().ok();
        log::info!("Tlog closed: {}", self.path);
    }
}
