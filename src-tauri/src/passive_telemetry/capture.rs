// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Raw-stream capture for passive telemetry validation (Phase B).
//
// Writes two timestamped files into `<raw-log-dir>/radiotelem/` on connect:
//   - `radiotelem_<ts>.bin`    — exact concatenated byte stream, losslessly re-parseable.
//   - `radiotelem_<ts>.jsonl`  — one record per transport read / BLE notification:
//                                {"t_ms":<since connect>,"len":<n>,"hex":"7e 10 .."}
//                                preserves chunk boundaries + timing (critical for BLE framing).
//
// This is a developer capture for offline analysis, NOT the future `.csv` telemetry recording.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Capture {
    bin: BufWriter<File>,
    jsonl: BufWriter<File>,
    start: Instant,
    bin_path: PathBuf,
    chunks: u64,
}

impl Capture {
    /// Create the capture files under `dir/radiotelem/`. Returns an error if the files can't be opened.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        let out_dir = dir.join("radiotelem");
        fs::create_dir_all(&out_dir)?;

        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let bin_path = out_dir.join(format!("radiotelem_{}.bin", ts));
        let jsonl_path = out_dir.join(format!("radiotelem_{}.jsonl", ts));

        let bin = BufWriter::new(File::create(&bin_path)?);
        let jsonl = BufWriter::new(File::create(&jsonl_path)?);

        Ok(Self {
            bin,
            jsonl,
            start: Instant::now(),
            bin_path,
            chunks: 0,
        })
    }

    /// Append one freshly-read chunk to both files.
    pub fn write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.chunks += 1;

        if let Err(e) = self.bin.write_all(data) {
            log::warn!("radiotelem .bin write failed: {}", e);
        }

        let t_ms = self.start.elapsed().as_millis();
        let mut hex = String::with_capacity(data.len() * 3);
        for (i, b) in data.iter().enumerate() {
            if i > 0 {
                hex.push(' ');
            }
            hex.push_str(&format!("{:02x}", b));
        }
        let line = format!("{{\"t_ms\":{},\"len\":{},\"hex\":\"{}\"}}\n", t_ms, data.len(), hex);
        if let Err(e) = self.jsonl.write_all(line.as_bytes()) {
            log::warn!("radiotelem .jsonl write failed: {}", e);
        }
    }

    /// Flush both writers (called periodically + on close so a crash keeps most of the capture).
    pub fn flush(&mut self) {
        let _ = self.bin.flush();
        let _ = self.jsonl.flush();
    }

    pub fn bin_path(&self) -> String {
        self.bin_path.to_string_lossy().to_string()
    }

    pub fn chunks(&self) -> u64 {
        self.chunks
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.flush();
    }
}
