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
// - One TXT file in the app data folder (`<AppData>/kite-gc/kite-gc.log`, or `data/` in portable
//   mode). The previous session's file is rotated to `kite-gc.log.prev` on each start, so there are
//   always exactly two: the current run + the one before. Bounded, easy to find, easy to send.
// - The level is user-configurable at runtime via Settings (OFF / Error / Warning / Debug). We rely
//   on `log::set_max_level` as the gate, so `set_level` is a single atomic store with no relocking.
// - Every record is flushed immediately: this is a low-volume diagnostic log, and flushing means a
//   crash mid-connect still leaves the last lines on disk.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Open log file + its path. `None` until `init` succeeds.
struct LoggerState {
    writer: Option<BufWriter<File>>,
    path: Option<PathBuf>,
}

struct FileLogger {
    state: Mutex<LoggerState>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        // The `log` macros already gate on `log::max_level()` before calling us, so the level filter
        // is handled there. We accept anything that reaches this point.
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
            if let Some(w) = guard.writer.as_mut() {
                let _ = w.write_all(line.as_bytes());
                let _ = w.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.state.lock() {
            if let Some(w) = guard.writer.as_mut() {
                let _ = w.flush();
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
    state: Mutex::new(LoggerState {
        writer: None,
        path: None,
    }),
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

    PathBuf::from(".")
}

/// Install the file logger. Call once, as early as possible in `run()` so startup is captured.
///
/// `level` is the initial filter (the frontend re-applies the persisted user choice on startup).
/// Rotates the previous run's log to `*.prev`, then opens a fresh file. Logger-init failures are
/// printed to stderr and otherwise ignored — the app must still run without a log file.
pub fn init(level: LevelFilter, portable: bool) {
    let dir = resolve_log_dir(portable);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("logging: cannot create log dir {}: {}", dir.display(), e);
        return;
    }
    let path = dir.join("kite-gc.log");

    // Rotate the previous session's log out of the way (best-effort).
    if path.exists() {
        let prev = dir.join("kite-gc.log.prev");
        let _ = std::fs::remove_file(&prev);
        let _ = std::fs::rename(&path, &prev);
    }

    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
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
        guard.writer = Some(BufWriter::new(file));
        guard.path = Some(path.clone());
    }

    // `set_logger` only succeeds once for the process lifetime; ignore a double-init.
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    } else {
        // Already installed (e.g. a mobile re-entry) — at least apply the level.
        log::set_max_level(level);
    }

    log::info!(
        "Logger initialized — level={}, file={}",
        level,
        path.display()
    );
}

/// Change the active log level at runtime (driven by Settings). Cheap atomic store.
pub fn set_level(level: LevelFilter) {
    log::set_max_level(level);
    log::info!("Log level changed to {}", level);
}

/// The active log file path (for "open log folder" in Settings), if logging is installed.
pub fn log_path() -> Option<PathBuf> {
    LOGGER.state.lock().ok().and_then(|g| g.path.clone())
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
