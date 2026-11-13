// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Application file logger.
//
// The codebase already logs liberally through the `log` facade (`log::info!`/`warn!`/`error!`/
// `debug!`), but until now no logger was installed — so every one of those calls was a silent no-op.
// This module installs a simple file logger so connection / handshake problems (e.g. a user who
// can't connect to a PX4 board) leave a diagnostic trail the user can hand back.
//
// Design:
// - One TXT file per day in the app data folder (`<AppData>/kite-gc/kite-gc-YYYY-MM-DD.log`, or
//   `data/` in portable mode; app-private storage on Android), sessions appended under a header
//   block, files older than `LOG_RETENTION_DAYS` pruned on start. Bounded, easy to find, easy to send
//   — on Android through the share sheet, which is why `log_files` lists the day-files.
// - A size limit per file as an emergency brake: a full day-file is kept and the log continues in
//   one overflow file that starts over when it is full too (`CappedLog`).
// - The level is user-configurable at runtime via Settings (OFF / Error / Warning / Debug). We rely
//   on `log::set_max_level` as the gate, so `set_level` is a single atomic store with no relocking.
// - Every record is flushed immediately: this is a low-volume diagnostic log, and flushing means a
//   crash mid-connect still leaves the last lines on disk.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Size limit of one log file. A day's log is at most two files of this size (see `CappedLog`).
const MAX_LOG_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// The day's log as at most two size-limited files — an emergency brake, not a rotation scheme. A
/// fault that logs on every pass of a loop once wrote 7 GB within minutes.
///
/// - The day-file (`kite-gc-YYYY-MM-DD.log`) is written up to `max` bytes and then left alone: it
///   holds the start of the day, and with it whatever set a flood off.
/// - From there the log continues in the overflow file (`kite-gc-YYYY-MM-DD-2.log`). When that is
///   full too it is emptied and written again, as often as needed: it always holds the latest lines.
///
/// A line is never split; a file may fall short of `max` by less than one line.
struct CappedLog {
    writer: BufWriter<File>,
    /// The file being written: the day-file, or the overflow file once the day-file is full.
    path: PathBuf,
    overflow_path: PathBuf,
    in_overflow: bool,
    /// Bytes in the file being written.
    written: u64,
    max: u64,
}

/// `kite-gc-2026-11-13.log` → `kite-gc-2026-11-13-2.log`.
fn overflow_path_of(day_file: &Path) -> PathBuf {
    let stem = day_file.file_stem().and_then(|s| s.to_str()).unwrap_or("kite-gc");
    day_file.with_file_name(format!("{stem}-2.log"))
}

/// Open the overflow file for appending, emptied first unless `keep`. Always an append handle, like
/// the day-file's: a second Kite instance writing the same file then adds its lines at the end instead
/// of writing over ours. (Emptied through a handle of its own — an append handle cannot truncate on
/// Windows.)
fn open_overflow(path: &Path, keep: bool) -> std::io::Result<File> {
    if !keep {
        File::create(path)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

impl CappedLog {
    /// Open the day's log for appending: the day-file while it has room, else the overflow file
    /// (emptied first when that is full as well).
    fn open(day_file: &Path, max: u64) -> std::io::Result<Self> {
        let overflow_path = overflow_path_of(day_file);
        let size_of = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let day_size = size_of(day_file);
        // An overflow file means the day-file was handed over already, even if that left it a few
        // bytes short of the limit: writing to it again would roll once more and empty the overflow
        // file — the latest lines of the session before this one.
        if day_size < max && !overflow_path.exists() {
            let file = OpenOptions::new().create(true).append(true).open(day_file)?;
            return Ok(Self {
                writer: BufWriter::new(file),
                path: day_file.to_path_buf(),
                overflow_path,
                in_overflow: false,
                written: day_size,
                max,
            });
        }
        let overflow_size = size_of(&overflow_path);
        let keep = overflow_size < max;
        let file = open_overflow(&overflow_path, keep)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path: overflow_path.clone(),
            overflow_path,
            in_overflow: true,
            written: if keep { overflow_size } else { 0 },
            max,
        })
    }

    /// Append one complete line (with its newline) and flush. Best-effort, like the whole logger: a
    /// failed write is dropped.
    fn write_line(&mut self, line: &[u8]) {
        if self.written > 0 && self.written + line.len() as u64 > self.max {
            self.roll();
        }
        if self.writer.write_all(line).is_ok() {
            self.written += line.len() as u64;
        }
        let _ = self.writer.flush();
    }

    /// The file being written is full: leave the day-file for the overflow file, or start the
    /// overflow file over. If the new file cannot be opened the current one keeps being written — a
    /// log over its limit is better than none.
    fn roll(&mut self) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let limit_mb = self.max / (1024 * 1024);
        let overflow_name = self.overflow_path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let file = match open_overflow(&self.overflow_path, false) {
            Ok(f) => f,
            Err(_) => return,
        };
        let note = if self.in_overflow {
            format!("{now} [WARN ] logging: size limit of {limit_mb} MB reached again \u{2014} this file was emptied, the lines before this one are lost\n")
        } else {
            let last = format!("{now} [WARN ] logging: size limit of {limit_mb} MB reached \u{2014} the log continues in {overflow_name}\n");
            let _ = self.writer.write_all(last.as_bytes());
            let _ = self.writer.flush();
            format!("{now} [WARN ] logging: continued from the day-file, which reached its size limit of {limit_mb} MB\n")
        };
        self.writer = BufWriter::new(file);
        self.path = self.overflow_path.clone();
        self.in_overflow = true;
        self.written = 0;
        if self.writer.write_all(note.as_bytes()).is_ok() {
            self.written = note.len() as u64;
        }
    }
}

/// Open log file. `None` until `init` succeeds.
struct LoggerState {
    log: Option<CappedLog>,
}

struct FileLogger {
    state: Mutex<LoggerState>,
}

/// Third-party crates whose Debug/Info output would bury ours. `reqwest`/`hyper` emit a line per HTTP
/// connection, and Kite polls ADS-B, tiles and the video API continuously — a Debug-level log was
/// ~90 % "starting new connection" noise, which is precisely the level a tester is asked to switch to.
/// They stay fully visible from **Warn** upwards, so a real failure is never hidden.
const NOISY_TARGETS: &[&str] = &[
    "reqwest", "hyper", "hyper_util", "h2", "rustls", "tokio_util", "want", "mio",
    "tao", "wry", "zbus", "tracing", "globset", "cranelift", "sqlx",
];

/// Is `target` one of `NOISY_TARGETS` (exact match or a `crate::module` child)?
fn is_noisy(target: &str) -> bool {
    NOISY_TARGETS.iter().any(|n| {
        target.starts_with(n) && (target.len() == n.len() || target[n.len()..].starts_with("::"))
    })
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // The `log` macros already gate on `log::max_level()` before calling us, so the level filter is
        // handled there. On top of that we drop sub-warning chatter from third-party crates (see
        // `NOISY_TARGETS`) — the level the user picks is meant to control OUR verbosity.
        if metadata.level() > Level::Warn && is_noisy(metadata.target()) {
            return false;
        }
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} [{}] {}: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            level_tag(record.level()),
            record.target(),
            record.args(),
        );
        if let Ok(mut guard) = self.state.lock() {
            if let Some(log) = guard.log.as_mut() {
                log.write_line(line.as_bytes());
            }
        }
        // Android: mirror to stderr → logcat (`RustStdoutStderr`); adb cannot read the app-private
        // log file of a release build.
        #[cfg(target_os = "android")]
        eprint!("{line}");
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.state.lock() {
            if let Some(log) = guard.log.as_mut() {
                let _ = log.writer.flush();
            }
        }
    }
}

/// Fixed-width level tag so columns line up in the file.
fn level_tag(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARN ",
        Level::Info => "INFO ",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}

static LOGGER: FileLogger = FileLogger {
    state: Mutex::new(LoggerState { log: None }),
};

/// Map a settings string ("off"/"error"/"warning"/"debug") to a `LevelFilter`.
/// "debug" intentionally also captures Info-level records (the connection/handshake milestones).
pub fn level_from_str(s: &str) -> LevelFilter {
    match s.to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warning" | "warn" => LevelFilter::Warn,
        "debug" => LevelFilter::Debug,
        _ => LevelFilter::Warn,
    }
}

/// Resolve the log directory, mirroring the DB-path logic (portable → `<exe>/data`, else AppData).
fn resolve_log_dir(portable: bool) -> PathBuf {
    if portable {
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            return exe_dir.join("data");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("kite-gc");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local").join("share").join("kite-gc");
        }
    }
    // iOS shares the macOS layout; `HOME` is the app sandbox container root there.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("kite-gc");
        }
    }
    // Android: app-private storage — see `android::app_data_dir`. This runs before Tauri exists
    // (the logger is installed first thing in `run()`), which is why that path is derived rather
    // than requested from an `AppHandle`.
    #[cfg(target_os = "android")]
    {
        return crate::android::app_data_dir();
    }

    #[allow(unreachable_code)]
    PathBuf::from(".")
}

/// Keep at most this many days of log files (older `kite-gc-*.log` are pruned on startup).
const LOG_RETENTION_DAYS: u64 = 30;

/// Install the file logger. Call once, as early as possible in `run()` so startup is captured.
///
/// `level` is the initial filter (the frontend re-applies the persisted user choice on startup).
/// MWPTools-style scheme: **one file per day** (`kite-gc-YYYY-MM-DD.log`), **appended** across
/// sessions on the same day, each session prefaced with a header block (time / version+build /
/// platform / level). Files older than `LOG_RETENTION_DAYS` are pruned. Logger-init failures are
/// printed to stderr and otherwise ignored — the app must still run without a log file.
pub fn init(level: LevelFilter, portable: bool) {
    let dir = resolve_log_dir(portable);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("logging: cannot create log dir {}: {}", dir.display(), e);
        return;
    }
    prune_old_logs(&dir);

    let date = chrono::Local::now().format("%Y-%m-%d");
    let path = dir.join(format!("kite-gc-{date}.log"));

    let capped = match CappedLog::open(&path, MAX_LOG_FILE_BYTES) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("logging: cannot open log file {}: {}", path.display(), e);
            return;
        }
    };

    {
        let mut guard = match LOGGER.state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.log = Some(capped);
    }

    // `set_logger` only succeeds once for the process lifetime; ignore a double-init.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);

    write_session_header(level, portable);
}

/// Write the per-session header block straight to the file (raw — bypasses the level filter + the
/// per-line timestamp/level prefix), so each day-file clearly delimits sessions.
fn write_session_header(level: LevelFilter, portable: bool) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let sep = "=".repeat(64);
    write_raw(&format!("\n{sep}"));
    write_raw(&format!("Session start: {now}"));
    write_raw(&format!(
        "Kite Ground Control v{} (build {}, {}) \u{b7} {}/{} \u{b7} portable={}",
        env!("KITE_APP_VERSION"),
        env!("KITE_GIT_HASH"),
        if cfg!(debug_assertions) { "debug" } else { "release" },
        std::env::consts::OS,
        std::env::consts::ARCH,
        portable,
    ));
    // Running format + resolved paths — one glance tells a bug report's install story: where the
    // binary actually runs from (.dmg-installed app vs a stray .app, portable dir, translocation)
    // and where on-demand helpers (blackbox_decode) land. A relative helper dir here is an
    // immediate red flag (the macOS Beta 3 "cannot write bin/blackbox_decode" case, issue #20).
    if let Ok(exe) = std::env::current_exe() {
        write_raw(&format!("Exe: {}", exe.display()));
    }
    write_raw(&format!(
        "Helper dir: {}",
        crate::flightlog::decoder::install_dir().display()
    ));
    write_raw(&format!("Log level: {level}"));
    write_raw(&sep);
}

/// Append a one-line settings snapshot to the current day-file. Called once by the frontend after it
/// loads the persisted settings (the backend can't see them at startup), so the session block records
/// the active config. Raw line (no level gate / prefix).
pub fn log_session_settings(summary: &str) {
    write_raw(&format!("Settings: {summary}"));
}

/// Write a raw line (+ newline) directly to the log file, ignoring the level filter. Used for the
/// session header / settings snapshot. Best-effort.
fn write_raw(line: &str) {
    if let Ok(mut guard) = LOGGER.state.lock() {
        if let Some(log) = guard.log.as_mut() {
            log.write_line(format!("{line}\n").as_bytes());
        }
    }
}

/// Delete `kite-gc*.log*` files older than `LOG_RETENTION_DAYS` (by mtime). Best-effort; covers the
/// legacy `kite-gc.log`/`.prev` files too.
fn prune_old_logs(dir: &Path) {
    let max_age = std::time::Duration::from_secs(LOG_RETENTION_DAYS * 24 * 60 * 60);
    let now = std::time::SystemTime::now();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = match name.to_str() {
            Some(n) => n,
            None => continue,
        };
        if !(name.starts_with("kite-gc") && name.contains(".log")) {
            continue;
        }
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
            if now.duration_since(modified).map(|age| age > max_age).unwrap_or(false) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Change the active log level at runtime (driven by Settings). Cheap atomic store.
pub fn set_level(level: LevelFilter) {
    log::set_max_level(level);
    log::info!("Log level changed to {}", level);
}

/// The log file being written (for "open log folder" in Settings), if logging is installed: the
/// day-file, or its overflow file once the day-file reached the size limit.
pub fn log_path() -> Option<PathBuf> {
    LOGGER.state.lock().ok().and_then(|g| g.log.as_ref().map(|l| l.path.clone()))
}

/// One day-file in the log folder, as the mobile "Share log" picker lists it.
#[derive(serde::Serialize)]
pub struct LogFileInfo {
    /// `kite-gc-YYYY-MM-DD.log`, or `kite-gc-YYYY-MM-DD-2.log` for a day's overflow file — the date
    /// lives in the name, the picker reads it from there.
    pub name: String,
    /// Absolute path, what `share_file` takes.
    pub path: String,
    /// Size in bytes at call time; the active file is flushed first so its figure is current.
    pub size: u64,
    /// The file this session appends to.
    pub active: bool,
}

/// Is `name` a day-file of the current scheme (`kite-gc-YYYY-MM-DD.log`, or the same with `-2` for
/// the overflow file of a day that hit the size limit)? Deliberately narrower than
/// the `prune_old_logs` match, which must also sweep the legacy `kite-gc.log`/`.prev` pair — the
/// picker has no date to show for those.
fn is_day_file(name: &str) -> bool {
    name.starts_with("kite-gc-") && name.ends_with(".log")
}

/// The day-files next to the active log, newest first; empty when logging is not installed.
///
/// On Android the log folder is app-private — no file manager and no MTP connection can reach it —
/// so the share sheet in Settings is the only way a log leaves the device, and after a restart the
/// file a tester needs is yesterday's, not the active one. This lists what is there so the frontend
/// can offer a choice. Newest first falls out of sorting by name: the zero-padded date is in it.
/// Best-effort: an unreadable entry is skipped, an unreadable folder yields an empty list.
pub fn log_files() -> Vec<LogFileInfo> {
    let active = match log_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let dir = match active.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };
    // Every record is flushed as it is written, but the size below must not depend on that detail.
    LOGGER.flush();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut files: Vec<LogFileInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !is_day_file(&name) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(LogFileInfo {
                active: active.file_name() == Some(entry.file_name().as_os_str()),
                path: entry.path().to_string_lossy().to_string(),
                size: meta.len(),
                name,
            })
        })
        .collect();
    files.sort_by(|a, b| b.name.cmp(&a.name));
    files
}

/// True when `p` is the current log file or its `.prev` sibling — used by callers that want to avoid
/// touching live log files. (Currently unused outside this module; kept for completeness.)
#[allow(dead_code)]
pub fn is_log_file(p: &Path) -> bool {
    if let Some(active) = log_path() {
        return p == active
            || active
                .with_extension("log.prev")
                .file_name()
                .map(|n| Some(n) == p.file_name())
                .unwrap_or(false);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the system temp folder; removed by the caller.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-logcap-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn line(i: usize) -> Vec<u8> {
        format!("line {i:04}\n").into_bytes() // 10 bytes
    }

    #[test]
    fn the_day_file_stops_at_the_limit_and_keeps_its_start() {
        let dir = temp_dir("first");
        let day = dir.join("kite-gc-2026-11-13.log");
        let mut log = CappedLog::open(&day, 100).unwrap();
        for i in 0..30 {
            log.write_line(&line(i));
        }
        drop(log);
        let first = std::fs::read_to_string(&day).unwrap();
        assert!(first.starts_with("line 0000\nline 0001\n"));
        assert!(first.contains("the log continues in kite-gc-2026-11-13-2.log"));
        // Ten lines fit; only the hand-over note goes past the limit.
        assert_eq!(first.matches("line ").count(), 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_overflow_file_is_emptied_when_full_and_holds_the_latest_lines() {
        let dir = temp_dir("second");
        let day = dir.join("kite-gc-2026-11-13.log");
        let mut log = CappedLog::open(&day, 200).unwrap();
        for i in 0..500 {
            log.write_line(&line(i));
        }
        assert!(log.in_overflow);
        drop(log);
        let overflow = dir.join("kite-gc-2026-11-13-2.log");
        let second = std::fs::read_to_string(&overflow).unwrap();
        assert!(second.len() as u64 <= 200, "overflow file is {} bytes", second.len());
        assert!(second.contains("this file was emptied"));
        assert!(second.ends_with("line 0499\n"));
        // The day-file was not touched again.
        assert!(std::fs::read_to_string(&day).unwrap().starts_with("line 0000\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_session_continues_where_the_day_left_off() {
        let dir = temp_dir("reopen");
        let day = dir.join("kite-gc-2026-11-13.log");
        let overflow = dir.join("kite-gc-2026-11-13-2.log");

        // Room left in the day-file: append there.
        std::fs::write(&day, b"earlier session\n").unwrap();
        let mut log = CappedLog::open(&day, 100).unwrap();
        assert!(!log.in_overflow);
        log.write_line(b"next\n");
        drop(log);
        assert_eq!(std::fs::read_to_string(&day).unwrap(), "earlier session\nnext\n");

        // Day-file full, overflow with room: append to the overflow file.
        std::fs::write(&day, vec![b'x'; 100]).unwrap();
        std::fs::write(&overflow, b"tail\n").unwrap();
        let mut log = CappedLog::open(&day, 100).unwrap();
        assert!(log.in_overflow);
        log.write_line(b"more\n");
        drop(log);
        assert_eq!(std::fs::read_to_string(&overflow).unwrap(), "tail\nmore\n");
        assert_eq!(std::fs::metadata(&day).unwrap().len(), 100);

        // Both full: the overflow file starts over.
        std::fs::write(&overflow, vec![b'y'; 100]).unwrap();
        let mut log = CappedLog::open(&day, 100).unwrap();
        log.write_line(b"fresh\n");
        drop(log);
        assert_eq!(std::fs::read_to_string(&overflow).unwrap(), "fresh\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restart_after_the_hand_over_does_not_empty_the_overflow_file() {
        let dir = temp_dir("handed-over");
        let day = dir.join("kite-gc-2026-11-13.log");
        let overflow = dir.join("kite-gc-2026-11-13-2.log");
        // The hand-over left the day-file short of the limit (the line that did not fit was longer
        // than the note written in its place), and the overflow file holds the end of that session.
        std::fs::write(&day, vec![b'x'; 60]).unwrap();
        std::fs::write(&overflow, b"end of the last session\n").unwrap();
        let mut log = CappedLog::open(&day, 100).unwrap();
        assert!(log.in_overflow);
        log.write_line(&[b'h'; 50]);
        drop(log);
        assert!(std::fs::read_to_string(&overflow).unwrap().starts_with("end of the last session\n"));
        assert_eq!(std::fs::metadata(&day).unwrap().len(), 60);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn both_files_of_a_day_are_day_files() {
        assert!(is_day_file("kite-gc-2026-11-13.log"));
        assert!(is_day_file("kite-gc-2026-11-13-2.log"));
        assert_eq!(overflow_path_of(Path::new("kite-gc-2026-11-13.log")), PathBuf::from("kite-gc-2026-11-13-2.log"));
    }
}
