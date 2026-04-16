// Raw text logger — writes parsed telemetry data as human-readable text files.
// One file per flight, named by timestamp and flight ID.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Utc};

use super::types::TelemetryRecord;

pub struct RawLogger {
    writer: BufWriter<File>,
}

impl RawLogger {
    /// Create a new raw logger. Creates a file like `raw_logs/2024-01-15_143022_flight_42.txt`
    pub fn new(
        base_dir: &Path,
        flight_id: i64,
        start_time: &DateTime<Utc>,
    ) -> Result<Self, String> {
        let log_dir = base_dir.join("raw_logs");
        fs::create_dir_all(&log_dir).map_err(|e| format!("Cannot create raw_logs dir: {}", e))?;

        let filename = format!(
            "{}_flight_{}.txt",
            start_time.format("%Y-%m-%d_%H%M%S"),
            flight_id
        );
        let path = log_dir.join(&filename);

        let file =
            File::create(&path).map_err(|e| format!("Cannot create raw log {}: {}", filename, e))?;
        let mut writer = BufWriter::new(file);

        // Write header
        writeln!(writer, "# KiteGC Raw Flight Log").ok();
        writeln!(writer, "# Flight ID: {}", flight_id).ok();
        writeln!(writer, "# Started: {}", start_time.to_rfc3339()).ok();
        writeln!(
            writer,
            "# Columns: timestamp_ms,lat,lon,alt_m,speed_ms,heading,vario_ms,voltage,current_a,mah_drawn,rssi,roll,pitch,yaw,fix,sat,cpu"
        )
        .ok();

        log::info!("Raw log: {}", path.display());

        Ok(Self { writer })
    }

    /// Write a single telemetry record as a CSV-like line
    pub fn write_record(&mut self, r: &TelemetryRecord) {
        let line = format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            r.timestamp_ms,
            fmt_opt_f64(r.lat, 7),
            fmt_opt_f64(r.lon, 7),
            fmt_opt_f64(r.alt_m, 2),
            fmt_opt_f64(r.speed_ms, 2),
            fmt_opt_i16(r.heading),
            fmt_opt_f64(r.vario_ms, 2),
            fmt_opt_f64(r.voltage, 2),
            fmt_opt_f64(r.current_a, 2),
            fmt_opt_u32(r.mah_drawn),
            fmt_opt_u16(r.rssi),
            fmt_opt_f64(r.roll, 1),
            fmt_opt_f64(r.pitch, 1),
            fmt_opt_i16(r.yaw),
            fmt_opt_u8(r.fix_type),
            fmt_opt_u8(r.num_sat),
            fmt_opt_u16(r.cpu_load),
        );
        writeln!(self.writer, "{}", line).ok();
    }

    /// Flush and close the log file
    pub fn close(&mut self) {
        self.writer.flush().ok();
    }
}

fn fmt_opt_f64(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(val) => format!("{:.prec$}", val, prec = decimals),
        None => String::new(),
    }
}

fn fmt_opt_i16(v: Option<i16>) -> String {
    match v {
        Some(val) => val.to_string(),
        None => String::new(),
    }
}

fn fmt_opt_u8(v: Option<u8>) -> String {
    match v {
        Some(val) => val.to_string(),
        None => String::new(),
    }
}

fn fmt_opt_u16(v: Option<u16>) -> String {
    match v {
        Some(val) => val.to_string(),
        None => String::new(),
    }
}

fn fmt_opt_u32(v: Option<u32>) -> String {
    match v {
        Some(val) => val.to_string(),
        None => String::new(),
    }
}
