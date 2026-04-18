// Flight data exchange — export/import complete flight records as .kflight files.
// A .kflight file is a self-contained SQLite database with the same schema as the
// main flights database, containing one or more flights with all their data:
// metadata, telemetry records, blackbox records, and archived blackbox files.

use std::path::Path;

use rusqlite::{params, Connection};

use super::db;
use super::types::{Flight, FlightSummary, TelemetryRecord};

/// Schema version stored in exported .kflight files
const KFLIGHT_SCHEMA_VERSION: u32 = 1;

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

    // Copy each flight
    let mut exported = 0;
    for &flight_id in flight_ids {
        match copy_flight(source_conn, &out, flight_id) {
            Ok(_) => exported += 1,
            Err(e) => {
                log::warn!("Failed to export flight {}: {}", flight_id, e);
            }
        }
    }

    if exported == 0 {
        // Clean up the empty file
        drop(out);
        std::fs::remove_file(output_path).ok();
        return Err("No flights could be exported".into());
    }

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

    log::info!(
        "Exported {} flight(s) to {}",
        exported,
        output_path.display()
    );
    Ok(exported)
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

    // Create all tables matching the main DB schema (v4)
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
            notes           TEXT
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
            rc_command_json TEXT
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
        CREATE INDEX IF NOT EXISTS idx_blackbox_records_flight
            ON blackbox_records(flight_id, timestamp_us);
        CREATE INDEX IF NOT EXISTS idx_blackbox_files_flight
            ON blackbox_files(flight_id);",
    )
    .map_err(|e| format!("Schema creation error: {}", e))?;

    Ok(conn)
}

/// Copy a single flight with all related data from source to destination DB.
fn copy_flight(
    src: &Connection,
    dst: &Connection,
    flight_id: i64,
) -> Result<i64, String> {
    // 1. Read the flight row
    let flight = db::get_flight(src, flight_id)
        .map_err(|e| format!("Read flight: {}", e))?
        .ok_or_else(|| format!("Flight {} not found", flight_id))?;

    // 2. Insert into destination (gets a new ID)
    let new_id = db::insert_flight(dst, &flight)
        .map_err(|e| format!("Insert flight: {}", e))?;

    // Finalize with stats + weather (insert_flight doesn't copy all computed fields)
    dst.execute(
        "UPDATE flights SET
            end_time = ?1, duration_sec = ?2,
            max_alt_m = ?3, max_speed_ms = ?4, max_distance_m = ?5,
            total_distance_m = ?6, battery_used_mah = ?7,
            location_name = ?8, weather_temp_c = ?9, weather_wind_ms = ?10,
            weather_wind_deg = ?11, weather_desc = ?12
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

    // 4. Copy blackbox records
    copy_blackbox_records(src, dst, flight_id, new_id)?;

    // 5. Copy blackbox files (BLOBs)
    copy_blackbox_files(src, dst, flight_id, new_id)?;

    Ok(new_id)
}

fn copy_blackbox_records(
    src: &Connection,
    dst: &Connection,
    old_flight_id: i64,
    new_flight_id: i64,
) -> Result<(), String> {
    let mut stmt = src
        .prepare("SELECT timestamp_us, csv_data FROM blackbox_records WHERE flight_id = ?1 ORDER BY timestamp_us")
        .map_err(|e| format!("Prepare blackbox_records: {}", e))?;

    let rows: Vec<(i64, String)> = stmt
        .query_map(params![old_flight_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|e| format!("Query blackbox_records: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Read blackbox_records: {}", e))?;

    if rows.is_empty() {
        return Ok(());
    }

    db::insert_blackbox_records(dst, new_flight_id, &rows)
        .map_err(|e| format!("Insert blackbox_records: {}", e))?;

    Ok(())
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

    let src = Connection::open(kflight_path)
        .map_err(|e| format!("Failed to open .kflight file: {}", e))?;

    // Verify it's a valid kflight file
    let has_meta: bool = src
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

    // List all flights in the source file
    let src_flights = db::list_flights(&src)
        .map_err(|e| format!("Failed to list flights in .kflight: {}", e))?;

    let mut result = ImportResult {
        imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    for summary in &src_flights {
        // Check for duplicate
        let is_dup = db::find_duplicate_flight(
            target_conn,
            &summary.craft_name,
            summary.start_time,
        )
        .map_err(|e| format!("Duplicate check: {}", e))?
        .is_some();

        if is_dup {
            result.skipped += 1;
            log::info!("Import: skipping duplicate flight {} ({})", summary.id, summary.start_time);
            continue;
        }

        match copy_flight(&src, target_conn, summary.id) {
            Ok(_) => result.imported += 1,
            Err(e) => {
                log::warn!("Import: failed to import flight {}: {}", summary.id, e);
                result.errors.push(format!("Flight {}: {}", summary.id, e));
            }
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

// ── Read from .kflight (for offline replay) ─────────────────────────

/// List flights in a .kflight file (for offline replay / preview)
pub fn list_flights_in_file(path: &Path) -> Result<Vec<FlightSummary>, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open .kflight file: {}", e))?;
    db::list_flights(&conn).map_err(|e| format!("Query error: {}", e))
}

/// Get a single flight from a .kflight file
pub fn get_flight_from_file(path: &Path, flight_id: i64) -> Result<Option<Flight>, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open .kflight file: {}", e))?;
    db::get_flight(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}

/// Get the telemetry track from a .kflight file
pub fn get_track_from_file(path: &Path, flight_id: i64) -> Result<Vec<TelemetryRecord>, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open .kflight file: {}", e))?;
    db::get_flight_track(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}
