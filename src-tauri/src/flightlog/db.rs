// SQLite database for flight logging
// Uses rusqlite with bundled SQLite — no external dependencies required.
// Schema evolution via PRAGMA user_version + sequential migrations.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqlResult};

use super::types::{Flight, FlightSummary, TelemetryRecord};

const CURRENT_SCHEMA_VERSION: u32 = 4;

/// Open (or create) the flight log database at the given path.
/// Runs migrations if needed.
pub fn open_database(path: &Path) -> SqlResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;

    // Performance pragmas for a write-heavy workload
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;

    migrate(&conn)?;
    Ok(conn)
}

/// Resolve the database file path based on settings.
/// - Empty db_path + portable mode → <exe_dir>/data/flights.db
/// - Empty db_path + normal mode → <AppData>/kite-gc/flights.db
/// - Non-empty db_path → <db_path>/flights.db
pub fn resolve_db_path(custom_path: &str, portable: bool) -> PathBuf {
    if !custom_path.is_empty() {
        return PathBuf::from(custom_path).join("flights.db");
    }

    if portable {
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            return exe_dir.join("data").join("flights.db");
        }
    }

    // Default: platform-specific AppData
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata)
                .join("kite-gc")
                .join("flights.db");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("kite-gc")
                .join("flights.db");
        }
    }

    // Fallback: current directory
    PathBuf::from("flights.db")
}

// ── Schema migrations ────────────────────────────────────────────────

fn get_user_version(conn: &Connection) -> SqlResult<u32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn set_user_version(conn: &Connection, version: u32) -> SqlResult<()> {
    conn.execute_batch(&format!("PRAGMA user_version = {};", version))
}

fn migrate(conn: &Connection) -> SqlResult<()> {
    let current = get_user_version(conn)?;

    if current < 1 {
        migrate_v0_to_v1(conn)?;
    }

    if current < 2 {
        migrate_v1_to_v2(conn)?;
    }

    if current < 3 {
        migrate_v2_to_v3(conn)?;
    }

    if current < 4 {
        migrate_v3_to_v4(conn)?;
    }

    Ok(())
}

fn migrate_v3_to_v4(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN baro_alt_m REAL;
         ALTER TABLE telemetry_records ADD COLUMN gps_hdop REAL;
         ALTER TABLE telemetry_records ADD COLUMN gps_eph REAL;
         ALTER TABLE telemetry_records ADD COLUMN gps_epv REAL;
         ALTER TABLE telemetry_records ADD COLUMN active_wp_number INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN active_flight_mode_flags INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN state_flags INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN nav_state INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN nav_flags INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN rx_signal_received INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN hw_health_status INTEGER;
         ALTER TABLE telemetry_records ADD COLUMN baro_temperature REAL;
         ALTER TABLE telemetry_records ADD COLUMN wind_n_ms REAL;
         ALTER TABLE telemetry_records ADD COLUMN wind_e_ms REAL;
         ALTER TABLE telemetry_records ADD COLUMN wind_d_ms REAL;
         ALTER TABLE telemetry_records ADD COLUMN rc_data_json TEXT;
         ALTER TABLE telemetry_records ADD COLUMN rc_command_json TEXT;",
    )?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_v2_to_v3(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER;",
    )?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_v0_to_v1(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS flights (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            start_time      TEXT NOT NULL,
            end_time        TEXT,
            duration_sec    INTEGER,
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
            cpu_load     INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_telemetry_flight
            ON telemetry_records(flight_id, timestamp_ms);",
    )?;

    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_v1_to_v2(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE flights ADD COLUMN source TEXT NOT NULL DEFAULT 'live';

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

        CREATE INDEX IF NOT EXISTS idx_blackbox_records_flight
            ON blackbox_records(flight_id, timestamp_us);

        CREATE INDEX IF NOT EXISTS idx_blackbox_files_flight
            ON blackbox_files(flight_id);",
    )?;

    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

// ── CRUD operations ─────────────────────────────────────────────────

/// Insert a new flight, returns the row ID.
pub fn insert_flight(conn: &Connection, flight: &Flight) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO flights (
            start_time, end_time, duration_sec, source,
            craft_name, fc_variant, fc_version, board_id, platform_type, protocol,
            start_lat, start_lon, location_name,
            weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc,
            max_alt_m, max_speed_ms, max_distance_m, total_distance_m,
            battery_used_mah, notes
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
        )",
        params![
            flight.start_time.to_rfc3339(),
            flight.end_time.map(|t| t.to_rfc3339()),
            flight.duration_sec,
            flight.source,
            flight.craft_name,
            flight.fc_variant,
            flight.fc_version,
            flight.board_id,
            flight.platform_type,
            flight.protocol,
            flight.start_lat,
            flight.start_lon,
            flight.location_name,
            flight.weather_temp_c,
            flight.weather_wind_ms,
            flight.weather_wind_deg,
            flight.weather_desc,
            flight.max_alt_m,
            flight.max_speed_ms,
            flight.max_distance_m,
            flight.total_distance_m,
            flight.battery_used_mah,
            flight.notes,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update a flight's end-time, duration, and statistics.
pub fn finalize_flight(
    conn: &Connection,
    flight_id: i64,
    end_time: DateTime<Utc>,
    duration_sec: i64,
    max_alt_m: Option<f64>,
    max_speed_ms: Option<f64>,
    max_distance_m: Option<f64>,
    total_distance_m: Option<f64>,
    battery_used_mah: Option<u32>,
    location_name: Option<&str>,
    weather_temp_c: Option<f64>,
    weather_wind_ms: Option<f64>,
    weather_wind_deg: Option<i32>,
    weather_desc: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET
            end_time = ?1, duration_sec = ?2,
            max_alt_m = ?3, max_speed_ms = ?4,
            max_distance_m = ?5, total_distance_m = ?6,
            battery_used_mah = ?7,
            location_name = ?8,
            weather_temp_c = ?9, weather_wind_ms = ?10,
            weather_wind_deg = ?11, weather_desc = ?12
        WHERE id = ?13",
        params![
            end_time.to_rfc3339(),
            duration_sec,
            max_alt_m,
            max_speed_ms,
            max_distance_m,
            total_distance_m,
            battery_used_mah,
            location_name,
            weather_temp_c,
            weather_wind_ms,
            weather_wind_deg,
            weather_desc,
            flight_id,
        ],
    )?;
    Ok(())
}

/// Batch-insert telemetry records for a flight.
pub fn insert_telemetry_batch(
    conn: &Connection,
    records: &[TelemetryRecord],
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO telemetry_records (
                flight_id, timestamp_ms, lat, lon, alt_m, speed_ms, heading,
                vario_ms, voltage, current_a, mah_drawn, rssi,
                roll, pitch, yaw, fix_type, num_sat, cpu_load, link_quality,
                baro_alt_m, gps_hdop, gps_eph, gps_epv,
                active_wp_number, active_flight_mode_flags, state_flags, nav_state, nav_flags,
                rx_signal_received, hw_health_status, baro_temperature,
                wind_n_ms, wind_e_ms, wind_d_ms,
                rc_data_json, rc_command_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23,
                ?24, ?25, ?26, ?27, ?28,
                ?29, ?30, ?31,
                ?32, ?33, ?34,
                ?35, ?36
            )",
        )?;
        for r in records {
            stmt.execute(params![
                r.flight_id,
                r.timestamp_ms,
                r.lat,
                r.lon,
                r.alt_m,
                r.speed_ms,
                r.heading,
                r.vario_ms,
                r.voltage,
                r.current_a,
                r.mah_drawn,
                r.rssi,
                r.roll,
                r.pitch,
                r.yaw,
                r.fix_type,
                r.num_sat,
                r.cpu_load,
                r.link_quality,
                r.baro_alt_m,
                r.gps_hdop,
                r.gps_eph,
                r.gps_epv,
                r.active_wp_number,
                r.active_flight_mode_flags,
                r.state_flags,
                r.nav_state,
                r.nav_flags,
                r.rx_signal_received,
                r.hw_health_status,
                r.baro_temperature,
                r.wind_n_ms,
                r.wind_e_ms,
                r.wind_d_ms,
                r.rc_data_json,
                r.rc_command_json,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// List flight summaries ordered by start_time DESC.
pub fn list_flights(conn: &Connection) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type
         FROM flights ORDER BY start_time DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let ts_str: String = row.get(1)?;
        let start_time = DateTime::parse_from_rfc3339(&ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(FlightSummary {
            id: row.get(0)?,
            start_time,
            duration_sec: row.get(2)?,
            source: row.get(3)?,
            craft_name: row.get(4)?,
            location_name: row.get(5)?,
            max_alt_m: row.get(6)?,
            max_speed_ms: row.get(7)?,
            total_distance_m: row.get(8)?,
            platform_type: row.get(9)?,
        })
    })?;

    rows.collect()
}

/// Get a single flight by ID.
pub fn get_flight(conn: &Connection, flight_id: i64) -> SqlResult<Option<Flight>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, end_time, duration_sec,
            source, craft_name, fc_variant, fc_version, board_id, platform_type, protocol,
                start_lat, start_lon, location_name,
                weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc,
                max_alt_m, max_speed_ms, max_distance_m, total_distance_m,
                battery_used_mah, notes
         FROM flights WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![flight_id], |row| {
        let start_str: String = row.get(1)?;
        let end_str: Option<String> = row.get(2)?;

        let start_time = DateTime::parse_from_rfc3339(&start_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let end_time = end_str.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        });

        Ok(Flight {
            id: row.get(0)?,
            start_time,
            end_time,
            duration_sec: row.get(3)?,
            source: row.get(4)?,
            craft_name: row.get(5)?,
            fc_variant: row.get(6)?,
            fc_version: row.get(7)?,
            board_id: row.get(8)?,
            platform_type: row.get(9)?,
            protocol: row.get(10)?,
            start_lat: row.get(11)?,
            start_lon: row.get(12)?,
            location_name: row.get(13)?,
            weather_temp_c: row.get(14)?,
            weather_wind_ms: row.get(15)?,
            weather_wind_deg: row.get(16)?,
            weather_desc: row.get(17)?,
            max_alt_m: row.get(18)?,
            max_speed_ms: row.get(19)?,
            max_distance_m: row.get(20)?,
            total_distance_m: row.get(21)?,
            battery_used_mah: row.get(22)?,
            notes: row.get(23)?,
        })
    })?;

    match rows.next() {
        Some(Ok(f)) => Ok(Some(f)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// Get the GPS track for a flight.
pub fn get_flight_track(
    conn: &Connection,
    flight_id: i64,
) -> SqlResult<Vec<TelemetryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, flight_id, timestamp_ms, lat, lon, alt_m, speed_ms,
                heading, vario_ms, voltage, current_a, mah_drawn, rssi,
            roll, pitch, yaw, fix_type, num_sat, cpu_load, link_quality,
            baro_alt_m, gps_hdop, gps_eph, gps_epv,
            active_wp_number, active_flight_mode_flags, state_flags, nav_state, nav_flags,
            rx_signal_received, hw_health_status, baro_temperature,
            wind_n_ms, wind_e_ms, wind_d_ms,
            rc_data_json, rc_command_json
         FROM telemetry_records
         WHERE flight_id = ?1
         ORDER BY timestamp_ms ASC",
    )?;

    let rows = stmt.query_map(params![flight_id], |row| {
        Ok(TelemetryRecord {
            id: row.get(0)?,
            flight_id: row.get(1)?,
            timestamp_ms: row.get(2)?,
            lat: row.get(3)?,
            lon: row.get(4)?,
            alt_m: row.get(5)?,
            speed_ms: row.get(6)?,
            heading: row.get(7)?,
            vario_ms: row.get(8)?,
            voltage: row.get(9)?,
            current_a: row.get(10)?,
            mah_drawn: row.get(11)?,
            rssi: row.get(12)?,
            roll: row.get(13)?,
            pitch: row.get(14)?,
            yaw: row.get(15)?,
            fix_type: row.get(16)?,
            num_sat: row.get(17)?,
            cpu_load: row.get(18)?,
            link_quality: row.get(19)?,
            baro_alt_m: row.get(20)?,
            gps_hdop: row.get(21)?,
            gps_eph: row.get(22)?,
            gps_epv: row.get(23)?,
            active_wp_number: row.get(24)?,
            active_flight_mode_flags: row.get(25)?,
            state_flags: row.get(26)?,
            nav_state: row.get(27)?,
            nav_flags: row.get(28)?,
            rx_signal_received: row.get(29)?,
            hw_health_status: row.get(30)?,
            baro_temperature: row.get(31)?,
            wind_n_ms: row.get(32)?,
            wind_e_ms: row.get(33)?,
            wind_d_ms: row.get(34)?,
            rc_data_json: row.get(35)?,
            rc_command_json: row.get(36)?,
        })
    })?;

    rows.collect()
}

/// Check for duplicate flights based on craft_name and start_time (±10s).
/// Returns the existing flight if found, or None.
pub fn find_duplicate_flight(
    conn: &Connection,
    craft_name: &str,
    start_time: DateTime<Utc>,
) -> Result<Option<Flight>, String> {
    let time_lower = (start_time - chrono::Duration::seconds(10)).to_rfc3339();
    let time_upper = (start_time + chrono::Duration::seconds(10)).to_rfc3339();

    eprintln!("[DUP-DB] Query: craft_name={:?}, time_lower={}, time_upper={}", craft_name, time_lower, time_upper);

    // Step 1: Quick COUNT check — avoids full deserialization unless needed
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM flights
             WHERE craft_name = ?1 AND start_time >= ?2 AND start_time <= ?3",
            params![craft_name, time_lower, time_upper],
            |row| row.get(0),
        )
        .map_err(|e| format!("Duplicate check COUNT failed: {}", e))?;

    eprintln!("[DUP-DB] COUNT result: {}", count);

    if count == 0 {
        return Ok(None);
    }

    // Step 2: Fetch the first matching flight
    let flight = conn
        .query_row(
            "SELECT id, start_time, end_time, duration_sec, source, craft_name, 
                    fc_variant, fc_version, board_id, platform_type, protocol,
                    start_lat, start_lon, location_name, weather_temp_c, weather_wind_ms,
                    weather_wind_deg, weather_desc, max_alt_m, max_speed_ms, max_distance_m,
                    total_distance_m, battery_used_mah, notes
             FROM flights
             WHERE craft_name = ?1 AND start_time >= ?2 AND start_time <= ?3
             ORDER BY id ASC LIMIT 1",
            params![craft_name, time_lower, time_upper],
            |row| {
                let start_str: String = row.get(1)?;
                let end_str: Option<String> = row.get(2)?;
                let start_time_found = DateTime::parse_from_rfc3339(&start_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let end_time = end_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok()
                });
                Ok(Flight {
                    id: row.get(0)?,
                    start_time: start_time_found,
                    end_time,
                    duration_sec: row.get(3)?,
                    source: row.get(4)?,
                    craft_name: row.get(5)?,
                    fc_variant: row.get(6)?,
                    fc_version: row.get(7)?,
                    board_id: row.get(8)?,
                    platform_type: row.get(9)?,
                    protocol: row.get(10)?,
                    start_lat: row.get(11)?,
                    start_lon: row.get(12)?,
                    location_name: row.get(13)?,
                    weather_temp_c: row.get(14)?,
                    weather_wind_ms: row.get(15)?,
                    weather_wind_deg: row.get(16)?,
                    weather_desc: row.get(17)?,
                    max_alt_m: row.get(18)?,
                    max_speed_ms: row.get(19)?,
                    max_distance_m: row.get(20)?,
                    total_distance_m: row.get(21)?,
                    battery_used_mah: row.get(22)?,
                    notes: row.get(23)?,
                })
            },
        )
        .map_err(|e| format!("Duplicate check flight fetch failed: {}", e))?;

    Ok(Some(flight))
}

/// Delete a flight and all related data (telemetry, blackbox records, archived files).
/// Explicitly deletes child rows first (in case foreign_keys is off), then VACUUMs.
pub fn delete_flight(conn: &Connection, flight_id: i64) -> SqlResult<bool> {
    // Explicitly delete child tables (don't rely solely on CASCADE)
    conn.execute("DELETE FROM blackbox_files WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM blackbox_records WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM telemetry_records WHERE flight_id = ?1", params![flight_id])?;
    let affected = conn.execute("DELETE FROM flights WHERE id = ?1", params![flight_id])?;

    // Reclaim disk space — blackbox_files stores large BLOBs
    if affected > 0 {
        conn.execute_batch("VACUUM;")?;
    }

    Ok(affected > 0)
}

/// Update the notes field of a flight.
pub fn update_flight_notes(
    conn: &Connection,
    flight_id: i64,
    notes: &str,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET notes = ?1 WHERE id = ?2",
        params![notes, flight_id],
    )?;
    Ok(())
}

pub fn insert_blackbox_records(
    conn: &Connection,
    flight_id: i64,
    records: &[(i64, String)],
) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO blackbox_records (flight_id, timestamp_us, csv_data)
             VALUES (?1, ?2, ?3)",
        )?;

        for (timestamp_us, csv_data) in records {
            stmt.execute(params![flight_id, timestamp_us, csv_data])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn insert_blackbox_file(
    conn: &Connection,
    flight_id: i64,
    original_filename: &str,
    log_index: u32,
    file_data: &[u8],
) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO blackbox_files (
            flight_id, original_filename, log_index, file_data, file_size
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            flight_id,
            original_filename,
            log_index,
            file_data,
            file_data.len() as i64,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn test_schema_creation() {
        let conn = test_db();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_insert_and_get_flight() {
        let conn = test_db();
        let now = Utc::now();
        let flight = Flight {
            id: 0,
            start_time: now,
            end_time: None,
            duration_sec: None,
            source: "live".into(),
            craft_name: "TestCraft".into(),
            fc_variant: "INAV".into(),
            fc_version: "7.1.2".into(),
            board_id: "MATF".into(),
            platform_type: 0,
            protocol: "MSP".into(),
            start_lat: Some(48.1234),
            start_lon: Some(11.5678),
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
        };
        let id = insert_flight(&conn, &flight).unwrap();
        let loaded = get_flight(&conn, id).unwrap().unwrap();
        assert_eq!(loaded.craft_name, "TestCraft");
        assert_eq!(loaded.fc_variant, "INAV");
    }

    #[test]
    fn test_finalize_and_list() {
        let conn = test_db();
        let now = Utc::now();
        let flight = Flight {
            id: 0,
            start_time: now,
            end_time: None,
            duration_sec: None,
            source: "live".into(),
            craft_name: "Wing".into(),
            fc_variant: "INAV".into(),
            fc_version: "8.0.0".into(),
            board_id: "SPRF".into(),
            platform_type: 1,
            protocol: "MSP".into(),
            start_lat: None,
            start_lon: None,
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
        };
        let id = insert_flight(&conn, &flight).unwrap();
        finalize_flight(
            &conn,
            id,
            Utc::now(),
            120,
            Some(50.0),
            Some(15.0),
            Some(200.0),
            Some(800.0),
            Some(450),
            Some("Munich"),
            Some(18.5),
            Some(3.2),
            Some(270),
            Some("Partly cloudy"),
        )
        .unwrap();

        let list = list_flights(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].craft_name, "Wing");
        assert_eq!(list[0].source, "live");
        assert_eq!(list[0].max_alt_m, Some(50.0));
    }

    #[test]
    fn test_telemetry_batch() {
        let conn = test_db();
        let flight = Flight {
            id: 0,
            start_time: Utc::now(),
            end_time: None,
            duration_sec: None,
            source: "live".into(),
            craft_name: "Quad".into(),
            fc_variant: "INAV".into(),
            fc_version: "7.1.2".into(),
            board_id: "MATF".into(),
            platform_type: 0,
            protocol: "MSP".into(),
            start_lat: None,
            start_lon: None,
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
        };
        let fid = insert_flight(&conn, &flight).unwrap();

        let records: Vec<TelemetryRecord> = (0..100)
            .map(|i| TelemetryRecord {
                id: 0,
                flight_id: fid,
                timestamp_ms: i * 200,
                lat: Some(48.0 + (i as f64) * 0.0001),
                lon: Some(11.0 + (i as f64) * 0.0001),
                alt_m: Some(100.0 + i as f64),
                speed_ms: Some(10.0),
                heading: Some(90),
                vario_ms: Some(0.5),
                voltage: Some(12.6),
                current_a: Some(15.0),
                mah_drawn: Some(i as u32 * 5),
                rssi: Some(95),
                roll: Some(2.0),
                pitch: Some(5.0),
                yaw: Some(90),
                fix_type: Some(3),
                num_sat: Some(12),
                cpu_load: Some(25),
                link_quality: None,
                baro_alt_m: None,
                gps_hdop: None,
                gps_eph: None,
                gps_epv: None,
                active_wp_number: None,
                active_flight_mode_flags: None,
                state_flags: None,
                nav_state: None,
                nav_flags: None,
                rx_signal_received: None,
                hw_health_status: None,
                baro_temperature: None,
                wind_n_ms: None,
                wind_e_ms: None,
                wind_d_ms: None,
                rc_data_json: None,
                rc_command_json: None,
            })
            .collect();

        insert_telemetry_batch(&conn, &records).unwrap();

        let track = get_flight_track(&conn, fid).unwrap();
        assert_eq!(track.len(), 100);
        assert!(track[0].timestamp_ms < track[99].timestamp_ms);
    }

    #[test]
    fn test_delete_flight_cascades() {
        let conn = test_db();
        let flight = Flight {
            id: 0,
            start_time: Utc::now(),
            end_time: None,
            duration_sec: None,
            source: "live".into(),
            craft_name: "Del".into(),
            fc_variant: "INAV".into(),
            fc_version: "7.0.0".into(),
            board_id: "TEST".into(),
            platform_type: 0,
            protocol: "MSP".into(),
            start_lat: None,
            start_lon: None,
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
        };
        let fid = insert_flight(&conn, &flight).unwrap();
        let rec = TelemetryRecord {
            id: 0,
            flight_id: fid,
            timestamp_ms: 0,
            lat: None,
            lon: None,
            alt_m: None,
            speed_ms: None,
            heading: None,
            vario_ms: None,
            voltage: None,
            current_a: None,
            mah_drawn: None,
            rssi: None,
            roll: None,
            pitch: None,
            yaw: None,
            fix_type: None,
            num_sat: None,
            cpu_load: None,
            link_quality: None,
            baro_alt_m: None,
            gps_hdop: None,
            gps_eph: None,
            gps_epv: None,
            active_wp_number: None,
            active_flight_mode_flags: None,
            state_flags: None,
            nav_state: None,
            nav_flags: None,
            rx_signal_received: None,
            hw_health_status: None,
            baro_temperature: None,
            wind_n_ms: None,
            wind_e_ms: None,
            wind_d_ms: None,
            rc_data_json: None,
            rc_command_json: None,
        };
        insert_telemetry_batch(&conn, &[rec]).unwrap();

        assert!(delete_flight(&conn, fid).unwrap());
        assert!(get_flight(&conn, fid).unwrap().is_none());

        let track = get_flight_track(&conn, fid).unwrap();
        assert!(track.is_empty());
    }

    #[test]
    fn test_blackbox_tables_accept_rows() {
        let conn = test_db();
        let flight = Flight {
            id: 0,
            start_time: Utc::now(),
            end_time: None,
            duration_sec: None,
            source: "blackbox".into(),
            craft_name: "BB".into(),
            fc_variant: "INAV".into(),
            fc_version: "9.0.0".into(),
            board_id: "TEST".into(),
            platform_type: 0,
            protocol: "BLACKBOX".into(),
            start_lat: None,
            start_lon: None,
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
        };
        let flight_id = insert_flight(&conn, &flight).unwrap();
        insert_blackbox_records(&conn, flight_id, &[(123_000, "{}".into())]).unwrap();
        insert_blackbox_file(&conn, flight_id, "test.TXT", 0, b"abc").unwrap();

        let blackbox_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM blackbox_records WHERE flight_id = ?1", params![flight_id], |row| row.get(0))
            .unwrap();
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM blackbox_files WHERE flight_id = ?1", params![flight_id], |row| row.get(0))
            .unwrap();

        assert_eq!(blackbox_count, 1);
        assert_eq!(file_count, 1);
    }
}
