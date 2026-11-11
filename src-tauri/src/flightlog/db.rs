// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// SQLite database for flight logging
// Uses rusqlite with bundled SQLite — no external dependencies required.
// Schema evolution via PRAGMA user_version + sequential migrations.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

use super::types::{
    BatteryAggregate, BatteryPack, BatteryPackInput, BatteryRecord, Flight, FlightEvent, FlightGroup,
    FlightSummary, Mission,
    MissionInput, TelemetryRecord, Vehicle, VehicleAggregate, VehicleInput, VehicleResolution,
};
use crate::msp::hardware_uid;
use crate::msp::types::INAV_SITL_UID;

const CURRENT_SCHEMA_VERSION: u32 = 21;

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
    throttle_pct, wall_ms";

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
    airspeed_ms REAL, throttle_pct REAL, wall_ms INTEGER
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
    // handed back via `PRAGMA incremental_vacuum` in the background after a delete (`spawn_reclaim`) —
    // cost ∝ freed pages, not the whole DB. (A full `VACUUM` rewrites the entire file, which is unusable once logs reach GBs.)
    // The mode can only be set before tables exist on a fresh DB, or converted with one VACUUM on an
    // existing one — so do that one-time conversion here when the DB isn't already INCREMENTAL (2).
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
    if auto_vacuum != 2 {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        // Cheap on a fresh/empty DB; a one-time rewrite on an existing one (then never again).
        conn.execute_batch("VACUUM;")?;
    }

    // Migration safety net (Dev-Docs active/DB_MIGRATIONS.md): when schema steps are pending on an
    // EXISTING database, snapshot it first — flight archives are irreplaceable. Exactly ONE backup
    // is kept (the file is multi-GB with blackbox blobs); each upgrade replaces the previous one.
    // A fresh DB (v0) has nothing to lose and gets none. If the snapshot fails (disk full, …) the
    // open fails too — migrating without the safety copy is not acceptable.
    let version = get_user_version(&conn)?;
    let upgrade = version > 0 && version < CURRENT_SCHEMA_VERSION;
    if upgrade {
        backup_before_migration(&conn, path, version)?;
    }

    migrate(&conn)?;

    // A large freelist left behind — a schema step that dropped a table (v21: `blackbox_records`;
    // the steps run in transactions and may not vacuum themselves), or a reclaim cut short by an app
    // exit — is handed back to the file system in the background. One cheap pragma per open (this runs
    // for every command connection); repeated triggers are harmless (`spawn_reclaim` is single-flight).
    if freelist_pages(&conn)? > RECLAIM_ON_OPEN_PAGES {
        spawn_reclaim(&conn);
    }
    Ok(conn)
}

/// Freelist size (pages) above which `open_database` starts a reclaim: 4096 pages = 16 MiB at 4 KiB
/// pages. Below that the next delete picks the pages up anyway; above it something big was left behind.
const RECLAIM_ON_OPEN_PAGES: i64 = 4096;

/// Pages per `PRAGMA incremental_vacuum(N)` call of the background reclaim. Each call is its own
/// write transaction: a small chunk keeps the write lock short — a recording or import waits at most
/// about one chunk, PROVIDED its write transaction takes the lock up front (`write_tx`; a deferred
/// read→write upgrade fails instead of waiting) — and lets the WAL auto-checkpoint between chunks
/// instead of growing by the whole freelist.
/// Measured on copies of a 1.2 GB archive (113k free pages, 4 KiB pages): a 1024-page chunk takes tens
/// of ms on an SSD (34 ms median / 46 ms p95 including the auto-checkpoint, ~4.2 s for the whole
/// freelist, WAL peak 7 MB); on a 7200 rpm hard disk chunks take a few hundred ms (2048-page chunks:
/// 0.39–0.53 s median, 22–29 s in total).
const RECLAIM_CHUNK_PAGES: i64 = 1024;

/// Pause of the background reclaim between two chunks. Writers waiting for the lock must get a real
/// window between chunks — also when the auto-checkpoint cannot run because a long-lived reader pins
/// the WAL and the chunks would otherwise follow back to back.
const RECLAIM_CHUNK_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// How long a reclaim chunk waits for the write lock before it gives up (the remainder stays on the
/// freelist for the next trigger).
const RECLAIM_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Directories whose databases are never reclaimed: the scratch store of "open a log without importing
/// it" (registered by `commands::flightlog::scratch_dir`). It is thrown away whole, so reclaiming it is
/// wasted I/O — and on Windows its wipe fails while a reclaim thread holds the database open.
static NO_RECLAIM_DIRS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Never reclaim a database inside `dir` (see `NO_RECLAIM_DIRS`).
pub(crate) fn exclude_from_reclaim(dir: &Path) {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut dirs = NO_RECLAIM_DIRS.lock().unwrap_or_else(|e| e.into_inner());
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// Whether the database file `db_file` (an absolute path, as `Connection::path` names it) lies in a
/// directory registered with `exclude_from_reclaim`. Compared in canonical form: SQLite names the file
/// with symlinks resolved (on macOS the temp folder sits behind `/var` → `/private/var`), which
/// `std::path::absolute` does not do.
fn reclaim_excluded(db_file: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let file = canonical(db_file);
    NO_RECLAIM_DIRS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|dir| file.starts_with(canonical(dir)))
}

/// Database files a background reclaim is running on, each with a "go round once more" flag set by a
/// trigger that arrived meanwhile (single flight per file).
static RECLAIMS_IN_FLIGHT: std::sync::Mutex<Vec<(PathBuf, bool)>> = std::sync::Mutex::new(Vec::new());

fn reclaims_in_flight() -> std::sync::MutexGuard<'static, Vec<(PathBuf, bool)>> {
    RECLAIMS_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Database files a "Compact Database" (full `VACUUM`, `compact_database`) is running on — single flight
/// per file. While a file is listed, no background reclaim starts on it and a running one stops before
/// its next chunk: the VACUUM empties the freelist anyway, and a reclaim chunk waiting for the write lock
/// the VACUUM holds for minutes would only time out.
///
/// Lock order: `RECLAIMS_IN_FLIGHT` → `COMPACTS_IN_FLIGHT`, never the reverse.
static COMPACTS_IN_FLIGHT: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn compacts_in_flight() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
    COMPACTS_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a compact is running on the database file `path` (as `Connection::path` names it).
pub(crate) fn compact_running(path: &Path) -> bool {
    compacts_in_flight().iter().any(|p| p == path)
}

fn freelist_pages(conn: &Connection) -> SqlResult<i64> {
    conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))
}

/// Bytes on the freelist (`freelist_count × page_size`).
fn freelist_bytes(conn: &Connection) -> SqlResult<i64> {
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    Ok(freelist_pages(conn)? * page_size)
}

/// What one `reclaim_free_pages` run did.
#[derive(Debug, Default)]
struct ReclaimStats {
    pages: i64,
    chunks: u32,
    /// The longest single chunk — the longest the write lock was held.
    longest: std::time::Duration,
}

/// Hand the whole freelist back to the file system (`auto_vacuum = INCREMENTAL`, see `open_database`),
/// `chunk_pages` at a time, each chunk its own implicit transaction. `PRAGMA incremental_vacuum(N)`
/// frees one page per result row it yields, so every call is stepped to the end (`execute_batch` steps
/// once and frees exactly one page). Loops until `freelist_count` is 0, so pages freed by others while
/// it runs go too; `pause` is slept before every chunk but the first. Synchronous — the app goes
/// through `spawn_reclaim`. `stop` is asked before every chunk; once it says yes the run ends there and
/// the rest stays on the freelist (a compact took the file over).
fn reclaim_free_pages(
    conn: &Connection,
    chunk_pages: i64,
    pause: std::time::Duration,
    stop: &dyn Fn() -> bool,
) -> SqlResult<ReclaimStats> {
    let mut stats = ReclaimStats::default();
    while freelist_pages(conn)? > 0 {
        if stats.chunks > 0 && !pause.is_zero() {
            std::thread::sleep(pause);
        }
        if stop() {
            break;
        }
        let started = std::time::Instant::now();
        let mut freed = 0;
        {
            let mut stmt = conn.prepare(&format!("PRAGMA incremental_vacuum({chunk_pages})"))?;
            let mut rows = stmt.query([])?;
            while rows.next()?.is_some() {
                freed += 1;
            }
        }
        if freed == 0 {
            break; // not an INCREMENTAL database — nothing can be handed back this way
        }
        stats.pages += freed;
        stats.chunks += 1;
        stats.longest = stats.longest.max(started.elapsed());
    }
    Ok(stats)
}

/// Reclaim the free pages of `conn`'s database file in a background thread, so a delete or an upgrade
/// never blocks its (often main-thread) caller and never fails on the reclaim. The thread opens its own
/// plain connection (no `open_database`: no migration, no backup). Single flight per file: a trigger
/// while one runs makes it go round once more instead of starting a second one. Errors are logged
/// only — whatever stays on the freelist is picked up by the next trigger. A connection without a
/// file (in-memory, the tests) is reclaimed synchronously right here. No reclaim starts on a file a
/// compact is running on (`COMPACTS_IN_FLIGHT`).
pub(crate) fn spawn_reclaim(conn: &Connection) {
    let path = match conn.path() {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            if let Err(e) = reclaim_free_pages(conn, RECLAIM_CHUNK_PAGES, std::time::Duration::ZERO, &|| false) {
                log::warn!("[flightlog] free-page reclaim failed: {e}");
            }
            return;
        }
    };
    if reclaim_excluded(&path) {
        return;
    }
    {
        let mut running = reclaims_in_flight();
        if compact_running(&path) {
            return; // the VACUUM empties the freelist itself
        }
        if let Some(entry) = running.iter_mut().find(|(p, _)| *p == path) {
            entry.1 = true;
            return;
        }
        running.push((path.clone(), false));
    }
    let spawned = std::thread::Builder::new()
        .name("db-reclaim".into())
        .spawn({
            let path = path.clone();
            move || reclaim_thread(&path)
        });
    if let Err(e) = spawned {
        reclaims_in_flight().retain(|(p, _)| *p != path);
        log::warn!("[flightlog] could not start the free-page reclaim: {e}");
    }
}

/// Removes the in-flight entry if the thread ends without doing so itself (error, panic).
struct ReclaimInFlight<'a> {
    path: &'a Path,
    armed: bool,
}

impl Drop for ReclaimInFlight<'_> {
    fn drop(&mut self) {
        if self.armed {
            reclaims_in_flight().retain(|(p, _)| p != self.path);
        }
    }
}

fn reclaim_thread(path: &Path) {
    let mut entry = ReclaimInFlight { path, armed: true };
    let started = std::time::Instant::now();
    let mut total = ReclaimStats::default();
    let mut freelist: Option<(i64, i64)> = None; // bytes before the first / after the last round
    loop {
        match reclaim_file(path) {
            Ok((before, stats, after)) => {
                freelist = Some((freelist.map_or(before, |(b, _)| b), after));
                total.pages += stats.pages;
                total.chunks += stats.chunks;
                total.longest = total.longest.max(stats.longest);
            }
            Err(e) => {
                log::warn!("[flightlog] free-page reclaim of {} failed: {e}", path.display());
                break;
            }
        }
        // The file is closed again (`reclaim_file`); go round once more if a trigger came meanwhile —
        // unless a compact has taken the file over (it waits for this thread to end).
        let mut running = reclaims_in_flight();
        match running.iter().position(|(p, _)| p == path) {
            Some(i) if running[i].1 && !compact_running(path) => running[i].1 = false,
            Some(i) => {
                running.remove(i);
                entry.armed = false;
                break;
            }
            None => {
                entry.armed = false;
                break;
            }
        }
    }
    if let Some((before, after)) = freelist.filter(|_| total.pages > 0) {
        log::info!(
            "[flightlog] reclaimed {} free pages in {} ms ({} chunks, longest {} ms): freelist {:.1} MB -> {:.1} MB",
            total.pages,
            started.elapsed().as_millis(),
            total.chunks,
            total.longest.as_millis(),
            before as f64 / 1_048_576.0,
            after as f64 / 1_048_576.0
        );
    }
}

/// One reclaim round on its own connection: freelist bytes before, what was done, bytes after. Opened
/// without CREATE — a database deleted meanwhile (the scratch store) must not come back as an empty file.
fn reclaim_file(path: &Path) -> SqlResult<(i64, ReclaimStats, i64)> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(RECLAIM_BUSY_TIMEOUT)?;
    let before = freelist_bytes(&conn)?;
    let stats = reclaim_free_pages(&conn, RECLAIM_CHUNK_PAGES, RECLAIM_CHUNK_PAUSE, &|| compact_running(path))?;
    Ok((before, stats, freelist_bytes(&conn)?))
}

/// Whether a background reclaim is running on the database file `path` (as `Connection::path` names it).
#[cfg(test)]
fn reclaim_running(path: &str) -> bool {
    reclaims_in_flight().iter().any(|(p, _)| p == Path::new(path))
}

/// Wait (bounded) until no background reclaim runs on `path`; panics after `limit`.
#[cfg(test)]
fn wait_for_reclaim(path: &str, limit: std::time::Duration) -> std::time::Duration {
    let started = std::time::Instant::now();
    while reclaim_running(path) {
        assert!(started.elapsed() < limit, "the background reclaim of {path} did not finish within {limit:?}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    started.elapsed()
}

/// Sibling file of the DB holding the single pre-migration backup — it lives next to `flights.db`
/// on purpose, so it follows the user-configured DB folder and can be moved/kept manually.
pub fn backup_path_for(db_path: &Path) -> PathBuf {
    db_path.with_file_name("flights.pre-migration-backup.db")
}

/// WAL-consistent snapshot of the whole DB via `VACUUM INTO` a temp file, then renamed over the
/// previous backup — the old backup survives until the new snapshot is complete.
fn backup_before_migration(conn: &Connection, db_path: &Path, from_version: u32) -> SqlResult<()> {
    let target = backup_path_for(db_path);
    let tmp = db_path.with_file_name("flights.pre-migration-backup.db.tmp");
    let _ = std::fs::remove_file(&tmp);
    log::warn!(
        "[flightlog] schema upgrade v{from_version} -> v{CURRENT_SCHEMA_VERSION} pending — backing up the database to {}",
        target.display()
    );
    if let Err(e) = conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let _ = std::fs::remove_file(&target); // single-backup policy: replaced by the new snapshot
    std::fs::rename(&tmp, &target).map_err(|e| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            Some(format!(
                "could not finalize the pre-migration backup at {}: {e}",
                target.display()
            )),
        )
    })?;
    Ok(())
}

/// Error of a compact started on a file that is already being compacted. The `compact-running:` prefix
/// is a STABLE marker the frontend matches on (`compactDb` in +page.svelte) — change both together.
pub const COMPACT_RUNNING_ERROR: &str = "compact-running: a database compact is already running";

/// How long the compact's VACUUM waits for the write lock another connection holds (an import batch, a
/// flight being saved) before it fails — the default 5 s is short for a slow drive.
const COMPACT_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a compact waits for a background reclaim of its file to stop. The reclaim ends before its
/// next chunk, and a chunk waits at most `RECLAIM_BUSY_TIMEOUT` for the lock; past this bound the
/// VACUUM starts anyway and its own busy timeout applies.
const COMPACT_RECLAIM_WAIT: std::time::Duration = std::time::Duration::from_secs(40);

/// A compact's entry in `COMPACTS_IN_FLIGHT`; removed when dropped (also on an error or a panic).
pub(crate) struct CompactGuard {
    path: Option<PathBuf>,
}

impl Drop for CompactGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            compacts_in_flight().retain(|p| p != path);
        }
    }
}

/// Register a compact of `conn`'s database file (single flight: a second one gets
/// `COMPACT_RUNNING_ERROR`) and wait until no background reclaim runs on it any more — a running one
/// stops before its next chunk, new ones do not start while the guard lives (`spawn_reclaim`). This
/// covers the reclaim the compact's own `open_database` starts on a large freelist as well. A
/// connection without a file (in-memory) is unreachable for anyone else and needs no registration.
/// Crate-visible for tests elsewhere that need a compact "running" on a file.
pub(crate) fn begin_compact(conn: &Connection) -> Result<CompactGuard, String> {
    let Some(path) = conn.path().filter(|p| !p.is_empty()).map(PathBuf::from) else {
        return Ok(CompactGuard { path: None });
    };
    {
        // Under the reclaim lock: `spawn_reclaim` checks for a compact and registers its reclaim in one
        // step under the same lock, so a reclaim is either visible to the wait below or never starts.
        let _reclaims = reclaims_in_flight();
        let mut compacts = compacts_in_flight();
        if compacts.contains(&path) {
            return Err(COMPACT_RUNNING_ERROR.to_string());
        }
        compacts.push(path.clone());
    }
    let guard = CompactGuard { path: Some(path.clone()) };
    let started = std::time::Instant::now();
    while reclaims_in_flight().iter().any(|(p, _)| *p == path) {
        if started.elapsed() > COMPACT_RECLAIM_WAIT {
            log::warn!(
                "[flightlog] the background reclaim of {} did not stop within {} s — compacting anyway",
                path.display(),
                COMPACT_RECLAIM_WAIT.as_secs()
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(guard)
}

/// Full defragmenting VACUUM — rebuilds the whole DB file to squeeze out fragmentation. Expensive
/// (cost ∝ DB size, minutes for a GB-sized archive on a hard disk or SD card); exposed only as the
/// explicit "Compact Database" maintenance action. Routine deletes use incremental_vacuum instead (see
/// `open_database`). Single flight per file, and no background reclaim runs on the file meanwhile
/// (`begin_compact`). Sets `conn`'s busy timeout to `COMPACT_BUSY_TIMEOUT`. Returns the resulting
/// database size in bytes.
pub fn compact_database(conn: &Connection) -> Result<i64, String> {
    let _guard = begin_compact(conn)?;
    let size = |conn: &Connection| -> SqlResult<i64> {
        let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        Ok(page_count * page_size)
    };
    let before = size(conn).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    conn.busy_timeout(COMPACT_BUSY_TIMEOUT).map_err(|e| e.to_string())?;
    if let Err(e) = conn.execute_batch("VACUUM;") {
        log::warn!(
            "[flightlog] database compact failed after {} ms: {e}",
            started.elapsed().as_millis()
        );
        return Err(format!("Compact failed: {e}"));
    }
    let after = size(conn).map_err(|e| e.to_string())?;
    log::info!(
        "[flightlog] database compacted in {} ms: {:.1} MB -> {:.1} MB",
        started.elapsed().as_millis(),
        before as f64 / 1_048_576.0,
        after as f64 / 1_048_576.0
    );
    Ok(after)
}

/// Resolve the database file path based on settings.
/// - Empty db_path + portable mode → <exe_dir>/data/flights.db
/// - Empty db_path + normal mode → <AppData>/kite-gc/flights.db
/// - Non-empty db_path → <db_path>/flights.db
pub fn resolve_db_path(custom_path: &str, portable: bool) -> PathBuf {
    // Android: a user-chosen location is a SAF tree URI, not a path. The database cannot live there
    // (SQLite needs POSIX open/seek/lock for the journal), so the live DB stays on the platform
    // default below and the session-end mirror copies a snapshot into the granted tree
    // (user_file::mirror_session).
    #[cfg(target_os = "android")]
    let custom_path = if custom_path.starts_with("content://") { "" } else { custom_path };

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
    // macOS and iOS both resolve to Library/Application Support. On iOS `HOME` is the app's sandbox
    // container root, so this stays inside the app's private, backed-up storage.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("kite-gc")
                .join("flights.db");
        }
    }

    // Android: app-private storage. Not a fallback — the working directory of an Android process is
    // `/`, so the generic fallback below would try to create the database on a read-only filesystem.
    #[cfg(target_os = "android")]
    {
        return crate::android::app_data_dir().join("flights.db");
    }

    // Fallback: current directory
    #[allow(unreachable_code)]
    PathBuf::from("flights.db")
}

/// Resolve the **raw-log base directory** (where `raw_logs/` with .tlog / raw MSP files live). This is
/// intentionally **separate from the database folder**: the DB defaults to AppData, but raw logs default
/// to the user's Documents (`Documents/KiteGC`) where they are easy to find / hand to Mission Planner.
/// `custom_path` (empty = use default) overrides it; both are configurable independently in Settings.
pub fn resolve_raw_log_dir(custom_path: &str, portable: bool) -> PathBuf {
    // Android: same as resolve_db_path — a SAF tree URI cannot be written through std::fs, so the
    // writers stay in the app-private staging dir below and the mirror copies finished files out.
    #[cfg(target_os = "android")]
    let custom_path = if custom_path.starts_with("content://") { "" } else { custom_path };

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

    // Android has no path-addressable Documents folder (scoped storage makes it a MediaStore
    // collection), so raw logs go to app-private storage and are shared out explicitly instead.
    #[cfg(target_os = "android")]
    {
        return crate::android::app_documents_dir().join("KiteGC");
    }

    // Default: the user's Documents folder → Documents/KiteGC (Windows honours OneDrive relocation,
    // Linux uses XDG_DOCUMENTS_DIR). Falls back to ~/Documents, then the current dir.
    #[allow(unreachable_code)]
    if let Some(docs) = dirs::document_dir() {
        return docs.join("KiteGC");
    }
    if let Some(home) = dirs::home_dir() {
        return home.join("Documents").join("KiteGC");
    }
    PathBuf::from(".")
}

/// Begin a write transaction (`BEGIN IMMEDIATE`). THE RULE for the flight database: every transaction
/// that writes takes the write lock up front through this helper — never a DEFERRED one. A deferred
/// transaction that reads first and writes later is upgraded to a writer at its first write, and that
/// upgrade fails at once with SQLITE_BUSY — the busy timeout is NOT applied — whenever another
/// connection holds the write lock or has committed since the read. The background reclaim
/// (`spawn_reclaim`) holds the lock in back-to-back chunks, and recordings, imports and a second open
/// write too. `BEGIN IMMEDIATE` waits under the busy timeout instead; for write-first transactions
/// it costs nothing.
pub(crate) fn write_tx(conn: &Connection) -> SqlResult<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
}

// ── Schema migrations ────────────────────────────────────────────────

fn get_user_version(conn: &Connection) -> SqlResult<u32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn set_user_version(conn: &Connection, version: u32) -> SqlResult<()> {
    conn.execute_batch(&format!("PRAGMA user_version = {};", version))
}

/// Error for a DB written by a NEWER Kite (the user downgraded the app). The `db-newer:` prefix is
/// a STABLE marker the frontend matches on to disable the logbook with a targeted message instead
/// of a generic failure — change it in lockstep with the loadLogbook handler in +page.svelte.
fn db_newer_error(found: u32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
        Some(format!(
            "db-newer: flight database schema is v{found}, this Kite build supports up to v{CURRENT_SCHEMA_VERSION}"
        )),
    )
}

/// One schema step: creates its objects and stamps its version.
type MigrationStep = fn(&Connection) -> SqlResult<()>;

/// The migration chain, one `(target version, step)` per schema version. Module-level so the tests
/// can build a DB at an older version from the real steps.
const MIGRATION_STEPS: [(u32, MigrationStep); 21] = [
    (1, migrate_v0_to_v1),
    (2, migrate_v1_to_v2),
    (3, migrate_v2_to_v3),
    (4, migrate_v3_to_v4),
    (5, migrate_v4_to_v5),
    (6, migrate_v5_to_v6),
    (7, migrate_v6_to_v7),
    (8, migrate_v7_to_v8),
    (9, migrate_v8_to_v9),
    (10, migrate_v9_to_v10),
    (11, migrate_v10_to_v11),
    (12, migrate_v11_to_v12),
    (13, migrate_v12_to_v13),
    (14, migrate_v13_to_v14),
    (15, migrate_v14_to_v15),
    (16, migrate_v15_to_v16),
    (17, migrate_v16_to_v17),
    (18, migrate_v17_to_v18),
    (19, migrate_v18_to_v19),
    (20, migrate_v19_to_v20),
    (21, migrate_v20_to_v21),
];

fn migrate(conn: &Connection) -> SqlResult<()> {
    let current = get_user_version(conn)?;

    // Downgrade guard: never touch (let alone migrate "up" to an OLDER schema) a DB written by a
    // newer Kite — refuse with the marker error and leave the file byte-identical. Fires ONLY
    // on a real schema mismatch; every other open error stays a generic failure.
    if current > CURRENT_SCHEMA_VERSION {
        return Err(db_newer_error(current));
    }

    // Every step runs in its OWN transaction: its schema objects and its version stamp commit
    // together or not at all. A failing step rolls back and leaves the DB cleanly on the last
    // reached version (the stamps are literal per-step targets for the same reason — a historical
    // stamp-to-CURRENT once produced a "newest version, missing objects" DB; the ensure_* block
    // below still self-heals that legacy case). PRAGMA user_version is transactional in SQLite.
    // None of the steps may contain VACUUM or its own BEGIN/COMMIT.
    for (target, step) in MIGRATION_STEPS {
        if current < target {
            let tx = write_tx(conn)?;
            step(&tx)?;
            tx.commit()?;
            log::info!("[flightlog] db schema migrated to v{target}");
        }
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
    ensure_v19_schema(conn)?;
    ensure_v20_schema(conn)?;
    ensure_v21_schema(conn)?;

    Ok(())
}

/// Whether `table` has a column named `column` (via PRAGMA table_info).
pub(crate) fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
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
    set_user_version(conn, 11)?;
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
    set_user_version(conn, 12)?;
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
    set_user_version(conn, 13)?;
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
    set_user_version(conn, 14)?;
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
    set_user_version(conn, 15)?;
    Ok(())
}

/// v16: the vehicle library — a `vehicles` table holding the per-aircraft build sheet. Flights
/// soft-linked by the existing `flights.craft_name` until v20 stored the link (`flights.vehicle_id`).
/// Idempotent; only creates the table if missing. Records (max time/dist/alt) are derived from the
/// linked flights, not stored here.
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
    set_user_version(conn, 16)?;
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
    set_user_version(conn, 17)?;
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
    set_user_version(conn, 18)?;
    Ok(())
}

/// v19: the FC hardware id (`fc_uid`) on flights and vehicles — MSP_UID / MAVLink AUTOPILOT_VERSION
/// uid; from v20 the first key of the flight → vehicle link. Additive + idempotent.
fn ensure_v19_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "flights", "fc_uid")? {
        conn.execute_batch("ALTER TABLE flights ADD COLUMN fc_uid TEXT;")?;
    }
    if !column_exists(conn, "vehicles", "fc_uid")? {
        conn.execute_batch("ALTER TABLE vehicles ADD COLUMN fc_uid TEXT;")?;
    }
    Ok(())
}

fn migrate_v18_to_v19(conn: &Connection) -> SqlResult<()> {
    ensure_v19_schema(conn)?;
    set_user_version(conn, 19)?;
    Ok(())
}

/// v20: group flights (Dev-Docs active/GROUP_FLIGHTS.md §3.1) — `flight_groups` (the header that ties
/// the member flights of a multi-vehicle flight together; no track, no telemetry), `flights.group_id`
/// (deleting a group dissolves it: members stay as single flights), the per-flight event timeline
/// `flight_events` (arm/disarm; the column set of INAV_PUSH_TELEMETRY §6.4) and a wall-clock stamp per
/// telemetry row (`telemetry_records.wall_ms`, UTC epoch ms; NULL for imports and older rows), and the
/// flight → library vehicle link `flights.vehicle_id` (Dev-Docs active/MULTI_VEHICLE.md §9.1; deleting a
/// vehicle un-links its flights). Additive + idempotent.
fn ensure_v20_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS flight_groups (
            id               TEXT PRIMARY KEY,
            start_time       TEXT NOT NULL,
            end_time         TEXT,
            utc_offset_min   INTEGER,
            start_lat        REAL,
            start_lon        REAL,
            start_alt_m      REAL,
            location_name    TEXT,
            weather_temp_c   REAL,
            weather_wind_ms  REAL,
            weather_wind_deg INTEGER,
            weather_desc     TEXT,
            notes            TEXT,
            created_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    if !column_exists(conn, "flights", "group_id")? {
        conn.execute_batch(
            "ALTER TABLE flights ADD COLUMN group_id TEXT REFERENCES flight_groups(id) ON DELETE SET NULL;",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_flights_group ON flights(group_id);
         CREATE TABLE IF NOT EXISTS flight_events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            flight_id    INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
            timestamp_ms INTEGER NOT NULL,
            wall_ms      INTEGER,
            kind         TEXT NOT NULL,
            code         INTEGER,
            text         TEXT,
            source       TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_events_flight ON flight_events(flight_id, timestamp_ms);",
    )?;
    if !column_exists(conn, "telemetry_records", "wall_ms")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN wall_ms INTEGER;")?;
    }
    if !column_exists(conn, "flights", "vehicle_id")? {
        // The column and its one-time back-fill commit together: a savepoint nests inside the
        // migration step's transaction and also covers the self-heal call (no transaction there). The
        // back-fill runs only here, when the column is born — later opens never re-link a flight the
        // user un-linked.
        conn.execute_batch("SAVEPOINT v20_vehicle_id;")?;
        let added = conn
            .execute_batch(
                "ALTER TABLE flights ADD COLUMN vehicle_id INTEGER REFERENCES vehicles(id) ON DELETE SET NULL;",
            )
            .and_then(|_| backfill_flight_vehicle_ids(conn));
        match added {
            Ok(linked) => {
                conn.execute_batch("RELEASE v20_vehicle_id;")?;
                log::info!("[flightlog] flights.vehicle_id added — {linked} existing flight(s) linked to library vehicles");
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK TO v20_vehicle_id; RELEASE v20_vehicle_id;");
                return Err(e);
            }
        }
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_flights_vehicle ON flights(vehicle_id);")?;
    Ok(())
}

/// Link every flight without a vehicle to the one the resolution rule names (`resolve_vehicle_id`:
/// `fc_uid` first, craft name as the fallback, the newest vehicle on a tie). On a database without FC
/// ids that is exactly what the logbook showed through its craft-name match before v20; a flight whose
/// FC id matches a vehicle goes to that vehicle instead. Only touches `vehicle_id IS NULL` rows, so it
/// is re-entrant. Returns the number of flights linked.
fn backfill_flight_vehicle_ids(conn: &Connection) -> SqlResult<usize> {
    let has_vehicles: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM vehicles)", [], |r| r.get(0))?;
    if !has_vehicles {
        return Ok(0);
    }
    let unlinked: Vec<(i64, Option<String>, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT id, fc_uid, craft_name FROM flights WHERE vehicle_id IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<SqlResult<_>>()?
    };
    let mut linked = 0;
    for (flight_id, fc_uid, craft) in unlinked {
        if let Some(vehicle_id) = resolve_vehicle_id(conn, fc_uid.as_deref(), craft.as_deref().unwrap_or(""))? {
            conn.execute(
                "UPDATE flights SET vehicle_id = ?1 WHERE id = ?2",
                params![vehicle_id, flight_id],
            )?;
            linked += 1;
        }
    }
    Ok(linked)
}

fn migrate_v19_to_v20(conn: &Connection) -> SqlResult<()> {
    ensure_v20_schema(conn)?;
    set_user_version(conn, 20)?;
    Ok(())
}

/// Whether the database has a table named `table`.
pub(crate) fn table_exists(conn: &Connection, table: &str) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get(0),
    )
}

/// v21: `blackbox_records` (the decoded CSV line of every kept INAV Blackbox row) is dropped — nothing
/// read it: replay uses `telemetry_records`, hi-res replay re-decodes the archived original in
/// `blackbox_files`. Its one use, the source restore when a live/log pair is unlinked, now reads
/// `flights.source_before_link`: the flight's own source (`live` / `blackbox`) while `source` reads
/// `both` for a linked pair, NULL for an unlinked flight. Idempotent; `open_database` starts the
/// background reclaim of the freed pages after the upgrade.
fn ensure_v21_schema(conn: &Connection) -> SqlResult<()> {
    if !column_exists(conn, "flights", "source_before_link")? {
        // Column and back-fill commit together, like `flights.vehicle_id` in v20 — and the back-fill
        // runs only here, when the column is born, while `blackbox_records` still exists.
        conn.execute_batch("SAVEPOINT v21_source_before_link;")?;
        let added = conn
            .execute_batch("ALTER TABLE flights ADD COLUMN source_before_link TEXT;")
            .and_then(|_| backfill_source_before_link(conn));
        match added {
            Ok(linked) => {
                conn.execute_batch("RELEASE v21_source_before_link;")?;
                log::info!("[flightlog] flights.source_before_link added — set on {linked} linked flight(s)");
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK TO v21_source_before_link; RELEASE v21_source_before_link;");
                return Err(e);
            }
        }
    }
    // Gated so that a healthy v21 open (this runs in the self-heal pass too) issues no DDL at all.
    if table_exists(conn, "blackbox_records")? {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_blackbox_records_flight;
             DROP TABLE IF EXISTS blackbox_records;",
        )?;
    }
    Ok(())
}

/// SQL condition on a `flights` row: the flight was made by a log importer. Only they write these
/// `protocol` values — `BLACKBOX` (blackbox.rs), `DATAFLASH` (ardupilot.rs), `ULOG` (ulog.rs); live
/// recordings and raw-log imports write `MSP` / `MAVLink` / `Telemetry …`. A macro so `END_LINK_SET`
/// can embed it at compile time.
macro_rules! log_protocol_sql {
    () => {
        "protocol IN ('BLACKBOX', 'DATAFLASH', 'ULOG')"
    };
}

/// `log_protocol_sql!` for a value already read into Rust.
pub(crate) fn is_log_protocol(protocol: &str) -> bool {
    matches!(protocol, "BLACKBOX" | "DATAFLASH" | "ULOG")
}

/// Set `source_before_link` on every linked flight: `blackbox` when a log importer made it, or it holds
/// decoded Blackbox rows (`blackbox_records`, if the table is still there) or an archived log
/// (`blackbox_files`), `live` otherwise. Returns the number of flights set.
fn backfill_source_before_link(conn: &Connection) -> SqlResult<usize> {
    let records = if table_exists(conn, "blackbox_records")? {
        "EXISTS(SELECT 1 FROM blackbox_records WHERE flight_id = flights.id) OR "
    } else {
        ""
    };
    conn.execute(
        &format!(
            "UPDATE flights SET source_before_link = CASE
                WHEN {} OR {records}EXISTS(SELECT 1 FROM blackbox_files WHERE flight_id = flights.id)
                    THEN 'blackbox'
                ELSE 'live'
             END
             WHERE linked_flight_id IS NOT NULL",
            log_protocol_sql!()
        ),
        [],
    )
}

fn migrate_v20_to_v21(conn: &Connection) -> SqlResult<()> {
    ensure_v21_schema(conn)?;
    set_user_version(conn, 21)?;
    Ok(())
}

fn migrate_v6_to_v7(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN battery_percentage INTEGER;",
    )?;
    set_user_version(conn, 7)?;
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
    set_user_version(conn, 6)?;
    Ok(())
}

fn migrate_v2_to_v3(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER;",
    )?;
    set_user_version(conn, 3)?;
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

    set_user_version(conn, 1)?;
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

    set_user_version(conn, 2)?;
    Ok(())
}

// ── CRUD operations ─────────────────────────────────────────────────

/// Insert a new flight, returns the row ID. Every flight insert goes through here (live commit,
/// crash recovery, every importer, `.kflight` import): a flight without an explicit `vehicle_id` is
/// linked to the library vehicle `resolve_vehicle_id` names (none → NULL).
pub fn insert_flight(conn: &Connection, flight: &Flight) -> SqlResult<i64> {
    let vehicle_id = match flight.vehicle_id {
        Some(id) => Some(id),
        None => resolve_vehicle_id(conn, flight.fc_uid.as_deref(), &flight.craft_name)?,
    };
    insert_flight_row(conn, flight, vehicle_id)
}

/// `insert_flight` without the vehicle resolution — `flight.vehicle_id` is written as it is. For a
/// database without a vehicle library (the `.kflight` export file).
pub fn insert_flight_unresolved(conn: &Connection, flight: &Flight) -> SqlResult<i64> {
    insert_flight_row(conn, flight, flight.vehicle_id)
}

fn insert_flight_row(conn: &Connection, flight: &Flight, vehicle_id: Option<i64>) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO flights (
            start_time, end_time, duration_sec, source,
            craft_name, fc_variant, fc_version, board_id, platform_type, protocol,
            start_lat, start_lon, location_name,
            weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc,
            max_alt_m, max_speed_ms, max_distance_m, total_distance_m,
            battery_used_mah, notes, pilot_name, pilot_id, battery_serial, utc_offset_min, fc_uid,
            group_id, vehicle_id
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30
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
            flight.fc_uid,
            flight.group_id,
            vehicle_id,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Batch-insert telemetry records for a flight.
pub fn insert_telemetry_batch(
    conn: &Connection,
    records: &[TelemetryRecord],
) -> SqlResult<()> {
    let tx = write_tx(conn)?;
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
                link_snr, link_rssi_dbm, airspeed_ms, throttle_pct, wall_ms
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
                ?43, ?44, ?45, ?46, ?47
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
                r.wall_ms,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Insert a batch of per-instance battery samples (multi-battery; see docs/active/MULTI_BATTERY.md).
pub fn insert_battery_records_batch(conn: &Connection, records: &[BatteryRecord]) -> SqlResult<()> {
    let tx = write_tx(conn)?;
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
            start_lon     REAL,
            fc_uid        TEXT,
            vehicle_key   TEXT,
            role          TEXT,
            group_id      TEXT,
            group_file    TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_session_telemetry
            ON telemetry_records(timestamp_ms);
        CREATE TABLE IF NOT EXISTS session_events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_ms INTEGER NOT NULL,
            wall_ms      INTEGER,
            kind         TEXT NOT NULL,
            code         INTEGER,
            text         TEXT,
            source       TEXT
        );",
    )?;
    // A `.ktmp` written before v19 (crash recovery of an old session) predates the column.
    if !column_exists(&conn, "session_meta", "fc_uid")? {
        conn.execute_batch("ALTER TABLE session_meta ADD COLUMN fc_uid TEXT;")?;
    }
    // …and one written before v20 predates the per-row wall clock (the commit copies TELEMETRY_COLS).
    if !column_exists(&conn, "telemetry_records", "wall_ms")? {
        conn.execute_batch("ALTER TABLE telemetry_records ADD COLUMN wall_ms INTEGER;")?;
    }
    // …and the group-flight membership (GROUP_FLIGHTS.md §3.3). `role` stays NULL on such a file: it
    // marks a session whose arm/disarm history was never recorded (see `SessionMetaRow::role`).
    for column in ["vehicle_key", "role", "group_id", "group_file"] {
        if !column_exists(&conn, "session_meta", column)? {
            conn.execute_batch(&format!("ALTER TABLE session_meta ADD COLUMN {column} TEXT;"))?;
        }
    }
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
    fc_uid: Option<&str>,
    protocol: &str,
    start_lat: Option<f64>,
    start_lon: Option<f64>,
) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO session_meta
            (id, start_time, craft_name, fc_variant, fc_version, board_id, platform_type,
             protocol, start_lat, start_lon, fc_uid)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            fc_uid,
        ],
    )?;
    Ok(())
}

/// Live platform-type override while a temp session is open (UAV Info panel dropdown).
pub fn update_session_meta_platform_type(conn: &Connection, platform_type: u8) -> SqlResult<()> {
    conn.execute(
        "UPDATE session_meta SET platform_type = ?1 WHERE id = 1",
        params![platform_type],
    )?;
    Ok(())
}

/// Set the home / stats reference of an open temp session (the first 3D-fix position after its arm,
/// GROUP_FLIGHTS.md §5e M1) — so a crash-recovered session keeps it.
pub fn update_session_meta_start(conn: &Connection, start_lat: f64, start_lon: f64) -> SqlResult<()> {
    conn.execute(
        "UPDATE session_meta SET start_lat = ?1, start_lon = ?2 WHERE id = 1",
        params![start_lat, start_lon],
    )?;
    Ok(())
}

/// Replace the FC identity of an open temp session — the MSP-over-MAVLink probe upgrades the MAVLink
/// heartbeat identity to the INAV one right after connect (see `FlightRecorder::set_fc_info`).
pub fn update_session_meta_identity(
    conn: &Connection,
    craft_name: &str,
    fc_variant: &str,
    fc_version: &str,
    board_id: &str,
    platform_type: u8,
    fc_uid: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE session_meta
            SET craft_name = ?1, fc_variant = ?2, fc_version = ?3, board_id = ?4, platform_type = ?5,
                fc_uid = ?6
          WHERE id = 1",
        params![craft_name, fc_variant, fc_version, board_id, platform_type, fc_uid],
    )?;
    Ok(())
}

/// Record who writes a temp session and whether it belongs to a group flight (GROUP_FLIGHTS.md §3.3):
/// the recorder's vehicle key, its role (`single` | `member`), and — for a member — the group id and
/// the file name of the group's `.kgrp` (it lives next to the `.ktmp`). Written at session start and
/// whenever the membership changes.
pub fn update_session_meta_membership(
    conn: &Connection,
    vehicle_key: &str,
    role: &str,
    group_id: Option<&str>,
    group_file: Option<&str>,
) -> SqlResult<()> {
    conn.execute(
        "UPDATE session_meta SET vehicle_key = ?1, role = ?2, group_id = ?3, group_file = ?4 WHERE id = 1",
        params![vehicle_key, role, group_id, group_file],
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
    pub fc_uid: Option<String>,
    /// Vehicle key of the recorder that wrote the session (`"L1:S2"`).
    #[allow(dead_code)] // read by the group recovery scan (GROUP_FLIGHTS.md step 7)
    pub vehicle_key: Option<String>,
    /// `single` | `member`. NULL = written before arm/disarm events were recorded (the whole session
    /// then counts as armed, the pre-group rule).
    pub role: Option<String>,
    /// Group flight the session belongs to (member sessions only).
    pub group_id: Option<String>,
    /// File name of the group's `.kgrp`, in the same directory as the `.ktmp`.
    pub group_file: Option<String>,
}

/// Read the single `session_meta` row of a temp session (None if absent — e.g. a malformed file).
pub fn read_session_meta(conn: &Connection) -> SqlResult<Option<SessionMetaRow>> {
    conn.query_row(
        "SELECT start_time, craft_name, fc_variant, fc_version, board_id, platform_type, \
                protocol, start_lat, start_lon, fc_uid, vehicle_key, role, group_id, group_file \
         FROM session_meta WHERE id = 1",
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
                fc_uid: row.get(9)?,
                vehicle_key: row.get(10)?,
                role: row.get(11)?,
                group_id: row.get(12)?,
                group_file: row.get(13)?,
            })
        },
    )
    .optional()
}

/// Append one timeline event (arm / disarm / link_lost / link_back) to a temp session. `id` and
/// `flight_id` of `e` are ignored — the commit rewrites the flight id (see `commit_session_to_main`).
pub fn insert_session_event(conn: &Connection, e: &FlightEvent) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO session_events (timestamp_ms, wall_ms, kind, code, text, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![e.timestamp_ms, e.wall_ms, e.kind, e.code, e.text, e.source],
    )?;
    Ok(())
}

/// A temp session's timeline events in time order (`flight_id` = 0, like its telemetry rows).
pub fn read_session_events(conn: &Connection) -> SqlResult<Vec<FlightEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp_ms, wall_ms, kind, code, text, source
         FROM session_events ORDER BY timestamp_ms ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(FlightEvent {
            id: row.get(0)?,
            flight_id: 0,
            timestamp_ms: row.get(1)?,
            wall_ms: row.get(2)?,
            kind: row.get(3)?,
            code: row.get(4)?,
            text: row.get(5)?,
            source: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// The latest point of a temp session at which its relative timeline (`timestamp_ms`) and the wall
/// clock (`wall_ms`) are both known — the last telemetry row or event carrying a wall stamp, whichever
/// is later on the timeline. `None` for a session without any wall stamp (written before v20).
pub fn last_session_time_pair(conn: &Connection) -> SqlResult<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT timestamp_ms, wall_ms FROM (
            SELECT timestamp_ms, wall_ms FROM telemetry_records WHERE wall_ms IS NOT NULL
            UNION ALL
            SELECT timestamp_ms, wall_ms FROM session_events WHERE wall_ms IS NOT NULL
         ) ORDER BY timestamp_ms DESC, wall_ms DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// Count telemetry rows in a temp session (cheap orphan triage — an empty `.ktmp` is worthless).
pub fn temp_session_row_count(conn: &Connection) -> SqlResult<i64> {
    conn.query_row("SELECT COUNT(*) FROM telemetry_records", [], |row| row.get(0))
}

/// The columns of one telemetry row that the flight statistics read (`recorder::stats_over_segments`).
#[derive(Debug, Clone, PartialEq)]
pub struct StatsSample {
    pub timestamp_ms: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub baro_alt_m: Option<f64>,
    pub alt_m: Option<f64>,
    pub speed_ms: Option<f64>,
    pub mah_drawn: Option<u32>,
    /// Unified GPS fix scale (`scheduler::telemetry::FIX_*`); positions below a 3D fix count for no
    /// distance (GROUP_FLIGHTS.md §5e M1).
    pub fix_type: Option<u8>,
}

/// A temp session's telemetry reduced to the statistics columns, in time order — the narrow read for
/// recomputing a flight's stats (a full `read_flight_track` decodes every column of every row).
pub fn read_stats_samples(conn: &Connection) -> SqlResult<Vec<StatsSample>> {
    let mut stmt = conn.prepare(
        "SELECT timestamp_ms, lat, lon, baro_alt_m, alt_m, speed_ms, mah_drawn, fix_type
         FROM telemetry_records WHERE flight_id = 0 ORDER BY timestamp_ms ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(StatsSample {
            timestamp_ms: row.get(0)?,
            lat: row.get(1)?,
            lon: row.get(2)?,
            baro_alt_m: row.get(3)?,
            alt_m: row.get(4)?,
            speed_ms: row.get(5)?,
            mah_drawn: row.get(6)?,
            fix_type: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Commit a finished temp session into the main DB atomically: insert the finalized `flights` row,
/// ATTACH the temp file, copy its `telemetry_records` (rewriting `flight_id` to the new main id),
/// then DETACH. Returns the new flight id. The main DB therefore only ever sees the flight as a
/// finished whole. `ATTACH`/`DETACH` cannot run inside a transaction, so they bracket it.
///
/// Group flights (GROUP_FLIGHTS.md §3.4): the session's `session_events` become the flight's
/// `flight_events`, and a group id in its `session_meta` (else `flight.group_id`) is written to
/// `flights.group_id`. The group row must exist already (the caller commits it first); a missing one
/// fails the commit instead of silently dropping the membership. A `.ktmp` from before these tables
/// commits as before.
pub fn commit_session_to_main(
    conn: &Connection,
    temp_path: &Path,
    flight: &Flight,
) -> SqlResult<i64> {
    let temp_str = temp_path.to_string_lossy().to_string();
    conn.execute("ATTACH DATABASE ?1 AS sess", params![temp_str])?;

    let outcome = (|| -> SqlResult<i64> {
        let meta_group_id: Option<String> = if attached_column_exists(conn, "session_meta", "group_id")? {
            conn.query_row("SELECT group_id FROM sess.session_meta WHERE id = 1", [], |r| r.get(0))
                .optional()?
                .flatten()
        } else {
            None
        };
        let has_events = attached_table_exists(conn, "session_events")?;

        let tx = write_tx(conn)?;
        let mut row = flight.clone();
        if let Some(gid) = meta_group_id {
            row.group_id = Some(gid);
        }
        if let Some(gid) = row.group_id.as_deref() {
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM main.flight_groups WHERE id = ?1)",
                params![gid],
                |r| r.get(0),
            )?;
            if !known {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                    Some(format!("group flight {gid} is not in the flight DB — commit the group first")),
                ));
            }
        }
        let flight_id = insert_flight(&tx, &row)?;
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
        if has_events {
            tx.execute(
                "INSERT INTO main.flight_events (flight_id, timestamp_ms, wall_ms, kind, code, text, source)
                 SELECT ?1, timestamp_ms, wall_ms, kind, code, text, source
                 FROM sess.session_events ORDER BY timestamp_ms ASC, id ASC",
                params![flight_id],
            )?;
        }
        tx.commit()?;
        Ok(flight_id)
    })();

    // Always detach, even if the transaction failed, so the connection isn't left with `sess` bound.
    let _ = conn.execute("DETACH DATABASE sess", []);
    outcome
}

/// Whether the temp session ATTACHed as `sess` has the table `table`.
fn attached_table_exists(conn: &Connection, table: &str) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sess.sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |r| r.get(0),
    )
}

/// Whether `table` of the temp session ATTACHed as `sess` has the column `column`.
fn attached_column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1, 'sess') WHERE name = ?2)",
        params![table, column],
        |r| r.get(0),
    )
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

/// Delete every temp `.ktmp` session in `dir` except the ones in `keep` (the live, pending,
/// continue-on-reconnect, group-pending and suspended group-member sessions). Enforces the
/// single-temp invariant whenever the user discards: stragglers from earlier crashes must not
/// resurface one per launch. Returns the number removed.
pub fn sweep_temp_sessions(dir: &Path, keep: &[PathBuf]) -> usize {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ktmp") || keep.contains(&path) {
            continue;
        }
        log::info!("Sweeping leftover temp session {}", path.display());
        remove_temp_session(&path);
        if !path.exists() {
            removed += 1;
        }
    }
    removed
}

/// List flight summaries ordered by start_time DESC.
pub fn list_flights(conn: &Connection) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min, group_id, vehicle_id
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
            group_id: row.get(13)?,
            vehicle_id: row.get(14)?,
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
                utc_offset_min, fc_uid, group_id, vehicle_id
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
            fc_uid: row.get(29)?,
            group_id: row.get(30)?,
            vehicle_id: row.get(31)?,
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
            utc_offset_min, group_id, vehicle_id
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
            group_id: row.get(13)?,
            vehicle_id: row.get(14)?,
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
            utc_offset_min, group_id, vehicle_id
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
            group_id: row.get(13)?,
            vehicle_id: row.get(14)?,
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
    created_at, updated_at, fc_uid";

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
        fc_uid: row.get(39)?,
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
    let fc_uid = v.fc_uid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    conn.execute(
        "INSERT INTO vehicles (
            name, craft_name, vehicle_type, status, image, notes,
            model, wingspan_mm, length_mm, weight_auw_g, weight_dry_g,
            motors, props, esc, recommended_cells, recommended_capacity_mah,
            rx, vtx, camera, gimbal_camera, datalink,
            sensor_airspeed, sensor_rangefinder, sensor_optical_flow, sensor_gps, sensor_rtk, sensor_compass,
            fc_model, fc_manufacturer, fc_firmware, fc_firmware_version, blackbox_available, fc_uid
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
            ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33
        )",
        params![
            v.name, craft, v.vehicle_type, v.status, v.image, v.notes,
            v.model, v.wingspan_mm, v.length_mm, v.weight_auw_g, v.weight_dry_g,
            v.motors, v.props, v.esc, v.recommended_cells, v.recommended_capacity_mah,
            v.rx, v.vtx, v.camera, v.gimbal_camera, v.datalink,
            v.sensor_airspeed, v.sensor_rangefinder, v.sensor_optical_flow, v.sensor_gps, v.sensor_rtk, v.sensor_compass,
            v.fc_model, v.fc_manufacturer, v.fc_firmware, v.fc_firmware_version, v.blackbox_available,
            fc_uid,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update a vehicle's fields (bumps `updated_at`).
pub fn update_vehicle(conn: &Connection, id: i64, v: &VehicleInput) -> SqlResult<()> {
    let craft = v.craft_name.as_deref().and_then(normalize_craft_name);
    let fc_uid = v.fc_uid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    conn.execute(
        "UPDATE vehicles SET
            name = ?1, craft_name = ?2, vehicle_type = ?3, status = ?4, image = ?5, notes = ?6,
            model = ?7, wingspan_mm = ?8, length_mm = ?9, weight_auw_g = ?10, weight_dry_g = ?11,
            motors = ?12, props = ?13, esc = ?14, recommended_cells = ?15, recommended_capacity_mah = ?16,
            rx = ?17, vtx = ?18, camera = ?19, gimbal_camera = ?20, datalink = ?21,
            sensor_airspeed = ?22, sensor_rangefinder = ?23, sensor_optical_flow = ?24,
            sensor_gps = ?25, sensor_rtk = ?26, sensor_compass = ?27,
            fc_model = ?28, fc_manufacturer = ?29, fc_firmware = ?30, fc_firmware_version = ?31,
            blackbox_available = ?32, fc_uid = ?34, updated_at = datetime('now')
         WHERE id = ?33",
        params![
            v.name, craft, v.vehicle_type, v.status, v.image, v.notes,
            v.model, v.wingspan_mm, v.length_mm, v.weight_auw_g, v.weight_dry_g,
            v.motors, v.props, v.esc, v.recommended_cells, v.recommended_capacity_mah,
            v.rx, v.vtx, v.camera, v.gimbal_camera, v.datalink,
            v.sensor_airspeed, v.sensor_rangefinder, v.sensor_optical_flow, v.sensor_gps, v.sensor_rtk, v.sensor_compass,
            v.fc_model, v.fc_manufacturer, v.fc_firmware, v.fc_firmware_version, v.blackbox_available, id,
            fc_uid,
        ],
    )?;
    Ok(())
}

/// Find a vehicle by FC hardware id (the UAV Info panel's "already in the library" check — works for
/// crafts without a name, e.g. ArduPilot). Exact match on the stored string; newest wins on a tie.
pub fn find_vehicle_by_fc_uid(conn: &Connection, fc_uid: &str) -> SqlResult<Option<Vehicle>> {
    let uid = fc_uid.trim();
    if uid.is_empty() {
        return Ok(None);
    }
    conn.query_row(
        &format!(
            "SELECT {} FROM vehicles WHERE TRIM(fc_uid) = ?1 COLLATE NOCASE \
             ORDER BY created_at DESC, id DESC LIMIT 1",
            VEHICLE_COLS
        ),
        params![uid],
        row_to_vehicle,
    )
    .optional()
}

/// List all vehicles (newest first).
pub fn list_vehicles(conn: &Connection) -> SqlResult<Vec<Vehicle>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {} FROM vehicles ORDER BY created_at DESC, id DESC", VEHICLE_COLS))?;
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
             ORDER BY created_at DESC, id DESC LIMIT 1",
            VEHICLE_COLS
        ),
        params![craft],
        row_to_vehicle,
    )
    .optional()
}

/// Delete a vehicle. Its flights are un-linked (`vehicle_id` NULL → "not in library"), never deleted;
/// they keep their craft name and FC id. Done explicitly as well as by the FK's `ON DELETE SET NULL`,
/// so a connection without `foreign_keys` cannot leave dangling ids.
pub fn delete_vehicle(conn: &Connection, id: i64) -> SqlResult<()> {
    let tx = write_tx(conn)?;
    tx.execute("UPDATE flights SET vehicle_id = NULL WHERE vehicle_id = ?1", params![id])?;
    tx.execute("DELETE FROM vehicles WHERE id = ?1", params![id])?;
    tx.commit()
}

/// The newest vehicle (by `created_at`, then id) storing this FC hardware id — never for an empty or
/// synthetic id (`hardware_uid`).
fn vehicle_id_by_fc_uid(conn: &Connection, fc_uid: &str) -> SqlResult<Option<i64>> {
    let Some(uid) = hardware_uid(fc_uid) else {
        return Ok(None);
    };
    conn.prepare_cached(
        "SELECT id FROM vehicles WHERE TRIM(fc_uid) = ?1 COLLATE NOCASE \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )?
    .query_row(params![uid], |r| r.get(0))
    .optional()
}

/// The newest vehicle (by `created_at`, then id) carrying this craft name (trimmed, case-insensitive) —
/// never for an empty one.
fn vehicle_id_by_craft_name(conn: &Connection, craft: &str) -> SqlResult<Option<i64>> {
    let Some(craft) = normalize_craft_name(craft) else {
        return Ok(None);
    };
    conn.prepare_cached(
        "SELECT id FROM vehicles WHERE TRIM(craft_name) = ?1 COLLATE NOCASE \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )?
    .query_row(params![craft], |r| r.get(0))
    .optional()
}

/// Where a flight flown with `fc_uid` / `craft_name` points in the library: the vehicle storing the FC
/// hardware id, else the newest vehicle carrying the craft name — then `conflict` is set when the flight
/// has a hardware id and that vehicle stores a different one (FC swapped, or a second aircraft with the
/// same craft name). Shared by `resolve_vehicle_id` and `resolve_vehicle`.
fn vehicle_candidate(conn: &Connection, fc_uid: Option<&str>, craft_name: &str) -> SqlResult<Option<(i64, &'static str, bool)>> {
    let uid = fc_uid.and_then(hardware_uid);
    if let Some(u) = uid.as_deref() {
        if let Some(id) = vehicle_id_by_fc_uid(conn, u)? {
            return Ok(Some((id, "uid", false)));
        }
    }
    let Some(id) = vehicle_id_by_craft_name(conn, craft_name)? else {
        return Ok(None);
    };
    // The vehicle's stored id counts only as a hardware id (a stored synthetic id is no identity).
    let stored: Option<String> = conn
        .query_row("SELECT fc_uid FROM vehicles WHERE id = ?1", params![id], |r| r.get(0))
        .optional()?
        .flatten();
    let stored = stored.as_deref().and_then(hardware_uid);
    let conflict = matches!((uid.as_deref(), stored.as_deref()), (Some(a), Some(b)) if !a.eq_ignore_ascii_case(b));
    Ok(Some((id, "craft", conflict)))
}

/// The library vehicle a flight flown with `fc_uid` / `craft_name` belongs to, for every automatic
/// link (flight insert, the v20 back-fill, `.kflight` import): the vehicle storing the FC hardware id
/// first, else the one carrying the craft name (vehicles and flights without a UID — 1.0 data, passive
/// telemetry); the newest vehicle when several match. When unsure, unlinked (Marc, 2026-11-11): a
/// flight with a hardware id never links by craft name to a vehicle that stores a DIFFERENT one — it
/// stays unlinked until the user decides. `None` = not in the library.
pub fn resolve_vehicle_id(conn: &Connection, fc_uid: Option<&str>, craft_name: &str) -> SqlResult<Option<i64>> {
    Ok(match vehicle_candidate(conn, fc_uid, craft_name)? {
        Some((id, _, false)) => Some(id),
        _ => None,
    })
}

/// The post-flight summary's pick: the vehicle `resolve_vehicle_id` names, how it matched — and, unlike
/// the automatic link, also a craft-name candidate whose stored FC id differs from the flight's, reported
/// with `uid_conflict` so the summary can ask (see `VehicleResolution::uid_conflict`).
pub fn resolve_vehicle(conn: &Connection, fc_uid: Option<&str>, craft_name: &str) -> SqlResult<VehicleResolution> {
    let Some((id, matched_by, uid_conflict)) = vehicle_candidate(conn, fc_uid, craft_name)? else {
        return Ok(VehicleResolution::default());
    };
    Ok(VehicleResolution { vehicle: get_vehicle(conn, id)?, matched_by: Some(matched_by.into()), uid_conflict })
}

/// Give vehicle `vehicle_id` the FC hardware id `fc_uid` (`None` / empty clears it) and take it off
/// every other vehicle that stores it (trimmed, case-insensitive) — an FC id belongs to one vehicle.
/// Returns how many other vehicles lost it.
pub fn set_vehicle_fc_uid(conn: &Connection, vehicle_id: i64, fc_uid: Option<&str>) -> SqlResult<usize> {
    let uid = fc_uid.map(str::trim).filter(|s| !s.is_empty());
    let tx = write_tx(conn)?;
    let moved = match uid {
        Some(u) => tx.execute(
            "UPDATE vehicles SET fc_uid = NULL, updated_at = datetime('now') \
             WHERE id != ?1 AND TRIM(fc_uid) = ?2 COLLATE NOCASE",
            params![vehicle_id, u],
        )?,
        None => 0,
    };
    let set = tx.execute(
        "UPDATE vehicles SET fc_uid = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![vehicle_id, uid],
    )?;
    if set == 0 {
        // Unknown vehicle: dropping `tx` rolls the removal from the other vehicles back.
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    tx.commit()?;
    Ok(moved)
}

/// Link a flight to a library vehicle, or un-link it (`None`). Returns whether the flight exists. An
/// unknown vehicle id fails on the foreign key.
pub fn set_flight_vehicle(conn: &Connection, flight_id: i64, vehicle_id: Option<i64>) -> SqlResult<bool> {
    let n = conn.execute(
        "UPDATE flights SET vehicle_id = ?1 WHERE id = ?2",
        params![vehicle_id, flight_id],
    )?;
    Ok(n > 0)
}

/// The flights a link-by-identity moves to vehicle `?1`: those flown with the craft name `?2` (trimmed,
/// case-insensitive) or with the FC hardware id `?3`, not linked to that vehicle yet — with `?4`
/// (only_unlinked), only flights without any vehicle. With an FC id key the craft-name arm never
/// crosses a different FC id: a flight matches by name only when it has no hardware id of its own
/// (none, empty, all zero or the INAV SITL constant `?5` — `msp::hardware_uid`) or the same one.
const LINKABLE_FLIGHTS_WHERE: &str = "((?2 IS NOT NULL AND TRIM(craft_name) = ?2 COLLATE NOCASE \
         AND (?3 IS NULL OR fc_uid IS NULL OR REPLACE(TRIM(fc_uid), '0', '') = '' \
              OR TRIM(fc_uid) = ?5 COLLATE NOCASE OR TRIM(fc_uid) = ?3 COLLATE NOCASE)) \
       OR (?3 IS NOT NULL AND TRIM(fc_uid) = ?3 COLLATE NOCASE)) \
     AND vehicle_id IS NOT ?1 \
     AND (?4 = 0 OR vehicle_id IS NULL)";

/// The normalized keys of a link-by-identity (craft name, FC hardware id); `None` when neither is
/// usable (absent, empty or a synthetic id) — such a link touches no flight.
fn link_keys(craft_name: Option<&str>, fc_uid: Option<&str>) -> Option<(Option<String>, Option<String>)> {
    let craft = craft_name.and_then(normalize_craft_name);
    let uid = fc_uid.and_then(hardware_uid);
    if craft.is_none() && uid.is_none() {
        return None;
    }
    Some((craft, uid))
}

/// How many flights `link_flights_to_vehicle` would link (the "link N flights?" question).
pub fn count_linkable_flights(
    conn: &Connection,
    vehicle_id: i64,
    craft_name: Option<&str>,
    fc_uid: Option<&str>,
    only_unlinked: bool,
) -> SqlResult<usize> {
    let Some((craft, uid)) = link_keys(craft_name, fc_uid) else {
        return Ok(0);
    };
    let n: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM flights WHERE {LINKABLE_FLIGHTS_WHERE}"),
        params![vehicle_id, craft, uid, only_unlinked, INAV_SITL_UID],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Link every flight flown with `craft_name` and/or `fc_uid` to vehicle `vehicle_id` (back-filling old
/// flights on the user's approval; see `LINKABLE_FLIGHTS_WHERE`). Returns the number linked. The
/// flights keep their craft name and FC id.
pub fn link_flights_to_vehicle(
    conn: &Connection,
    vehicle_id: i64,
    craft_name: Option<&str>,
    fc_uid: Option<&str>,
    only_unlinked: bool,
) -> SqlResult<usize> {
    let Some((craft, uid)) = link_keys(craft_name, fc_uid) else {
        return Ok(0);
    };
    conn.execute(
        &format!("UPDATE flights SET vehicle_id = ?1 WHERE {LINKABLE_FLIGHTS_WHERE}"),
        params![vehicle_id, craft, uid, only_unlinked, INAV_SITL_UID],
    )
}

/// Move every flight of vehicle `from_id` to vehicle `to_id` (a replaced aircraft's flights stay with
/// the pilot's new one). Returns the number moved; `from_id == to_id` moves nothing. An unknown target
/// fails on the foreign key.
pub fn transfer_vehicle_flights(conn: &Connection, from_id: i64, to_id: i64) -> SqlResult<usize> {
    if from_id == to_id {
        return Ok(0);
    }
    conn.execute(
        "UPDATE flights SET vehicle_id = ?2 WHERE vehicle_id = ?1",
        params![from_id, to_id],
    )
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

/// Aggregate the flights linked to a vehicle (`flights.vehicle_id`): totals + the per-flight records
/// (max flight time / distance / altitude, each with the achieving flight id).
pub fn vehicle_aggregate(conn: &Connection, vehicle_id: i64) -> SqlResult<VehicleAggregate> {
    let mut agg = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(duration_sec), 0), COALESCE(SUM(total_distance_m), 0),
                MIN(start_time), MAX(start_time)
         FROM flights WHERE vehicle_id = ?1",
        params![vehicle_id],
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
             WHERE vehicle_id = ?1 AND duration_sec IS NOT NULL
             ORDER BY duration_sec DESC LIMIT 1",
            params![vehicle_id],
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
             WHERE vehicle_id = ?1 AND total_distance_m IS NOT NULL
             ORDER BY total_distance_m DESC LIMIT 1",
            params![vehicle_id],
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
             WHERE vehicle_id = ?1 AND max_alt_m IS NOT NULL
             ORDER BY max_alt_m DESC LIMIT 1",
            params![vehicle_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()?
    {
        agg.max_altitude_flight_id = Some(id);
        agg.max_altitude_m = Some(v);
    }
    Ok(agg)
}

/// List the flight summaries linked to a vehicle (Manager detail + the delete warning's
/// "N linked flights").
pub fn list_flights_for_vehicle(conn: &Connection, vehicle_id: i64) -> SqlResult<Vec<FlightSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, start_time, duration_sec, source, craft_name, location_name,
            max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id, notes,
            utc_offset_min, group_id, vehicle_id
         FROM flights WHERE vehicle_id = ?1 ORDER BY start_time DESC",
    )?;
    let rows = stmt.query_map(params![vehicle_id], |row| {
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
            group_id: row.get(13)?,
            vehicle_id: row.get(14)?,
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
    // The variant selects how the stored mode flags are decoded (see `read_flight_track`).
    let fc_variant: Option<String> = conn
        .query_row(
            "SELECT fc_variant FROM flights WHERE id = ?1",
            params![flight_id],
            |row| row.get(0),
        )
        .optional()?;
    read_flight_track(conn, flight_id, fc_variant.as_deref())
}

/// Get the GPS track for a flight with its `fc_variant` supplied by the caller — the form for stores
/// without a `flights` table (the per-session `.ktmp`, where the variant comes from `session_meta`).
/// `get_flight_track` is the main-DB entry that looks the variant up itself.
pub fn read_flight_track(
    conn: &Connection,
    flight_id: i64,
    fc_variant: Option<&str>,
) -> SqlResult<Vec<TelemetryRecord>> {
    // Flights recorded before the unified flight-mode model (f8e5699, 2026-06-14) stored the raw
    // mode flags but no canonical mode, and replay them as N/A. An import can be repeated, a live
    // recording cannot, so the mode is derived here from the flags that are already in the row.
    // `active_flight_mode_flags` holds INAV's bitmask or MAVLink's custom_mode depending on the
    // source, which is what the variant selects between.
    let mut stmt = conn.prepare(&format!(
        "SELECT id, {TELEMETRY_COLS} FROM telemetry_records          WHERE flight_id = ?1 ORDER BY timestamp_ms ASC"
    ))?;

    let rows = stmt.query_map(params![flight_id], read_telemetry_record)?;
    let mut out = Vec::new();
    for rec in rows {
        let mut rec = rec?;
        if rec.mode_primary.is_none() {
            if let (Some(flags), Some(variant)) = (rec.active_flight_mode_flags, fc_variant) {
                let derived = if variant.eq_ignore_ascii_case("INAV") {
                    crate::flightmode::classify_inav(flags as u32)
                } else {
                    crate::flightmode::classify_mavlink(flags as u32, variant)
                };
                rec.mode_primary = Some(derived.primary.clone());
                if rec.mode_modifiers.is_none() && !derived.modifiers.is_empty() {
                    rec.mode_modifiers = Some(derived.modifiers.join(","));
                }
            }
        }
        out.push(rec);
    }
    Ok(out)
}

/// Map one `SELECT id, {TELEMETRY_COLS}` row to a `TelemetryRecord` (stored values only — the
/// legacy mode fix-up in `read_flight_track` post-processes the record where it applies).
fn read_telemetry_record(row: &rusqlite::Row) -> SqlResult<TelemetryRecord> {
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
        wall_ms: row.get(47)?,
    })
}

/// Latest hi-res row at or before `timestamp_ms` from a hi-res cache DB (HIRES_REPLAY plan).
/// The cache holds exactly one flight (`flight_id` 0) and its rows always carry `mode_primary`
/// (set at parse time), so no `flights`-table fix-up is needed — the file has no such table.
pub fn get_hires_sample(conn: &Connection, timestamp_ms: i64) -> SqlResult<Option<TelemetryRecord>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT id, {TELEMETRY_COLS} FROM telemetry_records \
         WHERE timestamp_ms <= ?1 ORDER BY timestamp_ms DESC LIMIT 1"
    ))?;
    stmt.query_row(params![timestamp_ms], read_telemetry_record)
        .optional()
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
                    fc_uid: None,
                    group_id: None,
                    vehicle_id: None,
                })
            },
        )
        .map_err(|e| format!("Duplicate check flight fetch failed: {}", e))?;

    Ok(Some(flight))
}

/// Delete a flight and all related data (telemetry, battery samples, events, archived files).
/// Explicitly deletes child rows first (in case foreign_keys is off), then starts the background
/// reclaim of the freed pages.
pub fn delete_flight(conn: &Connection, flight_id: i64) -> SqlResult<bool> {
    let deleted = delete_flight_rows(conn, flight_id)?;

    // Hand the freed pages (large blackbox BLOBs) back in the background — incremental, not a full
    // rewrite; the delete itself is committed and succeeds whatever the reclaim does.
    if deleted {
        spawn_reclaim(conn);
    }

    Ok(deleted)
}

/// SQL expression on a `flights` row of a `both` flight: its own source (schema v21) — what it was
/// before `link_flights`, or, without a stored value (an orphan `both`, or a pair whose value was
/// lost), `blackbox` for a log importer's flight or one that holds an archived log, `live` otherwise.
/// A macro so the consts below can embed it at compile time.
macro_rules! own_source_sql {
    () => {
        concat!(
            "COALESCE(source_before_link, CASE WHEN ",
            log_protocol_sql!(),
            " OR EXISTS(SELECT 1 FROM blackbox_files WHERE flight_id = flights.id) THEN 'blackbox' ELSE 'live' END)"
        )
    };
}

/// `own_source_sql!` as a value, for the `.kflight` export (exchange.rs), whose `flights` table has the
/// same columns.
pub(crate) const OWN_SOURCE_SQL: &str = own_source_sql!();

/// `SET` clause that ends a flight's link (schema v21): `source` goes back to the flight's own source
/// (`own_source_sql!`). Also applied to `.kflight` files and imports (exchange.rs).
pub(crate) const END_LINK_SET: &str = concat!(
    "linked_flight_id = NULL, source = ",
    own_source_sql!(),
    ", source_before_link = NULL"
);

/// The row work of `delete_flight` without the page reclaim, so it can run inside a transaction.
fn delete_flight_rows(conn: &Connection, flight_id: i64) -> SqlResult<bool> {
    // Clear any linked_flight_id references pointing to this flight
    conn.execute(
        &format!("UPDATE flights SET {END_LINK_SET} WHERE linked_flight_id = ?1"),
        params![flight_id],
    )?;

    // Explicitly delete child tables (don't rely solely on CASCADE; `battery_records` has no FK at
    // all, so without this its rows outlived the flight).
    conn.execute("DELETE FROM blackbox_files WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM telemetry_records WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM battery_records WHERE flight_id = ?1", params![flight_id])?;
    conn.execute("DELETE FROM flight_events WHERE flight_id = ?1", params![flight_id])?;
    let affected = conn.execute("DELETE FROM flights WHERE id = ?1", params![flight_id])?;
    Ok(affected > 0)
}

// ── Group flights (schema v20, Dev-Docs active/GROUP_FLIGHTS.md) ─────────────────────────

#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
const FLIGHT_GROUP_COLS: &str = "id, start_time, end_time, utc_offset_min, start_lat, start_lon, \
    start_alt_m, location_name, weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc, \
    notes, created_at";

/// Map one `SELECT {FLIGHT_GROUP_COLS}` row.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
fn read_flight_group(row: &rusqlite::Row) -> SqlResult<FlightGroup> {
    let start_str: String = row.get(1)?;
    let end_str: Option<String> = row.get(2)?;
    Ok(FlightGroup {
        id: row.get(0)?,
        start_time: DateTime::parse_from_rfc3339(&start_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        end_time: end_str.and_then(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc)).ok()),
        utc_offset_min: row.get(3)?,
        start_lat: row.get(4)?,
        start_lon: row.get(5)?,
        start_alt_m: row.get(6)?,
        location_name: row.get(7)?,
        weather_temp_c: row.get(8)?,
        weather_wind_ms: row.get(9)?,
        weather_wind_deg: row.get(10)?,
        weather_desc: row.get(11)?,
        notes: row.get(12)?,
        created_at: row.get(13)?,
    })
}

/// Insert a group flight. Idempotent (`INSERT OR IGNORE` on the immutable id): committing a further
/// member into an existing group keeps the stored header and its notes. `created_at` is set by the
/// database. Returns whether a row was inserted.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn insert_flight_group(conn: &Connection, g: &FlightGroup) -> SqlResult<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO flight_groups (
            id, start_time, end_time, utc_offset_min, start_lat, start_lon, start_alt_m,
            location_name, weather_temp_c, weather_wind_ms, weather_wind_deg, weather_desc, notes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            g.id,
            g.start_time.to_rfc3339(),
            g.end_time.map(|t| t.to_rfc3339()),
            g.utc_offset_min,
            g.start_lat,
            g.start_lon,
            g.start_alt_m,
            g.location_name,
            g.weather_temp_c,
            g.weather_wind_ms,
            g.weather_wind_deg,
            g.weather_desc,
            g.notes,
        ],
    )?;
    Ok(n > 0)
}

/// One group flight by id.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn get_flight_group(conn: &Connection, id: &str) -> SqlResult<Option<FlightGroup>> {
    conn.query_row(
        &format!("SELECT {FLIGHT_GROUP_COLS} FROM flight_groups WHERE id = ?1"),
        params![id],
        read_flight_group,
    )
    .optional()
}

/// Every group flight, newest first.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn list_flight_groups(conn: &Connection) -> SqlResult<Vec<FlightGroup>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FLIGHT_GROUP_COLS} FROM flight_groups ORDER BY start_time DESC"
    ))?;
    let rows = stmt.query_map([], read_flight_group)?;
    rows.collect()
}

/// Ids of the member flights of a group, oldest first.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn list_flight_group_member_ids(conn: &Connection, group_id: &str) -> SqlResult<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id FROM flights WHERE group_id = ?1 ORDER BY start_time ASC, id ASC")?;
    let rows = stmt.query_map(params![group_id], |row| row.get(0))?;
    rows.collect()
}

/// Update the notes of a group flight.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn update_flight_group_notes(conn: &Connection, id: &str, notes: &str) -> SqlResult<()> {
    conn.execute("UPDATE flight_groups SET notes = ?1 WHERE id = ?2", params![notes, id])?;
    Ok(())
}

/// Delete a group flight. `with_members = false` **dissolves** it: the members stay as single flights
/// (`group_id` → NULL). `with_members = true` deletes the member flights with all their data too.
/// One transaction; returns whether the group existed.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn delete_flight_group(conn: &Connection, id: &str, with_members: bool) -> SqlResult<bool> {
    let tx = write_tx(conn)?;
    let mut members_deleted = false;
    if with_members {
        for member in list_flight_group_member_ids(&tx, id)? {
            members_deleted |= delete_flight_rows(&tx, member)?;
        }
    } else {
        // Explicit, like the child deletes in `delete_flight` (ON DELETE SET NULL needs foreign_keys on).
        tx.execute("UPDATE flights SET group_id = NULL WHERE group_id = ?1", params![id])?;
    }
    let affected = tx.execute("DELETE FROM flight_groups WHERE id = ?1", params![id])?;
    tx.commit()?;
    if members_deleted {
        spawn_reclaim(conn);
    }
    Ok(affected > 0)
}

/// Append timeline events to their flights (`id` is ignored — assigned by the database).
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn insert_flight_events(conn: &Connection, events: &[FlightEvent]) -> SqlResult<()> {
    let tx = write_tx(conn)?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO flight_events (flight_id, timestamp_ms, wall_ms, kind, code, text, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for e in events {
            stmt.execute(params![e.flight_id, e.timestamp_ms, e.wall_ms, e.kind, e.code, e.text, e.source])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// A flight's timeline events in time order.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
pub fn get_flight_events(conn: &Connection, flight_id: i64) -> SqlResult<Vec<FlightEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, flight_id, timestamp_ms, wall_ms, kind, code, text, source
         FROM flight_events WHERE flight_id = ?1 ORDER BY timestamp_ms ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![flight_id], |row| {
        Ok(FlightEvent {
            id: row.get(0)?,
            flight_id: row.get(1)?,
            timestamp_ms: row.get(2)?,
            wall_ms: row.get(3)?,
            kind: row.get(4)?,
            code: row.get(5)?,
            text: row.get(6)?,
            source: row.get(7)?,
        })
    })?;
    rows.collect()
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

/// Update the craft_name field of a flight (the logbook's manual correction). The vehicle link
/// (`vehicle_id`) is not touched — it is changed explicitly (`set_flight_vehicle`).
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

/// Link two flights together (bidirectional). Sets `linked_flight_id` on both and updates their
/// `source` to "both"; the source each had before is kept in `source_before_link` for the unlink (a
/// flight that already reads "both" keeps the value it has).
pub fn link_flights(conn: &Connection, flight_a: i64, flight_b: i64) -> SqlResult<()> {
    eprintln!("[LINK] Linking flights {} <-> {}", flight_a, flight_b);
    const LINK: &str = "UPDATE flights SET linked_flight_id = ?1,
            source_before_link = CASE WHEN source = 'both' THEN source_before_link ELSE source END,
            source = 'both'
         WHERE id = ?2";
    conn.execute(LINK, params![flight_b, flight_a])?;
    conn.execute(LINK, params![flight_a, flight_b])?;
    Ok(())
}

/// Remove the link from a flight and its partner. Each gets back the source it had before the link
/// (`END_LINK_SET`).
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
    let end_link = format!("UPDATE flights SET {END_LINK_SET} WHERE id = ?1");
    conn.execute(&end_link, params![flight_id])?;

    // Clear link on partner (if exists)
    if let Some(pid) = partner_id {
        conn.execute(&end_link, params![pid])?;
        eprintln!("[UNLINK] Unlinked flight {} from {}", flight_id, pid);
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
                                        max_alt_m, max_speed_ms, total_distance_m, platform_type, linked_flight_id,
                    group_id, vehicle_id
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
                    group_id: row.get(11)?,
                    vehicle_id: row.get(12)?,
                })
            },
        )
        .optional()?;

    Ok(result)
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
/// flight row and its `telemetry_records` intact — replay still works from the DB; the "export
/// original" capability and hi-res replay (which re-decodes the original) are lost. Returns the
/// deleted file's original filename (None if there was nothing to delete).
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
        // Hand the freed BLOB pages back in the background, not via a full-DB VACUUM rewrite.
        spawn_reclaim(conn);
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
    fn migrate_is_reentrant() {
        let conn = test_db(); // already migrated once
        migrate(&conn).unwrap(); // a second pass must be a clean no-op
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    /// Unique on-disk DB path (backup/guard logic needs real files, not :memory:). One directory per
    /// test: `backup_path_for` is a fixed sibling name, so DBs sharing a folder would race on it.
    fn temp_db_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-db-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("flights.db")
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn downgrade_guard_refuses_newer_db() {
        let path = temp_db_path("downgrade");
        cleanup(&path);
        drop(open_database(&path).unwrap()); // fresh, v = CURRENT
        {
            let conn = Connection::open(&path).unwrap();
            set_user_version(&conn, CURRENT_SCHEMA_VERSION + 1).unwrap();
        }
        let err = open_database(&path).unwrap_err().to_string();
        assert!(err.contains("db-newer"), "expected the db-newer marker, got: {err}");
        // The file must be untouched: still stamped as the newer version, no backup made.
        let conn = Connection::open(&path).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION + 1);
        assert!(!backup_path_for(&path).exists());
        drop(conn);
        cleanup(&path);
    }

    #[test]
    fn backup_created_before_pending_migration_only() {
        let path = temp_db_path("backup");
        cleanup(&path);
        // Fresh DB (v0): full chain runs, but there is nothing to back up.
        drop(open_database(&path).unwrap());
        assert!(!backup_path_for(&path).exists(), "fresh DB must not create a backup");
        // Roll the stamp back one version (the v17→v18 step is idempotent) → reopen must
        // snapshot BEFORE migrating and land on CURRENT again.
        {
            let conn = Connection::open(&path).unwrap();
            set_user_version(&conn, CURRENT_SCHEMA_VERSION - 1).unwrap();
        }
        let conn = open_database(&path).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        let backup = backup_path_for(&path);
        assert!(backup.exists(), "pending migration must create the backup");
        // The snapshot is a valid SQLite DB stamped with the PRE-migration version.
        let bconn = Connection::open(&backup).unwrap();
        assert_eq!(get_user_version(&bconn).unwrap(), CURRENT_SCHEMA_VERSION - 1);
        drop((conn, bconn));
        cleanup(&path);
    }

    /// Hi-res cache sampling (HIRES_REPLAY): nearest row at-or-before the timestamp, None before
    /// the first row (the player then falls back to the 10 Hz sample).
    #[test]
    fn hires_sample_picks_row_at_or_before_timestamp() {
        let path = temp_db_path("hires-sample");
        cleanup(&path);
        let conn = open_temp_session(&path).unwrap();
        for ts in [0i64, 100, 200] {
            conn.execute(
                "INSERT INTO telemetry_records (flight_id, timestamp_ms) VALUES (0, ?1)",
                params![ts],
            )
            .unwrap();
        }
        assert!(get_hires_sample(&conn, -1).unwrap().is_none());
        assert_eq!(get_hires_sample(&conn, 0).unwrap().unwrap().timestamp_ms, 0);
        assert_eq!(get_hires_sample(&conn, 150).unwrap().unwrap().timestamp_ms, 100);
        assert_eq!(get_hires_sample(&conn, 99_999).unwrap().unwrap().timestamp_ms, 200);
        drop(conn);
        remove_temp_session(&path);
    }

    /// Full open/migrate pass over a COPY of a real flight database (Marc's RC2 archive,
    /// F:\DEVELOPMENT\TEST_DATA\Kite-GC-testdata\flights-real.db — outside the repo).
    /// `KITE_TEST_DB=<path> cargo test migrates_the_real_database -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn migrates_the_real_database() {
        let Ok(src) = std::env::var("KITE_TEST_DB") else {
            eprintln!("KITE_TEST_DB not set — skipping");
            return;
        };
        let src = PathBuf::from(src);
        // Work on a sibling copy so the fixture keeps its original schema version for reruns.
        let work = src.with_extension("test-copy.db");
        let _ = std::fs::remove_file(&work);
        let _ = std::fs::remove_file(backup_path_for(&work));
        std::fs::copy(&src, &work).expect("copy fixture");
        let mb = |p: &Path| std::fs::metadata(p).map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
        let (from_version, records_before, linked_before): (u32, i64, i64) = {
            let c = Connection::open(&work).unwrap();
            let records = if table_exists(&c, "blackbox_records").unwrap() {
                c.query_row("SELECT COUNT(*) FROM blackbox_records", [], |r| r.get(0)).unwrap()
            } else {
                0
            };
            let linked = c
                .query_row("SELECT COUNT(*) FROM flights WHERE linked_flight_id IS NOT NULL", [], |r| r.get(0))
                .unwrap();
            (get_user_version(&c).unwrap(), records, linked)
        };
        eprintln!(
            "real DB before: v{from_version}, {:.1} MB, {records_before} blackbox_records rows, {linked_before} linked flights",
            mb(&work)
        );
        let opened = std::time::Instant::now();
        let conn = open_database(&work).expect("open + migrate the real DB");
        let open_ms = opened.elapsed().as_millis();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        // v21: the decoded rows are gone, the freed pages were handed back (open_database starts the
        // background reclaim when the freelist is large), and every linked flight knows its own source.
        let db_file = conn.path().unwrap().to_string();
        let reclaim = wait_for_reclaim(&db_file, std::time::Duration::from_secs(120));
        eprintln!(
            "real DB open + migrate: {open_ms} ms (backup included); background reclaim finished {} ms after the open returned",
            reclaim.as_millis()
        );
        assert!(!table_exists(&conn, "blackbox_records").unwrap());
        assert!(!object_exists(&conn, "index", "idx_blackbox_records_flight"));
        let free_after = freelist_bytes(&conn).unwrap();
        let (unset, as_live, as_blackbox): (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*) FILTER (WHERE source_before_link IS NULL),
                        COUNT(*) FILTER (WHERE source_before_link = 'live'),
                        COUNT(*) FILTER (WHERE source_before_link = 'blackbox')
                 FROM flights WHERE linked_flight_id IS NOT NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(unset, 0, "every linked flight gets source_before_link");
        let stray: i64 = conn
            .query_row("SELECT COUNT(*) FROM flights WHERE linked_flight_id IS NULL AND source_before_link IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stray, 0, "unlinked flights keep source_before_link NULL");
        eprintln!(
            "real DB after: source_before_link live={as_live} blackbox={as_blackbox}, freelist {:.1} MB",
            free_after as f64 / 1_048_576.0
        );
        assert_eq!(free_after, 0, "the upgrade reclaim must empty the freelist");
        let flights: i64 =
            conn.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        let telemetry: i64 =
            conn.query_row("SELECT COUNT(*) FROM telemetry_records", [], |r| r.get(0)).unwrap();
        eprintln!("real DB: {flights} flights, {telemetry} telemetry rows");
        assert!(flights > 0, "expected flights in the real archive");
        assert!(telemetry > 0, "expected telemetry in the real archive");
        // v20 objects landed next to the existing data; the old rows carry no group / wall clock.
        assert!(column_exists(&conn, "flights", "group_id").unwrap());
        assert!(column_exists(&conn, "telemetry_records", "wall_ms").unwrap());
        let grouped: i64 = conn
            .query_row("SELECT COUNT(*) FROM flights WHERE group_id IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(grouped, 0);
        assert!(list_flight_groups(&conn).unwrap().is_empty());
        // flights.vehicle_id landed and was back-filled by the resolution rule: every link names an
        // existing vehicle and is exactly what `resolve_vehicle_id` picks for the flight.
        assert!(column_exists(&conn, "flights", "vehicle_id").unwrap());
        assert!(object_exists(&conn, "index", "idx_flights_vehicle"));
        let vehicles = list_vehicles(&conn).unwrap().len();
        let rows: Vec<(i64, Option<String>, String, Option<i64>)> = conn
            .prepare("SELECT id, fc_uid, craft_name, vehicle_id FROM flights")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<SqlResult<_>>()
            .unwrap();
        let linked = rows.iter().filter(|r| r.3.is_some()).count();
        for (id, uid, craft, vehicle_id) in &rows {
            assert_eq!(*vehicle_id, resolve_vehicle_id(&conn, uid.as_deref(), craft).unwrap(), "flight {id}");
            if let Some(v) = vehicle_id {
                assert!(get_vehicle(&conn, *v).unwrap().is_some(), "flight {id} links a missing vehicle {v}");
            }
        }
        eprintln!("real DB: {vehicles} vehicles, {linked} of {flights} flights linked by the back-fill");
        // The reclaim thread closed its connection before it left the in-flight list, so this is the
        // last close: it checkpoints the WAL, which truncates the file to its page count.
        drop(conn);
        eprintln!(
            "real DB file: {:.1} MB after the upgrade (pre-migration backup {:.1} MB)",
            mb(&work),
            mb(&backup_path_for(&work))
        );
        remove_temp_session(&work); // the copy + its -wal / -shm
        let _ = std::fs::remove_file(backup_path_for(&work));
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
            fc_uid: None,
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
            group_id: None,
            vehicle_id: None,
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
            fc_uid: None,
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
            group_id: None,
            vehicle_id: None,
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
                wall_ms: None,
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
            fc_uid: None,
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
            group_id: None,
            vehicle_id: None,
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
            wall_ms: None,
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
            fc_uid: None,
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
            group_id: None,
            vehicle_id: None,
        };
        let flight_id = insert_flight(&conn, &flight).unwrap();
        insert_blackbox_file(&conn, flight_id, "test.TXT", 0, b"abc").unwrap();

        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM blackbox_files WHERE flight_id = ?1", params![flight_id], |row| row.get(0))
            .unwrap();

        assert_eq!(file_count, 1);
        // v21: the decoded rows are no longer stored.
        assert!(!table_exists(&conn, "blackbox_records").unwrap());
    }

    #[test]
    fn sweep_temp_sessions_keeps_protected_and_removes_the_rest() {
        let dir = std::env::temp_dir().join(format!("kite-sweep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("active_live.ktmp");
        let old_a = dir.join("active_old_a.ktmp");
        let old_b = dir.join("active_old_b.ktmp");
        for p in [&live, &old_a, &old_b] {
            drop(open_temp_session(p).unwrap());
        }
        std::fs::write(dir.join("notes.txt"), b"unrelated").unwrap();

        let removed = sweep_temp_sessions(&dir, std::slice::from_ref(&live));

        assert_eq!(removed, 2);
        assert!(live.exists(), "the protected live session must survive the sweep");
        assert!(!old_a.exists() && !old_b.exists(), "stragglers must be gone");
        assert!(dir.join("notes.txt").exists(), "only .ktmp files are touched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Schema v20: group flights, flight events, per-row wall clock ─────────────────────────

    /// Apply the real migration steps up to `version` (no self-heal pass) — a DB as that Kite left it.
    fn migrate_to(conn: &Connection, version: u32) {
        for (target, step) in MIGRATION_STEPS {
            if target <= version {
                let tx = write_tx(conn).unwrap();
                step(&tx).unwrap();
                tx.commit().unwrap();
            }
        }
        assert_eq!(get_user_version(conn).unwrap(), version);
    }

    fn object_exists(conn: &Connection, kind: &str, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
            params![kind, name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    fn flight(craft: &str, group_id: Option<&str>) -> Flight {
        Flight {
            id: 0,
            start_time: DateTime::parse_from_rfc3339("2026-09-30T10:00:00Z").unwrap().with_timezone(&Utc),
            end_time: None,
            duration_sec: Some(60),
            source: "live".into(),
            craft_name: craft.into(),
            fc_variant: "ArduPlane".into(),
            fc_version: "4.6.0".into(),
            board_id: String::new(),
            platform_type: 1,
            fc_uid: None,
            protocol: "MAVLink".into(),
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
            utc_offset_min: Some(120),
            group_id: group_id.map(String::from),
            vehicle_id: None,
        }
    }

    fn telemetry(flight_id: i64, timestamp_ms: i64, wall_ms: Option<i64>) -> TelemetryRecord {
        TelemetryRecord {
            id: 0,
            flight_id,
            timestamp_ms,
            lat: Some(48.1),
            lon: Some(11.5),
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
            mode_primary: Some("manual".into()),
            mode_modifiers: None,
            link_snr: None,
            link_rssi_dbm: None,
            wall_ms,
        }
    }

    fn battery(flight_id: i64, timestamp_ms: i64) -> BatteryRecord {
        BatteryRecord {
            id: 0,
            flight_id,
            timestamp_ms,
            instance: 0,
            voltage: Some(16.0),
            current_a: None,
            mah_drawn: None,
            battery_percentage: None,
            cell_count: None,
            temperature: None,
        }
    }

    fn event(flight_id: i64, timestamp_ms: i64, kind: &str) -> FlightEvent {
        FlightEvent {
            id: 0,
            flight_id,
            timestamp_ms,
            wall_ms: Some(1_790_000_000_000 + timestamp_ms),
            kind: kind.into(),
            code: None,
            text: None,
            source: Some("live".into()),
        }
    }

    fn group(id: &str, notes: Option<&str>) -> FlightGroup {
        FlightGroup {
            id: id.into(),
            start_time: DateTime::parse_from_rfc3339("2026-09-30T10:00:00Z").unwrap().with_timezone(&Utc),
            end_time: Some(DateTime::parse_from_rfc3339("2026-09-30T10:20:00Z").unwrap().with_timezone(&Utc)),
            utc_offset_min: Some(120),
            start_lat: Some(48.1),
            start_lon: Some(11.5),
            start_alt_m: Some(512.0),
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            notes: notes.map(String::from),
            created_at: String::new(),
        }
    }

    fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
        conn.query_row(sql, params![id], |r| r.get(0)).unwrap()
    }

    /// A real v19 database (built from the v1…v19 steps) gains the v20 objects, keeps its rows, and a
    /// second pass is a clean no-op.
    #[test]
    fn v19_database_migrates_to_v20_and_is_reentrant() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate_to(&conn, 19);
        assert!(!column_exists(&conn, "flights", "group_id").unwrap());
        assert!(!column_exists(&conn, "telemetry_records", "wall_ms").unwrap());
        conn.execute(
            "INSERT INTO flights (start_time, craft_name) VALUES ('2026-09-01T10:00:00+00:00', 'Old')",
            [],
        )
        .unwrap();
        let old_id = conn.last_insert_rowid();
        conn.execute("INSERT INTO telemetry_records (flight_id, timestamp_ms) VALUES (?1, 0)", params![old_id])
            .unwrap();

        migrate(&conn).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        assert!(object_exists(&conn, "table", "flight_groups"));
        assert!(object_exists(&conn, "table", "flight_events"));
        assert!(object_exists(&conn, "index", "idx_flights_group"));
        assert!(object_exists(&conn, "index", "idx_events_flight"));
        assert!(column_exists(&conn, "flights", "group_id").unwrap());
        assert!(column_exists(&conn, "telemetry_records", "wall_ms").unwrap());
        let old = get_flight(&conn, old_id).unwrap().unwrap();
        assert_eq!(old.craft_name, "Old");
        assert!(old.group_id.is_none());
        let track = get_flight_track(&conn, old_id).unwrap();
        assert_eq!(track.len(), 1);
        assert!(track[0].wall_ms.is_none());

        migrate(&conn).unwrap(); // re-entrant
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        // The v19 → v20 step itself is idempotent too (the backup test re-runs it on a v20 DB).
        let tx = conn.unchecked_transaction().unwrap();
        migrate_v19_to_v20(&tx).unwrap();
        tx.commit().unwrap();
    }

    /// Opening a v19 database file snapshots it before the v20 step (the pre-migration backup).
    #[test]
    fn v19_database_file_is_backed_up_before_v20() {
        let path = temp_db_path("v19-backup");
        cleanup(&path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            migrate_to(&conn, 19);
        }
        let conn = open_database(&path).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        let backup = backup_path_for(&path);
        assert!(backup.exists(), "the v19 -> v20 step must take the pre-migration backup");
        let bconn = Connection::open(&backup).unwrap();
        assert_eq!(get_user_version(&bconn).unwrap(), 19);
        drop((conn, bconn));
        cleanup(&path);
    }

    #[test]
    fn group_member_and_events_round_trip() {
        let conn = test_db();
        assert!(insert_flight_group(&conn, &group("0123456789abcdef", Some("first"))).unwrap());
        // Idempotent: a later member commit into the same group keeps the stored header + notes.
        assert!(!insert_flight_group(&conn, &group("0123456789abcdef", Some("second"))).unwrap());

        let member = insert_flight(&conn, &flight("Wing", Some("0123456789abcdef"))).unwrap();
        let single = insert_flight(&conn, &flight("Quad", None)).unwrap();
        insert_flight_events(&conn, &[event(member, 5_000, "disarm"), event(member, 0, "arm"), event(member, 9_000, "arm")])
            .unwrap();

        let g = get_flight_group(&conn, "0123456789abcdef").unwrap().unwrap();
        assert_eq!(g.notes.as_deref(), Some("first"));
        assert_eq!(g.start_alt_m, Some(512.0));
        assert_eq!(g.utc_offset_min, Some(120));
        assert!(!g.created_at.is_empty());
        assert_eq!(get_flight(&conn, member).unwrap().unwrap().group_id.as_deref(), Some("0123456789abcdef"));
        assert!(get_flight(&conn, single).unwrap().unwrap().group_id.is_none());
        let listed = list_flights(&conn).unwrap();
        assert_eq!(listed.iter().find(|f| f.id == member).unwrap().group_id.as_deref(), Some("0123456789abcdef"));
        assert_eq!(list_flight_group_member_ids(&conn, "0123456789abcdef").unwrap(), vec![member]);

        let events = get_flight_events(&conn, member).unwrap();
        let kinds: Vec<(&str, i64)> = events.iter().map(|e| (e.kind.as_str(), e.timestamp_ms)).collect();
        assert_eq!(kinds, [("arm", 0), ("disarm", 5_000), ("arm", 9_000)]);
        assert_eq!(events[1].wall_ms, Some(1_790_000_005_000));
        assert_eq!(events[1].source.as_deref(), Some("live"));

        update_flight_group_notes(&conn, "0123456789abcdef", "edited").unwrap();
        assert_eq!(get_flight_group(&conn, "0123456789abcdef").unwrap().unwrap().notes.as_deref(), Some("edited"));
        assert_eq!(list_flight_groups(&conn).unwrap().len(), 1);
        // A member must point at an existing group (FK).
        assert!(insert_flight(&conn, &flight("Ghost", Some("ffffffffffffffff"))).is_err());
    }

    #[test]
    fn dissolving_a_group_keeps_its_members() {
        let conn = test_db();
        insert_flight_group(&conn, &group("aaaaaaaaaaaaaaaa", None)).unwrap();
        let a = insert_flight(&conn, &flight("A", Some("aaaaaaaaaaaaaaaa"))).unwrap();
        let b = insert_flight(&conn, &flight("B", Some("aaaaaaaaaaaaaaaa"))).unwrap();
        insert_flight_events(&conn, &[event(a, 0, "arm")]).unwrap();

        assert!(delete_flight_group(&conn, "aaaaaaaaaaaaaaaa", false).unwrap());
        assert!(get_flight_group(&conn, "aaaaaaaaaaaaaaaa").unwrap().is_none());
        for id in [a, b] {
            let f = get_flight(&conn, id).unwrap().expect("member survives a dissolve");
            assert!(f.group_id.is_none());
        }
        assert_eq!(get_flight_events(&conn, a).unwrap().len(), 1);
        assert!(!delete_flight_group(&conn, "aaaaaaaaaaaaaaaa", false).unwrap());
    }

    #[test]
    fn deleting_a_group_with_members_removes_all_their_data() {
        let conn = test_db();
        insert_flight_group(&conn, &group("bbbbbbbbbbbbbbbb", None)).unwrap();
        let a = insert_flight(&conn, &flight("A", Some("bbbbbbbbbbbbbbbb"))).unwrap();
        let b = insert_flight(&conn, &flight("B", Some("bbbbbbbbbbbbbbbb"))).unwrap();
        let other = insert_flight(&conn, &flight("Other", None)).unwrap();
        for id in [a, b, other] {
            insert_telemetry_batch(&conn, &[telemetry(id, 0, None), telemetry(id, 100, None)]).unwrap();
            insert_battery_records_batch(&conn, &[battery(id, 0), battery(id, 100)]).unwrap();
            insert_flight_events(&conn, &[event(id, 0, "arm")]).unwrap();
        }

        assert!(delete_flight_group(&conn, "bbbbbbbbbbbbbbbb", true).unwrap());
        assert!(get_flight_group(&conn, "bbbbbbbbbbbbbbbb").unwrap().is_none());
        for id in [a, b] {
            assert!(get_flight(&conn, id).unwrap().is_none());
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM telemetry_records WHERE flight_id = ?1", id), 0);
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM battery_records WHERE flight_id = ?1", id), 0);
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM flight_events WHERE flight_id = ?1", id), 0);
        }
        // A flight outside the group is untouched.
        assert!(get_flight(&conn, other).unwrap().is_some());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM telemetry_records WHERE flight_id = ?1", other), 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM battery_records WHERE flight_id = ?1", other), 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM flight_events WHERE flight_id = ?1", other), 1);
    }

    /// `battery_records` has no FK — `delete_flight` must remove them (and the events) explicitly.
    #[test]
    fn delete_flight_removes_battery_rows_and_events() {
        let conn = test_db();
        let id = insert_flight(&conn, &flight("Del", None)).unwrap();
        insert_battery_records_batch(&conn, &[battery(id, 0)]).unwrap();
        insert_flight_events(&conn, &[event(id, 0, "arm")]).unwrap();
        assert!(delete_flight(&conn, id).unwrap());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM battery_records WHERE flight_id = ?1", id), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM flight_events WHERE flight_id = ?1", id), 0);
    }

    /// `wall_ms` round-trips through the main DB, and through a temp session commit — including a
    /// `.ktmp` written before v20 (no `wall_ms` column), which `open_temp_session` back-fills.
    #[test]
    fn wall_ms_round_trips_through_main_and_temp_store() {
        let conn = test_db();
        let id = insert_flight(&conn, &flight("Wall", None)).unwrap();
        insert_telemetry_batch(&conn, &[telemetry(id, 0, Some(1_790_000_000_123)), telemetry(id, 100, None)]).unwrap();
        let track = get_flight_track(&conn, id).unwrap();
        assert_eq!(track[0].wall_ms, Some(1_790_000_000_123));
        assert_eq!(track[1].wall_ms, None);

        // Temp store → commit.
        let temp = temp_db_path("wall-ms-ktmp").with_file_name("active_test_L1-S1.ktmp");
        remove_temp_session(&temp);
        {
            let t = open_temp_session(&temp).unwrap();
            insert_telemetry_batch(&t, &[telemetry(0, 0, Some(1_790_000_000_500)), telemetry(0, 200, Some(1_790_000_000_700))])
                .unwrap();
        }
        let committed = commit_session_to_main(&conn, &temp, &flight("Wall", None)).unwrap();
        let track = get_flight_track(&conn, committed).unwrap();
        assert_eq!(track.iter().map(|r| r.wall_ms).collect::<Vec<_>>(), [Some(1_790_000_000_500), Some(1_790_000_000_700)]);
        remove_temp_session(&temp);

        // A pre-v20 `.ktmp` (telemetry DDL without wall_ms) is back-filled on open and still commits.
        {
            let legacy = Connection::open(&temp).unwrap();
            legacy
                .execute_batch(&TELEMETRY_RECORDS_DDL_FULL.replace(", wall_ms INTEGER", ""))
                .unwrap();
            legacy.execute_batch(BATTERY_RECORDS_DDL_FULL).unwrap();
            legacy.execute("INSERT INTO telemetry_records (flight_id, timestamp_ms) VALUES (0, 0)", []).unwrap();
            assert!(!column_exists(&legacy, "telemetry_records", "wall_ms").unwrap());
        }
        drop(open_temp_session(&temp).unwrap());
        let committed = commit_session_to_main(&conn, &temp, &flight("Legacy", None)).unwrap();
        let track = get_flight_track(&conn, committed).unwrap();
        assert_eq!(track.len(), 1);
        assert!(track[0].wall_ms.is_none());
        remove_temp_session(&temp);
        cleanup(&temp);
    }

    // ── Schema v20: flight → library vehicle (`flights.vehicle_id`) ─────────────────────────

    /// INAV SITL's MSP_UID as the MSP handshake renders it (see `msp::hardware_uid`).
    const SITL_UID: &str = "000000000100000002000000";

    /// A library vehicle with an explicit `created_at` (the "newest wins" order), by raw SQL so it
    /// works on a v19 database too. Returns its id.
    fn vehicle(conn: &Connection, name: &str, craft: Option<&str>, uid: Option<&str>, created_at: &str) -> i64 {
        conn.execute(
            "INSERT INTO vehicles (name, craft_name, fc_uid, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, craft, uid, created_at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn flight_with(craft: &str, uid: Option<&str>) -> Flight {
        Flight { fc_uid: uid.map(String::from), ..flight(craft, None) }
    }

    fn vehicle_of(conn: &Connection, flight_id: i64) -> Option<i64> {
        get_flight(conn, flight_id).unwrap().unwrap().vehicle_id
    }

    /// The v19 → v20 step adds `flights.vehicle_id` + its index and links the existing flights the way
    /// the craft-name display did, with the FC id first: UID beats craft name, the newest vehicle wins,
    /// synthetic ids never match, unmatched flights stay NULL. The back-fill only runs when the column is
    /// born — a later open never re-links a flight the user un-linked — and is itself re-entrant.
    #[test]
    fn v20_backfills_flight_vehicle_ids() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate_to(&conn, 19);
        assert!(!column_exists(&conn, "flights", "vehicle_id").unwrap());

        let by_uid = vehicle(&conn, "By UID", Some("Other"), Some(" uida "), "2026-01-01 10:00:00");
        let wing_old = vehicle(&conn, "Wing old", Some("Wing"), None, "2026-01-01 10:00:00");
        let wing_new = vehicle(&conn, "Wing new", Some(" wing"), None, "2026-01-03 10:00:00");
        // Created later by id, but older by created_at — must lose against `wing_new`.
        let _wing_stale = vehicle(&conn, "Wing stale", Some("WING"), None, "2026-01-02 10:00:00");
        let sitl = vehicle(&conn, "SITL", None, Some(SITL_UID), "2026-01-05 10:00:00");

        let raw = |craft: &str, uid: Option<&str>| {
            conn.execute(
                "INSERT INTO flights (start_time, craft_name, fc_uid) VALUES ('2026-09-01T10:00:00+00:00', ?1, ?2)",
                params![craft, uid],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let uid_and_craft = raw("Wing", Some("UIDA"));
        let craft_only = raw(" wing ", None);
        let unmatched = raw("Nobody", None);
        let synthetic = raw("", Some(SITL_UID));
        let unknown_uid = raw("Wing", Some("UIDX"));

        migrate(&conn).unwrap();
        assert!(column_exists(&conn, "flights", "vehicle_id").unwrap());
        assert!(object_exists(&conn, "index", "idx_flights_vehicle"));
        let fk: (String, String) = conn
            .query_row(
                "SELECT \"table\", on_delete FROM pragma_foreign_key_list('flights') WHERE \"from\" = 'vehicle_id'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(fk, ("vehicles".to_string(), "SET NULL".to_string()));
        assert_eq!(vehicle_of(&conn, uid_and_craft), Some(by_uid), "the FC id beats the craft name");
        assert_eq!(vehicle_of(&conn, craft_only), Some(wing_new), "the newest vehicle with the craft name");
        assert_eq!(vehicle_of(&conn, unmatched), None);
        assert_eq!(vehicle_of(&conn, synthetic), None, "a synthetic id matches no vehicle (not even {sitl})");
        assert_eq!(vehicle_of(&conn, unknown_uid), Some(wing_new), "unknown FC id → craft-name fallback");
        assert_ne!(wing_old, wing_new);
        // The flights keep what they were flown with.
        let f = get_flight(&conn, uid_and_craft).unwrap().unwrap();
        assert_eq!((f.craft_name.as_str(), f.fc_uid.as_deref()), ("Wing", Some("UIDA")));

        // Re-entrant: a second back-fill changes nothing.
        assert_eq!(backfill_flight_vehicle_ids(&conn).unwrap(), 0);
        // A flight the user un-linked stays un-linked across a reopen.
        set_flight_vehicle(&conn, craft_only, None).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(vehicle_of(&conn, craft_only), None);
        let tx = conn.unchecked_transaction().unwrap();
        migrate_v19_to_v20(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(vehicle_of(&conn, craft_only), None);
    }

    /// A database already stamped v20 without the column (a dev build of the branch before
    /// `flights.vehicle_id`) gets column, index and back-fill from the self-heal pass.
    #[test]
    fn v20_stamped_database_without_the_column_self_heals() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate_to(&conn, 19);
        set_user_version(&conn, 20).unwrap();
        let wing = vehicle(&conn, "Wing", Some("Wing"), None, "2026-01-01 10:00:00");
        conn.execute(
            "INSERT INTO flights (start_time, craft_name) VALUES ('2026-09-01T10:00:00+00:00', 'Wing')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        assert!(!column_exists(&conn, "flights", "vehicle_id").unwrap());

        migrate(&conn).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        assert!(column_exists(&conn, "flights", "vehicle_id").unwrap());
        assert!(object_exists(&conn, "index", "idx_flights_vehicle"));
        assert_eq!(vehicle_of(&conn, id), Some(wing));
    }

    /// Deleting a vehicle un-links its flights — they stay, with their craft name and FC id.
    #[test]
    fn deleting_a_vehicle_unlinks_its_flights() {
        let conn = test_db();
        let v = vehicle(&conn, "Wing", Some("Wing"), Some("UID1"), "2026-01-01 10:00:00");
        let keep = vehicle(&conn, "Quad", Some("Quad"), None, "2026-01-01 10:00:00");
        let a = insert_flight(&conn, &flight_with("Wing", Some("UID1"))).unwrap();
        let b = insert_flight(&conn, &flight_with("Quad", None)).unwrap();
        assert_eq!(vehicle_of(&conn, a), Some(v));

        delete_vehicle(&conn, v).unwrap();
        let f = get_flight(&conn, a).unwrap().expect("the flight survives its vehicle");
        assert_eq!((f.vehicle_id, f.craft_name.as_str(), f.fc_uid.as_deref()), (None, "Wing", Some("UID1")));
        assert_eq!(vehicle_of(&conn, b), Some(keep));
        assert!(list_flights_for_vehicle(&conn, v).unwrap().is_empty());
        // The FK alone (ON DELETE SET NULL) un-links as well.
        conn.execute("DELETE FROM vehicles WHERE id = ?1", params![keep]).unwrap();
        assert_eq!(vehicle_of(&conn, b), None);
        assert!(get_flight(&conn, b).unwrap().is_some());
    }

    #[test]
    fn transfer_moves_every_flight_of_a_vehicle() {
        let conn = test_db();
        let old = vehicle(&conn, "Old", Some("Wing"), None, "2026-01-01 10:00:00");
        let new = vehicle(&conn, "New", None, None, "2026-01-02 10:00:00");
        let other = vehicle(&conn, "Other", Some("Quad"), None, "2026-01-01 10:00:00");
        let a = insert_flight(&conn, &flight("Wing", None)).unwrap();
        let b = insert_flight(&conn, &flight("Wing", None)).unwrap();
        let c = insert_flight(&conn, &flight("Quad", None)).unwrap();

        assert_eq!(transfer_vehicle_flights(&conn, old, old).unwrap(), 0);
        assert_eq!(transfer_vehicle_flights(&conn, old, new).unwrap(), 2);
        assert_eq!((vehicle_of(&conn, a), vehicle_of(&conn, b), vehicle_of(&conn, c)), (Some(new), Some(new), Some(other)));
        assert_eq!(get_flight(&conn, a).unwrap().unwrap().craft_name, "Wing");
        assert_eq!(vehicle_aggregate(&conn, new).unwrap().flight_count, 2);
        assert_eq!(vehicle_aggregate(&conn, old).unwrap().flight_count, 0);
        assert!(transfer_vehicle_flights(&conn, new, 9_999).is_err(), "unknown target (FK)");
        assert_eq!(vehicle_of(&conn, a), Some(new));
    }

    /// Link-by-identity: craft name (trimmed, case-insensitive) and/or FC id; `only_unlinked` leaves
    /// other vehicles' flights alone; the dry-run count matches what the link changes.
    #[test]
    fn link_flights_by_craft_name_and_uid() {
        let conn = test_db();
        // Flights stored before the target vehicle existed → unlinked.
        let a = insert_flight(&conn, &flight("Wing", None)).unwrap();
        let b = insert_flight(&conn, &flight("WING ", None)).unwrap();
        let other = vehicle(&conn, "Other", None, None, "2026-01-01 10:00:00");
        let c = insert_flight(&conn, &Flight { vehicle_id: Some(other), ..flight("Wing", None) }).unwrap();
        let d = insert_flight(&conn, &flight("Quad", None)).unwrap();
        let e = insert_flight(&conn, &flight_with("", Some("UIDZ"))).unwrap();
        let s = insert_flight(&conn, &flight_with("", Some(SITL_UID))).unwrap();
        let target = vehicle(&conn, "Target", None, None, "2026-01-02 10:00:00");

        assert_eq!(count_linkable_flights(&conn, target, Some("wing"), None, true).unwrap(), 2);
        assert_eq!(link_flights_to_vehicle(&conn, target, Some("wing"), None, true).unwrap(), 2);
        assert_eq!((vehicle_of(&conn, a), vehicle_of(&conn, b), vehicle_of(&conn, c)), (Some(target), Some(target), Some(other)));
        // Without only_unlinked the other vehicle's flight moves too; the already linked ones don't count.
        assert_eq!(count_linkable_flights(&conn, target, Some("Wing"), None, false).unwrap(), 1);
        assert_eq!(link_flights_to_vehicle(&conn, target, Some("Wing"), None, false).unwrap(), 1);
        assert_eq!(vehicle_of(&conn, c), Some(target));
        assert_eq!(vehicle_of(&conn, d), None);
        // By FC id; nothing for no key, an empty key or a synthetic id.
        assert_eq!(link_flights_to_vehicle(&conn, target, None, Some(" uidz "), true).unwrap(), 1);
        assert_eq!(vehicle_of(&conn, e), Some(target));
        assert_eq!(count_linkable_flights(&conn, target, None, None, false).unwrap(), 0);
        assert_eq!(link_flights_to_vehicle(&conn, target, Some("  "), Some(""), false).unwrap(), 0);
        assert_eq!(link_flights_to_vehicle(&conn, target, None, Some(SITL_UID), false).unwrap(), 0);
        assert_eq!(vehicle_of(&conn, s), None);
        // The flights keep their craft name.
        assert_eq!(get_flight(&conn, b).unwrap().unwrap().craft_name, "WING ");
        assert_eq!(list_flights_for_vehicle(&conn, target).unwrap().len(), 4);
    }

    #[test]
    fn resolve_reports_how_it_matched_and_uid_conflicts() {
        let conn = test_db();
        let by_uid = vehicle(&conn, "U", Some("Wing"), Some("UID1"), "2026-01-01 10:00:00");
        let quad = vehicle(&conn, "Q", Some("Quad"), Some("UID9"), "2026-01-01 10:00:00");
        let plane = vehicle(&conn, "P", Some("Plane"), Some("  "), "2026-01-01 10:00:00");

        let r = resolve_vehicle(&conn, Some("uid1"), "Quad").unwrap();
        assert_eq!((r.vehicle.map(|v| v.id), r.matched_by.as_deref(), r.uid_conflict), (Some(by_uid), Some("uid"), false));
        // Craft-name match, the vehicle stores another FC id → FC swapped or a second aircraft.
        let r = resolve_vehicle(&conn, Some("UID2"), "quad").unwrap();
        assert_eq!((r.vehicle.map(|v| v.id), r.matched_by.as_deref(), r.uid_conflict), (Some(quad), Some("craft"), true));
        // No hardware id on the flight (passive, 1.0 data, a simulator) → no conflict to ask about.
        let r = resolve_vehicle(&conn, None, "Quad").unwrap();
        assert_eq!((r.vehicle.map(|v| v.id), r.uid_conflict), (Some(quad), false));
        let r = resolve_vehicle(&conn, Some(SITL_UID), "Quad").unwrap();
        assert_eq!((r.matched_by.as_deref(), r.uid_conflict), (Some("craft"), false));
        // The vehicle stores no (only a blank) FC id → nothing to replace.
        let r = resolve_vehicle(&conn, Some("UID2"), "Plane").unwrap();
        assert_eq!((r.vehicle.map(|v| v.id), r.uid_conflict), (Some(plane), false));
        let r = resolve_vehicle(&conn, Some("UID2"), "Nothing").unwrap();
        assert_eq!((r.vehicle.is_none(), r.matched_by, r.uid_conflict), (true, None, false));
        assert_eq!(resolve_vehicle_id(&conn, Some("UID2"), "Nothing").unwrap(), None);
    }

    /// When unsure, unlinked: a flight with a hardware FC id does not link by craft name to a vehicle that
    /// stores a different one — on insert, in the back-fill and on `.kflight` import (see exchange.rs) —
    /// while the post-flight summary still reports that candidate with `uid_conflict`. Without an FC id
    /// on the flight or on the vehicle the craft-name link still works.
    #[test]
    fn craft_name_fallback_never_crosses_a_different_fc_id() {
        let conn = test_db();
        let other_fc = vehicle(&conn, "Wing A", Some("Wing"), Some("UIDA"), "2026-01-01 10:00:00");
        let quad = vehicle(&conn, "Quad", Some("Quad"), None, "2026-01-01 10:00:00");

        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Wing", Some("UIDB"))).unwrap()), None);
        let r = resolve_vehicle(&conn, Some("UIDB"), "Wing").unwrap();
        assert_eq!((r.vehicle.map(|v| v.id), r.uid_conflict), (Some(other_fc), true));
        // No FC id on the flight / on the vehicle → the craft name links.
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Wing", None)).unwrap()), Some(other_fc));
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Quad", Some("UIDQ"))).unwrap()), Some(quad));
        // A synthetic id on the flight is no hardware id → links by craft name.
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Wing", Some(SITL_UID))).unwrap()), Some(other_fc));
        // A synthetic id stored on the vehicle is no hardware id either → no conflict.
        let sim = vehicle(&conn, "Sim", Some("Sim"), Some(SITL_UID), "2026-01-01 10:00:00");
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Sim", Some("UIDS"))).unwrap()), Some(sim));
    }

    /// The v20 back-fill follows the same rule: a conflicting craft-name vehicle leaves the flight NULL.
    #[test]
    fn v20_backfill_leaves_fc_id_conflicts_unlinked() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate_to(&conn, 19);
        let wing = vehicle(&conn, "Wing", Some("Wing"), Some("UIDA"), "2026-01-01 10:00:00");
        let raw = |uid: Option<&str>| {
            conn.execute(
                "INSERT INTO flights (start_time, craft_name, fc_uid) VALUES ('2026-09-01T10:00:00+00:00', 'Wing', ?1)",
                params![uid],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let conflicting = raw(Some("UIDB"));
        let no_uid = raw(None);
        migrate(&conn).unwrap();
        assert_eq!(vehicle_of(&conn, conflicting), None);
        assert_eq!(vehicle_of(&conn, no_uid), Some(wing));
    }

    /// Setting a vehicle's FC id takes it off every other vehicle: afterwards exactly one carries it.
    #[test]
    fn set_vehicle_fc_uid_moves_the_id() {
        let conn = test_db();
        let a = vehicle(&conn, "A", None, None, "2026-01-01 10:00:00");
        let b = vehicle(&conn, "B", None, Some(" uid1 "), "2026-01-01 10:00:00");
        let c = vehicle(&conn, "C", None, Some("UID1"), "2026-01-01 10:00:00");
        let d = vehicle(&conn, "D", None, Some("UID2"), "2026-01-01 10:00:00");
        let b_flight = insert_flight(&conn, &Flight { vehicle_id: Some(b), ..flight_with("", Some("UID1")) }).unwrap();
        assert_eq!(set_vehicle_fc_uid(&conn, a, Some("UID1")).unwrap(), 2);
        // The ID moves, the flights do not: B's flight stays B's.
        assert_eq!(vehicle_of(&conn, b_flight), Some(b));
        let carriers: i64 = conn
            .query_row("SELECT COUNT(*) FROM vehicles WHERE TRIM(fc_uid) = 'UID1' COLLATE NOCASE", [], |r| r.get(0))
            .unwrap();
        assert_eq!(carriers, 1);
        assert_eq!(get_vehicle(&conn, a).unwrap().unwrap().fc_uid.as_deref(), Some("UID1"));
        assert_eq!(get_vehicle(&conn, b).unwrap().unwrap().fc_uid, None);
        assert_eq!(get_vehicle(&conn, c).unwrap().unwrap().fc_uid, None);
        assert_eq!(get_vehicle(&conn, d).unwrap().unwrap().fc_uid.as_deref(), Some("UID2"));
        // An unknown vehicle fails and rolls the removal back: D keeps its id.
        assert!(set_vehicle_fc_uid(&conn, 9_999, Some("UID2")).is_err());
        assert_eq!(get_vehicle(&conn, d).unwrap().unwrap().fc_uid.as_deref(), Some("UID2"));
        // Clearing touches nobody else.
        assert_eq!(set_vehicle_fc_uid(&conn, a, Some("  ")).unwrap(), 0);
        assert_eq!(get_vehicle(&conn, a).unwrap().unwrap().fc_uid, None);
        assert_eq!(get_vehicle(&conn, d).unwrap().unwrap().fc_uid.as_deref(), Some("UID2"));
    }

    /// Two aircraft with the same craft name and different FC ids, all flights unlinked: the link offer
    /// for UID1's vehicle (craft name + UID1) takes UID1's flights and the id-less ones (none, empty,
    /// synthetic), never UID2's.
    #[test]
    fn link_by_craft_name_never_crosses_a_different_fc_id() {
        let conn = test_db();
        let ids: Vec<i64> = [Some("UID1"), Some("uid1 "), Some("UID2"), None, Some(""), Some(SITL_UID), Some("UID2")]
            .iter()
            .map(|u| insert_flight(&conn, &flight_with("Wing", *u)).unwrap())
            .collect();
        let v1 = vehicle(&conn, "Wing 1", Some("Wing"), Some("UID1"), "2026-01-01 10:00:00");
        assert_eq!(count_linkable_flights(&conn, v1, Some("Wing"), Some("UID1"), true).unwrap(), 5);
        assert_eq!(link_flights_to_vehicle(&conn, v1, Some("Wing"), Some("UID1"), true).unwrap(), 5);
        let linked: Vec<Option<i64>> = ids.iter().map(|id| vehicle_of(&conn, *id)).collect();
        assert_eq!(linked, [Some(v1), Some(v1), None, Some(v1), Some(v1), Some(v1), None]);
        // Without an FC id key the craft name alone links the rest (the vehicle has no id to protect).
        let v2 = vehicle(&conn, "Wing 2", Some("Wing"), None, "2026-01-02 10:00:00");
        assert_eq!(link_flights_to_vehicle(&conn, v2, Some("Wing"), None, true).unwrap(), 2);
    }

    /// Every insert resolves the vehicle unless the caller names one: the plain insert (importers,
    /// `.kflight` import) and the live / recovery commit; the unresolved variant writes what it gets.
    #[test]
    fn inserts_resolve_the_vehicle() {
        let conn = test_db();
        let wing = vehicle(&conn, "Wing", Some("Wing"), None, "2026-01-01 10:00:00");
        let by_uid = vehicle(&conn, "U", Some("Other"), Some("UID1"), "2026-01-01 10:00:00");

        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight("wing", None)).unwrap()), Some(wing));
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight_with("Wing", Some("UID1"))).unwrap()), Some(by_uid));
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &flight("Nobody", None)).unwrap()), None);
        let explicit = Flight { vehicle_id: Some(by_uid), ..flight("Wing", None) };
        assert_eq!(vehicle_of(&conn, insert_flight(&conn, &explicit).unwrap()), Some(by_uid));
        assert_eq!(vehicle_of(&conn, insert_flight_unresolved(&conn, &flight("Wing", None)).unwrap()), None);

        let temp = temp_db_path("vehicle-commit").with_file_name("active_test_L1-S1.ktmp");
        remove_temp_session(&temp);
        {
            let t = open_temp_session(&temp).unwrap();
            insert_telemetry_batch(&t, &[telemetry(0, 0, None)]).unwrap();
        }
        let committed = commit_session_to_main(&conn, &temp, &flight_with("", Some("uid1"))).unwrap();
        assert_eq!(vehicle_of(&conn, committed), Some(by_uid));
        remove_temp_session(&temp);
        cleanup(&temp);
    }

    #[test]
    fn set_flight_vehicle_links_and_unlinks() {
        let conn = test_db();
        let v = vehicle(&conn, "V", None, None, "2026-01-01 10:00:00");
        let id = insert_flight(&conn, &flight("Wing", None)).unwrap();
        assert!(set_flight_vehicle(&conn, id, Some(v)).unwrap());
        assert_eq!(vehicle_of(&conn, id), Some(v));
        assert!(set_flight_vehicle(&conn, id, None).unwrap());
        assert_eq!(vehicle_of(&conn, id), None);
        assert!(set_flight_vehicle(&conn, id, Some(9_999)).is_err(), "unknown vehicle (FK)");
        assert!(!set_flight_vehicle(&conn, 9_999, Some(v)).unwrap(), "unknown flight");
    }

    /// The logbook's craft-name edit corrects the name only — the vehicle link stays.
    #[test]
    fn craft_name_edit_keeps_the_vehicle_link() {
        let conn = test_db();
        let wing = vehicle(&conn, "Wing", Some("Wing"), None, "2026-01-01 10:00:00");
        vehicle(&conn, "Quad", Some("Quad"), None, "2026-01-01 10:00:00");
        let id = insert_flight(&conn, &flight("Wing", None)).unwrap();
        assert_eq!(vehicle_of(&conn, id), Some(wing));
        update_flight_craft_name(&conn, id, "quad").unwrap();
        assert_eq!(vehicle_of(&conn, id), Some(wing));
        assert_eq!(get_flight(&conn, id).unwrap().unwrap().craft_name, "quad");
    }

    // ── Schema v21: no decoded Blackbox rows, `flights.source_before_link` ───────────────────

    fn source_of(conn: &Connection, flight_id: i64) -> (String, Option<String>, Option<i64>) {
        conn.query_row(
            "SELECT source, source_before_link, linked_flight_id FROM flights WHERE id = ?1",
            params![flight_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn linked(source_before_link: &str, partner: i64) -> (String, Option<String>, Option<i64>) {
        ("both".into(), Some(source_before_link.into()), Some(partner))
    }

    fn unlinked(source: &str) -> (String, Option<String>, Option<i64>) {
        (source.into(), None, None)
    }

    /// A v20 database (built from the real steps) with linked pairs and stored decoded rows: the v21
    /// step derives `source_before_link` for the linked flights from the decoded rows OR an archived
    /// log, leaves unlinked flights NULL, drops the table + index, and a second pass is a no-op.
    #[test]
    fn v20_database_migrates_to_v21() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate_to(&conn, 20);
        assert!(table_exists(&conn, "blackbox_records").unwrap());
        let raw = |source: &str| {
            conn.execute(
                "INSERT INTO flights (start_time, craft_name, source) VALUES ('2026-09-01T10:00:00+00:00', 'W', ?1)",
                params![source],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let pair = |a: i64, b: i64| {
            conn.execute("UPDATE flights SET linked_flight_id = ?1 WHERE id = ?2", params![b, a]).unwrap();
            conn.execute("UPDATE flights SET linked_flight_id = ?1 WHERE id = ?2", params![a, b]).unwrap();
        };
        let records = |id: i64| {
            conn.execute(
                "INSERT INTO blackbox_records (flight_id, timestamp_us, csv_data) VALUES (?1, 0, '1,2,3')",
                params![id],
            )
            .unwrap();
        };
        // Pair 1: the log flight has decoded rows but no archived original any more.
        let live_1 = raw("both");
        let log_1 = raw("both");
        pair(live_1, log_1);
        records(log_1);
        // Pair 2: the log flight has only an archived original (an ArduPilot / PX4 import).
        let live_2 = raw("both");
        let log_2 = raw("both");
        pair(live_2, log_2);
        insert_blackbox_file(&conn, log_2, "00000042.BIN", 0, b"bin").unwrap();
        // Pair 3: an ArduPilot import with neither decoded rows nor an archived original — only its
        // protocol says it is a log flight.
        let live_3 = raw("both");
        let log_3 = raw("both");
        conn.execute("UPDATE flights SET protocol = 'DATAFLASH' WHERE id = ?1", params![log_3]).unwrap();
        pair(live_3, log_3);
        // Unlinked flights stay NULL, whatever they hold.
        let lone_log = raw("blackbox");
        records(lone_log);
        insert_blackbox_file(&conn, lone_log, "LOG00001.TXT", 0, b"txt").unwrap();
        let lone_live = raw("live");

        migrate(&conn).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), 21);
        assert!(!table_exists(&conn, "blackbox_records").unwrap());
        assert!(!object_exists(&conn, "index", "idx_blackbox_records_flight"));
        assert_eq!(source_of(&conn, live_1), linked("live", log_1));
        assert_eq!(source_of(&conn, log_1), linked("blackbox", live_1));
        assert_eq!(source_of(&conn, live_2), linked("live", log_2));
        assert_eq!(source_of(&conn, log_2), linked("blackbox", live_2));
        assert_eq!(source_of(&conn, live_3), linked("live", log_3));
        assert_eq!(source_of(&conn, log_3), linked("blackbox", live_3));
        assert_eq!(source_of(&conn, lone_log), unlinked("blackbox"));
        assert_eq!(source_of(&conn, lone_live), unlinked("live"));
        // The archived originals stay.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM blackbox_files WHERE flight_id = ?1", log_2), 1);

        // Re-entrant: the self-heal pass and the step itself change nothing.
        migrate(&conn).unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        migrate_v20_to_v21(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), 21);
        assert_eq!(source_of(&conn, log_1), linked("blackbox", live_1));
        assert_eq!(source_of(&conn, lone_log), unlinked("blackbox"));
        // The unlink restores what the back-fill derived — also for the log flight without a file.
        unlink_flight(&conn, live_1).unwrap();
        assert_eq!(source_of(&conn, live_1), unlinked("live"));
        assert_eq!(source_of(&conn, log_1), unlinked("blackbox"));
    }

    /// The back-fill also runs on a database whose `blackbox_records` table is already gone.
    #[test]
    fn v21_backfill_without_the_records_table() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_to(&conn, 20);
        conn.execute_batch("DROP TABLE blackbox_records;").unwrap();
        conn.execute_batch(
            "INSERT INTO flights (id, start_time, source, linked_flight_id) VALUES (1, 't', 'both', 2), (2, 't', 'both', 1);",
        )
        .unwrap();
        insert_blackbox_file(&conn, 2, "LOG00001.TXT", 0, b"txt").unwrap();
        migrate(&conn).unwrap();
        assert_eq!(source_of(&conn, 1), linked("live", 2));
        assert_eq!(source_of(&conn, 2), linked("blackbox", 1));
    }

    /// Link and unlink round-trip the sources of both flights — also for a log flight whose archived
    /// original is gone (deleted by the user, or never stored), where the old "has decoded rows" check
    /// and the fallback "has a file" check would both say `live`. Deleting one flight of a pair
    /// restores the partner.
    #[test]
    fn link_unlink_and_delete_restore_the_sources() {
        let conn = test_db();
        let new = |source: &str| insert_flight(&conn, &Flight { source: source.into(), ..flight("Wing", None) }).unwrap();

        let live = new("live");
        let log = new("blackbox");
        insert_blackbox_file(&conn, log, "LOG00001.TXT", 0, b"txt").unwrap();
        link_flights(&conn, live, log).unwrap();
        assert_eq!(source_of(&conn, live), linked("live", log));
        assert_eq!(source_of(&conn, log), linked("blackbox", live));
        // Linking again keeps the stored sources (the flights already read "both").
        link_flights(&conn, log, live).unwrap();
        assert_eq!(source_of(&conn, log), linked("blackbox", live));
        unlink_flight(&conn, log).unwrap();
        assert_eq!(source_of(&conn, live), unlinked("live"));
        assert_eq!(source_of(&conn, log), unlinked("blackbox"));

        // The original is deleted while the pair is linked (it may sit on either flight).
        link_flights(&conn, live, log).unwrap();
        assert_eq!(delete_blackbox_file(&conn, live).unwrap().as_deref(), Some("LOG00001.TXT"));
        unlink_flight(&conn, live).unwrap();
        assert_eq!(source_of(&conn, log), unlinked("blackbox"));
        assert_eq!(source_of(&conn, live), unlinked("live"));

        // Delete one flight of a pair → the partner gets its own source back.
        let live_2 = new("live");
        let log_2 = new("blackbox");
        link_flights(&conn, live_2, log_2).unwrap();
        assert!(delete_flight(&conn, live_2).unwrap());
        assert_eq!(source_of(&conn, log_2), unlinked("blackbox"));
        link_flights(&conn, live, log_2).unwrap();
        assert!(delete_flight(&conn, log_2).unwrap());
        assert_eq!(source_of(&conn, live), unlinked("live"));

        // Without a stored value the fallback decides: an archived log, or a log importer's protocol
        // (an ArduPilot flight with no file at all) mean "blackbox".
        let live_3 = new("live");
        let log_3 = new("blackbox");
        insert_blackbox_file(&conn, log_3, "LOG00002.TXT", 0, b"txt").unwrap();
        let live_4 = new("live");
        let log_4 = insert_flight(
            &conn,
            &Flight { source: "blackbox".into(), protocol: "DATAFLASH".into(), ..flight("Wing", None) },
        )
        .unwrap();
        link_flights(&conn, live_3, log_3).unwrap();
        link_flights(&conn, live_4, log_4).unwrap();
        conn.execute("UPDATE flights SET source_before_link = NULL", []).unwrap();
        unlink_flight(&conn, live_3).unwrap();
        assert_eq!(source_of(&conn, live_3), unlinked("live"));
        assert_eq!(source_of(&conn, log_3), unlinked("blackbox"));
        assert!(delete_flight(&conn, live_4).unwrap());
        assert_eq!(source_of(&conn, log_4), unlinked("blackbox"));
    }

    /// `delete_flight` on a file database hands the freed pages back (in the background): the freelist
    /// reaches 0. The old one-step `execute_batch("PRAGMA incremental_vacuum;")` freed one page only.
    #[test]
    fn delete_flight_reclaims_the_freed_pages() {
        let path = temp_db_path("delete-reclaim");
        cleanup(&path);
        let conn = open_database(&path).unwrap();
        let id = insert_flight(&conn, &Flight { source: "blackbox".into(), ..flight("Wing", None) }).unwrap();
        insert_blackbox_file(&conn, id, "LOG00001.TXT", 0, &vec![0x5a; 4 * 1024 * 1024]).unwrap();
        let file = conn.path().unwrap().to_string();
        assert!(delete_flight(&conn, id).unwrap());
        wait_freelist_empty(&conn, "the delete");
        wait_for_reclaim(&file, std::time::Duration::from_secs(5));
        drop(conn);
        cleanup(&path);
    }

    /// Polls (bounded, 5 s) until the freelist is empty; panics naming `after` otherwise.
    fn wait_freelist_empty(conn: &Connection, after: &str) {
        let started = std::time::Instant::now();
        while freelist_pages(conn).unwrap() > 0 {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "freelist still {} pages after {after}",
                freelist_pages(conn).unwrap()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Same for deleting only the archived original (`delete_blackbox_file`).
    #[test]
    fn delete_blackbox_file_reclaims_the_freed_pages() {
        let path = temp_db_path("delete-file-reclaim");
        cleanup(&path);
        let conn = open_database(&path).unwrap();
        let id = insert_flight(&conn, &Flight { source: "blackbox".into(), ..flight("Wing", None) }).unwrap();
        insert_blackbox_file(&conn, id, "LOG00001.TXT", 0, &vec![0x5a; 4 * 1024 * 1024]).unwrap();
        let file = conn.path().unwrap().to_string();
        assert_eq!(delete_blackbox_file(&conn, id).unwrap().as_deref(), Some("LOG00001.TXT"));
        wait_freelist_empty(&conn, "deleting the archived original");
        wait_for_reclaim(&file, std::time::Duration::from_secs(5));
        drop(conn);
        cleanup(&path);
    }

    /// The scratch store ("open a log without importing it") is never reclaimed — neither after a
    /// delete nor on open with a large freelist: it is thrown away whole, and on Windows its wipe
    /// fails while a reclaim thread holds the file.
    #[test]
    fn scratch_store_is_not_reclaimed() {
        let path = temp_db_path("scratch-no-reclaim");
        cleanup(&path);
        exclude_from_reclaim(path.parent().unwrap());
        let conn = open_database(&path).unwrap();
        let file = conn.path().unwrap().to_string();
        let id = insert_flight(&conn, &Flight { source: "blackbox".into(), ..flight("Wing", None) }).unwrap();
        let blob = vec![0x5a; ((RECLAIM_ON_OPEN_PAGES + 512) * 4096) as usize];
        insert_blackbox_file(&conn, id, "LOG00001.TXT", 0, &blob).unwrap();
        assert!(delete_flight(&conn, id).unwrap());
        assert!(!reclaim_running(&file), "a delete in the scratch store must not start a reclaim");
        drop(conn);
        let conn = open_database(&path).unwrap();
        assert!(!reclaim_running(&file), "opening the scratch store must not start a reclaim");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let free = freelist_pages(&conn).unwrap();
        assert!(free > RECLAIM_ON_OPEN_PAGES, "the scratch freelist was touched: {free} pages left");
        drop(conn);
        cleanup(&path);
    }

    /// `write_tx` takes the write lock up front: while a second connection holds it, the transaction
    /// waits (busy timeout) and then reads and writes normally.
    #[test]
    fn write_tx_waits_for_the_write_lock() {
        let path = temp_db_path("write-tx-waits");
        cleanup(&path);
        let a = open_database(&path).unwrap();
        let b = Connection::open(&path).unwrap();
        b.execute_batch("BEGIN IMMEDIATE; INSERT INTO flights (start_time) VALUES ('2026-11-11T10:00:00Z');")
            .unwrap();
        let hold = std::time::Duration::from_millis(300);
        let started = std::time::Instant::now();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(hold);
            b.execute_batch("COMMIT;").unwrap();
        });
        let tx = write_tx(&a).unwrap();
        let seen: i64 = tx.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        tx.execute("INSERT INTO flights (start_time) VALUES ('2026-11-11T10:05:00Z')", []).unwrap();
        tx.commit().unwrap();
        assert!(started.elapsed() >= hold - std::time::Duration::from_millis(50), "it must have waited");
        assert_eq!(seen, 1, "the read after the wait sees the other connection's commit");
        holder.join().unwrap();
        drop(a);
        cleanup(&path);
    }

    fn is_busy(e: &rusqlite::Error) -> bool {
        matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::DatabaseBusy)
    }

    /// Why `write_tx` exists: a DEFERRED transaction that reads first cannot become a writer once
    /// another connection has committed since its read, or while another connection holds the write
    /// lock — SQLite answers SQLITE_BUSY at once, without the busy timeout. This is what made a flight
    /// save fail while the background reclaim was running.
    #[test]
    fn deferred_read_then_write_fails_without_waiting() {
        let path = temp_db_path("deferred-busy");
        cleanup(&path);
        let a = open_database(&path).unwrap();
        let b = Connection::open(&path).unwrap();
        let insert = "INSERT INTO flights (start_time) VALUES ('2026-11-11T10:00:00Z')";

        // 1. Another connection committed after the read: the snapshot is stale.
        let tx = a.unchecked_transaction().unwrap();
        let _: i64 = tx.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        b.execute(insert, []).unwrap();
        let err = tx.execute(insert, []).unwrap_err();
        assert!(is_busy(&err), "{err:?}");
        drop(tx);

        // 2. Another connection holds the write lock: no waiting either (busy timeout is 5 s).
        let tx = a.unchecked_transaction().unwrap();
        let _: i64 = tx.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        b.execute_batch(&format!("BEGIN IMMEDIATE; {insert};")).unwrap();
        let started = std::time::Instant::now();
        let err = tx.execute(insert, []).unwrap_err();
        assert!(is_busy(&err), "{err:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "it failed without waiting");
        drop(tx);
        b.execute_batch("COMMIT;").unwrap();

        // The same with `write_tx` works (nothing holds the lock now).
        let tx = write_tx(&a).unwrap();
        let _: i64 = tx.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        tx.execute(insert, []).unwrap();
        tx.commit().unwrap();
        drop((a, b));
        cleanup(&path);
    }

    /// The save path under load: flights are committed from temp sessions over and over while a real
    /// chunked reclaim of a large freelist runs on another connection — every commit must succeed.
    #[test]
    fn session_commits_succeed_while_a_reclaim_runs() {
        let path = db_with_freelist("commit-during-reclaim", 16_384);
        let conn = open_database(&path).unwrap(); // starts the background reclaim (freelist > threshold)
        let file = conn.path().unwrap().to_string();
        assert!(reclaim_running(&file), "the reclaim must be running for this test to mean anything");
        let mut during = 0;
        for n in 0..400 {
            let temp = path.with_file_name(format!("active_test_L1-S{n}.ktmp"));
            remove_temp_session(&temp);
            {
                let t = open_temp_session(&temp).unwrap();
                insert_telemetry_batch(&t, &[telemetry(0, 0, None), telemetry(0, 100, None)]).unwrap();
            }
            let running = reclaim_running(&file);
            commit_session_to_main(&conn, &temp, &flight("Wing", None))
                .unwrap_or_else(|e| panic!("commit {n} failed while the reclaim ran: {e}"));
            remove_temp_session(&temp);
            if running && reclaim_running(&file) {
                during += 1;
            } else if !reclaim_running(&file) {
                break;
            }
        }
        assert!(during >= 3, "only {during} commits overlapped the reclaim");
        wait_for_reclaim(&file, std::time::Duration::from_secs(30));
        assert_eq!(freelist_pages(&conn).unwrap(), 0);
        drop(conn);
        cleanup(&path);
    }

    /// A file with `pages` pages on its freelist and nothing else changed (a dropped table).
    fn db_with_freelist(name: &str, pages: i64) -> PathBuf {
        let path = temp_db_path(name);
        cleanup(&path);
        let conn = open_database(&path).unwrap();
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        conn.execute_batch("CREATE TABLE junk (b BLOB);").unwrap();
        conn.execute("INSERT INTO junk (b) VALUES (zeroblob(?1))", params![pages * page_size]).unwrap();
        conn.execute_batch("DROP TABLE junk;").unwrap();
        let free = freelist_pages(&conn).unwrap();
        // A little more than `pages`: the table's own pages and the auto-vacuum pointer-map pages.
        assert!((pages..pages + 16 + pages / 256).contains(&free), "expected ~{pages} free pages, got {free}");
        path
    }

    /// Opening a database whose freelist is above `RECLAIM_ON_OPEN_PAGES` reclaims it — no delete
    /// needed (an interrupted reclaim, a dropped table); one below the threshold is left alone.
    #[test]
    fn open_reclaims_a_large_freelist_only() {
        let big = db_with_freelist("open-reclaim-big", RECLAIM_ON_OPEN_PAGES + 512);
        let conn = open_database(&big).unwrap();
        let file = conn.path().unwrap().to_string();
        let started = std::time::Instant::now();
        while freelist_pages(&conn).unwrap() > 0 {
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "large freelist not reclaimed on open");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        wait_for_reclaim(&file, std::time::Duration::from_secs(10));
        drop(conn);
        cleanup(&big);

        let small = db_with_freelist("open-reclaim-small", RECLAIM_ON_OPEN_PAGES / 8);
        let conn = open_database(&small).unwrap();
        let file = conn.path().unwrap().to_string();
        assert!(!reclaim_running(&file), "a small freelist must not start a reclaim");
        std::thread::sleep(std::time::Duration::from_millis(200));
        let free = freelist_pages(&conn).unwrap();
        assert!(free >= RECLAIM_ON_OPEN_PAGES / 8, "the small freelist was touched: {free} pages left");
        drop(conn);
        cleanup(&small);
    }

    /// `PRAGMA incremental_vacuum` has to be stepped to the end: one result row per freed page.
    #[test]
    fn reclaim_free_pages_empties_the_freelist() {
        let path = temp_db_path("reclaim");
        cleanup(&path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA auto_vacuum = INCREMENTAL;
             CREATE TABLE filler (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for i in 0..5_000 {
            tx.execute("INSERT INTO filler (payload) VALUES (?1)", params![format!("{i:0>200}")]).unwrap();
        }
        tx.commit().unwrap();
        conn.execute_batch("DELETE FROM filler;").unwrap();
        let free = |c: &Connection| c.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0)).unwrap();
        let before = free(&conn);
        assert!(before > 1, "the delete must leave more than one free page (got {before})");
        // A chunk smaller than the freelist, so the loop has to go round several times.
        let chunk = before / 4;
        let stats = reclaim_free_pages(&conn, chunk, std::time::Duration::ZERO, &|| false).unwrap();
        assert_eq!(free(&conn), 0);
        assert_eq!(stats.pages, before);
        assert!(stats.chunks >= 4, "{stats:?}");
        drop(conn);
        cleanup(&path);
    }

    fn compact_running_on(path: &str) -> bool {
        compact_running(Path::new(path))
    }

    /// Compact is single flight per database file: a second one on the same file is refused with the
    /// stable marker while the first runs (also through a second connection), another file is not
    /// affected, and the slot is free again once the first is done.
    #[test]
    fn compact_is_single_flight_per_file() {
        let a = temp_db_path("compact-single-a");
        let b = temp_db_path("compact-single-b");
        cleanup(&a);
        cleanup(&b);
        let conn_a = open_database(&a).unwrap();
        let conn_a2 = open_database(&a).unwrap();
        let conn_b = open_database(&b).unwrap();
        let file_a = conn_a.path().unwrap().to_string();

        let guard = begin_compact(&conn_a).unwrap();
        assert!(compact_running_on(&file_a));
        let err = compact_database(&conn_a2).unwrap_err();
        assert_eq!(err, COMPACT_RUNNING_ERROR);
        assert!(err.starts_with("compact-running:"), "the frontend matches this prefix");
        assert!(compact_running_on(&file_a), "the refused second call must not release the first one's slot");
        compact_database(&conn_b).expect("another file compacts meanwhile");

        drop(guard);
        assert!(!compact_running_on(&file_a));
        compact_database(&conn_a2).expect("the slot is free again");
        assert!(!compact_running_on(&file_a), "a finished compact releases its slot");
        drop((conn_a, conn_a2, conn_b));
        cleanup(&a);
        cleanup(&b);
    }

    /// A compact stops a running background reclaim before the VACUUM (the reclaim ends before its next
    /// chunk — no reclaim thread fights the VACUUM for the write lock), and while it runs no new reclaim
    /// starts: neither from an open with a large freelist nor from an explicit trigger.
    #[test]
    fn compact_stops_a_running_reclaim_and_blocks_new_ones() {
        let path = db_with_freelist("compact-vs-reclaim", 16_384);
        let conn = open_database(&path).unwrap(); // starts the background reclaim (freelist > threshold)
        let file = conn.path().unwrap().to_string();
        assert!(reclaim_running(&file), "the reclaim must be running for this test to mean anything");

        let guard = begin_compact(&conn).unwrap();
        assert!(!reclaim_running(&file), "begin_compact returns only once the reclaim has stopped");
        let left = freelist_pages(&conn).unwrap();
        assert!(left > 0, "the reclaim was stopped early, not run to the end");

        let second = open_database(&path).unwrap();
        assert!(!reclaim_running(&file), "an open during the compact must not start a reclaim");
        spawn_reclaim(&second);
        assert!(!reclaim_running(&file), "a trigger during the compact must not start a reclaim");
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(freelist_pages(&conn).unwrap(), left, "nothing reclaimed while the compact holds the file");
        drop(second);

        drop(guard);
        spawn_reclaim(&conn);
        assert!(reclaim_running(&file), "after the compact the reclaim works again");
        wait_for_reclaim(&file, std::time::Duration::from_secs(30));
        drop(conn);
        cleanup(&path);
    }

    /// The whole compact on a file whose own open started a reclaim (large freelist): it completes
    /// instead of failing with SQLITE_BUSY, the freelist is empty and the file has shrunk.
    #[test]
    fn compact_completes_while_its_open_started_a_reclaim() {
        let path = db_with_freelist("compact-completes", 16_384);
        let conn = open_database(&path).unwrap();
        let file = conn.path().unwrap().to_string();
        assert!(reclaim_running(&file), "the reclaim must be running for this test to mean anything");
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        let size = compact_database(&conn).expect("the compact completes");
        assert_eq!(freelist_pages(&conn).unwrap(), 0);
        let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        assert_eq!(size, pages * page_size);
        assert!(pages < 1024, "the dropped table's pages are gone ({pages} pages left)");
        assert!(!reclaim_running(&file) && !compact_running_on(&file));
        drop(conn);
        cleanup(&path);
    }

    /// Compact timing on a COPY of a real flight database (see `migrates_the_real_database`): opens it
    /// like the app (which may start a reclaim) and compacts right away.
    /// `KITE_TEST_DB=<path> cargo test compacts_the_real_database -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn compacts_the_real_database() {
        let Ok(src) = std::env::var("KITE_TEST_DB") else {
            eprintln!("KITE_TEST_DB not set — skipping");
            return;
        };
        let src = PathBuf::from(src);
        let work = src.with_extension("compact-copy.db");
        let _ = std::fs::remove_file(&work);
        std::fs::copy(&src, &work).expect("copy fixture");
        let mb = |p: &Path| std::fs::metadata(p).map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
        let wal = work.with_extension("db-wal");
        let file_before = mb(&work);
        let opened = std::time::Instant::now();
        let conn = open_database(&work).expect("open the real DB");
        let file = conn.path().unwrap().to_string();
        let flights: i64 = conn.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        let free_before = freelist_bytes(&conn).unwrap();
        eprintln!(
            "real DB: {file_before:.1} MB on disk, freelist {:.1} MB, reclaim on open: {}, open {} ms",
            free_before as f64 / 1_048_576.0,
            reclaim_running(&file),
            opened.elapsed().as_millis()
        );
        let started = std::time::Instant::now();
        let (size, conn) = std::thread::scope(|s| {
            let compact = s.spawn(|| (compact_database(&conn), conn));
            // A second compact while the first runs: the app opens the DB first (`open_db`), so that open
            // must not block on the running VACUUM, and the compact is then refused.
            std::thread::sleep(std::time::Duration::from_millis(500));
            let second = std::time::Instant::now();
            let other = open_database(&work).expect("open during the compact");
            let refused = compact_database(&other);
            eprintln!("second compact during the first: {:?} after {} ms", refused, second.elapsed().as_millis());
            assert_eq!(refused.unwrap_err(), COMPACT_RUNNING_ERROR);
            drop(other);
            let (size, conn) = compact.join().unwrap();
            (size.expect("compact the real DB"), conn)
        });
        eprintln!(
            "compact: {} ms, logical size {:.1} MB; on disk now: db {:.1} MB, wal {:.1} MB",
            started.elapsed().as_millis(),
            size as f64 / 1_048_576.0,
            mb(&work),
            mb(&wal)
        );
        assert_eq!(freelist_pages(&conn).unwrap(), 0);
        let flights_after: i64 = conn.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        assert_eq!(flights_after, flights);
        drop(conn);
        eprintln!("after close: db {:.1} MB, wal {:.1} MB", mb(&work), mb(&wal));
        let _ = std::fs::remove_file(&work);
        let _ = std::fs::remove_file(&wal);
        let _ = std::fs::remove_file(work.with_extension("db-shm"));
        let _ = std::fs::remove_file(backup_path_for(&work));
    }
}
