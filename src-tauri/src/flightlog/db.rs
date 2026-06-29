// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// SQLite database for flight logging
// Uses rusqlite with bundled SQLite — no external dependencies required.
// Schema evolution via PRAGMA user_version + sequential migrations.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

use super::types::{
    BatteryAggregate, BatteryPack, BatteryPackInput, BatteryRecord, Flight, FlightSummary, Mission,
    MissionInput, TelemetryRecord, Vehicle, VehicleAggregate, VehicleInput,
};

const CURRENT_SCHEMA_VERSION: u32 = 18;

/// Column list (excluding the autoincrement `id`) for `telemetry_records`, shared by the temp-session
/// copy so the SELECT and INSERT column orders can never drift apart. `flight_id` is first so the
/// copy can swap it for the freshly inserted main id (see `commit_session_to_main`).
const TELEMETRY_COLS: &str = "flight_id, timestamp_ms, lat, lon, alt_m, speed_ms, heading, \
    vario_ms, voltage, current_a, mah_drawn, rssi, battery_percentage, \
    roll, pitch, yaw, fix_type, num_sat, cpu_load, link_quality, \
    baro_alt_m, gps_hdop, gps_eph, gps_epv, \
    active_wp_number, active_flight_mode_flags, state_flags, nav_state, nav_flags, \
    rx_signal_received, hw_health_status, baro_temperature, \
    wind_n_ms, wind_e_ms, wind_d_ms, rc_data_json, rc_command_json, \
    nav_lat, nav_lon, nav_alt_m, mode_primary, mode_modifiers, link_snr, link_rssi_dbm, airspeed_ms, \
    throttle_pct";

/// Full single-statement DDL for `telemetry_records` at the current field set. The main DB grows
/// this table via the migration chain; the per-session temp DB (no migration history) creates it
/// in one shot. The two MUST describe the same columns (the temp rows are copied into the main
/// table on disarm). No FK here — the temp DB has no `flights` table.
const TELEMETRY_RECORDS_DDL_FULL: &str = "CREATE TABLE IF NOT EXISTS telemetry_records (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    flight_id    INTEGER NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    lat REAL, lon REAL, alt_m REAL, speed_ms REAL, heading INTEGER, vario_ms REAL,
    voltage REAL, current_a REAL, mah_drawn INTEGER, rssi INTEGER, battery_percentage INTEGER,
    roll REAL, pitch REAL, yaw INTEGER, fix_type INTEGER, num_sat INTEGER, cpu_load INTEGER,
    link_quality INTEGER, baro_alt_m REAL, gps_hdop REAL, gps_eph REAL, gps_epv REAL,
    active_wp_number INTEGER, active_flight_mode_flags INTEGER, state_flags INTEGER,
    nav_state INTEGER, nav_flags INTEGER, rx_signal_received INTEGER, hw_health_status INTEGER,
    baro_temperature REAL, wind_n_ms REAL, wind_e_ms REAL, wind_d_ms REAL,
    rc_data_json TEXT, rc_command_json TEXT, nav_lat REAL, nav_lon REAL, nav_alt_m REAL,
    mode_primary TEXT, mode_modifiers TEXT, link_snr INTEGER, link_rssi_dbm INTEGER,
    airspeed_ms REAL, throttle_pct REAL
);";

/// Per-instance battery samples (ArduPilot/PX4 multi-monitor). `flight_id` first so the temp-session
/// copy can swap it for the new main id, mirroring TELEMETRY_COLS. See docs/active/MULTI_BATTERY.md.
const BATTERY_RECORDS_COLS: &str = "flight_id, timestamp_ms, instance, voltage, current_a, mah_drawn, \
    battery_percentage, cell_count, temperature";

/// Full DDL for `battery_records`, shared by the main DB (via the migration chain) and the temp
/// session store (one-shot). Both MUST describe the same columns (temp rows are copied to main).
const BATTERY_RECORDS_DDL_FULL: &str = "CREATE TABLE IF NOT EXISTS battery_records (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    flight_id     INTEGER NOT NULL,
    timestamp_ms  INTEGER NOT NULL,
    instance      INTEGER NOT NULL,
    voltage       REAL,
    current_a     REAL,
    mah_drawn     INTEGER,
    battery_percentage INTEGER,
    cell_count    INTEGER,
    temperature   REAL
);";

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

    // Incremental auto-vacuum: freed pages (e.g. a deleted blackbox BLOB) go on a freelist and are
    // reclaimed cheaply via `PRAGMA incremental_vacuum` after a delete — cost ∝ freed pages, not the
    // whole DB. (A full `VACUUM` rewrites the entire file, which is unusable once logs reach GBs.)
    // The mode can only be set before tables exist on a fresh DB, or converted with one VACUUM on an
    // existing one — so do that one-time conversion here when the DB isn't already INCREMENTAL (2).
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
    if auto_vacuum != 2 {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        // Cheap on a fresh/empty DB; a one-time rewrite on an existing one (then never again).
        conn.execute_batch("VACUUM;")?;
    }

    migrate(&conn)?;
    Ok(conn)
}

/// Full defragmenting VACUUM — rebuilds the whole DB file to squeeze out fragmentation. Expensive
/// (cost ∝ DB size); exposed only as an explicit "Compact Database" maintenance action. Routine
/// deletes use incremental_vacuum instead (see `open_database`).
pub fn compact_database(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("VACUUM;")
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

/// Resolve the **raw-log base directory** (where `raw_logs/` with .tlog / raw MSP files live). This is
/// intentionally **separate from the database folder**: the DB defaults to AppData, but raw logs default
/// to the user's Documents (`Documents/KiteGC`) where they are easy to find / hand to Mission Planner.
/// `custom_path` (empty = use default) overrides it; both are configurable independently in Settings.
pub fn resolve_raw_log_dir(custom_path: &str, portable: bool) -> PathBuf {
    if !custom_path.is_empty() {
        return PathBuf::from(custom_path);
    }

    if portable {
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            return exe_dir.join("data");
        }
    }

    // Default: the user's Documents folder → Documents/KiteGC (Windows honours OneDrive relocation,
    // Linux uses XDG_DOCUMENTS_DIR). Falls back to ~/Documents, then the current dir.
    if let Some(docs) = dirs::document_dir() {
        return docs.join("KiteGC");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join("Documents").join("KiteGC");
    }
    PathBuf::from(".")
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

    if current < 5 {
        migrate_v4_to_v5(conn)?;
    }

    if current < 6 {
        migrate_v5_to_v6(conn)?;
    }

    if current < 7 {
        migrate_v6_to_v7(conn)?;
    }

    if current < 8 {
        migrate_v7_to_v8(conn)?;
    }

    if current < 9 {
        migrate_v8_to_v9(conn)?;
    }

    if current < 10 {
        migrate_v9_to_v10(conn)?;
    }

    if current < 11 {
        migrate_v10_to_v11(conn)?;
    }

    if current < 12 {
        migrate_v11_to_v12(conn)?;
    }

    if current < 13 {
        migrate_v12_to_v13(conn)?;
    }

    if current < 14 {
        migrate_v13_to_v14(conn)?;
    }

    if current < 15 {
        migrate_v14_to_v15(conn)?;
    }

    if current < 16 {
        migrate_v15_to_v16(conn)?;
    }

    if current < 17 {
        migrate_v16_to_v17(conn)?;
    }

    if current < 18 {
        migrate_v17_to_v18(conn)?;
    }

    // Self-heal: ensure the latest schema actually exists even if a prior version bump left it
    // incomplete. Legacy migrations call set_user_version(CURRENT), so a CURRENT bump can
    // mark the DB as the newest version without the matching migration ever creating its
    // objects. Idempotent, so a healthy DB is unaffected.
    ensure_v8_schema(conn)?;
    ensure_v9_schema(conn)?;
    ensure_v10_schema(conn)?;
    ensure_v11_schema(conn)?;
    ensure_v12_schema(conn)?;
    ensure_v13_schema(conn)?;
    ensure_v14_schema(conn)?;
    ensure_v15_schema(conn)?;
    ensure_v16_schema(conn)?;
    ensure_v17_schema(conn)?;
    ensure_v18_schema(conn)?;

    Ok(())
}

/// Whether `table` has a column named `column` (via PRAGMA table_info).
fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    // `table` is always a hardcoded literal here, so the format! is injection-safe.
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Idempotently create the v8 mission-library objects (missions table + the two `flights`
/// columns). Safe to call on every open; only adds what's missing.
fn ensure_v8_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS missions (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            content_hash     TEXT NOT NULL UNIQUE,
            name             TEXT NOT NULL DEFAULT '',
            format           TEXT NOT NULL DEFAULT 'inav',
            waypoints_json   TEXT NOT NULL,
            source_xml       TEXT,
            wp_count         INTEGER NOT NULL DEFAULT 0,
            total_distance_m REAL,
            alt_diff_m       REAL,
            max_alt_m        REAL,
            min_alt_m        REAL,
            bndbox_min_lat   REAL,
            bndbox_min_lon   REAL,
            bndbox_max_lat   REAL,
            bndbox_max_lon   REAL,
            location_name    TEXT,
            created_at       TEXT NOT NULL DEFAULT (datetime('now')),
            notes            TEXT
        );",
    )?;
    // A `missions` table created by an interim build may predate the location_name column.
    if !column_exists(conn, "missions", "location_name")? {
        conn.execute_batch("ALTER TABLE missions ADD COLUMN location_name TEXT;")?;
    }
    if !column_exists(conn, "flights", "mission_id")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN mission_id INTEGER REFERENCES missions(id);")?;
    }
    if !column_exists(conn, "flights", "logged_wp_count")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN logged_wp_count INTEGER;")?;
    }
    Ok(())
}

fn migrate_v7_to_v8(conn: &Connection) -> SqlResult<()> {
    ensure_v8_schema(conn)?;
    set_user_version(conn, 8)?;
    Ok(())
}

/// Idempotently add the pilot metadata columns (manually editable; a future operator/login
/// system can prefill them). Safe to call on every open; only adds what's missing.
fn ensure_v9_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "flights", "pilot_name")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN pilot_name TEXT;")?;
    }
    if !column_exists(conn, "flights", "pilot_id")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN pilot_id TEXT;")?;
    }
    Ok(())
}

fn migrate_v8_to_v9(conn: &Connection) -> SqlResult<()> {
    ensure_v9_schema(conn)?;
    set_user_version(conn, 9)?;
    Ok(())
}

/// Idempotently create the v10 battery objects: the `battery_packs` table + the soft-link
/// `flights.battery_serial` column. Safe to call on every open; only adds what's missing.
fn ensure_v10_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS battery_packs (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            serial              TEXT NOT NULL UNIQUE,
            label               TEXT,
            manufacturer        TEXT,
            model               TEXT,
            chemistry           TEXT,
            cell_count          INTEGER,
            capacity_mah        INTEGER,
            c_rating_discharge  INTEGER,
            c_rating_charge     INTEGER,
            connector           TEXT,
            in_service_date     TEXT,
            status              TEXT NOT NULL DEFAULT 'active',
            notes               TEXT,
            created_at          TEXT NOT NULL DEFAULT (datetime('now')),
            base_flight_seconds INTEGER NOT NULL DEFAULT 0,
            base_mah            INTEGER NOT NULL DEFAULT 0,
            base_cycles         REAL NOT NULL DEFAULT 0,
            base_charges        INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    if !column_exists(conn, "flights", "battery_serial")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN battery_serial TEXT;")?;
    }
    Ok(())
}

fn migrate_v9_to_v10(conn: &Connection) -> SqlResult<()> {
    ensure_v10_schema(conn)?;
    set_user_version(conn, 10)?;
    Ok(())
}

/// Idempotently add the mission launch/home columns (the planned takeoff reference — the
/// base for REL waypoint altitudes + the 3D mission preview's height). Safe to call on every
/// open; only adds what's missing.
fn ensure_v11_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "missions", "home_lat")? {
        conn.execute_batch("ALTER TABLE missions ADD COLUMN home_lat REAL;")?;
    }
    if !column_exists(conn, "missions", "home_lon")? {
        conn.execute_batch("ALTER TABLE missions ADD COLUMN home_lon REAL;")?;
    }
    Ok(())
}

fn migrate_v10_to_v11(conn: &Connection) -> SqlResult<()> {
    ensure_v11_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v12: canonical flight-mode columns on telemetry_records (see docs/active/FLIGHT_MODE_UNIFIED.md).
/// Additive + idempotent; old rows keep NULL (render as the `other` category on replay — acceptable
/// in alpha). The raw `active_flight_mode_flags` column stays as a forensic field.
fn ensure_v12_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "telemetry_records", "mode_primary")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN mode_primary TEXT;")?;
    }
    if !column_exists(conn, "telemetry_records", "mode_modifiers")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN mode_modifiers TEXT;")?;
    }
    Ok(())
}

fn migrate_v11_to_v12(conn: &Connection) -> SqlResult<()> {
    ensure_v12_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v13: flight-local time. `flights.utc_offset_min` = the local UTC offset (minutes, east-positive) at
/// the flight location, DST-aware (ADR-048). Additive + idempotent; old rows keep NULL → displayed in
/// UTC. `start_time` itself is unchanged (always true UTC going forward).
fn ensure_v13_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "flights", "utc_offset_min")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN utc_offset_min INTEGER;")?;
    }
    Ok(())
}

fn migrate_v12_to_v13(conn: &Connection) -> SqlResult<()> {
    ensure_v13_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v14: live RC-link metrics on telemetry_records — `link_snr` (dB) + `link_rssi_dbm` (raw dBm) from
/// the unified link-stats pipeline (CRSF 0x14 / INAV 9.1 MSP2_INAV_GET_LINK_STATS). `link_quality` (v3)
/// is now also populated live (was Blackbox-only). Additive + idempotent; old rows keep NULL.
fn ensure_v14_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "telemetry_records", "link_snr")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN link_snr INTEGER;")?;
    }
    if !column_exists(conn, "telemetry_records", "link_rssi_dbm")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN link_rssi_dbm INTEGER;")?;
    }
    Ok(())
}

fn migrate_v13_to_v14(conn: &Connection) -> SqlResult<()> {
    ensure_v14_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v15: airspeed (m/s) on telemetry_records — VFR_HUD (MAVLink) / MSP2_INAV_AIR_SPEED (live), ARSP
/// sensor airspeed (ArduPilot DataFlash), `airspeed` column (INAV blackbox). Additive + idempotent.
fn ensure_v15_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "telemetry_records", "airspeed_ms")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN airspeed_ms REAL;")?;
    }
    Ok(())
}

fn migrate_v14_to_v15(conn: &Connection) -> SqlResult<()> {
    ensure_v15_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v16: the vehicle library — a `vehicles` table holding the per-aircraft build sheet. Flights
/// soft-link by the existing `flights.craft_name` (no FK, no flight-row change), so the link works
/// retroactively. Idempotent; only creates the table if missing. Records (max time/dist/alt) are
/// derived from the linked flights, not stored here.
fn ensure_v16_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS vehicles (
            id                        INTEGER PRIMARY KEY AUTOINCREMENT,
            name                      TEXT NOT NULL,
            craft_name                TEXT,
            vehicle_type              TEXT NOT NULL DEFAULT 'other',
            status                    TEXT NOT NULL DEFAULT 'active',
            image                     TEXT,
            notes                     TEXT,
            model                     TEXT,
            wingspan_mm               INTEGER,
            length_mm                 INTEGER,
            weight_auw_g              INTEGER,
            weight_dry_g              INTEGER,
            motors                    TEXT,
            props                     TEXT,
            esc                       TEXT,
            recommended_cells         TEXT,
            recommended_capacity_mah  INTEGER,
            rx                        TEXT,
            vtx                       TEXT,
            camera                    TEXT,
            gimbal_camera             TEXT,
            datalink                  TEXT,
            sensor_airspeed           INTEGER NOT NULL DEFAULT 0,
            sensor_rangefinder        INTEGER NOT NULL DEFAULT 0,
            sensor_optical_flow       INTEGER NOT NULL DEFAULT 0,
            sensor_gps                INTEGER NOT NULL DEFAULT 0,
            sensor_rtk                INTEGER NOT NULL DEFAULT 0,
            sensor_compass            INTEGER NOT NULL DEFAULT 0,
            fc_model                  TEXT,
            fc_manufacturer           TEXT,
            fc_firmware               TEXT,
            fc_firmware_version       TEXT,
            blackbox_available        INTEGER NOT NULL DEFAULT 0,
            base_flight_count         INTEGER NOT NULL DEFAULT 0,
            base_total_time_s         INTEGER NOT NULL DEFAULT 0,
            base_total_dist_m         INTEGER NOT NULL DEFAULT 0,
            base_total_energy         INTEGER NOT NULL DEFAULT 0,
            created_at                TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at                TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    // The base_* baseline columns were added after the initial v16 table — idempotently add them to
    // a table created before they existed (this session's dev builds), so no schema bump is needed.
    for col in [
        "base_flight_count",
        "base_total_time_s",
        "base_total_dist_m",
        "base_total_energy",
    ] {
        if !column_exists(conn, "vehicles", col)? {
            conn.execute_batch(&format!(
                "ALTER TABLE vehicles ADD COLUMN {} INTEGER NOT NULL DEFAULT 0;",
                col
            ))?;
        }
    }
    Ok(())
}

fn migrate_v15_to_v16(conn: &Connection) -> SqlResult<()> {
    ensure_v16_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v17: throttle (%) on telemetry_records — live (INAV MSP2_INAV_MISC2 / MAVLink VFR_HUD), INAV
/// blackbox (RC throttle channel), ArduPilot DataFlash (CTUN throttle-out). Additive + idempotent.
fn ensure_v17_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "telemetry_records", "throttle_pct")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN throttle_pct REAL;")?;
    }
    Ok(())
}

fn migrate_v16_to_v17(conn: &Connection) -> SqlResult<()> {
    ensure_v17_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

/// v18: per-instance battery samples (`battery_records`) for ArduPilot/PX4 multi-monitor logging —
/// imported (BAT.Instance) and live (BATTERY_STATUS.id). INAV stays single-battery (no rows here).
/// Additive + idempotent. See docs/active/MULTI_BATTERY.md.
fn ensure_v18_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(BATTERY_RECORDS_DDL_FULL)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_battery_flight \
         ON battery_records(flight_id, timestamp_ms);",
    )?;
    Ok(())
}

fn migrate_v17_to_v18(conn: &Connection) -> SqlResult<()> {
    ensure_v18_schema(conn)?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_v6_to_v7(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN battery_percentage INTEGER;",
    )?;
    set_user_version(conn, CURRENT_SCHEMA_VERSION)?;
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
    set_user_version(conn, 4)?;
    Ok(())
}

fn migrate_v4_to_v5(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN nav_lat REAL;
         ALTER TABLE telemetry_records ADD COLUMN nav_lon REAL;
         ALTER TABLE telemetry_records ADD COLUMN nav_alt_m REAL;",
    )?;
    set_user_version(conn, 5)?;
    Ok(())
}

fn migrate_v5_to_v6(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE flights ADD COLUMN linked_flight_id INTEGER REFERENCES flights(id);",
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
            battery_used_mah, notes, pilot_name, pilot_id, battery_serial, utc_offset_min
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27
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
            flight.pilot_name,
            flight.pilot_id,
            flight.battery_serial.as_deref().map(normalize_serial_list).filter(|s| !s.is_empty()),
            flight.utc_offset_min,
        ],
    )?;
    Ok(conn.last_insert_rowid())
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
                vario_ms, voltage, current_a, mah_drawn, rssi, battery_percentage,
                roll, pitch, yaw, fix_type, num_sat, cpu_load, link_quality,
                baro_alt_m, gps_hdop, gps_eph, gps_epv,
                active_wp_number, active_flight_mode_flags, state_flags, nav_state, nav_flags,
                rx_signal_received, hw_health_status, baro_temperature,
                wind_n_ms, wind_e_ms, wind_d_ms,
                rc_data_json, rc_command_json,
                nav_lat, nav_lon, nav_alt_m,
                mode_primary, mode_modifiers,
                link_snr, link_rssi_dbm, airspeed_ms, throttle_pct
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29,
                ?30, ?31, ?32,
                ?33, ?34, ?35,
                ?36, ?37,
                ?38, ?39, ?40,
                ?41, ?42,
                ?43, ?44, ?45, ?46
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
                r.battery_percentage,
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
                r.nav_lat,
                r.nav_lon,
                r.nav_alt_m,
                r.mode_primary,
                r.mode_modifiers,
                r.link_snr,
                r.link_rssi_dbm,
                r.airspeed_ms,
                r.throttle_pct,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Insert a batch of per-instance battery samples (multi-battery; see docs/active/MULTI_BATTERY.md).
pub fn insert_battery_records_batch(conn: &Connection, records: &[BatteryRecord]) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO battery_records (
                flight_id, timestamp_ms, instance, voltage, current_a, mah_drawn,
                battery_percentage, cell_count, temperature
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for r in records {
            stmt.execute(params![
                r.flight_id,
                r.timestamp_ms,
                r.instance,
                r.voltage,
                r.current_a,
                r.mah_drawn,
                r.battery_percentage,
                r.cell_count,
                r.temperature,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Load all per-instance battery samples for a flight, ordered by time then instance. Empty for
/// single-battery flights (INAV / ArduPilot with one monitor) — the frontend then falls back to the
/// denormalised primary battery on `telemetry_records`.
pub fn get_flight_battery_records(conn: &Connection, flight_id: i64) -> SqlResult<Vec<BatteryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, flight_id, timestamp_ms, instance, voltage, current_a, mah_drawn,
                battery_percentage, cell_count, temperature
         FROM battery_records
         WHERE flight_id = ?1
         ORDER BY timestamp_ms ASC, instance ASC",
    )?;
    let rows = stmt.query_map(params![flight_id], |row| {
        Ok(BatteryRecord {
            id: row.get(0)?,
            flight_id: row.get(1)?,
            timestamp_ms: row.get(2)?,
            instance: row.get(3)?,
            voltage: row.get(4)?,
            current_a: row.get(5)?,
            mah_drawn: row.get(6)?,
            battery_percentage: row.get(7)?,
            cell_count: row.get(8)?,
            temperature: row.get(9)?,
        })
    })?;
    rows.collect()
}

// ── Live recording: per-session temp store (commit-on-disarm, ADR-040) ──────────────

/// Open (creating its parent dir) a per-session temp SQLite file: the `telemetry_records` table
/// (full current field set) plus a self-describing `session_meta` table so an orphaned `.ktmp`
/// left by a crash can be interpreted on its own (crash recovery is a later phase). WAL +
/// `synchronous = NORMAL`: the file is the durable buffer for the in-flight stream.
pub fn open_temp_session(path: &Path) -> SqlResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    conn.execute_batch(TELEMETRY_RECORDS_DDL_FULL)?;
    conn.execute_batch(BATTERY_RECORDS_DDL_FULL)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_meta (
            id            INTEGER PRIMARY KEY CHECK (id = 1),
            start_time    TEXT NOT NULL,
            craft_name    TEXT,
            fc_variant    TEXT,
            fc_version    TEXT,
            board_id      TEXT,
            platform_type INTEGER,
            protocol      TEXT,
            start_lat     REAL,
            start_lon     REAL
        );
        CREATE INDEX IF NOT EXISTS idx_session_telemetry
            ON telemetry_records(timestamp_ms);",
    )?;
    Ok(conn)
}

/// Write the single `session_meta` row that makes a temp session self-describing.
#[allow(clippy::too_many_arguments)]
pub fn write_session_meta(
    conn: &Connection,
    start_time: &DateTime<Utc>,
    craft_name: &str,
    fc_variant: &str,
    fc_version: &str,
    board_id: &str,
    platform_type: u8,
    protocol: &str,
    start_lat: Option<f64>,
    start_lon: Option<f64>,
) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO session_meta
            (id, start_time, craft_name, fc_variant, fc_version, board_id, platform_type,
             protocol, start_lat, start_lon)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            start_time.to_rfc3339(),
            craft_name,
            fc_variant,
            fc_version,
            board_id,
            platform_type,
            protocol,
            start_lat,
            start_lon,
        ],
    )?;
    Ok(())
}

/// The self-describing metadata of a temp session (from its `session_meta` row).
pub struct SessionMetaRow {
    pub start_time: String,
    pub craft_name: String,
    pub fc_variant: String,
    pub fc_version: String,
    pub board_id: String,
    pub platform_type: u8,
    pub protocol: String,
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
}

/// Read the single `session_meta` row of a temp session (None if absent — e.g. a malformed file).
pub fn read_session_meta(conn: &Connection) -> SqlResult<Option<SessionMetaRow>> {
    conn.query_row(
        "SELECT start_time, craft_name, fc_variant, fc_version, board_id, platform_type, \
                protocol, start_lat, start_lon FROM session_meta WHERE id = 1",
        [],
        |row| {
            Ok(SessionMetaRow {
                start_time: row.get(0)?,
                craft_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                fc_variant: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                fc_version: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                board_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                platform_type: row.get::<_, Option<i64>>(5)?.unwrap_or(0) as u8,
                protocol: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "MSP".into()),
                start_lat: row.get(7)?,
                start_lon: row.get(8)?,
            })
        },
    )
    .optional()
}

/// Count telemetry rows in a temp session (cheap orphan triage — an empty `.ktmp` is worthless).
pub fn temp_session_row_count(conn: &Connection) -> SqlResult<i64> {
    conn.query_row("SELECT COUNT(*) FROM telemetry_records", [], |row| row.get(0))
}

/// Commit a finished temp session into the main DB atomically: insert the finalized `flights` row,
/// ATTACH the temp file, copy its `telemetry_records` (rewriting `flight_id` to the new main id),
/// then DETACH. Returns the new flight id. The main DB therefore only ever sees the flight as a
/// finished whole. `ATTACH`/`DETACH` cannot run inside a transaction, so they bracket it.
pub fn commit_session_to_main(
    conn: &Connection,
    temp_path: &Path,
    flight: &Flight,
) -> SqlResult<i64> {
    let temp_str = temp_path.to_string_lossy().to_string();
    conn.execute("ATTACH DATABASE ?1 AS sess", params![temp_str])?;

    let outcome = (|| -> SqlResult<i64> {
        let tx = conn.unchecked_transaction()?;
        let flight_id = insert_flight(&tx, flight)?;
        // Swap the leading `flight_id` column name for the new id literal in the SELECT.
        let select_cols = TELEMETRY_COLS.replacen("flight_id", &flight_id.to_string(), 1);
        tx.execute(
            &format!(
                "INSERT INTO main.telemetry_records ({cols}) \
                 SELECT {sel} FROM sess.telemetry_records ORDER BY timestamp_ms ASC",
                cols = TELEMETRY_COLS,
                sel = select_cols,
            ),
            [],
        )?;
        // Copy per-instance battery samples too (multi-battery), rewriting flight_id like above.
        let bat_select = BATTERY_RECORDS_COLS.replacen("flight_id", &flight_id.to_string(), 1);
        tx.execute(
            &format!(
                "INSERT INTO main.battery_records ({cols}) \
                 SELECT {sel} FROM sess.battery_records ORDER BY timestamp_ms ASC",
                cols = BATTERY_RECORDS_COLS,
                sel = bat_select,
            ),
            [],
        )?;
        tx.commit()?;
        Ok(flight_id)
    })();

    // Always detach, even if the transaction failed, so the connection isn't left with `sess` bound.
    let _ = conn.execute("DETACH DATABASE sess", []);
    outcome
}

/// Best-effort removal of a temp session file and its WAL/SHM sidecars (after a successful commit).
pub fn remove_temp_session(temp_path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let p = if suffix.is_empty() {
            temp_path.to_path_buf()
        } else {
            let mut s = temp_path.as_os_str().to_os_string();
            s.push(suffix);
            PathBuf::from(s)
        };
        if p.exists() {
            if let Err(e) = std::fs::remove_file(&p) {
                log::warn!("Failed to remove temp session file {}: {}", p.display(), e);
            }
        }
    }
}

/// List flight summaries ordered by start_time DESC.
pub fn list_flights(conn: &Connection) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min
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
            linked_flight_id: row.get(10)?,
            notes: row.get(11)?,
            utc_offset_min: row.get(12)?,
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
                battery_used_mah, notes, linked_flight_id, pilot_name, pilot_id, battery_serial,
                utc_offset_min
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
            linked_flight_id: row.get(24)?,
            pilot_name: row.get(25)?,
            pilot_id: row.get(26)?,
            battery_serial: row.get(27)?,
            utc_offset_min: row.get(28)?,
        })
    })?;

    match rows.next() {
        Some(Ok(f)) => Ok(Some(f)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

// ── Mission library ─────────────────────────────────────────────────

const MISSION_COLS: &str = "id, content_hash, name, format, waypoints_json, source_xml, \
    wp_count, total_distance_m, alt_diff_m, max_alt_m, min_alt_m, \
    bndbox_min_lat, bndbox_min_lon, bndbox_max_lat, bndbox_max_lon, location_name, \
    created_at, notes, home_lat, home_lon";

fn row_to_mission(row: &rusqlite::Row) -> SqlResult<Mission> {
    Ok(Mission {
        id: row.get(0)?,
        content_hash: row.get(1)?,
        name: row.get(2)?,
        format: row.get(3)?,
        waypoints_json: row.get(4)?,
        source_xml: row.get(5)?,
        wp_count: row.get(6)?,
        total_distance_m: row.get(7)?,
        alt_diff_m: row.get(8)?,
        max_alt_m: row.get(9)?,
        min_alt_m: row.get(10)?,
        bndbox_min_lat: row.get(11)?,
        bndbox_min_lon: row.get(12)?,
        bndbox_max_lat: row.get(13)?,
        bndbox_max_lon: row.get(14)?,
        location_name: row.get(15)?,
        created_at: row.get(16)?,
        notes: row.get(17)?,
        home_lat: row.get(18)?,
        home_lon: row.get(19)?,
    })
}

/// Insert a mission, or return the id of an existing one with the same content hash
/// (dedup — the same mission is stored once and shared across flights).
pub fn upsert_mission(conn: &Connection, m: &MissionInput) -> SqlResult<i64> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM missions WHERE content_hash = ?1",
            params![m.content_hash],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO missions (
            content_hash, name, format, waypoints_json, source_xml, wp_count,
            total_distance_m, alt_diff_m, max_alt_m, min_alt_m,
            bndbox_min_lat, bndbox_min_lon, bndbox_max_lat, bndbox_max_lon, notes,
            home_lat, home_lon
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            m.content_hash,
            m.name,
            m.format,
            m.waypoints_json,
            m.source_xml,
            m.wp_count,
            m.total_distance_m,
            m.alt_diff_m,
            m.max_alt_m,
            m.min_alt_m,
            m.bndbox_min_lat,
            m.bndbox_min_lon,
            m.bndbox_max_lat,
            m.bndbox_max_lon,
            m.notes,
            m.home_lat,
            m.home_lon,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// List all library missions (newest first).
pub fn list_missions(conn: &Connection) -> SqlResult<Vec<Mission>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {} FROM missions ORDER BY created_at DESC", MISSION_COLS))?;
    let rows = stmt.query_map([], row_to_mission)?;
    rows.collect()
}

/// Fetch a mission by id.
pub fn get_mission(conn: &Connection, id: i64) -> SqlResult<Option<Mission>> {
    conn.query_row(
        &format!("SELECT {} FROM missions WHERE id = ?1", MISSION_COLS),
        params![id],
        row_to_mission,
    )
    .optional()
}

/// Find a mission by its content hash (import dedup-match / save NEW-vs-OVERWRITE check).
pub fn find_mission_by_hash(conn: &Connection, content_hash: &str) -> SqlResult<Option<Mission>> {
    conn.query_row(
        &format!("SELECT {} FROM missions WHERE content_hash = ?1", MISSION_COLS),
        params![content_hash],
        row_to_mission,
    )
    .optional()
}

/// Overwrite an existing library mission in place (OVERWRITE on save). Updates content + all
/// computed metadata; `created_at` and `location_name` are preserved. The caller should
/// pre-check `find_mission_by_hash` so it does not collide with a *different* existing row.
pub fn update_mission(conn: &Connection, id: i64, m: &MissionInput) -> SqlResult<()> {
    conn.execute(
        "UPDATE missions SET
            content_hash = ?1, name = ?2, format = ?3, waypoints_json = ?4, source_xml = ?5,
            wp_count = ?6, total_distance_m = ?7, alt_diff_m = ?8, max_alt_m = ?9, min_alt_m = ?10,
            bndbox_min_lat = ?11, bndbox_min_lon = ?12, bndbox_max_lat = ?13, bndbox_max_lon = ?14,
            notes = ?15, home_lat = ?16, home_lon = ?17
         WHERE id = ?18",
        params![
            m.content_hash,
            m.name,
            m.format,
            m.waypoints_json,
            m.source_xml,
            m.wp_count,
            m.total_distance_m,
            m.alt_diff_m,
            m.max_alt_m,
            m.min_alt_m,
            m.bndbox_min_lat,
            m.bndbox_min_lon,
            m.bndbox_max_lat,
            m.bndbox_max_lon,
            m.notes,
            m.home_lat,
            m.home_lon,
            id,
        ],
    )?;
    Ok(())
}

/// Update only a mission's name + notes (rename / notes edit in the Manager; content unchanged).
pub fn update_mission_meta(
    conn: &Connection,
    id: i64,
    name: &str,
    notes: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE missions SET name = ?1, notes = ?2 WHERE id = ?3",
        params![name, notes, id],
    )?;
    Ok(())
}

/// Fetch the mission linked to a flight (if any).
pub fn get_mission_for_flight(conn: &Connection, flight_id: i64) -> SqlResult<Option<Mission>> {
    let mission_id: Option<i64> = conn
        .query_row(
            "SELECT mission_id FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .unwrap_or_default();
    match mission_id {
        Some(id) => get_mission(conn, id),
        None => Ok(None),
    }
}

/// Link a recorded flight to a library mission.
pub fn link_flight_mission(conn: &Connection, flight_id: i64, mission_id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET mission_id = ?1 WHERE id = ?2",
        params![mission_id, flight_id],
    )?;
    Ok(())
}

/// Store the waypoint count parsed from a Blackbox header (fallback `X` for replay when no
/// mission is linked).
pub fn set_flight_logged_wp_count(conn: &Connection, flight_id: i64, count: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET logged_wp_count = ?1 WHERE id = ?2",
        params![count, flight_id],
    )?;
    Ok(())
}

/// Unlink a flight from its mission (Logbook unlink). The flight + telemetry are kept.
pub fn unlink_flight_mission(conn: &Connection, flight_id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET mission_id = NULL WHERE id = ?1",
        params![flight_id],
    )?;
    Ok(())
}

/// Delete a library mission, first unlinking any flights that reference it (those flights keep
/// their telemetry and the Blackbox-header WP fallback). The FK has no ON DELETE, so a bare
/// delete of a referenced mission would fail — hence the explicit unlink.
pub fn delete_mission(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET mission_id = NULL WHERE mission_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM missions WHERE id = ?1", params![id])?;
    Ok(())
}

/// List flight summaries that link a given mission (reverse lookup for the Manager + the delete
/// reference warning).
pub fn list_flights_for_mission(conn: &Connection, mission_id: i64) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min
         FROM flights WHERE mission_id = ?1 ORDER BY start_time DESC",
    )?;
    let rows = stmt.query_map(params![mission_id], |row| {
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
            linked_flight_id: row.get(10)?,
            notes: row.get(11)?,
            utc_offset_min: row.get(12)?,
        })
    })?;
    rows.collect()
}

// ── Battery library ─────────────────────────────────────────────────

const BATTERY_COLS: &str = "id, serial, label, manufacturer, model, chemistry, cell_count, \
    capacity_mah, c_rating_discharge, c_rating_charge, connector, in_service_date, status, \
    notes, created_at, base_flight_seconds, base_mah, base_cycles, base_charges";

fn row_to_battery(row: &rusqlite::Row) -> SqlResult<BatteryPack> {
    Ok(BatteryPack {
        id: row.get(0)?,
        serial: row.get(1)?,
        label: row.get(2)?,
        manufacturer: row.get(3)?,
        model: row.get(4)?,
        chemistry: row.get(5)?,
        cell_count: row.get(6)?,
        capacity_mah: row.get(7)?,
        c_rating_discharge: row.get(8)?,
        c_rating_charge: row.get(9)?,
        connector: row.get(10)?,
        in_service_date: row.get(11)?,
        status: row.get(12)?,
        notes: row.get(13)?,
        created_at: row.get(14)?,
        base_flight_seconds: row.get(15)?,
        base_mah: row.get(16)?,
        base_cycles: row.get(17)?,
        base_charges: row.get(18)?,
    })
}

/// Canonical battery serial: ASCII alphanumerics only, upper-cased. The serial is the soft-link key,
/// and free-form spaces/punctuation/case are a common source of links that silently fail to resolve
/// (and hardware barcodes are upper alnum anyway). We hard-normalize at every store + lookup so the
/// exact-match link is reliable; keep this identical to the frontend `normalizeSerial`.
pub fn normalize_serial(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Canonical battery-serial *list*: a single flight may link several packs, stored comma-separated in
/// the freetext `flights.battery_serial` column. Split on commas, normalize each token with
/// `normalize_serial`, drop empties, de-duplicate (order-preserving), and rejoin with a plain comma
/// (no spaces) — a canonical token list the `*_for_serial` lookups match exactly. Keep in sync with the
/// frontend `normalizeSerialList`.
pub fn normalize_serial_list(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in s.split(',') {
        let n = normalize_serial(tok);
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    out.join(",")
}

/// Create a new battery pack. The `serial` is UNIQUE → a duplicate surfaces as an error
/// (the caller should pre-check `find_battery_by_serial`). The `base_*` baseline starts at 0.
pub fn create_battery(conn: &Connection, b: &BatteryPackInput) -> SqlResult<i64> {
    let serial = normalize_serial(&b.serial);
    conn.execute(
        "INSERT INTO battery_packs (
            serial, label, manufacturer, model, chemistry, cell_count, capacity_mah,
            c_rating_discharge, c_rating_charge, connector, in_service_date, status, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            serial, b.label, b.manufacturer, b.model, b.chemistry, b.cell_count, b.capacity_mah,
            b.c_rating_discharge, b.c_rating_charge, b.connector, b.in_service_date, b.status, b.notes,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update a pack's identity/spec fields. **Does not touch `serial`** (the soft-link key — kept
/// stable so existing flight links don't break) **nor the `base_*` baseline** (additive only).
pub fn update_battery(conn: &Connection, id: i64, b: &BatteryPackInput) -> SqlResult<()> {
    conn.execute(
        "UPDATE battery_packs SET
            label = ?1, manufacturer = ?2, model = ?3, chemistry = ?4, cell_count = ?5,
            capacity_mah = ?6, c_rating_discharge = ?7, c_rating_charge = ?8, connector = ?9,
            in_service_date = ?10, status = ?11, notes = ?12
         WHERE id = ?13",
        params![
            b.label, b.manufacturer, b.model, b.chemistry, b.cell_count, b.capacity_mah,
            b.c_rating_discharge, b.c_rating_charge, b.connector, b.in_service_date, b.status,
            b.notes, id,
        ],
    )?;
    Ok(())
}

/// List all battery packs (newest first).
pub fn list_batteries(conn: &Connection) -> SqlResult<Vec<BatteryPack>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {} FROM battery_packs ORDER BY created_at DESC", BATTERY_COLS))?;
    let rows = stmt.query_map([], row_to_battery)?;
    rows.collect()
}

/// Fetch a pack by id.
pub fn get_battery(conn: &Connection, id: i64) -> SqlResult<Option<BatteryPack>> {
    conn.query_row(
        &format!("SELECT {} FROM battery_packs WHERE id = ?1", BATTERY_COLS),
        params![id],
        row_to_battery,
    )
    .optional()
}

/// Find a pack by serial (link resolution / unknown-serial dedup check).
pub fn find_battery_by_serial(conn: &Connection, serial: &str) -> SqlResult<Option<BatteryPack>> {
    let serial = normalize_serial(serial);
    conn.query_row(
        &format!("SELECT {} FROM battery_packs WHERE serial = ?1", BATTERY_COLS),
        params![serial],
        row_to_battery,
    )
    .optional()
}

/// Delete a pack. Flights keep their `battery_serial` and fall back to "not in library" (the
/// link is by serial, not an FK) — no NULLing needed.
pub fn delete_battery(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM battery_packs WHERE id = ?1", params![id])?;
    Ok(())
}

/// Add consumption to the persistent baseline (additive only — manual usage editor and the
/// flight-deletion "transfer to baseline" path). Never sets absolutes.
pub fn add_battery_usage(
    conn: &Connection,
    id: i64,
    d_flight_seconds: i64,
    d_mah: i64,
    d_cycles: f64,
    d_charges: i64,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE battery_packs SET
            base_flight_seconds = base_flight_seconds + ?1,
            base_mah            = base_mah + ?2,
            base_cycles         = base_cycles + ?3,
            base_charges        = base_charges + ?4
         WHERE id = ?5",
        params![d_flight_seconds, d_mah, d_cycles, d_charges, id],
    )?;
    Ok(())
}

/// Set the persistent baseline to absolute values (import "new" / "overwrite"; not additive).
pub fn set_battery_baseline(
    conn: &Connection,
    id: i64,
    flight_seconds: i64,
    mah: i64,
    cycles: f64,
    charges: i64,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE battery_packs SET
            base_flight_seconds = ?1, base_mah = ?2, base_cycles = ?3, base_charges = ?4
         WHERE id = ?5",
        params![flight_seconds, mah, cycles, charges, id],
    )?;
    Ok(())
}

/// Aggregate the flights linked to a serial (the dynamic part of the lifetime; combined with the
/// pack's `base_*` baseline on the frontend).
pub fn battery_aggregate(conn: &Connection, serial: &str) -> SqlResult<BatteryAggregate> {
    conn.query_row(
        // A flight may link several packs (comma-separated list); match this serial as one token of
        // that list. Stored values are canonical (alnum tokens, comma-joined, no spaces), so wrapping
        // both sides in commas makes this an exact per-token match (and still matches a lone serial).
        "SELECT COUNT(*), COALESCE(SUM(duration_sec), 0), COALESCE(SUM(battery_used_mah), 0),
                MIN(start_time), MAX(start_time)
         FROM flights WHERE (',' || battery_serial || ',') LIKE ('%,' || ?1 || ',%')",
        params![serial],
        |row| {
            Ok(BatteryAggregate {
                flight_count: row.get(0)?,
                sum_duration_sec: row.get(1)?,
                sum_mah: row.get(2)?,
                first_used: row.get(3)?,
                last_used: row.get(4)?,
            })
        },
    )
}

/// List flight summaries linked to a serial (Manager detail + the delete reference warning).
pub fn list_flights_for_serial(conn: &Connection, serial: &str) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min
         FROM flights WHERE (',' || battery_serial || ',') LIKE ('%,' || ?1 || ',%')
         ORDER BY start_time DESC",
    )?;
    let rows = stmt.query_map(params![serial], |row| {
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
            linked_flight_id: row.get(10)?,
            notes: row.get(11)?,
            utc_offset_min: row.get(12)?,
        })
    })?;
    rows.collect()
}

/// Set (or clear, with `None`) the soft battery-serial link on a flight.
pub fn set_flight_battery_serial(
    conn: &Connection,
    flight_id: i64,
    serial: Option<&str>,
) -> SqlResult<()> {
    // Normalize the (possibly multi-pack, comma-separated) list + treat an empty result (blank /
    // punctuation-only) as clearing the link.
    let serial = serial.map(normalize_serial_list).filter(|s| !s.is_empty());
    conn.execute(
        "UPDATE flights SET battery_serial = ?1 WHERE id = ?2",
        params![serial, flight_id],
    )?;
    Ok(())
}

// ── Vehicle library ─────────────────────────────────────────────────

const VEHICLE_COLS: &str = "id, name, craft_name, vehicle_type, status, image, notes, \
    model, wingspan_mm, length_mm, weight_auw_g, weight_dry_g, \
    motors, props, esc, recommended_cells, recommended_capacity_mah, \
    rx, vtx, camera, gimbal_camera, datalink, \
    sensor_airspeed, sensor_rangefinder, sensor_optical_flow, sensor_gps, sensor_rtk, sensor_compass, \
    fc_model, fc_manufacturer, fc_firmware, fc_firmware_version, blackbox_available, \
    base_flight_count, base_total_time_s, base_total_dist_m, base_total_energy, \
    created_at, updated_at";

fn row_to_vehicle(row: &rusqlite::Row) -> SqlResult<Vehicle> {
    Ok(Vehicle {
        id: row.get(0)?,
        name: row.get(1)?,
        craft_name: row.get(2)?,
        vehicle_type: row.get(3)?,
        status: row.get(4)?,
        image: row.get(5)?,
        notes: row.get(6)?,
        model: row.get(7)?,
        wingspan_mm: row.get(8)?,
        length_mm: row.get(9)?,
        weight_auw_g: row.get(10)?,
        weight_dry_g: row.get(11)?,
        motors: row.get(12)?,
        props: row.get(13)?,
        esc: row.get(14)?,
        recommended_cells: row.get(15)?,
        recommended_capacity_mah: row.get(16)?,
        rx: row.get(17)?,
        vtx: row.get(18)?,
        camera: row.get(19)?,
        gimbal_camera: row.get(20)?,
        datalink: row.get(21)?,
        sensor_airspeed: row.get(22)?,
        sensor_rangefinder: row.get(23)?,
        sensor_optical_flow: row.get(24)?,
        sensor_gps: row.get(25)?,
        sensor_rtk: row.get(26)?,
        sensor_compass: row.get(27)?,
        fc_model: row.get(28)?,
        fc_manufacturer: row.get(29)?,
        fc_firmware: row.get(30)?,
        fc_firmware_version: row.get(31)?,
        blackbox_available: row.get(32)?,
        base_flight_count: row.get(33)?,
        base_total_time_s: row.get(34)?,
        base_total_dist_m: row.get(35)?,
        base_total_energy: row.get(36)?,
        created_at: row.get(37)?,
        updated_at: row.get(38)?,
    })
}

/// Canonical craft name: trim leading/trailing whitespace, preserve case + inner content (it is a
/// user-facing display string, unlike a battery serial). The flight↔vehicle match is done
/// case-insensitively (`COLLATE NOCASE`) on the trimmed value. Keep identical to the frontend
/// `normalizeCraftName`. Returns `None` for an empty/whitespace-only string.
pub fn normalize_craft_name(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Create a vehicle. Returns the new row id.
pub fn create_vehicle(conn: &Connection, v: &VehicleInput) -> SqlResult<i64> {
    let craft = v.craft_name.as_deref().and_then(normalize_craft_name);
    conn.execute(
        "INSERT INTO vehicles (
            name, craft_name, vehicle_type, status, image, notes,
            model, wingspan_mm, length_mm, weight_auw_g, weight_dry_g,
            motors, props, esc, recommended_cells, recommended_capacity_mah,
            rx, vtx, camera, gimbal_camera, datalink,
            sensor_airspeed, sensor_rangefinder, sensor_optical_flow, sensor_gps, sensor_rtk, sensor_compass,
            fc_model, fc_manufacturer, fc_firmware, fc_firmware_version, blackbox_available
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
            ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32
        )",
        params![
            v.name, craft, v.vehicle_type, v.status, v.image, v.notes,
            v.model, v.wingspan_mm, v.length_mm, v.weight_auw_g, v.weight_dry_g,
            v.motors, v.props, v.esc, v.recommended_cells, v.recommended_capacity_mah,
            v.rx, v.vtx, v.camera, v.gimbal_camera, v.datalink,
            v.sensor_airspeed, v.sensor_rangefinder, v.sensor_optical_flow, v.sensor_gps, v.sensor_rtk, v.sensor_compass,
            v.fc_model, v.fc_manufacturer, v.fc_firmware, v.fc_firmware_version, v.blackbox_available,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update a vehicle's fields (bumps `updated_at`).
pub fn update_vehicle(conn: &Connection, id: i64, v: &VehicleInput) -> SqlResult<()> {
    let craft = v.craft_name.as_deref().and_then(normalize_craft_name);
    conn.execute(
        "UPDATE vehicles SET
            name = ?1, craft_name = ?2, vehicle_type = ?3, status = ?4, image = ?5, notes = ?6,
            model = ?7, wingspan_mm = ?8, length_mm = ?9, weight_auw_g = ?10, weight_dry_g = ?11,
            motors = ?12, props = ?13, esc = ?14, recommended_cells = ?15, recommended_capacity_mah = ?16,
            rx = ?17, vtx = ?18, camera = ?19, gimbal_camera = ?20, datalink = ?21,
            sensor_airspeed = ?22, sensor_rangefinder = ?23, sensor_optical_flow = ?24,
            sensor_gps = ?25, sensor_rtk = ?26, sensor_compass = ?27,
            fc_model = ?28, fc_manufacturer = ?29, fc_firmware = ?30, fc_firmware_version = ?31,
            blackbox_available = ?32, updated_at = datetime('now')
         WHERE id = ?33",
        params![
            v.name, craft, v.vehicle_type, v.status, v.image, v.notes,
            v.model, v.wingspan_mm, v.length_mm, v.weight_auw_g, v.weight_dry_g,
            v.motors, v.props, v.esc, v.recommended_cells, v.recommended_capacity_mah,
            v.rx, v.vtx, v.camera, v.gimbal_camera, v.datalink,
            v.sensor_airspeed, v.sensor_rangefinder, v.sensor_optical_flow, v.sensor_gps, v.sensor_rtk, v.sensor_compass,
            v.fc_model, v.fc_manufacturer, v.fc_firmware, v.fc_firmware_version, v.blackbox_available, id,
        ],
    )?;
    Ok(())
}

/// List all vehicles (newest first).
pub fn list_vehicles(conn: &Connection) -> SqlResult<Vec<Vehicle>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {} FROM vehicles ORDER BY created_at DESC", VEHICLE_COLS))?;
    let rows = stmt.query_map([], row_to_vehicle)?;
    rows.collect()
}

/// Fetch a vehicle by id.
pub fn get_vehicle(conn: &Connection, id: i64) -> SqlResult<Option<Vehicle>> {
    conn.query_row(
        &format!("SELECT {} FROM vehicles WHERE id = ?1", VEHICLE_COLS),
        params![id],
        row_to_vehicle,
    )
    .optional()
}

/// Find a vehicle by craft name (link resolution / "create from craft name" check). Trimmed +
/// case-insensitive. Returns the most recently created match if several share a craft name.
pub fn find_vehicle_by_craft_name(conn: &Connection, craft: &str) -> SqlResult<Option<Vehicle>> {
    let Some(craft) = normalize_craft_name(craft) else {
        return Ok(None);
    };
    conn.query_row(
        &format!(
            "SELECT {} FROM vehicles WHERE TRIM(craft_name) = ?1 COLLATE NOCASE \
             ORDER BY created_at DESC LIMIT 1",
            VEHICLE_COLS
        ),
        params![craft],
        row_to_vehicle,
    )
    .optional()
}

/// Delete a vehicle. Flights keep their `craft_name` and fall back to "not in library".
pub fn delete_vehicle(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM vehicles WHERE id = ?1", params![id])?;
    Ok(())
}

/// Set the persistent lifetime baseline to absolute values (adopt the INAV FC `stats` totals, or
/// restore from a `.kvehicle` import). Not additive — overwrites the stored baseline.
pub fn set_vehicle_baseline(
    conn: &Connection,
    id: i64,
    flight_count: i64,
    total_time_s: i64,
    total_dist_m: i64,
    total_energy: i64,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE vehicles SET
            base_flight_count = ?1, base_total_time_s = ?2, base_total_dist_m = ?3, base_total_energy = ?4
         WHERE id = ?5",
        params![flight_count, total_time_s, total_dist_m, total_energy, id],
    )?;
    Ok(())
}

/// Aggregate the flights linked to a craft name: totals + the per-flight records (max flight
/// time / distance / altitude, each with the achieving flight id). Trimmed + case-insensitive.
pub fn vehicle_aggregate(conn: &Connection, craft: &str) -> SqlResult<VehicleAggregate> {
    let Some(craft) = normalize_craft_name(craft) else {
        return Ok(VehicleAggregate::default());
    };
    let mut agg = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(duration_sec), 0), COALESCE(SUM(total_distance_m), 0),
                MIN(start_time), MAX(start_time)
         FROM flights WHERE TRIM(craft_name) = ?1 COLLATE NOCASE",
        params![craft],
        |row| {
            Ok(VehicleAggregate {
                flight_count: row.get(0)?,
                sum_duration_sec: row.get(1)?,
                sum_distance_m: row.get(2)?,
                first_used: row.get(3)?,
                last_used: row.get(4)?,
                ..Default::default()
            })
        },
    )?;

    // Records: the single best flight per metric (argmax), each as (flight_id, value).
    if let Some((id, v)) = conn
        .query_row(
            "SELECT id, duration_sec FROM flights
             WHERE TRIM(craft_name) = ?1 COLLATE NOCASE AND duration_sec IS NOT NULL
             ORDER BY duration_sec DESC LIMIT 1",
            params![craft],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?
    {
        agg.max_flight_time_flight_id = Some(id);
        agg.max_flight_time_sec = Some(v);
    }
    if let Some((id, v)) = conn
        .query_row(
            "SELECT id, total_distance_m FROM flights
             WHERE TRIM(craft_name) = ?1 COLLATE NOCASE AND total_distance_m IS NOT NULL
             ORDER BY total_distance_m DESC LIMIT 1",
            params![craft],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()?
    {
        agg.max_distance_flight_id = Some(id);
        agg.max_distance_m = Some(v);
    }
    if let Some((id, v)) = conn
        .query_row(
            "SELECT id, max_alt_m FROM flights
             WHERE TRIM(craft_name) = ?1 COLLATE NOCASE AND max_alt_m IS NOT NULL
             ORDER BY max_alt_m DESC LIMIT 1",
            params![craft],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()?
    {
        agg.max_altitude_flight_id = Some(id);
        agg.max_altitude_m = Some(v);
    }
    Ok(agg)
}

/// List the flight summaries linked to a craft name (Manager detail + delete reference warning).
pub fn list_flights_for_craft(conn: &Connection, craft: &str) -> SqlResult<Vec<FlightSummary>> {
    let Some(craft) = normalize_craft_name(craft) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min
         FROM flights WHERE TRIM(craft_name) = ?1 COLLATE NOCASE ORDER BY start_time DESC",
    )?;
    let rows = stmt.query_map(params![craft], |row| {
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
            linked_flight_id: row.get(10)?,
            notes: row.get(11)?,
            utc_offset_min: row.get(12)?,
        })
    })?;
    rows.collect()
}

/// Read the Blackbox-header waypoint count for a flight (fallback `X` for replay).
pub fn get_flight_logged_wp_count(conn: &Connection, flight_id: i64) -> SqlResult<Option<i64>> {
    match conn
        .query_row(
            "SELECT logged_wp_count FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
    {
        Some(inner) => Ok(inner),
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
                heading, vario_ms, voltage, current_a, mah_drawn, rssi, battery_percentage,
            roll, pitch, yaw, fix_type, num_sat, cpu_load, link_quality,
            baro_alt_m, gps_hdop, gps_eph, gps_epv,
            active_wp_number, active_flight_mode_flags, state_flags, nav_state, nav_flags,
            rx_signal_received, hw_health_status, baro_temperature,
            wind_n_ms, wind_e_ms, wind_d_ms,
            rc_data_json, rc_command_json,
            nav_lat, nav_lon, nav_alt_m,
            mode_primary, mode_modifiers,
            link_snr, link_rssi_dbm, airspeed_ms, throttle_pct
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
            battery_percentage: row.get(13)?,
            roll: row.get(14)?,
            pitch: row.get(15)?,
            yaw: row.get(16)?,
            fix_type: row.get(17)?,
            num_sat: row.get(18)?,
            cpu_load: row.get(19)?,
            link_quality: row.get(20)?,
            baro_alt_m: row.get(21)?,
            gps_hdop: row.get(22)?,
            gps_eph: row.get(23)?,
            gps_epv: row.get(24)?,
            active_wp_number: row.get(25)?,
            active_flight_mode_flags: row.get(26)?,
            state_flags: row.get(27)?,
            nav_state: row.get(28)?,
            nav_flags: row.get(29)?,
            rx_signal_received: row.get(30)?,
            hw_health_status: row.get(31)?,
            baro_temperature: row.get(32)?,
            wind_n_ms: row.get(33)?,
            wind_e_ms: row.get(34)?,
            wind_d_ms: row.get(35)?,
            rc_data_json: row.get(36)?,
            rc_command_json: row.get(37)?,
            nav_lat: row.get(38)?,
            nav_lon: row.get(39)?,
            nav_alt_m: row.get(40)?,
            mode_primary: row.get(41)?,
            mode_modifiers: row.get(42)?,
            link_snr: row.get(43)?,
            link_rssi_dbm: row.get(44)?,
            airspeed_ms: row.get(45)?,
            throttle_pct: row.get(46)?,
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
    // Only count blackbox-source flights as duplicates — live flights are linkable, not duplicates
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM flights
             WHERE craft_name = ?1 AND start_time >= ?2 AND start_time <= ?3
               AND source IN ('blackbox', 'both')",
            params![craft_name, time_lower, time_upper],
            |row| row.get(0),
        )
        .map_err(|e| format!("Duplicate check COUNT failed: {}", e))?;

    eprintln!("[DUP-DB] COUNT result: {}", count);

    if count == 0 {
        return Ok(None);
    }

    // Step 2: Fetch the first matching blackbox flight
    let flight = conn
        .query_row(
            "SELECT id, start_time, end_time, duration_sec, source, craft_name, 
                    fc_variant, fc_version, board_id, platform_type, protocol,
                    start_lat, start_lon, location_name, weather_temp_c, weather_wind_ms,
                    weather_wind_deg, weather_desc, max_alt_m, max_speed_ms, max_distance_m,
                    total_distance_m, battery_used_mah, notes, linked_flight_id
             FROM flights
             WHERE craft_name = ?1 AND start_time >= ?2 AND start_time <= ?3
               AND source IN ('blackbox', 'both')
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
                    linked_flight_id: row.get(24)?,
                    // Not selected here (duplicate-detection lookup only).
                    pilot_name: None,
                    pilot_id: None,
                    battery_serial: None,
                    utc_offset_min: None,
                })
            },
        )
        .map_err(|e| format!("Duplicate check flight fetch failed: {}", e))?;

    Ok(Some(flight))
}

/// Delete a flight and all related data (telemetry, blackbox records, archived files).
/// Explicitly deletes child rows first (in case foreign_keys is off), then VACUUMs.
pub fn delete_flight(conn: &Connection, flight_id: i64) -> SqlResult<bool> {
    // Clear any linked_flight_id references pointing to this flight
    conn.execute(
        "UPDATE flights SET linked_flight_id = NULL, source = CASE
            WHEN (SELECT COUNT(*) FROM blackbox_records WHERE flight_id = flights.id) > 0 THEN 'blackbox'
            ELSE 'live'
         END
         WHERE linked_flight_id = ?1",
        params![flight_id],
    )?;

    // Explicitly delete child tables (don't rely solely on CASCADE)
    conn.execute("DELETE FROM blackbox_files WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM blackbox_records WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM telemetry_records WHERE flight_id = ?1", params![flight_id])?;
    let affected = conn.execute("DELETE FROM flights WHERE id = ?1", params![flight_id])?;

    // Reclaim the freed pages (large blackbox BLOBs) — cheap incremental reclaim, not a full rewrite.
    if affected > 0 {
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
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

/// Update the craft_name field of a flight.
pub fn update_flight_craft_name(
    conn: &Connection,
    flight_id: i64,
    craft_name: &str,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET craft_name = ?1 WHERE id = ?2",
        params![craft_name, flight_id],
    )?;
    Ok(())
}

/// Update the UAV platform type of a flight (INAV mixer enum: 0=multirotor … 6=other).
/// Manually editable in the flight detail (also drives the map replay symbol).
pub fn update_flight_platform_type(
    conn: &Connection,
    flight_id: i64,
    platform_type: u8,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET platform_type = ?1 WHERE id = ?2",
        params![platform_type, flight_id],
    )?;
    Ok(())
}

/// Update the pilot metadata (name + id) of a flight. Empty strings are stored as NULL.
pub fn update_flight_pilot(
    conn: &Connection,
    flight_id: i64,
    pilot_name: Option<&str>,
    pilot_id: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE flights SET pilot_name = ?1, pilot_id = ?2 WHERE id = ?3",
        params![pilot_name, pilot_id, flight_id],
    )?;
    Ok(())
}

/// Link two flights together (bidirectional). Sets `linked_flight_id` on both
/// and updates their `source` to "both".
pub fn link_flights(conn: &Connection, flight_a: i64, flight_b: i64) -> SqlResult<()> {
    eprintln!("[LINK] Linking flights {} <-> {}", flight_a, flight_b);
    conn.execute(
        "UPDATE flights SET linked_flight_id = ?1, source = 'both' WHERE id = ?2",
        params![flight_b, flight_a],
    )?;
    conn.execute(
        "UPDATE flights SET linked_flight_id = ?1, source = 'both' WHERE id = ?2",
        params![flight_a, flight_b],
    )?;
    Ok(())
}

/// Remove the link from a flight and its partner. Resets source based on
/// whether the flight has blackbox_records (→ "blackbox") or not (→ "live").
pub fn unlink_flight(conn: &Connection, flight_id: i64) -> SqlResult<()> {
    // Find the partner
    let partner_id: Option<i64> = conn
        .query_row(
            "SELECT linked_flight_id FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    // Clear link on this flight and restore source
    let has_bbx: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM blackbox_records WHERE flight_id = ?1",
        params![flight_id],
        |row| row.get(0),
    )?;
    let source = if has_bbx { "blackbox" } else { "live" };
    conn.execute(
        "UPDATE flights SET linked_flight_id = NULL, source = ?1 WHERE id = ?2",
        params![source, flight_id],
    )?;

    // Clear link on partner (if exists)
    if let Some(pid) = partner_id {
        let partner_has_bbx: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM blackbox_records WHERE flight_id = ?1",
            params![pid],
            |row| row.get(0),
        )?;
        let partner_source = if partner_has_bbx { "blackbox" } else { "live" };
        conn.execute(
            "UPDATE flights SET linked_flight_id = NULL, source = ?1 WHERE id = ?2",
            params![partner_source, pid],
        )?;
        eprintln!("[UNLINK] Unlinked flight {} (source={}) from {} (source={})", flight_id, source, pid, partner_source);
    } else {
        eprintln!("[UNLINK] Flight {} had no partner", flight_id);
    }

    Ok(())
}

/// Find a live flight that could be linked to a blackbox import.
/// Matches on craft_name and overlapping time window (±60 seconds).
/// Duration-match tolerance (seconds) for the craft-name-less fallback (covers the recorder's arm/disarm
/// grace + detection latency). Passive telemetry (FrSky/CRSF) carries no craft name, so a blackbox of the
/// same flight can't match on name — fall back to a near-identical duration within the ±60 s start window.
const DURATION_MATCH_TOLERANCE_SEC: i64 = 10;

pub fn find_linkable_live_flight(
    conn: &Connection,
    craft_name: &str,
    start_time: DateTime<Utc>,
    duration_sec: i64,
) -> SqlResult<Option<FlightSummary>> {
    let time_lower = (start_time - chrono::Duration::seconds(60)).to_rfc3339();
    let time_upper = (start_time + chrono::Duration::seconds(60)).to_rfc3339();

    let result = conn
        .query_row(
            "SELECT id, start_time, duration_sec, source, craft_name, location_name,
                                        max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id
             FROM flights
             WHERE source = 'live' AND linked_flight_id IS NULL
               AND start_time >= ?2 AND start_time <= ?3
               AND ( craft_name = ?1
                     OR (craft_name = '' AND ABS(duration_sec - ?4) <= ?5) )
             ORDER BY (craft_name = ?1) DESC, ABS(duration_sec - ?4) ASC, id DESC LIMIT 1",
            params![craft_name, time_lower, time_upper, duration_sec, DURATION_MATCH_TOLERANCE_SEC],
            |row| {
                let ts_str: String = row.get(1)?;
                let st = DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(FlightSummary {
                    id: row.get(0)?,
                    start_time: st,
                    duration_sec: row.get(2)?,
                    source: row.get(3)?,
                    craft_name: row.get(4)?,
                    location_name: row.get(5)?,
                    max_alt_m: row.get(6)?,
                    max_speed_ms: row.get(7)?,
                    total_distance_m: row.get(8)?,
                    platform_type: row.get(9)?,
                    linked_flight_id: row.get(10)?,
                    notes: None,
                    utc_offset_min: None,
                })
            },
        )
        .optional()?;

    Ok(result)
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

/// Retrieve the first blackbox file BLOB + original filename for a flight.
/// Also checks the linked partner flight if no file is found directly.
/// Returns None if no blackbox file is attached.
pub fn get_blackbox_file(
    conn: &Connection,
    flight_id: i64,
) -> SqlResult<Option<(String, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT original_filename, file_data FROM blackbox_files WHERE flight_id = ?1 LIMIT 1",
    )?;
    let result = stmt
        .query_row(params![flight_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .optional()?;

    if result.is_some() {
        return Ok(result);
    }

    // Check linked partner flight
    let linked_id: Option<i64> = conn
        .query_row(
            "SELECT linked_flight_id FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    if let Some(partner_id) = linked_id {
        let partner_result = conn
            .prepare(
                "SELECT original_filename, file_data FROM blackbox_files WHERE flight_id = ?1 LIMIT 1",
            )?
            .query_row(params![partner_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()?;
        return Ok(partner_result);
    }

    Ok(None)
}

/// Original blackbox file info (filename + size in bytes) stored for this flight (or its linked
/// partner). Lightweight — does not load the BLOB. `None` if there's no stored original. Gates the
/// export / delete-original buttons and supplies the size shown next to them.
pub fn blackbox_file_info(conn: &Connection, flight_id: i64) -> SqlResult<Option<(String, i64)>> {
    let here: Option<(String, i64)> = conn
        .query_row(
            "SELECT original_filename, file_size FROM blackbox_files WHERE flight_id = ?1 LIMIT 1",
            params![flight_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if here.is_some() {
        return Ok(here);
    }
    // A 'both' flight may hold the file on its linked live/blackbox partner.
    let linked_id: Option<i64> = conn
        .query_row(
            "SELECT linked_flight_id FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(partner) = linked_id {
        return conn
            .query_row(
                "SELECT original_filename, file_size FROM blackbox_files WHERE flight_id = ?1 LIMIT 1",
                params![partner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional();
    }
    Ok(None)
}

/// Delete the stored original blackbox file BLOB for a flight (or its linked partner), keeping the
/// parsed `blackbox_records` + the flight row intact — replay still works from the DB, only the
/// "export original" capability is lost. Returns the deleted file's original filename (None if there
/// was nothing to delete).
pub fn delete_blackbox_file(conn: &Connection, flight_id: i64) -> SqlResult<Option<String>> {
    // Resolve which flight actually holds the file: this one, else its linked partner.
    let mut target: Option<i64> = None;
    let here: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM blackbox_files WHERE flight_id = ?1)",
        params![flight_id],
        |row| row.get(0),
    )?;
    if here {
        target = Some(flight_id);
    } else if let Some(partner) = conn
        .query_row(
            "SELECT linked_flight_id FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten()
    {
        let there: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM blackbox_files WHERE flight_id = ?1)",
            params![partner],
            |row| row.get(0),
        )?;
        if there {
            target = Some(partner);
        }
    }

    let Some(target_id) = target else {
        return Ok(None);
    };

    let filename: Option<String> = conn
        .query_row(
            "SELECT original_filename FROM blackbox_files WHERE flight_id = ?1 LIMIT 1",
            params![target_id],
            |row| row.get(0),
        )
        .optional()?;
    let affected = conn.execute(
        "DELETE FROM blackbox_files WHERE flight_id = ?1",
        params![target_id],
    )?;
    if affected > 0 {
        // Reclaim the freed BLOB pages incrementally (cheap), not via a full-DB VACUUM rewrite.
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
    }
    Ok(filename)
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
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
        };
        let id = insert_flight(&conn, &flight).unwrap();
        let loaded = get_flight(&conn, id).unwrap().unwrap();
        assert_eq!(loaded.craft_name, "TestCraft");
        assert_eq!(loaded.fc_variant, "INAV");
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
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
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
                airspeed_ms: Some(11.0),
                throttle_pct: Some(50.0),
                heading: Some(90.0),
                vario_ms: Some(0.5),
                voltage: Some(12.6),
                current_a: Some(15.0),
                mah_drawn: Some(i as u32 * 5),
                rssi: Some(95),
                battery_percentage: None,
                roll: Some(2.0),
                pitch: Some(5.0),
                yaw: Some(90.0),
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
                nav_lat: None,
                nav_lon: None,
                nav_alt_m: None,
                mode_primary: None,
                mode_modifiers: None,
                link_snr: None,
                link_rssi_dbm: None,
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
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
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
            airspeed_ms: None,
            throttle_pct: None,
            heading: None,
            vario_ms: None,
            voltage: None,
            current_a: None,
            mah_drawn: None,
            rssi: None,
            battery_percentage: None,
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
            nav_lat: None,
            nav_lon: None,
            nav_alt_m: None,
            mode_primary: None,
            mode_modifiers: None,
            link_snr: None,
            link_rssi_dbm: None,
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
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
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
