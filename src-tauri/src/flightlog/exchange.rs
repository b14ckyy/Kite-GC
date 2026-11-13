// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight data exchange — export/import complete flight records as .kflight files.
// A .kflight file is a self-contained SQLite database with the same schema as the
// main flights database, containing one or more flights with all their data:
// metadata, telemetry records, battery samples, and archived blackbox files.

use std::fmt::Write;
use std::path::Path;

use rusqlite::{params, Connection};

use super::db;
use super::types::{BatteryRecord, Flight, FlightSummary, TelemetryRecord};

/// Schema version stored in exported .kflight files
const KFLIGHT_SCHEMA_VERSION: u32 = 2;

/// Application identifier stored in the export header
const KFLIGHT_APP_ID: &str = "KiteGC";

// ── Export ───────────────────────────────────────────────────────────

/// Export one or more flights (by ID) from the main database into a .kflight file.
/// The output file is a self-contained SQLite database.
pub fn export_flights(
    source_conn: &Connection,
    flight_ids: &[i64],
    output_path: &Path,
) -> Result<usize, String> {
    if flight_ids.is_empty() {
        return Err("No flights selected for export".into());
    }

    // Remove existing file if present (user confirmed overwrite in the save dialog)
    if output_path.exists() {
        std::fs::remove_file(output_path)
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }

    // Create the export database with full schema
    let out = create_export_db(output_path)?;

    // Copy each flight, tracking old→new ID mapping for linked_flight_id remapping
    let mut exported = 0;
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut exported_flights: Vec<Flight> = Vec::new();
    for &flight_id in flight_ids {
        match copy_flight(source_conn, &out, flight_id, false) {
            Ok(new_id) => {
                id_map.insert(flight_id, new_id);
                exported += 1;
                if let Ok(Some(flight)) = db::get_flight(source_conn, flight_id) {
                    exported_flights.push(flight);
                }
            }
            Err(e) => {
                log::warn!("Failed to export flight {}: {}", flight_id, e);
            }
        }
    }

    // Remap linked_flight_id references using the old→new ID mapping
    for (&old_id, &new_id) in &id_map {
        let old_linked: Option<i64> = source_conn
            .query_row(
                "SELECT linked_flight_id FROM flights WHERE id = ?1",
                params![old_id],
                |row| row.get(0),
            )
            .unwrap_or(None);
        if let Some(old_linked_id) = old_linked {
            if let Some(&new_linked_id) = id_map.get(&old_linked_id) {
                out.execute(
                    "UPDATE flights SET linked_flight_id = ?1 WHERE id = ?2",
                    params![new_linked_id, new_id],
                ).ok();
            }
        }
    }

    if exported == 0 {
        // Clean up the empty file
        drop(out);
        std::fs::remove_file(output_path).ok();
        return Err("No flights could be exported".into());
    }

    // A flight of a pair exported without its partner travels as the single flight it was before the
    // link: its own source back, no `source_before_link` (that is only set on a linked flight).
    out.execute(
        &format!("UPDATE flights SET {} WHERE linked_flight_id IS NULL AND source = 'both'", db::END_LINK_SET),
        [],
    )
    .map_err(|e| format!("Failed to unlink the flights exported without their partner: {}", e))?;

    // Kite 1.0 restores a flight's source on unlink from "has rows in `blackbox_records`": one empty
    // marker row per log flight (own source `blackbox`) keeps that right for a pair imported and
    // unlinked there. This version's importer never copies the table's rows.
    out.execute(
        &format!(
            "INSERT INTO blackbox_records (flight_id, timestamp_us, csv_data)
             SELECT id, 0, '' FROM flights
             WHERE source = 'blackbox' OR (source = 'both' AND {} = 'blackbox')",
            db::OWN_SOURCE_SQL
        ),
        [],
    )
    .map_err(|e| format!("Failed to write the blackbox_records markers: {}", e))?;

    // Store export metadata
    out.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS _kflight_meta (key TEXT PRIMARY KEY, value TEXT);
         INSERT OR REPLACE INTO _kflight_meta VALUES ('app', '{}');
         INSERT OR REPLACE INTO _kflight_meta VALUES ('schema_version', '{}');
         INSERT OR REPLACE INTO _kflight_meta VALUES ('exported_at', datetime('now'));
         INSERT OR REPLACE INTO _kflight_meta VALUES ('flight_count', '{}');",
        KFLIGHT_APP_ID, KFLIGHT_SCHEMA_VERSION, exported,
    ))
    .map_err(|e| format!("Failed to write export metadata: {}", e))?;

    // Compact the file
    out.execute_batch("VACUUM;")
        .map_err(|e| format!("VACUUM failed: {}", e))?;

    // Export a human-readable summary sidecar next to the .kflight file.
    // Example: flights_export.kflight -> flights_export.txt
    write_export_info_sidecar(output_path, &exported_flights)?;

    log::info!(
        "Exported {} flight(s) to {}",
        exported,
        output_path.display()
    );
    Ok(exported)
}

fn write_export_info_sidecar(output_path: &Path, flights: &[Flight]) -> Result<(), String> {
    let txt_path = output_path.with_extension("txt");
    let mut text = String::new();

    writeln!(&mut text, "KiteGC Flight Export Info").ok();
    writeln!(&mut text, "=======================").ok();
    writeln!(&mut text, "Export file: {}", output_path.display()).ok();
    writeln!(&mut text, "Exported at (UTC): {}", chrono::Utc::now().to_rfc3339()).ok();
    writeln!(&mut text, "Flight count: {}", flights.len()).ok();
    writeln!(&mut text).ok();

    for (idx, f) in flights.iter().enumerate() {
        writeln!(&mut text, "[Flight {}]", idx + 1).ok();
        writeln!(&mut text, "ID: {}", f.id).ok();
        writeln!(&mut text, "Start: {}", f.start_time.to_rfc3339()).ok();
        writeln!(
            &mut text,
            "End: {}",
            f.end_time
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| "-".to_string())
        )
        .ok();
        writeln!(&mut text, "Duration sec: {}", f.duration_sec.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())).ok();
        writeln!(&mut text, "Source: {}", f.source).ok();
        writeln!(&mut text, "Craft: {}", f.craft_name).ok();
        writeln!(&mut text, "FC: {} {}", f.fc_variant, f.fc_version).ok();
        writeln!(&mut text, "Board: {}", f.board_id).ok();
        writeln!(&mut text, "Location: {}", f.location_name.clone().unwrap_or_else(|| "-".to_string())).ok();
        writeln!(&mut text, "Max alt m: {}", f.max_alt_m.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string())).ok();
        writeln!(&mut text, "Max speed m/s: {}", f.max_speed_ms.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string())).ok();
        writeln!(&mut text, "Total distance m: {}", f.total_distance_m.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "-".to_string())).ok();
        writeln!(
            &mut text,
            "Weather: temp={} wind={} wind_deg={} desc={}",
            f.weather_temp_c.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".to_string()),
            f.weather_wind_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".to_string()),
            f.weather_wind_deg.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
            f.weather_desc.clone().unwrap_or_else(|| "-".to_string())
        )
        .ok();
        writeln!(&mut text, "Linked flight ID: {}", f.linked_flight_id.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())).ok();
        writeln!(&mut text, "Notes:").ok();
        writeln!(
            &mut text,
            "{}",
            f.notes
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "-".to_string())
        )
        .ok();
        writeln!(&mut text).ok();
    }

    std::fs::write(&txt_path, text)
        .map_err(|e| format!("Failed to write export info TXT '{}': {}", txt_path.display(), e))?;
    Ok(())
}

/// Create a new SQLite database with the full flight log schema.
fn create_export_db(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to create export file: {}", e))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| format!("Pragma error: {}", e))?;

    // The flight-data tables of the main DB schema (flights, telemetry, battery samples, archived
    // files; not the libraries, groups or events). `flights.vehicle_id` stays NULL in an export: the
    // vehicle library is local, an import resolves the flight again (`copy_flight`).
    // `blackbox_records` is gone from the main DB since v21, but the file keeps the table: the importer
    // of Kite 1.0 reads it for every flight and fails the flight when it is missing. It holds only one
    // empty marker row per log flight (`export_flights`), never decoded rows.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS flights (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            start_time      TEXT NOT NULL,
            end_time        TEXT,
            duration_sec    INTEGER,
            source          TEXT NOT NULL DEFAULT 'live',
            craft_name      TEXT NOT NULL DEFAULT '',
            fc_variant      TEXT NOT NULL DEFAULT '',
            fc_version      TEXT NOT NULL DEFAULT '',
            board_id        TEXT NOT NULL DEFAULT '',
            platform_type   INTEGER NOT NULL DEFAULT 0,
            protocol        TEXT NOT NULL DEFAULT 'MSP',
            start_lat       REAL,
            start_lon       REAL,
            location_name   TEXT,
            weather_temp_c  REAL,
            weather_wind_ms REAL,
            weather_wind_deg INTEGER,
            weather_desc    TEXT,
            max_alt_m       REAL,
            max_speed_ms    REAL,
            max_distance_m  REAL,
            total_distance_m REAL,
            battery_used_mah INTEGER,
            notes           TEXT,
            linked_flight_id INTEGER REFERENCES flights(id),
            pilot_name      TEXT,
            pilot_id        TEXT,
            battery_serial  TEXT,
            utc_offset_min  INTEGER,
            fc_uid          TEXT,
            group_id        TEXT,
            vehicle_id      INTEGER,
            source_before_link TEXT
        );

        CREATE TABLE IF NOT EXISTS telemetry_records (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            flight_id    INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
            timestamp_ms INTEGER NOT NULL,
            lat          REAL,
            lon          REAL,
            alt_m        REAL,
            speed_ms     REAL,
            heading      INTEGER,
            vario_ms     REAL,
            voltage      REAL,
            current_a    REAL,
            mah_drawn    INTEGER,
            rssi         INTEGER,
            roll         REAL,
            pitch        REAL,
            yaw          INTEGER,
            fix_type     INTEGER,
            num_sat      INTEGER,
            cpu_load     INTEGER,
            link_quality INTEGER,
            baro_alt_m   REAL,
            gps_hdop     REAL,
            gps_eph      REAL,
            gps_epv      REAL,
            active_wp_number INTEGER,
            active_flight_mode_flags INTEGER,
            state_flags  INTEGER,
            nav_state    INTEGER,
            nav_flags    INTEGER,
            rx_signal_received INTEGER,
            hw_health_status INTEGER,
            baro_temperature REAL,
            wind_n_ms    REAL,
            wind_e_ms    REAL,
            wind_d_ms    REAL,
            rc_data_json TEXT,
            rc_command_json TEXT,
            nav_lat      REAL,
            nav_lon      REAL,
            nav_alt_m    REAL,
            battery_percentage INTEGER,
            mode_primary TEXT,
            mode_modifiers TEXT,
            link_snr     INTEGER,
            link_rssi_dbm INTEGER,
            airspeed_ms  REAL,
            throttle_pct REAL,
            wall_ms      INTEGER
        );

        CREATE TABLE IF NOT EXISTS battery_records (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            flight_id     INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
            timestamp_ms  INTEGER NOT NULL,
            instance      INTEGER NOT NULL,
            voltage       REAL,
            current_a     REAL,
            mah_drawn     INTEGER,
            battery_percentage INTEGER,
            cell_count    INTEGER,
            temperature   REAL
        );

        CREATE TABLE IF NOT EXISTS blackbox_records (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            flight_id     INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
            timestamp_us  INTEGER NOT NULL,
            csv_data      TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS blackbox_files (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            flight_id         INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
            original_filename TEXT NOT NULL,
            log_index         INTEGER NOT NULL DEFAULT 0,
            file_data         BLOB NOT NULL,
            file_size         INTEGER NOT NULL,
            imported_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_telemetry_flight
            ON telemetry_records(flight_id, timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_battery_flight
            ON battery_records(flight_id, timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_blackbox_records_flight
            ON blackbox_records(flight_id, timestamp_us);
        CREATE INDEX IF NOT EXISTS idx_blackbox_files_flight
            ON blackbox_files(flight_id);",
    )
    .map_err(|e| format!("Schema creation error: {}", e))?;

    Ok(conn)
}

/// Copy a single flight with all related data from source to destination DB. Always across databases
/// (export: main → `.kflight`, `resolve_vehicle` false; import: `.kflight` → main, true). The source's
/// `vehicle_id` names a vehicle of the source library, so it never travels: an export stores none, an
/// import resolves the flight against the target's library (`db::insert_flight`).
fn copy_flight(
    src: &Connection,
    dst: &Connection,
    flight_id: i64,
    resolve_vehicle: bool,
) -> Result<i64, String> {
    // 1. Read the flight row
    let mut flight = db::get_flight(src, flight_id)
        .map_err(|e| format!("Read flight: {}", e))?
        .ok_or_else(|| format!("Flight {} not found", flight_id))?;
    // A v2 `.kflight` carries no `flight_groups` rows, so a group reference could not resolve on the
    // other side (FK). Group bundles are the `.kflight` v3 step (GROUP_FLIGHTS.md §3.7); until then a
    // member travels as a single flight.
    flight.group_id = None;
    flight.vehicle_id = None;
    let before_link = source_before_link(src, flight_id, &flight)?;

    // 2. Insert into destination (gets a new ID)
    let inserted = if resolve_vehicle {
        db::insert_flight(dst, &flight)
    } else {
        db::insert_flight_unresolved(dst, &flight)
    };
    let new_id = inserted.map_err(|e| format!("Insert flight: {}", e))?;

    // Finalize with stats + weather (insert_flight doesn't copy all computed fields)
    dst.execute(
        "UPDATE flights SET
            end_time = ?1, duration_sec = ?2,
            max_alt_m = ?3, max_speed_ms = ?4, max_distance_m = ?5,
            total_distance_m = ?6, battery_used_mah = ?7,
            location_name = ?8, weather_temp_c = ?9, weather_wind_ms = ?10,
            weather_wind_deg = ?11, weather_desc = ?12, source_before_link = ?14
         WHERE id = ?13",
        params![
            flight.end_time.map(|t| t.to_rfc3339()),
            flight.duration_sec,
            flight.max_alt_m,
            flight.max_speed_ms,
            flight.max_distance_m,
            flight.total_distance_m,
            flight.battery_used_mah,
            flight.location_name,
            flight.weather_temp_c,
            flight.weather_wind_ms,
            flight.weather_wind_deg,
            flight.weather_desc,
            new_id,
            before_link,
        ],
    )
    .map_err(|e| format!("Finalize exported flight: {}", e))?;

    // 3. Copy telemetry records
    let track = db::get_flight_track(src, flight_id)
        .map_err(|e| format!("Read telemetry: {}", e))?;
    if !track.is_empty() {
        let remapped: Vec<TelemetryRecord> = track
            .into_iter()
            .map(|mut r| {
                r.flight_id = new_id;
                r
            })
            .collect();
        db::insert_telemetry_batch(dst, &remapped)
            .map_err(|e| format!("Insert telemetry: {}", e))?;
    }

    // 3b. Copy per-instance battery samples (multi-battery)
    let bat = db::get_flight_battery_records(src, flight_id)
        .map_err(|e| format!("Read battery records: {}", e))?;
    if !bat.is_empty() {
        let remapped: Vec<BatteryRecord> = bat
            .into_iter()
            .map(|mut r| {
                r.flight_id = new_id;
                r
            })
            .collect();
        db::insert_battery_records_batch(dst, &remapped)
            .map_err(|e| format!("Insert battery records: {}", e))?;
    }

    // 4. Copy blackbox files (BLOBs) — the archived originals (INAV blackbox logs, ArduPilot
    //    dataflash). Not on mobile: the mobile database never archives originals (ANDROID_SUPPORT.md
    //    §4 — nothing there could decode them, and dataflash alone runs to hundreds of megabytes), so
    //    an imported .kflight brings the flight, its track and its records and leaves the blobs
    //    behind. The desktop copy it came from still has them.
    if cfg!(any(target_os = "android", target_os = "ios")) {
        log::info!("Import: flight {flight_id} — archived original log files are not stored on mobile");
    } else {
        copy_blackbox_files(src, dst, flight_id, new_id)?;
    }

    Ok(new_id)
}

/// The flight's own source behind a `both` (`flights.source_before_link`, schema v21), read from `src`
/// — the main DB on export, the file on import. A `.kflight` written before v21 has no such column
/// (`add_missing_columns` deliberately does not add it): there it is derived for a `both` flight the
/// way the v21 back-fill does — `blackbox` when a log importer made it (`protocol`), or the file holds
/// rows in `blackbox_records` (decoded rows up to v20, or the marker of a newer export) or an archived
/// log for it, `live` otherwise.
fn source_before_link(src: &Connection, flight_id: i64, flight: &Flight) -> Result<Option<String>, String> {
    let has_rows = |table: &str| -> rusqlite::Result<bool> {
        // `table` is always a hardcoded literal here, so the format! is injection-safe.
        src.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE flight_id = ?1)"),
            params![flight_id],
            |row| row.get(0),
        )
    };
    let resolve = || -> rusqlite::Result<Option<String>> {
        if db::column_exists(src, "flights", "source_before_link")? {
            return src.query_row(
                "SELECT source_before_link FROM flights WHERE id = ?1",
                params![flight_id],
                |row| row.get(0),
            );
        }
        if flight.source != "both" {
            return Ok(None);
        }
        let log = db::is_log_protocol(&flight.protocol)
            || (db::table_exists(src, "blackbox_records")? && has_rows("blackbox_records")?)
            || has_rows("blackbox_files")?;
        Ok(Some(if log { "blackbox" } else { "live" }.to_string()))
    };
    resolve().map_err(|e| format!("Read source_before_link: {}", e))
}

fn copy_blackbox_files(
    src: &Connection,
    dst: &Connection,
    old_flight_id: i64,
    new_flight_id: i64,
) -> Result<(), String> {
    let mut stmt = src
        .prepare(
            "SELECT original_filename, log_index, file_data, file_size, imported_at
             FROM blackbox_files WHERE flight_id = ?1",
        )
        .map_err(|e| format!("Prepare blackbox_files: {}", e))?;

    let files: Vec<(String, i32, Vec<u8>, i64, String)> = stmt
        .query_map(params![old_flight_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| format!("Query blackbox_files: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Read blackbox_files: {}", e))?;

    for (filename, log_index, file_data, file_size, imported_at) in files {
        dst.execute(
            "INSERT INTO blackbox_files (flight_id, original_filename, log_index, file_data, file_size, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![new_flight_id, filename, log_index, file_data, file_size, imported_at],
        )
        .map_err(|e| format!("Insert blackbox_file: {}", e))?;
    }

    Ok(())
}

// ── Import ──────────────────────────────────────────────────────────

/// Result of importing flights from a .kflight file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Import flights from a .kflight file into the main database.
/// Skips duplicates (same craft_name + start_time ±10s).
pub fn import_flights(
    target_conn: &Connection,
    kflight_path: &Path,
) -> Result<ImportResult, String> {
    if !kflight_path.exists() {
        return Err("File not found".into());
    }
    with_kflight_copy(kflight_path, |src| import_from(target_conn, src, kflight_path))
}

/// `import_flights` on the open working copy `src` (`kflight_path` names the user's file in the log).
fn import_from(
    target_conn: &Connection,
    src: &Connection,
    kflight_path: &Path,
) -> Result<ImportResult, String> {
    // Verify it's a valid kflight file
    let _has_meta: bool = src
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='_kflight_meta'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);

    let has_flights: bool = src
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='flights'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);

    if !has_flights {
        return Err("Not a valid .kflight file: missing flights table".into());
    }
    add_missing_columns(src);

    // List all flights in the source file
    let src_flights = db::list_flights(src)
        .map_err(|e| format!("Failed to list flights in .kflight: {}", e))?;

    // Snapshot target flights BEFORE import starts.
    // Duplicate detection must only compare against pre-existing flights,
    // otherwise linked pairs inside the same .kflight will self-conflict.
    let baseline_target_flights = db::list_flights(target_conn)
        .map_err(|e| format!("Failed to list existing target flights: {}", e))?;

    // old source ID -> target ID (newly imported OR matched existing duplicate)
    let mut id_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut imported_ids: Vec<i64> = Vec::new();

    let mut result = ImportResult {
        imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    for summary in &src_flights {
        // Check duplicate only against baseline target flights.
        if let Some(existing_id) = find_duplicate_in_summaries(&baseline_target_flights, summary) {
            result.skipped += 1;
            id_map.insert(summary.id, existing_id);
            log::info!("Import: skipping duplicate flight {} ({})", summary.id, summary.start_time);
            continue;
        }

        match copy_flight(src, target_conn, summary.id, true) {
            Ok(new_id) => {
                result.imported += 1;
                id_map.insert(summary.id, new_id);
                imported_ids.push(new_id);
            }
            Err(e) => {
                log::warn!("Import: failed to import flight {}: {}", summary.id, e);
                result.errors.push(format!("Flight {}: {}", summary.id, e));
            }
        }
    }

    // Restore linked relationships from source file using old->target ID mapping.
    // This also supports mixed cases where one side was skipped as duplicate and
    // mapped to an existing target flight.
    let mut linked_pairs_done: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    for summary in &src_flights {
        let Some(old_linked_id) = summary.linked_flight_id else {
            continue;
        };
        let Some(&target_a) = id_map.get(&summary.id) else {
            continue;
        };
        let Some(&target_b) = id_map.get(&old_linked_id) else {
            continue;
        };
        if target_a == target_b {
            continue;
        }

        let pair = if target_a < target_b {
            (target_a, target_b)
        } else {
            (target_b, target_a)
        };
        if linked_pairs_done.contains(&pair) {
            continue;
        }

        if let Err(e) = db::link_flights(target_conn, pair.0, pair.1) {
            log::warn!("Import: failed to relink flights {} <-> {}: {}", pair.0, pair.1, e);
            result
                .errors
                .push(format!("Relink {}<->{}: {}", pair.0, pair.1, e));
        } else {
            linked_pairs_done.insert(pair);
        }
    }

    // An imported flight that arrived as `both` but ended up without a partner (a file from before the
    // export unlinked such flights, or the relink failed) becomes the single flight it was before the
    // link — `source_before_link` as the file gave or `copy_flight` derived it.
    let end_link = format!(
        "UPDATE flights SET {} WHERE id = ?1 AND linked_flight_id IS NULL AND source = 'both'",
        db::END_LINK_SET
    );
    for id in imported_ids {
        if let Err(e) = target_conn.execute(&end_link, params![id]) {
            log::warn!("Import: failed to unlink flight {} without a partner: {}", id, e);
            result.errors.push(format!("Unlink {}: {}", id, e));
        }
    }

    log::info!(
        "Imported {} flight(s) from {} ({} skipped, {} errors)",
        result.imported,
        kflight_path.display(),
        result.skipped,
        result.errors.len(),
    );

    Ok(result)
}

/// Run `f` on a private working copy of the `.kflight` at `path` (in the app's temp dir, deleted
/// afterwards). The callers back-fill missing columns (`add_missing_columns`) and SQLite may create
/// `-wal`/`-shm` files next to the database, so the user's file — possibly read-only or on a share —
/// is never opened itself and never modified.
fn with_kflight_copy<T>(
    path: &Path,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let copy = crate::user_file::temp_path("kflight")?;
    if let Err(e) = std::fs::copy(path, &copy) {
        db::remove_temp_session(&copy); // a partial copy
        return Err(format!("Failed to open .kflight file: {}", e));
    }
    // A WAL left by an unclean close holds committed pages of its own — bring it along.
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    if wal.exists() {
        let mut copy_wal = copy.as_os_str().to_os_string();
        copy_wal.push("-wal");
        if let Err(e) = std::fs::copy(&wal, std::path::PathBuf::from(copy_wal)) {
            db::remove_temp_session(&copy);
            return Err(format!("Failed to open .kflight file: {}", e));
        }
    }
    let result = Connection::open(&copy)
        .map_err(|e| format!("Failed to open .kflight file: {}", e))
        .and_then(|conn| f(&conn));
    db::remove_temp_session(&copy); // the copy + its -wal / -shm
    result
}

/// A `.kflight` written by an older Kite lacks the columns added since (flights `fc_uid` v19,
/// `group_id` + `vehicle_id` v20; telemetry `wall_ms` v20). Add them so the shared row readers apply.
/// `flights.source_before_link` (v21) is NOT added: its absence is what marks a file whose value has
/// to be derived (`source_before_link`).
fn add_missing_columns(conn: &Connection) {
    for (table, column, ddl) in [
        ("flights", "fc_uid", "ALTER TABLE flights ADD COLUMN fc_uid TEXT;"),
        ("flights", "group_id", "ALTER TABLE flights ADD COLUMN group_id TEXT;"),
        ("flights", "vehicle_id", "ALTER TABLE flights ADD COLUMN vehicle_id INTEGER;"),
        ("telemetry_records", "wall_ms", "ALTER TABLE telemetry_records ADD COLUMN wall_ms INTEGER;"),
    ] {
        if !db::column_exists(conn, table, column).unwrap_or(true) {
            if let Err(e) = conn.execute_batch(ddl) {
                log::warn!(".kflight: could not add {table}.{column}: {e}");
            }
        }
    }
}

fn find_duplicate_in_summaries(
    baseline: &[FlightSummary],
    incoming: &FlightSummary,
) -> Option<i64> {
    let lower = incoming.start_time - chrono::Duration::seconds(10);
    let upper = incoming.start_time + chrono::Duration::seconds(10);

    baseline
        .iter()
        // Keep behavior aligned with find_duplicate_flight(): only blackbox/both are duplicate sources.
        .filter(|f| f.source == "blackbox" || f.source == "both")
        .find(|f| {
            f.craft_name == incoming.craft_name
                && f.start_time >= lower
                && f.start_time <= upper
        })
        .map(|f| f.id)
}

// ── Read from .kflight (for offline replay) ─────────────────────────

/// List flights in a .kflight file (for offline replay / preview)
pub fn list_flights_in_file(path: &Path) -> Result<Vec<FlightSummary>, String> {
    with_kflight_copy(path, |conn| {
        add_missing_columns(conn);
        db::list_flights(conn).map_err(|e| format!("Query error: {}", e))
    })
}

/// Get a single flight from a .kflight file
pub fn get_flight_from_file(path: &Path, flight_id: i64) -> Result<Option<Flight>, String> {
    with_kflight_copy(path, |conn| {
        add_missing_columns(conn);
        db::get_flight(conn, flight_id).map_err(|e| format!("Query error: {}", e))
    })
}

/// Get the telemetry track from a .kflight file
pub fn get_track_from_file(path: &Path, flight_id: i64) -> Result<Vec<TelemetryRecord>, String> {
    with_kflight_copy(path, |conn| {
        add_missing_columns(conn);
        db::get_flight_track(conn, flight_id).map_err(|e| format!("Query error: {}", e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-kflight-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn vehicle(conn: &Connection, name: &str, craft: Option<&str>, uid: Option<&str>) -> i64 {
        conn.execute(
            "INSERT INTO vehicles (name, craft_name, fc_uid) VALUES (?1, ?2, ?3)",
            params![name, craft, uid],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn flight(craft: &str, uid: Option<&str>) -> Flight {
        Flight {
            id: 0,
            start_time: chrono::DateTime::parse_from_rfc3339("2026-11-01T10:00:00Z").unwrap().with_timezone(&chrono::Utc),
            end_time: None,
            duration_sec: Some(60),
            source: "live".into(),
            craft_name: craft.into(),
            fc_variant: "INAV".into(),
            fc_version: "9.0.0".into(),
            board_id: String::new(),
            platform_type: 1,
            protocol: "MSP".into(),
            start_lat: Some(48.1),
            start_lon: Some(11.5),
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            max_alt_m: None,
            max_speed_ms: None,
            max_distance_m: None,
            total_distance_m: None,
            battery_used_mah: None,
            notes: None,
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
            fc_uid: uid.map(String::from),
            group_id: None,
            vehicle_id: None,
        }
    }

    /// The local vehicle link never travels in a `.kflight`: the export stores none, the import resolves
    /// the flight against the target's library (FC id first) — also for a file written before v20.
    #[test]
    fn kflight_round_trip_resolves_the_vehicle_in_the_target_library() {
        let dir = temp_dir("vehicle");
        let file = dir.join("export.kflight");
        {
            let src = db::open_database(&dir.join("src").join("flights.db")).unwrap();
            let src_vehicle = vehicle(&src, "Wing", Some("Wing"), Some("UIDA"));
            let id = db::insert_flight(&src, &flight("Wing", Some("UIDA"))).unwrap();
            assert_eq!(db::get_flight(&src, id).unwrap().unwrap().vehicle_id, Some(src_vehicle));
            assert_eq!(export_flights(&src, &[id], &file).unwrap(), 1);
        }
        {
            let k = Connection::open(&file).unwrap();
            let v: Option<i64> = k.query_row("SELECT vehicle_id FROM flights", [], |r| r.get(0)).unwrap();
            assert_eq!(v, None, "the export carries no local vehicle id");
        }

        {
            let dst = db::open_database(&dir.join("dst").join("flights.db")).unwrap();
            vehicle(&dst, "Filler A", Some("Quad"), None);
            vehicle(&dst, "Filler B", Some("Plane"), None);
            let by_craft = vehicle(&dst, "Same name", Some("wing"), None);
            let same_fc = vehicle(&dst, "Same FC", None, Some("uida"));
            let r = import_flights(&dst, &file).unwrap();
            assert_eq!((r.imported, r.errors.len()), (1, 0));
            let imported = db::list_flights(&dst).unwrap();
            assert_eq!(imported.len(), 1);
            assert_eq!(imported[0].vehicle_id, Some(same_fc), "the FC id beats the craft name ({by_craft})");
            let f = db::get_flight(&dst, imported[0].id).unwrap().unwrap();
            assert_eq!((f.craft_name.as_str(), f.fc_uid.as_deref()), ("Wing", Some("UIDA")));
        }
        // When unsure, unlinked: only a craft-name vehicle that stores another FC id → no link.
        {
            let dst = db::open_database(&dir.join("dst-conflict").join("flights.db")).unwrap();
            vehicle(&dst, "Other FC", Some("Wing"), Some("UIDZ"));
            let r = import_flights(&dst, &file).unwrap();
            assert_eq!((r.imported, r.errors.len()), (1, 0));
            assert_eq!(db::list_flights(&dst).unwrap()[0].vehicle_id, None);
        }

        // A `.kflight` from before v20 has no vehicle_id column: listed, imported, resolved.
        {
            let k = Connection::open(&file).unwrap();
            k.execute_batch("ALTER TABLE flights DROP COLUMN vehicle_id;").unwrap();
        }
        assert_eq!(list_flights_in_file(&file).unwrap()[0].vehicle_id, None);
        {
            let dst = db::open_database(&dir.join("dst-old-file").join("flights.db")).unwrap();
            let by_craft = vehicle(&dst, "By craft", Some(" WING "), None);
            let r = import_flights(&dst, &file).unwrap();
            assert_eq!((r.imported, r.errors.len()), (1, 0));
            assert_eq!(db::list_flights(&dst).unwrap()[0].vehicle_id, Some(by_craft));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn source_of(conn: &Connection, flight_id: i64) -> (String, Option<String>) {
        conn.query_row(
            "SELECT source, source_before_link FROM flights WHERE id = ?1",
            params![flight_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    /// Imported flights by start minute → id (each test flight starts at its own minute).
    fn by_minute(conn: &Connection) -> std::collections::HashMap<u32, i64> {
        use chrono::Timelike;
        db::list_flights(conn).unwrap().iter().map(|f| (f.start_time.minute(), f.id)).collect()
    }

    /// A flight in the `src` main DB, starting at `hh:mm` (tests tell flights apart by start minute).
    fn at(src: &Connection, hour: u32, minute: u32, source: &str, protocol: &str) -> i64 {
        let start = chrono::DateTime::parse_from_rfc3339(&format!("2026-11-01T{hour:02}:{minute:02}:00Z"))
            .unwrap()
            .with_timezone(&chrono::Utc);
        let f = Flight { start_time: start, source: source.into(), protocol: protocol.into(), ..flight("Wing", None) };
        db::insert_flight(src, &f).unwrap()
    }

    /// The file's `blackbox_records` rows as (flight id, timestamp_us, csv_data).
    fn records_in(k: &Connection) -> Vec<(i64, i64, String)> {
        k.prepare("SELECT flight_id, timestamp_us, csv_data FROM blackbox_records ORDER BY flight_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// Two linked pairs exported in one `.kflight`: pair 1's log flight has an archived original, pair
    /// 2's has none (an import whose original was deleted). The file carries `source_before_link` and
    /// one empty marker row in `blackbox_records` per log flight (for Kite 1.0's unlink); the import
    /// keeps both values, never copies the markers, and an unlink in the target restores each flight's
    /// own source.
    #[test]
    fn kflight_round_trip_keeps_source_before_link() {
        let dir = temp_dir("source-before-link");
        let file = dir.join("pairs.kflight");
        {
            let src = db::open_database(&dir.join("src").join("flights.db")).unwrap();
            let (live_1, log_1) = (at(&src, 10, 1, "live", "MSP"), at(&src, 10, 2, "blackbox", "MSP"));
            let (live_2, log_2) = (at(&src, 10, 3, "live", "MSP"), at(&src, 10, 4, "blackbox", "MSP"));
            db::insert_blackbox_file(&src, log_1, "LOG00001.TXT", 0, b"txt").unwrap();
            db::link_flights(&src, live_1, log_1).unwrap();
            db::link_flights(&src, live_2, log_2).unwrap();
            assert_eq!(export_flights(&src, &[live_1, log_1, live_2, log_2], &file).unwrap(), 4);
        }
        {
            let k = Connection::open(&file).unwrap();
            let m = by_minute(&k);
            assert_eq!(
                records_in(&k),
                [(m[&2], 0, String::new()), (m[&4], 0, String::new())],
                "exactly one empty marker row per log flight, none for the live flights"
            );
            assert_eq!(source_of(&k, m[&2]), ("both".into(), Some("blackbox".into())));
            assert_eq!(source_of(&k, m[&4]), ("both".into(), Some("blackbox".into())));
            assert_eq!(source_of(&k, m[&3]), ("both".into(), Some("live".into())));
        }
        let dst = db::open_database(&dir.join("dst").join("flights.db")).unwrap();
        let r = import_flights(&dst, &file).unwrap();
        assert_eq!((r.imported, r.errors.len()), (4, 0));
        let m = by_minute(&dst);
        for (live, log) in [(m[&1], m[&2]), (m[&3], m[&4])] {
            assert_eq!(db::get_flight(&dst, live).unwrap().unwrap().linked_flight_id, Some(log));
            assert_eq!(source_of(&dst, live), ("both".into(), Some("live".into())));
            assert_eq!(source_of(&dst, log), ("both".into(), Some("blackbox".into())));
            db::unlink_flight(&dst, live).unwrap();
            assert_eq!(source_of(&dst, live), ("live".into(), None));
            assert_eq!(source_of(&dst, log), ("blackbox".into(), None), "flight {log}");
        }
        assert!(!db::table_exists(&dst, "blackbox_records").unwrap());
        drop(dst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A flight of a pair exported without its partner travels as the single flight it was: its own
    /// source, no `source_before_link`, no link — and the marker row only if it is the log flight.
    #[test]
    fn flight_exported_without_its_partner_is_unlinked_in_the_file() {
        let dir = temp_dir("lone-export");
        let (log_file, live_file) = (dir.join("log.kflight"), dir.join("live.kflight"));
        {
            let src = db::open_database(&dir.join("src").join("flights.db")).unwrap();
            let (live_1, log_1) = (at(&src, 12, 1, "live", "MSP"), at(&src, 12, 2, "blackbox", "DATAFLASH"));
            let (live_2, log_2) = (at(&src, 12, 3, "live", "MSP"), at(&src, 12, 4, "blackbox", "BLACKBOX"));
            db::link_flights(&src, live_1, log_1).unwrap();
            db::link_flights(&src, live_2, log_2).unwrap();
            assert_eq!(export_flights(&src, &[log_1], &log_file).unwrap(), 1);
            assert_eq!(export_flights(&src, &[live_2], &live_file).unwrap(), 1);
            // The main DB keeps its pairs.
            assert_eq!(source_of(&src, log_1), ("both".into(), Some("blackbox".into())));
        }
        {
            let k = Connection::open(&log_file).unwrap();
            let id = by_minute(&k)[&2];
            assert_eq!(source_of(&k, id), ("blackbox".into(), None));
            assert_eq!(db::get_flight(&k, id).unwrap().unwrap().linked_flight_id, None);
            assert_eq!(records_in(&k), [(id, 0, String::new())]);
            let k = Connection::open(&live_file).unwrap();
            assert_eq!(source_of(&k, by_minute(&k)[&3]), ("live".into(), None));
            assert!(records_in(&k).is_empty());
        }
        let dst = db::open_database(&dir.join("dst").join("flights.db")).unwrap();
        for f in [&log_file, &live_file] {
            let r = import_flights(&dst, f).unwrap();
            assert_eq!((r.imported, r.errors.len()), (1, 0));
        }
        let m = by_minute(&dst);
        assert_eq!(source_of(&dst, m[&2]), ("blackbox".into(), None));
        assert_eq!(source_of(&dst, m[&3]), ("live".into(), None));
        drop(dst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `.kflight` from before v21 has no `source_before_link` column: the import derives it for the
    /// `both` flights from the FILE — a log importer's protocol, rows in its `blackbox_records` (real
    /// decoded rows and a marker row alike) or an archived log mean `blackbox` — and never copies those
    /// rows into the main database. A `both` flight that arrives without its partner (such a file
    /// exported one flight of a pair alone) is imported as the single flight it was.
    #[test]
    fn old_kflight_import_derives_source_before_link() {
        let dir = temp_dir("old-source");
        let file = dir.join("old.kflight");
        {
            let src = db::open_database(&dir.join("src").join("flights.db")).unwrap();
            let (live_1, log_1) = (at(&src, 11, 1, "live", "MSP"), at(&src, 11, 2, "blackbox", "MSP"));
            let (live_2, log_2) = (at(&src, 11, 3, "live", "MSP"), at(&src, 11, 4, "blackbox", "MSP"));
            let lone = at(&src, 11, 5, "blackbox", "MSP");
            let (live_3, log_3) = (at(&src, 11, 6, "live", "MSP"), at(&src, 11, 7, "blackbox", "MSP"));
            let (live_4, log_4) = (at(&src, 11, 8, "live", "MSP"), at(&src, 11, 9, "blackbox", "DATAFLASH"));
            let (live_5, log_5) = (at(&src, 11, 10, "live", "MSP"), at(&src, 11, 11, "blackbox", "MSP"));
            db::insert_blackbox_file(&src, log_2, "00000042.BIN", 0, b"bin").unwrap();
            db::insert_blackbox_file(&src, log_5, "LOG00005.TXT", 0, b"txt").unwrap();
            for (a, b) in [(live_1, log_1), (live_2, log_2), (live_3, log_3), (live_4, log_4), (live_5, log_5)] {
                db::link_flights(&src, a, b).unwrap();
            }
            let all = [live_1, log_1, live_2, log_2, lone, live_3, log_3, live_4, log_4, live_5, log_5];
            assert_eq!(export_flights(&src, &all, &file).unwrap(), all.len());
        }
        {
            // Make it an old file: no source_before_link column, no markers; decoded rows for pair 1's
            // log flight and the unlinked one, a marker-shaped row for pair 3's; pair 4's ArduPilot flight
            // has nothing but its protocol; pair 5 lost its link (each flight exported alone).
            let k = Connection::open(&file).unwrap();
            k.execute_batch("ALTER TABLE flights DROP COLUMN source_before_link; DELETE FROM blackbox_records;")
                .unwrap();
            let m = by_minute(&k);
            for (id, csv) in [(m[&2], "1,2,3"), (m[&5], "1,2,3"), (m[&7], "")] {
                k.execute(
                    "INSERT INTO blackbox_records (flight_id, timestamp_us, csv_data) VALUES (?1, 0, ?2)",
                    params![id, csv],
                )
                .unwrap();
            }
            k.execute("UPDATE flights SET linked_flight_id = NULL WHERE id IN (?1, ?2)", params![m[&10], m[&11]])
                .unwrap();
        }
        let dst = db::open_database(&dir.join("dst").join("flights.db")).unwrap();
        let r = import_flights(&dst, &file).unwrap();
        assert_eq!((r.imported, r.errors.len()), (11, 0));
        let m = by_minute(&dst);
        let both = |before: &str| ("both".to_string(), Some(before.to_string()));
        assert_eq!(source_of(&dst, m[&1]), both("live"));
        assert_eq!(source_of(&dst, m[&2]), both("blackbox"), "decoded rows in the file");
        assert_eq!(source_of(&dst, m[&3]), both("live"));
        assert_eq!(source_of(&dst, m[&4]), both("blackbox"), "archived log in the file");
        assert_eq!(source_of(&dst, m[&5]), ("blackbox".into(), None), "unlinked flights stay NULL");
        assert_eq!(source_of(&dst, m[&6]), both("live"));
        assert_eq!(source_of(&dst, m[&7]), both("blackbox"), "a marker row counts like decoded rows");
        assert_eq!(source_of(&dst, m[&8]), both("live"));
        assert_eq!(source_of(&dst, m[&9]), both("blackbox"), "a log importer's protocol");
        assert_eq!(source_of(&dst, m[&10]), ("live".into(), None), "arrived without its partner");
        assert_eq!(source_of(&dst, m[&11]), ("blackbox".into(), None), "arrived without its partner");
        assert_eq!(db::get_flight(&dst, m[&11]).unwrap().unwrap().linked_flight_id, None);
        assert!(!db::table_exists(&dst, "blackbox_records").unwrap(), "decoded rows never reach the main DB");
        db::unlink_flight(&dst, m[&2]).unwrap();
        assert_eq!(source_of(&dst, m[&1]), ("live".into(), None));
        assert_eq!(source_of(&dst, m[&2]), ("blackbox".into(), None));
        drop(dst);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
