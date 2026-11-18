// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Moving the flight database between folders (Settings → Data). Only `flights.db` and its
//! pre-migration backup travel; sessions, the hi-res cache and the scratch store are working files
//! in the app's own data folder.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::db;

/// What a folder holds, read without creating or upgrading anything.
#[derive(Debug, serde::Serialize)]
pub struct FolderState {
    /// The database file with its folder resolved: two settings naming one file compare equal.
    pub path: String,
    pub exists: bool,
    /// 0 for a missing or unreadable file.
    pub schema_version: u32,
    pub current_version: u32,
    pub size_bytes: u64,
    pub flights: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub struct MoveOutcome {
    /// The database arrived, but files of it are still in the source folder.
    pub source_left: bool,
}

pub fn folder_state(db_file: &Path) -> FolderState {
    let size_bytes = std::fs::metadata(db_file).map(|m| m.len()).unwrap_or(0);
    let conn = db_file
        .exists()
        .then(|| Connection::open_with_flags(db_file, OpenFlags::SQLITE_OPEN_READ_ONLY).ok())
        .flatten();
    let schema_version =
        conn.as_ref().and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)).ok()).unwrap_or(0);
    FolderState {
        path: canonical(db_file).to_string_lossy().into_owned(),
        exists: db_file.exists(),
        schema_version,
        current_version: db::CURRENT_SCHEMA_VERSION,
        size_bytes,
        flights: conn.as_ref().and_then(flight_count),
    }
}

fn flight_count(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).ok()
}

/// SQLite's `-wal` / `-shm` files of `db_file`.
fn sidecars(db_file: &Path) -> [PathBuf; 2] {
    let with = |suffix: &str| {
        let mut name = db_file.as_os_str().to_os_string();
        name.push(suffix);
        PathBuf::from(name)
    };
    [with("-wal"), with("-shm")]
}

fn incoming(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".incoming");
    PathBuf::from(name)
}

/// `file` with its folder resolved (the file itself need not exist).
fn canonical(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).join(file.file_name().unwrap_or_default())
}

fn same_file(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

/// `VACUUM INTO` includes the WAL and stays consistent while others read; `dest` is removed on any failure.
fn copy_verified(conn: &Connection, dest: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(dest);
    let fail = |what: &str, e: String| {
        let _ = std::fs::remove_file(dest);
        format!("{what}: {e}")
    };
    conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(|e| e.to_string())?;
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])
        .map_err(|e| fail("could not write the copy", e.to_string()))?;
    let copy = Connection::open_with_flags(dest, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| fail("could not open the copy", e.to_string()))?;
    let check: String = copy
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| fail("could not check the copy", e.to_string()))?;
    if check != "ok" {
        drop(copy);
        return Err(fail("the copy is damaged", check));
    }
    let (have, want) = (flight_count(&copy), flight_count(conn));
    drop(copy);
    if have != want {
        return Err(fail("the copy is incomplete", format!("{have:?} of {want:?} flights")));
    }
    Ok(())
}

/// Remove a database with everything that belongs to it in its folder. `dispose` takes the database
/// and its backup (trash or delete, the caller's choice); the rest is deleted. `false` = something stayed.
fn remove_database(db_file: &Path, dispose: &dyn Fn(&Path) -> Result<(), String>) -> bool {
    let mut gone = true;
    for file in [db_file.to_path_buf(), db::backup_path_for(db_file)] {
        if file.exists() {
            if let Err(e) = dispose(&file) {
                log::warn!("[flightlog] could not remove {}: {e}", file.display());
                gone = false;
            }
        }
    }
    for file in sidecars(db_file) {
        let _ = std::fs::remove_file(file);
    }
    // Work folders Kite up to 2.0-b1 kept next to the database. A database may have been moved INTO
    // one of them, so nothing but Kite's own cache files is deleted.
    if let Some(dir) = db_file.parent() {
        super::hires::remove_cache_dir(&dir.join("hires-cache"));
        let _ = std::fs::remove_dir(dir.join("scratch"));
    }
    gone
}

fn delete(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Open the source for a copy and keep the background reclaim off it until the guard drops.
fn open_source(src: &Path) -> Result<(Connection, db::CompactGuard), String> {
    let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| format!("could not open {}: {e}", src.display()))?;
    let guard = db::begin_compact(&conn)?;
    Ok((conn, guard))
}

/// Move the database `src` and its pre-migration backup to `dest`. The source is removed only after
/// the copy passed its check. A database already at `dest` needs `overwrite` and goes to `dispose`.
pub fn move_database(
    src: &Path,
    dest: &Path,
    overwrite: bool,
    dispose: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<MoveOutcome, String> {
    if same_file(src, dest) {
        return Err("the database is already in this folder".into());
    }
    if !src.exists() {
        return Err(format!("no database at {}", src.display()));
    }
    if dest.exists() && !overwrite {
        return Err(format!("{} already exists", dest.display()));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let (conn, _guard) = open_source(src)?;
    let (db_in, backup_in) = (incoming(dest), incoming(&db::backup_path_for(dest)));
    copy_verified(&conn, &db_in)?;
    drop(conn);

    let src_backup = db::backup_path_for(src);
    let _ = std::fs::remove_file(&backup_in);
    if src_backup.exists() {
        let copied = std::fs::copy(&src_backup, &backup_in).map_err(|e| e.to_string()).and_then(|n| {
            let want = std::fs::metadata(&src_backup).map(|m| m.len()).unwrap_or(0);
            (n == want).then_some(()).ok_or_else(|| format!("{n} of {want} bytes"))
        });
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&db_in);
            let _ = std::fs::remove_file(&backup_in);
            return Err(format!("could not copy the database backup: {e}"));
        }
    }

    if dest.exists() && !remove_database(dest, dispose) {
        let _ = std::fs::remove_file(&db_in);
        let _ = std::fs::remove_file(&backup_in);
        return Err(format!("could not replace {}", dest.display()));
    }
    // A stray WAL of an earlier database there would be replayed into the moved one.
    for file in sidecars(dest) {
        let _ = std::fs::remove_file(file);
    }
    if let Err(e) = std::fs::rename(&db_in, dest) {
        let _ = std::fs::remove_file(&db_in);
        let _ = std::fs::remove_file(&backup_in);
        return Err(format!("could not finish the copy at {}: {e}", dest.display()));
    }
    if backup_in.exists() {
        if let Err(e) = std::fs::rename(&backup_in, db::backup_path_for(dest)) {
            let _ = std::fs::remove_file(&backup_in);
            log::warn!("[flightlog] database backup not moved along ({e}) — it stays next to the old database");
            return Ok(MoveOutcome { source_left: true });
        }
    }
    log::info!("[flightlog] database moved from {} to {}", src.display(), dest.display());
    Ok(MoveOutcome { source_left: !remove_database(src, &delete) })
}

/// Write a copy of the database to `dest` (a path, or a document URI on Android) for the user to keep.
pub fn export_database(db_file: &Path, dest: &str) -> Result<(), String> {
    if !db_file.exists() {
        return Err("there is no flight database yet".into());
    }
    #[cfg(target_os = "android")]
    if crate::android::content::is_content_uri(dest) {
        return stream_database(db_file, dest);
    }
    let dest = Path::new(dest);
    if same_file(db_file, dest) {
        return Err("this is the database in use".into());
    }
    let (conn, _guard) = open_source(db_file)?;
    copy_verified(&conn, dest)
}

/// The database file itself goes out block by block — no second copy on a device that may be short of
/// space. The file is whole only with an empty WAL and no writer, hence checkpoint, then write lock.
#[cfg(target_os = "android")]
fn stream_database(db_file: &Path, uri: &str) -> Result<(), String> {
    let (conn, _guard) = open_source(db_file)?;
    conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(|e| e.to_string())?;
    for _ in 0..3 {
        let busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(|e| format!("could not prepare the database: {e}"))?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| format!("could not lock the database: {e}"))?;
        let wal = std::fs::metadata(&sidecars(db_file)[0]).map(|m| m.len()).unwrap_or(0);
        let written = (busy == 0 && wal == 0).then(|| crate::android::content::write_uri(uri, db_file));
        let _ = conn.execute_batch("ROLLBACK");
        if let Some(result) = written {
            return result;
        }
    }
    Err("the database is in use — try again in a moment".into())
}

/// Fetch a database the user picked (a path, or a document URI on Android) next to `db_file` and look
/// at it. Nothing is replaced yet; `commit_import` or `cancel_import` follows.
pub fn stage_import(src: &str, db_file: &Path) -> Result<FolderState, String> {
    let staged = incoming(db_file);
    let _ = std::fs::remove_file(&staged);
    if let Some(dir) = db_file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let fetched = fetch(src, db_file, &staged);
    let state = fetched.map(|()| folder_state(&staged)).and_then(|state| {
        // A `.kflight` has a flights table too, but no schema version: it is no database to run on.
        if state.flights.is_none() || state.schema_version == 0 {
            return Err("this file is not a Kite flight database".to_string());
        }
        if state.schema_version > state.current_version {
            return Err("this database was written by a newer Kite version".to_string());
        }
        let check = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|c| c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))
            .map_err(|e| e.to_string())?;
        if check != "ok" {
            return Err(format!("this database is damaged: {check}"));
        }
        Ok(state)
    });
    if state.is_err() {
        cancel_import(db_file);
    }
    state
}

fn fetch(src: &str, db_file: &Path, staged: &Path) -> Result<(), String> {
    #[cfg(target_os = "android")]
    if crate::android::content::is_content_uri(src) {
        return crate::android::content::read_uri(src, staged);
    }
    if same_file(Path::new(src), db_file) {
        return Err("this is the database in use".into());
    }
    std::fs::copy(src, staged).map(|_| ()).map_err(|e| format!("could not read {src}: {e}"))
}

pub fn cancel_import(db_file: &Path) {
    let staged = incoming(db_file);
    for file in sidecars(&staged) {
        let _ = std::fs::remove_file(file);
    }
    let _ = std::fs::remove_file(staged);
}

/// Put the staged database in the place of `db_file`. The old one is set aside until the new one
/// has its name, then it and its backup are deleted.
pub fn commit_import(db_file: &Path) -> Result<(), String> {
    let staged = incoming(db_file);
    if !staged.exists() {
        return Err("no database is waiting to be imported".into());
    }
    let mut aside = db_file.as_os_str().to_os_string();
    aside.push(".replaced");
    let aside = PathBuf::from(aside);
    let _guard;
    if db_file.exists() {
        let (conn, guard) = open_source(db_file)?;
        _guard = guard;
        // The sidecars go below: only with an emptied WAL do they take no committed data along,
        // should the old database have to come back.
        let busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(|e| format!("could not prepare the current database: {e}"))?;
        drop(conn);
        if busy != 0 {
            return Err("the current database is in use — try again in a moment".into());
        }
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(db_file, &aside).map_err(|e| format!("could not set the current database aside: {e}"))?;
    }
    for file in sidecars(db_file).into_iter().chain(sidecars(&staged)) {
        let _ = std::fs::remove_file(file);
    }
    if let Err(e) = std::fs::rename(&staged, db_file) {
        let _ = std::fs::rename(&aside, db_file);
        return Err(format!("could not put the imported database in place: {e}"));
    }
    let _ = std::fs::remove_file(&aside);
    let _ = std::fs::remove_file(db::backup_path_for(db_file));
    log::info!("[flightlog] database replaced by an imported one at {}", db_file.display());
    Ok(())
}

/// Take the database `old` out of use after `in_use` was opened. With `keep_backup` a checked copy is
/// written next to `in_use` first and the old files are deleted for good (the point is the freed
/// space); without it they go to `dispose`. Returns the copy's path.
pub fn retire_database(
    old: &Path,
    in_use: &Path,
    keep_backup: bool,
    dispose: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Option<PathBuf>, String> {
    if same_file(old, in_use) {
        return Err("this is the database in use".into());
    }
    if !old.exists() {
        return Ok(None);
    }
    let (conn, _guard) = open_source(old)?;
    let copy = match in_use.parent().filter(|_| keep_backup) {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let file = dir.join(format!("flights.backup-{stamp}.db"));
            copy_verified(&conn, &file)?;
            Some(file)
        }
        None => None,
    };
    drop(conn);
    let gone = if copy.is_some() { remove_database(old, &delete) } else { remove_database(old, dispose) };
    if !gone {
        return Err(format!("could not remove {}", old.display()));
    }
    log::info!("[flightlog] database {} retired (copy: {:?})", old.display(), copy);
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dirs(PathBuf);
    impl Dirs {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("kite-dbmove-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn db(&self, folder: &str) -> PathBuf {
            self.0.join(folder).join("flights.db")
        }
        /// A database with `flights` flights, left in WAL mode with its sidecars as the app leaves it.
        fn make(&self, folder: &str, flights: usize) -> PathBuf {
            let path = self.db(folder);
            let conn = db::open_database(&path).unwrap();
            for n in 0..flights {
                conn.execute("INSERT INTO flights (start_time, craft_name) VALUES (?1, 'x')", [format!("t{n}")]).unwrap();
            }
            path
        }
    }
    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn flights(path: &Path) -> i64 {
        folder_state(path).flights.unwrap()
    }

    /// The database and its backup arrive, the source folder keeps nothing of them, and a hi-res
    /// cache left next to the source is thrown away rather than moved.
    #[test]
    fn move_takes_database_and_backup_and_clears_the_source() {
        let dirs = Dirs::new("move");
        let src = dirs.make("a", 3);
        std::fs::write(db::backup_path_for(&src), b"backup").unwrap();
        std::fs::create_dir_all(src.parent().unwrap().join("hires-cache")).unwrap();
        std::fs::write(src.parent().unwrap().join("hires-cache").join("hires_7.db"), b"cache").unwrap();
        std::fs::create_dir_all(src.parent().unwrap().join("sessions")).unwrap();
        let dest = dirs.db("b");
        // A WAL left in the target folder without its database must not meet the moved one.
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&sidecars(&dest)[0], b"stale").unwrap();

        let outcome = move_database(&src, &dest, false, &delete).unwrap();

        assert!(!outcome.source_left);
        assert_eq!(flights(&dest), 3);
        let auto_vacuum: i64 = Connection::open(&dest).unwrap().query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(auto_vacuum, 2, "the moved database must not need the one-time VACUUM again");
        assert_eq!(std::fs::read(db::backup_path_for(&dest)).unwrap(), b"backup");
        assert!(!src.exists() && !db::backup_path_for(&src).exists());
        assert!(sidecars(&src).iter().all(|f| !f.exists()));
        assert!(!src.parent().unwrap().join("hires-cache").exists());
        assert!(src.parent().unwrap().join("sessions").exists(), "sessions are not the move's business");
        assert!(!incoming(&dest).exists());
    }

    /// A database at the destination is replaced only on request, and then it goes to `dispose`
    /// together with its own backup.
    #[test]
    fn an_existing_database_is_replaced_only_with_overwrite() {
        let dirs = Dirs::new("overwrite");
        let src = dirs.make("a", 2);
        let dest = dirs.make("b", 5);
        std::fs::write(db::backup_path_for(&dest), b"theirs").unwrap();

        assert!(move_database(&src, &dest, false, &delete).is_err());
        assert_eq!((flights(&src), flights(&dest)), (2, 5), "a refused move changes nothing");

        let disposed = std::sync::Mutex::new(Vec::new());
        let dispose = |p: &Path| {
            disposed.lock().unwrap().push(p.file_name().unwrap().to_string_lossy().to_string());
            delete(p)
        };
        move_database(&src, &dest, true, &dispose).unwrap();

        assert_eq!(flights(&dest), 2);
        assert!(!db::backup_path_for(&dest).exists(), "the replaced database's backup is not ours");
        assert_eq!(*disposed.lock().unwrap(), ["flights.db", "flights.pre-migration-backup.db"]);
        assert!(!src.exists());
    }

    /// Two settings can name one file (the default folder picked by hand): neither a move nor a
    /// retire may then touch it.
    #[test]
    fn the_database_in_use_is_never_moved_onto_itself_or_retired() {
        let dirs = Dirs::new("same");
        let src = dirs.make("a", 1);
        let same = dirs.0.join("a").join(".").join("flights.db");
        assert!(move_database(&src, &same, true, &delete).is_err());
        assert!(retire_database(&src, &same, true, &delete).is_err());
        assert!(retire_database(&src, &same, false, &delete).is_err());
        assert_eq!(flights(&src), 1);
        assert_eq!(folder_state(&src).path, folder_state(&same).path);
    }

    /// A destination inside the source's old cache folder: clearing the source must not take the
    /// moved database with it.
    #[test]
    fn a_move_into_the_old_cache_folder_survives_the_cleanup() {
        let dirs = Dirs::new("nested");
        let src = dirs.make("a", 2);
        let dest = dirs.0.join("a").join("hires-cache").join("flights.db");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(dest.parent().unwrap().join("hires_3.db"), b"cache").unwrap();

        move_database(&src, &dest, false, &delete).unwrap();

        assert_eq!(flights(&dest), 2);
        assert!(!dest.parent().unwrap().join("hires_3.db").exists());
        assert!(!src.exists());
    }

    /// Retiring with a backup folder leaves one checked copy there and nothing at the old place;
    /// without one the old files go to `dispose`.
    #[test]
    fn retire_keeps_a_copy_or_disposes() {
        let dirs = Dirs::new("retire");
        let old = dirs.make("a", 4);
        std::fs::write(db::backup_path_for(&old), b"backup").unwrap();
        let target = dirs.0.join("b");
        let in_use = target.join("flights.db");

        let copy = retire_database(&old, &in_use, true, &|_| Err("not for a kept copy".into())).unwrap().unwrap();
        assert_eq!(copy.parent(), Some(target.as_path()));
        assert_eq!(flights(&copy), 4);
        assert!(!old.exists() && !db::backup_path_for(&old).exists());

        let old = dirs.make("c", 1);
        let disposed = std::sync::Mutex::new(0);
        let dispose = |p: &Path| {
            *disposed.lock().unwrap() += 1;
            delete(p)
        };
        assert_eq!(retire_database(&old, &in_use, false, &dispose).unwrap(), None);
        assert!(!old.exists());
        assert_eq!(*disposed.lock().unwrap(), 1);
    }

    /// An export is a complete database of its own; an import replaces the current one only at
    /// `commit_import`, and takes the old database's backup and WAL out of the way.
    #[test]
    fn export_then_import_replaces_the_database() {
        let dirs = Dirs::new("export-import");
        let src = dirs.make("a", 3);
        let file = dirs.0.join("backup.db");
        export_database(&src, &file.to_string_lossy()).unwrap();
        assert_eq!(flights(&file), 3);
        assert_eq!(flights(&src), 3);

        let current = dirs.make("b", 1);
        std::fs::write(db::backup_path_for(&current), b"old backup").unwrap();
        let staged = stage_import(&file.to_string_lossy(), &current).unwrap();
        assert_eq!(staged.flights, Some(3));
        assert_eq!(flights(&current), 1, "staging replaces nothing");

        commit_import(&current).unwrap();
        assert_eq!(flights(&current), 3);
        assert!(!db::backup_path_for(&current).exists());
        assert!(!incoming(&current).exists());
        assert!(commit_import(&current).is_err(), "nothing is staged any more");
    }

    /// A file that is no flight database is refused at staging and leaves nothing behind.
    #[test]
    fn a_foreign_file_is_not_staged() {
        let dirs = Dirs::new("foreign");
        let current = dirs.make("a", 2);
        let junk = dirs.0.join("junk.db");
        std::fs::write(&junk, b"not a database at all").unwrap();
        assert!(stage_import(&junk.to_string_lossy(), &current).is_err());
        assert!(stage_import(&current.to_string_lossy(), &current).is_err(), "the database in use");
        let kflight = dirs.0.join("some.kflight");
        let conn = Connection::open(&kflight).unwrap();
        conn.execute_batch("CREATE TABLE flights (id INTEGER PRIMARY KEY); INSERT INTO flights DEFAULT VALUES;").unwrap();
        drop(conn);
        assert!(stage_import(&kflight.to_string_lossy(), &current).is_err(), "a flight export is no database");
        assert!(!incoming(&current).exists());
        assert_eq!(flights(&current), 2);
    }

    #[test]
    fn folder_state_tells_missing_older_and_current() {
        let dirs = Dirs::new("state");
        let missing = folder_state(&dirs.db("none"));
        assert!(!missing.exists && missing.flights.is_none());

        let path = dirs.make("a", 2);
        let current = folder_state(&path);
        assert!(current.exists);
        assert_eq!((current.schema_version, current.flights), (db::CURRENT_SCHEMA_VERSION, Some(2)));
        assert!(!db::backup_path_for(&path).exists(), "the look must not start an upgrade");
    }
}
