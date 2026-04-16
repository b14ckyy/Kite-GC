// SQLite database for flight logging
// Uses rusqlite with bundled SQLite — no external dependencies required.
// Schema evolution via PRAGMA user_version + sequential migrations.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqlResult};

use super::types::{Flight, FlightSummary, TelemetryRecord};

const CURRENT_SCHEMA_VERSION: u32 = 1;

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

    // Future migrations go here:
    // if current < 2 { migrate_v1_to_v2(conn)?; }

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

// ── CRUD operations ─────────────────────────────────────────────────

/// Insert a new flight, returns the row ID.
pub fn insert_flight(conn: &Connection, flight: &Flight) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO flights (
            start_time, end_time, duration_sec,
            craft_name, fc_variant, fc_version, board_id, platform_type, protocol,
            start_lat, start_lon, location_name,
            weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc,
            max_alt_m, max_speed_ms, max_distance_m, total_distance_m,
            battery_used_mah, notes
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
        )",
        params![
            flight.start_time.to_rfc3339(),
            flight.end_time.map(|t| t.to_rfc3339()),
            flight.duration_sec,
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
                roll, pitch, yaw, fix_type, num_sat, cpu_load
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18
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
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// List flight summaries ordered by start_time DESC.
pub fn list_flights(conn: &Connection) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, craft_name, location_name,
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
            craft_name: row.get(3)?,
            location_name: row.get(4)?,
            max_alt_m: row.get(5)?,
            max_speed_ms: row.get(6)?,
            total_distance_m: row.get(7)?,
            platform_type: row.get(8)?,
        })
    })?;

    rows.collect()
}

/// Get a single flight by ID.
pub fn get_flight(conn: &Connection, flight_id: i64) -> SqlResult<Option<Flight>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, end_time, duration_sec,
                craft_name, fc_variant, fc_version, board_id, platform_type, protocol,
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
            craft_name: row.get(4)?,
            fc_variant: row.get(5)?,
            fc_version: row.get(6)?,
            board_id: row.get(7)?,
            platform_type: row.get(8)?,
            protocol: row.get(9)?,
            start_lat: row.get(10)?,
            start_lon: row.get(11)?,
            location_name: row.get(12)?,
            weather_temp_c: row.get(13)?,
            weather_wind_ms: row.get(14)?,
            weather_wind_deg: row.get(15)?,
            weather_desc: row.get(16)?,
            max_alt_m: row.get(17)?,
            max_speed_ms: row.get(18)?,
            max_distance_m: row.get(19)?,
            total_distance_m: row.get(20)?,
            battery_used_mah: row.get(21)?,
            notes: row.get(22)?,
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
                roll, pitch, yaw, fix_type, num_sat, cpu_load
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
        })
    })?;

    rows.collect()
}

/// Delete a flight and all its telemetry records (cascade).
pub fn delete_flight(conn: &Connection, flight_id: i64) -> SqlResult<bool> {
    let affected = conn.execute("DELETE FROM flights WHERE id = ?1", params![flight_id])?;
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
        };
        insert_telemetry_batch(&conn, &[rec]).unwrap();

        assert!(delete_flight(&conn, fid).unwrap());
        assert!(get_flight(&conn, fid).unwrap().is_none());

        let track = get_flight_track(&conn, fid).unwrap();
        assert!(track.is_empty());
    }
}
