// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use csv::StringRecord;
use rusqlite::Connection;
use serde_json::json;

use super::db;
use super::types::{BlackboxImportStatus, Flight, TelemetryRecord};

const WINDOWS_BINARY: &str = "blackbox_decode.exe";
const UNIX_BINARY: &str = "blackbox_decode";

struct ParsedRow {
    timestamp_us: i64,
    csv_data: String,
    telemetry: TelemetryRecord,
}

struct HeaderMetadata {
    craft_name: Option<String>,
    fc_version: Option<String>,
    board_id: Option<String>,
    /// Waypoint count from `H waypoints:<count>,<flag>` (only the count is used).
    logged_wp_count: Option<i64>,
    start_time: Option<DateTime<Utc>>,
    /// Heuristic platform type (0=multirotor, 1=airplane) derived from the logged field set.
    platform_type: u8,
}

#[inline]
fn is_valid_gps_coord(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// Haversine distance in metres between two GPS coordinates.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0; // Earth radius in metres
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().asin()
}

pub fn import_blackbox_log_with_progress<F>(
    conn: &Connection,
    file_path: &Path,
    log_index: Option<u32>,
    force_import: bool,
    mut report: F,
) -> Result<BlackboxImportStatus, String>
where
    F: FnMut(u8, &str, &str),
{
    report(5, "prepare", "Reading Blackbox file...");
    let file_data = fs::read(file_path)
        .map_err(|e| format!("Failed to read Blackbox file {}: {}", file_path.display(), e))?;

    // Extract header for the specific log index (not always log 0)
    let header = extract_header_metadata_for_log(&file_data, log_index);
    let craft_name = header
        .craft_name.clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    let start_time = resolve_start_time(&header, file_path);
    eprintln!("[BBX-HEADER] craft={:?}, header_start_time={:?}, resolved_start_time={}", craft_name, header.start_time, start_time.to_rfc3339());

    // NOTE: the duplicate check runs *after* decode + the UTC correction below, not here. The stored
    // start_time is the true-UTC value (header local time minus the location offset, ADR-048), and the
    // offset is only known once the GPS start position has been decoded — so a header-only early check
    // would compare an uncorrected time against the corrected stored one and miss every duplicate of a
    // log whose header carries a non-UTC local time. See the post-correction check below.

    report(10, "decoder", "Searching for blackbox_decode...");
    let decoder = find_decoder().ok_or_else(|| {
        "blackbox_decode not found. Place it next to the application executable or add it to PATH.".to_string()
    })?;

    report(25, "decode", "Running blackbox_decode...");
    let csv_output = run_decoder_capture_stdout(&decoder, file_path, log_index)?;

    report(55, "parse", "Parsing decoded CSV...");
    let rows = parse_csv_rows(&csv_output)?;
    if rows.is_empty() {
        return Err("Blackbox import failed: decoder produced no rows".into());
    }

    let rows_imported = rows.len();
    let first_us = rows.first().map(|row| row.timestamp_us).unwrap_or(0);
    let last_us = rows.last().map(|row| row.timestamp_us).unwrap_or(0);
    let duration_us = (last_us - first_us).max(0);
    let duration_sec = Some((duration_us / 1_000_000).max(0));

    let mut start_lat = None;
    let mut start_lon = None;
    let mut max_alt_m = None;
    let mut max_speed_ms = None;
    let mut max_mah: Option<u32> = None;
    let mut total_distance_m: f64 = 0.0;
    let mut max_distance_m: f64 = 0.0;
    let mut prev_lat: Option<f64> = None;
    let mut prev_lon: Option<f64> = None;
    let mut telemetry_rows = Vec::with_capacity(rows.len());
    let mut blackbox_rows = Vec::with_capacity(rows.len());

    for row in rows {
        // Use raw GPS for position, nav fused for altitude
        let best_alt = row.telemetry.nav_alt_m.or(row.telemetry.baro_alt_m).or(row.telemetry.alt_m);

        if start_lat.is_none() || start_lon.is_none() {
            if let (Some(lat), Some(lon)) = (row.telemetry.lat, row.telemetry.lon) {
                if is_valid_gps_coord(lat, lon) {
                    start_lat = Some(lat);
                    start_lon = Some(lon);
                }
            }
        }
        if let Some(alt) = best_alt {
            max_alt_m = Some(max_alt_m.map_or(alt, |current: f64| current.max(alt)));
        }
        if let Some(speed) = row.telemetry.speed_ms {
            max_speed_ms = Some(max_speed_ms.map_or(speed, |current: f64| current.max(speed)));
        }
        if let Some(mah) = row.telemetry.mah_drawn {
            max_mah = Some(max_mah.map_or(mah, |current: u32| current.max(mah)));
        }

        // Accumulate total distance and max distance from home
        if let (Some(lat), Some(lon)) = (row.telemetry.lat, row.telemetry.lon) {
            if is_valid_gps_coord(lat, lon) {
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

        blackbox_rows.push((row.timestamp_us, row.csv_data));
        telemetry_rows.push(row.telemetry);
    }

    eprintln!("[BBX-DISTANCE] total_distance_m={:.2}, max_distance_m={:.2}, start_lat={:?}, start_lon={:?}, prev_lat={:?}, prev_lon={:?}",
        total_distance_m, max_distance_m, start_lat, start_lon, prev_lat, prev_lon);

    // Capture before constructing Flight (which moves other header fields out).
    let logged_wp_count = header.logged_wp_count;

    // INAV Blackbox stores the pilot's LOCAL time in the header (it lands in `start_time`'s UTC
    // components without conversion). Resolve the flight-location offset from the start coordinates,
    // then back-compute the true-UTC start so `start_time` is a real absolute instant (ADR-048). This
    // also repairs the 3D real-lighting sun for these imports (it reads `start_time` as true UTC).
    let utc_offset_min = match (start_lat, start_lon) {
        (Some(la), Some(lo)) => super::timezone::offset_min_at(la, lo, start_time),
        _ => None,
    };
    let start_time = match utc_offset_min {
        Some(off) => start_time - Duration::minutes(off as i64),
        None => start_time,
    };
    let end_time = start_time + Duration::microseconds(duration_us);

    // Duplicate check on the *corrected* (true-UTC) start_time — the same value stored below, so a
    // re-import of the same log matches regardless of the header's local-time offset (ADR-048).
    if !force_import {
        report(70, "check-dup", "Checking for duplicate flights...");
        eprintln!("[DUP-CHECK] craft_name={:?}, start_time={}, force_import={}", craft_name, start_time.to_rfc3339(), force_import);
        match db::find_duplicate_flight(conn, &craft_name, start_time) {
            Ok(Some(existing_flight)) => {
                eprintln!("[DUP-CHECK] DUPLICATE FOUND: flight_id={}", existing_flight.id);
                return Ok(BlackboxImportStatus::DuplicateDetected {
                    existing_flight,
                    duplicate_craft_name: craft_name,
                    duplicate_start_time: start_time,
                    duplicate_duration_sec: duration_sec,
                    duplicate_lat: start_lat,
                    duplicate_lon: start_lon,
                });
            }
            Ok(None) => {
                eprintln!("[DUP-CHECK] No duplicate found, proceeding with import");
            }
            Err(e) => {
                eprintln!("[DUP-CHECK] ERROR: {}", e);
                return Err(format!("Duplicate check failed: {}", e));
            }
        }
    } else {
        eprintln!("[DUP-CHECK] Skipped (force_import=true)");
    }

    let flight = Flight {
        id: 0,
        start_time,
        end_time: Some(end_time),
        duration_sec,
        source: "blackbox".into(),
        craft_name: craft_name.clone(),
        fc_variant: "INAV".into(),
        fc_version: header.fc_version.unwrap_or_default(),
        board_id: header.board_id.unwrap_or_default(),
        platform_type: header.platform_type,
        protocol: "BLACKBOX".into(),
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
        battery_serial: None,
        utc_offset_min,
    };

    report(72, "store-flight", "Creating logbook entry...");
    let flight_id = db::insert_flight(conn, &flight)
        .map_err(|e| format!("Failed to create imported Blackbox flight: {}", e))?;

    // Replay `WP N/X` fallback when no mission is linked: persist the header's WP count.
    match logged_wp_count {
        Some(wp_count) => {
            eprintln!("[BBX-HEADER] waypoints count = {} (flight {})", wp_count, flight_id);
            if let Err(e) = db::set_flight_logged_wp_count(conn, flight_id, wp_count) {
                eprintln!("[BBX-HEADER] failed to store logged_wp_count: {}", e);
            }
        }
        None => eprintln!("[BBX-HEADER] no 'waypoints' header found in log"),
    }

    for row in &mut telemetry_rows {
        row.flight_id = flight_id;
    }

    report(82, "store-track", "Storing track data...");
    db::insert_telemetry_batch(conn, &telemetry_rows)
        .map_err(|e| format!("Failed to store imported Blackbox telemetry: {}", e))?;

    report(90, "store-blackbox", "Archiving parsed Blackbox rows...");
    db::insert_blackbox_records(conn, flight_id, &blackbox_rows)
        .map_err(|e| format!("Failed to store Blackbox rows: {}", e))?;

    report(96, "archive", "Archiving original Blackbox file...");
    db::insert_blackbox_file(
        conn,
        flight_id,
        &file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        log_index.unwrap_or(0),
        &file_data,
    )
    .map_err(|e| format!("Failed to archive original Blackbox file: {}", e))?;

    report(100, "done", "Blackbox import complete.");
    Ok(BlackboxImportStatus::Success {
        flight_id,
        rows_imported,
    })
}

fn run_decoder_capture_stdout(
    decoder: &Path,
    file_path: &Path,
    log_index: Option<u32>,
) -> Result<String, String> {
    let mut command = build_decoder_command(decoder, file_path, log_index);
    let output = command
        .output()
        .map_err(|e| format!("Failed to start blackbox_decode: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("blackbox_decode failed: {}", stderr.trim()));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("blackbox_decode returned invalid UTF-8 CSV: {}", e))
}

fn build_decoder_command(decoder: &Path, file_path: &Path, log_index: Option<u32>) -> Command {
    let mut command = Command::new(decoder);
    command
        .arg("--merge-gps")
        .arg("--datetime")
        .arg("--unit-height")
        .arg("m")
        .arg("--unit-gps-speed")
        .arg("mps")
        .arg("--stdout");

    if let Some(index) = log_index {
        // blackbox_decode uses 1-based log indices; our API uses 0-based
        command.arg("--index").arg((index + 1).to_string());
    }

    command.arg(file_path);
    command
}

fn parse_csv_rows(csv_output: &str) -> Result<Vec<ParsedRow>, String> {
    // Target 10 Hz output regardless of raw blackbox rate.
    // Use time-based filtering (not row-count) because blackbox_decode --merge-gps
    // may output at a different rate than the raw header suggests.
    let target_interval_us: i64 = 100_000; // 10 Hz = 100ms

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_output.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| format!("Failed to read Blackbox CSV header: {}", e))?
        .clone();

    // Pre-build column index map and resolve column positions — done once per file.
    let index_map = build_index_map(&headers);
    let cols = ColumnIndices::from_map(&index_map);

    // Debug: log resolved column indices for GPS
    eprintln!("[BBX-COLS] lat={:?}, lon={:?}, alt={:?}, speed={:?}, time={:?}", cols.lat, cols.lon, cols.alt, cols.speed, cols.time);

    let mut rows = Vec::new();
    let mut raw_row_count: usize = 0;
    let mut last_kept_us: i64 = i64::MIN;
    let mut gps_debug_count: usize = 0;

    for record in reader.records() {
        let record = record.map_err(|e| format!("Failed to read Blackbox CSV row: {}", e))?;
        raw_row_count += 1;

        // Extract timestamp cheaply for time-based downsampling decision
        let timestamp_us = cols.time
            .and_then(|i| record.get(i))
            .and_then(|v| parse_loose_i64(v))
            .unwrap_or(raw_row_count as i64);

        // Keep first row always, then only if ≥ target_interval has elapsed
        if !rows.is_empty() && (timestamp_us - last_kept_us) < target_interval_us {
            continue;
        }
        last_kept_us = timestamp_us;

        // Store the raw CSV line (comma-joined) — cheap, no JSON serialization.
        let csv_data: String = record.iter().collect::<Vec<_>>().join(",");

        let telemetry = build_telemetry_record_indexed(&cols, &record, raw_row_count as i64);

        // Debug: log first 5 GPS values to verify parsing
        if gps_debug_count < 5 {
            let raw_lat = cols.lat.and_then(|i| record.get(i)).unwrap_or("<none>");
            let raw_lon = cols.lon.and_then(|i| record.get(i)).unwrap_or("<none>");
            eprintln!("[BBX-GPS-DEBUG] row={} raw_lat={:?} raw_lon={:?} parsed_lat={:?} parsed_lon={:?}",
                raw_row_count, raw_lat, raw_lon, telemetry.lat, telemetry.lon);
            gps_debug_count += 1;
        }

        let timestamp_us = telemetry.timestamp_ms * 1000;
        rows.push(ParsedRow {
            timestamp_us,
            csv_data,
            telemetry,
        });
    }

    eprintln!("[BBX-DOWNSAMPLE] raw_rows={}, kept_rows={}", raw_row_count, rows.len());
    if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        let span_ms = (last.timestamp_us - first.timestamp_us) / 1000;
        let rate_hz = if span_ms > 0 { rows.len() as f64 / (span_ms as f64 / 1000.0) } else { 0.0 };
        eprintln!("[BBX-DOWNSAMPLE] time_span={}ms, effective_rate={:.1}Hz", span_ms, rate_hz);
    }

    // Count distinct GPS positions to diagnose distance=0
    {
        use std::collections::HashSet;
        let mut distinct: HashSet<(i64, i64)> = HashSet::new();
        let mut valid_gps_count = 0usize;
        for r in &rows {
            if let (Some(lat), Some(lon)) = (r.telemetry.lat, r.telemetry.lon) {
                if is_valid_gps_coord(lat, lon) {
                    valid_gps_count += 1;
                    // Use integer representation (nanodegrees) for exact comparison
                    distinct.insert(((lat * 1e9) as i64, (lon * 1e9) as i64));
                }
            }
        }
        eprintln!("[BBX-GPS-STATS] valid_gps_rows={}/{}, distinct_positions={}", valid_gps_count, rows.len(), distinct.len());
    }

    Ok(rows)
}

/// Pre-compute a mapping from normalized header name -> column index.
fn build_index_map(headers: &StringRecord) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(i, h)| (normalize_header(h), i))
        .collect()
}

/// Resolve the first matching column index from a list of candidate names.
#[inline]
fn resolve_col(index_map: &HashMap<String, usize>, names: &[&str]) -> Option<usize> {
    for name in names {
        let key = normalize_header(name);
        if let Some(&idx) = index_map.get(&key) {
            return Some(idx);
        }
    }
    None
}

/// Pre-resolved column indices — built once per CSV file, reused per row.
struct ColumnIndices {
    time: Option<usize>,
    lat: Option<usize>,
    lon: Option<usize>,
    alt: Option<usize>,
    baro_alt: Option<usize>,
    speed: Option<usize>,
    airspeed: Option<usize>,
    heading: Option<usize>,
    vario: Option<usize>,
    gps_vel_d: Option<usize>,
    voltage: Option<usize>,
    current: Option<usize>,
    mah: Option<usize>,
    rssi: Option<usize>,
    roll: Option<usize>,
    pitch: Option<usize>,
    yaw: Option<usize>,
    sats: Option<usize>,
    lq: Option<usize>,
    gps_hdop: Option<usize>,
    gps_eph: Option<usize>,
    gps_epv: Option<usize>,
    active_wp_number: Option<usize>,
    active_flight_mode_flags: Option<usize>,
    state_flags: Option<usize>,
    nav_state: Option<usize>,
    nav_flags: Option<usize>,
    rx_signal_received: Option<usize>,
    hw_health_status: Option<usize>,
    baro_temperature: Option<usize>,
    wind_n: Option<usize>,
    wind_e: Option<usize>,
    wind_d: Option<usize>,
    rc_data: Vec<usize>,
    rc_command: Vec<usize>,
    nav_pos_north: Option<usize>,
    nav_pos_east: Option<usize>,
    nav_pos_up: Option<usize>,
    gps_home_lat: Option<usize>,
    gps_home_lon: Option<usize>,
}

impl ColumnIndices {
    fn from_map(m: &HashMap<String, usize>) -> Self {
        // Time column: "time (us)" normalizes to "time"
        let time = resolve_col(m, &["time"]);

        ColumnIndices {
            time,
            lat: resolve_col(m, &["gps_coord[0]", "gps_lat", "lat"]),
            lon: resolve_col(m, &["gps_coord[1]", "gps_lon", "lon"]),
            // INAV blackbox: altitude in metres with --unit-height m
            alt: resolve_col(m, &["gps_altitude", "altitude", "baroalt_cm"]),
            baro_alt: resolve_col(m, &["baroalt", "baroaltitude", "baro_alt", "baro_altitude"]),
            speed: resolve_col(m, &["gps_speed", "speed"]),
            // INAV logs `airspeed` in cm/s (pitot or virtual, depending on config) → /100 below.
            airspeed: resolve_col(m, &["airspeed", "airspeed_cms", "air_speed"]),
            // The DB `heading` column holds course-over-ground (consistent with the live recorder, which
            // stores gps.course there); the FC fused heading goes to the `yaw` column below. INAV logs
            // COG as "gps_ground_course".
            heading: resolve_col(m, &["gps_ground_course", "gps_cog", "course"]),
            vario: resolve_col(m, &["vario", "vertical_speed"]),
            gps_vel_d: resolve_col(m, &["gps_velned[2]", "gps_velned2", "gps_vertical_speed"]),
            voltage: resolve_col(m, &["vbat", "voltage"]),
            current: resolve_col(m, &["amperage", "current"]),
            mah: resolve_col(m, &["energyCumulative", "mahdrawn", "mah_drawn"]),
            rssi: resolve_col(m, &["rssi"]),
            roll: resolve_col(m, &["roll", "attitude0", "attitude_roll"]),
            pitch: resolve_col(m, &["pitch", "attitude1", "attitude_pitch"]),
            // The DB `yaw` column holds the FC fused heading (INAV's "heading" field is the AHRS heading
            // in decidegrees; fall back to the raw IMU yaw / attitude[2]).
            yaw: resolve_col(m, &["heading", "yaw", "attitude2", "attitude_yaw"]),
            sats: resolve_col(m, &["gps_numsat", "gps_sats"]),
            lq: resolve_col(m, &["lq", "link_quality", "rxlq"]),
            gps_hdop: resolve_col(m, &["gps_hdop"]),
            gps_eph: resolve_col(m, &["gps_eph"]),
            gps_epv: resolve_col(m, &["gps_epv"]),
            active_wp_number: resolve_col(m, &["activewpnumber", "active_wp_number"]),
            active_flight_mode_flags: resolve_col(m, &["activeflightmodeflags", "active_flight_mode_flags"]),
            state_flags: resolve_col(m, &["stateflags", "state_flags"]),
            nav_state: resolve_col(m, &["navstate", "nav_state"]),
            nav_flags: resolve_col(m, &["navflags", "nav_flags"]),
            rx_signal_received: resolve_col(m, &["rxsignalreceived", "rx_signal_received"]),
            hw_health_status: resolve_col(m, &["hwhealthstatus", "hw_health_status"]),
            baro_temperature: resolve_col(m, &["barotemperature", "baro_temperature"]),
            wind_n: resolve_col(m, &["wind[0]", "wind0", "wind_n", "wind_n_ms"]),
            wind_e: resolve_col(m, &["wind[1]", "wind1", "wind_e", "wind_e_ms"]),
            wind_d: resolve_col(m, &["wind[2]", "wind2", "wind_d", "wind_d_ms"]),
            rc_data: resolve_indexed_channels(m, "rcdata"),
            rc_command: resolve_indexed_channels(m, "rccommand"),
            nav_pos_north: resolve_col(m, &["navpos[0]", "navpos0"]),
            nav_pos_east: resolve_col(m, &["navpos[1]", "navpos1"]),
            nav_pos_up: resolve_col(m, &["navpos[2]", "navpos2"]),
            gps_home_lat: resolve_col(m, &["gps_home_lat"]),
            gps_home_lon: resolve_col(m, &["gps_home_lon"]),
        }
    }
}

fn resolve_indexed_channels(m: &HashMap<String, usize>, prefix: &str) -> Vec<usize> {
    let mut channels: Vec<(usize, usize)> = m
        .iter()
        .filter_map(|(name, &index)| {
            if !name.starts_with(prefix) {
                return None;
            }
            let channel = name[prefix.len()..].parse::<usize>().ok()?;
            Some((channel, index))
        })
        .collect();

    channels.sort_by_key(|(channel, _)| *channel);
    channels.into_iter().map(|(_, index)| index).collect()
}

fn read_f64(cols: Option<usize>, record: &StringRecord) -> Option<f64> {
    cols.and_then(|i| record.get(i)).and_then(parse_loose_f64)
}

fn read_i32(cols: Option<usize>, record: &StringRecord) -> Option<i32> {
    read_f64(cols, record).map(|v| v.round() as i32)
}

fn read_i64(cols: Option<usize>, record: &StringRecord) -> Option<i64> {
    read_f64(cols, record).map(|v| v.round() as i64)
}

fn read_u8(cols: Option<usize>, record: &StringRecord) -> Option<u8> {
    read_f64(cols, record).map(|v| v.round() as u8)
}

fn read_json_array(indices: &[usize], record: &StringRecord) -> Option<String> {
    if indices.is_empty() {
        return None;
    }

    let values: Vec<i32> = indices
        .iter()
        .filter_map(|&idx| record.get(idx).and_then(parse_loose_f64).map(|v| v.round() as i32))
        .collect();

    if values.is_empty() {
        None
    } else {
        Some(json!(values).to_string())
    }
}

fn build_telemetry_record_indexed(
    cols: &ColumnIndices,
    record: &StringRecord,
    row_fallback: i64,
) -> TelemetryRecord {

    let timestamp_us = cols
        .time
        .and_then(|i| record.get(i))
        .and_then(|v| parse_loose_i64(v))
        .unwrap_or(row_fallback);

    // Course over ground. blackbox_decode emits gps_ground_course in degrees (decimals preserved).
    // Keep the >360 fallback for any source that hands us decidegrees, but never round (full precision).
    let heading = cols
        .heading
        .and_then(|i| record.get(i))
        .and_then(|v| parse_loose_f64(v))
        .map(|v| if v > 360.0 { v / 10.0 } else { v });

    let alt = read_f64(cols.alt, record).map(|v| if v > 10_000.0 { v / 100.0 } else { v });

    // gps_vel_d is NED "down" velocity in cm/s; negate (positive=climbing) and convert to m/s.
    // The fallback "vario" column from blackbox_decode is also in cm/s.
    let vario_ms = read_f64(cols.gps_vel_d, record).map(|v| -v / 100.0)
        .or_else(|| read_f64(cols.vario, record).map(|v| v / 100.0));

    // INAV blackbox GPS coordinates are stored as degrees × 1e7 (e.g. 511234567 = 51.1234567°).
    // Detect and convert: if |value| > 90 (lat) or > 180 (lon), it's raw integer format.
    let lat = read_f64(cols.lat, record).map(|v| if v.abs() > 90.0 { v / 1e7 } else { v });
    let lon = read_f64(cols.lon, record).map(|v| if v.abs() > 180.0 { v / 1e7 } else { v });

    // Canonical flight mode from the logged INAV flightModeFlags (same bit layout as classify_inav).
    let mode_flags = read_i64(cols.active_flight_mode_flags, record);
    let (mode_primary, mode_modifiers) = match mode_flags {
        Some(f) => {
            let fm = crate::flightmode::classify_inav(f as u32);
            let mods = if fm.modifiers.is_empty() { None } else { Some(fm.modifiers.join(",")) };
            (Some(fm.primary), mods)
        }
        None => (None, None),
    };

    TelemetryRecord {
        id: 0,
        flight_id: 0,
        timestamp_ms: timestamp_us / 1000,
        lat,
        lon,
        alt_m: alt,
        speed_ms: read_f64(cols.speed, record),
        heading,
        vario_ms,
        voltage: read_f64(cols.voltage, record),
        current_a: read_f64(cols.current, record),
        mah_drawn: read_f64(cols.mah, record).map(|v| v.round() as u32),
        rssi: read_f64(cols.rssi, record).map(|v| v.round() as u16),
        battery_percentage: None, // not present in INAV blackbox logs
        // INAV blackbox attitude values (incl. yaw = attitude[2]) are always in decidegrees —
        // unconditionally /10, decimals preserved (0.1° resolution).
        roll: read_f64(cols.roll, record).map(|v| v / 10.0),
        pitch: read_f64(cols.pitch, record).map(|v| v / 10.0),
        yaw: read_f64(cols.yaw, record).map(|v| v / 10.0),
        fix_type: read_f64(cols.sats, record).map(|_| 3u8),
        num_sat: read_f64(cols.sats, record).map(|v| v.round() as u8),
        cpu_load: None,
        link_quality: read_f64(cols.lq, record).map(|v| v.clamp(0.0, 100.0).round() as u8),
        baro_alt_m: read_f64(cols.baro_alt, record),
        gps_hdop: read_f64(cols.gps_hdop, record).map(|v| v / 100.0), // INAV stores as integer*100
        gps_eph: read_f64(cols.gps_eph, record),
        gps_epv: read_f64(cols.gps_epv, record),
        active_wp_number: read_i32(cols.active_wp_number, record),
        active_flight_mode_flags: mode_flags,
        state_flags: read_i64(cols.state_flags, record),
        nav_state: read_i32(cols.nav_state, record),
        nav_flags: read_i64(cols.nav_flags, record),
        rx_signal_received: read_u8(cols.rx_signal_received, record),
        hw_health_status: read_i64(cols.hw_health_status, record),
        baro_temperature: read_f64(cols.baro_temperature, record),
        wind_n_ms: read_f64(cols.wind_n, record),
        wind_e_ms: read_f64(cols.wind_e, record),
        wind_d_ms: read_f64(cols.wind_d, record),
        rc_data_json: read_json_array(&cols.rc_data, record),
        rc_command_json: read_json_array(&cols.rc_command, record),
        // navPos[0,1] are local-frame NE offsets in cm — NOT useful as geographic coords.
        // Only navPos[2] (fused altitude) is used; lat/lon come from raw GPS.
        nav_lat: None,
        nav_lon: None,
        // navPos[2] = fused altitude in cm relative to home, always /100
        nav_alt_m: read_f64(cols.nav_pos_up, record).map(|v| v / 100.0),
        mode_primary,
        mode_modifiers,
        link_snr: None,      // INAV blackbox has no SNR column
        link_rssi_dbm: None, // RSSI is the legacy 0–1023 `rssi` field, not dBm
        // INAV blackbox `airspeed` is cm/s → m/s.
        airspeed_ms: read_f64(cols.airspeed, record).map(|v| v / 100.0),
    }
}

/// Parse `H looptime:<us>` and `H P interval:1/<N>` from the raw log file header,
/// then compute the effective interval between logged samples in microseconds.
///
fn parse_loose_i64(raw: &str) -> Option<i64> {
    let cleaned: String = raw
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-' || *ch == '+')
        .collect();
    cleaned.parse::<i64>().ok()
}

fn parse_loose_f64(raw: &str) -> Option<f64> {
    let cleaned: String = raw
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.'))
        .collect();
    cleaned.parse::<f64>().ok()
}

fn normalize_header(value: &str) -> String {
    // Strip unit suffix like "(m/s)", "(V)", "(mAh)", "(flags)" before normalizing
    let base = value.split('(').next().unwrap_or(value).trim();
    base.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '[' && *ch != ']')
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn extract_header_metadata(file_data: &[u8]) -> HeaderMetadata {
    extract_header_metadata_for_log(file_data, None)
}

/// Extract header metadata for a specific log index within a multi-log Blackbox file.
/// Each log starts with a new set of `H ...` header lines.
/// If log_index is None or 0, reads the first header block.
fn extract_header_metadata_for_log(file_data: &[u8], log_index: Option<u32>) -> HeaderMetadata {
    let text = String::from_utf8_lossy(file_data);
    let target_idx = log_index.unwrap_or(0) as usize;

    // Find all header block start positions.
    // Each log begins with a sequence of "H " lines. The first non-header line after
    // a header block marks the start of data. The next "H " block is the next log.
    let mut header_blocks: Vec<(usize, usize)> = Vec::new();
    let mut block_start: Option<usize> = None;
    let mut last_header_end: usize = 0;

    for (offset, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("H ") {
            if block_start.is_none() {
                block_start = Some(offset);
            }
            last_header_end = offset;
        } else if block_start.is_some() {
            // Non-header line after header block — close this block
            header_blocks.push((block_start.unwrap(), last_header_end));
            block_start = None;
        }
    }
    // Close final block if file ends with header lines
    if let Some(start) = block_start {
        header_blocks.push((start, last_header_end));
    }

    // Collect lines from the target header block
    let lines: Vec<&str> = text.lines().collect();
    let header_text = if target_idx < header_blocks.len() {
        let (start, end) = header_blocks[target_idx];
        lines[start..=end].join("\n")
    } else if !header_blocks.is_empty() {
        // Fallback to first block
        let (start, end) = header_blocks[0];
        lines[start..=end].join("\n")
    } else {
        text.to_string()
    };

    HeaderMetadata {
        craft_name: find_header_value(&header_text, &["Craft name", "Pilot name", "Name"]),
        fc_version: find_header_value(&header_text, &["Firmware revision", "Firmware version"]),
        board_id: find_header_value(&header_text, &["Board information", "Board"]),
        logged_wp_count: parse_waypoint_count(&header_text),
        start_time: find_header_datetime(&header_text),
        platform_type: parse_platform_type(&header_text),
    }
}

/// Heuristic platform type for INAV blackbox: there is no explicit platform header, but the
/// logged field set differs by airframe — fixed-wing logs a single `motor[0]` plus `servo[...]`
/// control surfaces, while multirotors log several `motor[N]`. We read the highest motor index
/// (and servo presence) from the `H Field ... name:` definition lines.
/// Returns 0 = multirotor (default / unknown), 1 = airplane. Display-only; no functional impact.
fn parse_platform_type(text: &str) -> u8 {
    let mut max_motor: i32 = -1;
    let mut has_servo = false;
    for line in text.lines() {
        let l = line.trim();
        if !l.starts_with("H Field") {
            continue;
        }
        if l.contains("servo[") {
            has_servo = true;
        }
        let mut rest = l;
        while let Some(pos) = rest.find("motor[") {
            let after = &rest[pos + 6..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<i32>() {
                if n > max_motor {
                    max_motor = n;
                }
            }
            rest = &after[digits.len()..];
        }
    }
    // >=3 motors → multirotor; a single motor with servos → fixed-wing; otherwise default.
    if max_motor >= 2 {
        0
    } else if max_motor == 0 && has_servo {
        1
    } else {
        0
    }
}

/// Parse `H waypoints:<count>,<flag>` — INAV logs the number of mission waypoints (and a
/// second field, valid/version). Only the count is used (replay `WP N/X` fallback).
fn parse_waypoint_count(text: &str) -> Option<i64> {
    let raw = find_header_value(text, &["waypoints"])?;
    raw.split(',').next()?.trim().parse::<i64>().ok()
}

fn find_header_value(text: &str, labels: &[&str]) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        for label in labels {
            if let Some(rest) = line.strip_prefix(&format!("H {}:", label)) {
                let value = rest.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn find_header_datetime(text: &str) -> Option<DateTime<Utc>> {
    let candidates = ["Log start datetime", "Datetime", "Date"];
    for label in candidates {
        if let Some(value) = find_header_value(text, &[label]) {
            eprintln!("[BBX-DATETIME] raw header '{}' = {:?}", label, value);
            if let Some(parsed) = parse_header_datetime(&value) {
                eprintln!("[BBX-DATETIME] parsed = {}", parsed.to_rfc3339());
                return Some(parsed);
            }
        }
    }
    None
}

fn parse_header_datetime(raw: &str) -> Option<DateTime<Utc>> {
    let formats = [
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%d %H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%d.%m.%Y %H:%M:%S",
    ];
    for format in formats {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(Utc.from_utc_datetime(&parsed));
        }
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn resolve_start_time(header: &HeaderMetadata, file_path: &Path) -> DateTime<Utc> {
    if let Some(start_time) = header.start_time {
        return start_time;
    }
    if let Ok(metadata) = fs::metadata(file_path) {
        if let Ok(modified) = metadata.modified() {
            return DateTime::<Utc>::from(modified);
        }
    }
    Utc::now()
}

/// Locate `blackbox_decode`: next to the app executable, in our auto-download install dir, or on PATH.
/// `pub(crate)` so the on-demand downloader (`decoder.rs`) can report availability.
pub(crate) fn find_decoder() -> Option<PathBuf> {
    let binary_name = if cfg!(target_os = "windows") {
        WINDOWS_BINARY
    } else {
        UNIX_BINARY
    };

    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
    {
        let candidate = exe_dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // The on-demand download installs here (writable; the exe dir may be read-only when installed).
    let installed = super::decoder::install_dir().join(binary_name);
    if installed.is_file() {
        return Some(installed);
    }

    let path_var = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
