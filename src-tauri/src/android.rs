// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Android-specific runtime support.
//!
//! Everything the desktop build derives from `%APPDATA%` / `$XDG_DATA_HOME` /
//! `~/Library/Application Support` has to come from somewhere else on Android: those variables do not
//! exist in an app process, and the desktop resolvers' last-resort `PathBuf::from(".")` would put the
//! flight database and the log files in the process's working directory — `/` on Android, which is
//! read-only. Every path resolver therefore takes an Android branch that lands here.
//!
//! We derive the directory ourselves instead of asking Tauri for `app_data_dir()`, because the file
//! logger is installed in `run()` *before* the Tauri builder exists and there is no `AppHandle` to ask
//! yet. Splitting the two would give the logger and the database different roots; deriving it makes
//! one answer available from the first line of `run()` onwards, and `log_resolved_dirs()` cross-checks
//! it against Tauri's own answer once the app is up.

pub mod content;
pub mod jvm;
pub mod native_video;
pub mod net;
pub mod screen;
pub mod storage;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Cached per process — the package name never changes, and the directory creation should happen once.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app's private files directory: `/data/user/0/<package>/files`.
///
/// This is the app-private storage Android guarantees: writable without any permission, excluded from
/// other apps, and removed on uninstall. It is where `flights.db`, the rotating log files and the
/// terrain tile cache live on mobile.
pub fn app_data_dir() -> PathBuf {
    DATA_DIR
        .get_or_init(|| {
            let dir = derive_data_dir();
            // Best-effort: if this fails the individual writers will report their own errors, which are
            // more specific than anything we could say here.
            let _ = std::fs::create_dir_all(&dir);
            dir
        })
        .clone()
}

/// Documents-equivalent for user-facing exports (raw logs, HID profiles, mission files).
///
/// Kept inside app-private storage on purpose. Writing to the shared `Documents/` collection is a
/// MediaStore operation on Android 10+ (scoped storage) — a content-resolver insert, not a path — so a
/// `PathBuf` cannot address it at all. Exports therefore land here, and sharing one out is a separate,
/// deliberate step (the system share sheet, via the `FileProvider` already declared in the manifest).
pub fn app_documents_dir() -> PathBuf {
    app_data_dir().join("Documents")
}

/// Read the package name from `/proc/self/cmdline` and build the files directory from it.
///
/// Android sets the process name to the package name (`com.kitegc.app`), and `/proc/self/cmdline` is
/// the one place a plain Rust process can read it without JNI. `/data/user/0/<pkg>` is the modern path
/// and what `getFilesDir()` returns on a single-user device; `/data/data/<pkg>` is the pre-multi-user
/// symlink to the same place, kept as a fallback for the rare device where the former is missing.
fn derive_data_dir() -> PathBuf {
    let package = read_package_name().unwrap_or_else(|| {
        log::warn!("[android] could not read package name from /proc/self/cmdline; using the default");
        // Matches `identifier` in tauri.conf.json / `applicationId` in app/build.gradle.kts.
        "com.kitegc.app".to_string()
    });

    for base in ["/data/user/0", "/data/data"] {
        let candidate = Path::new(base).join(&package);
        if candidate.is_dir() {
            return candidate.join("files");
        }
    }

    // Neither exists yet (or is not visible). `/data/user/0` is the canonical modern location, so
    // create there rather than in the legacy symlink.
    Path::new("/data/user/0").join(&package).join("files")
}

/// The package name, i.e. `cmdline` up to the first NUL. Android appends `:<name>` for processes other
/// than the main one (`com.kitegc.app:remote`); we only ever run in the main process, but strip it
/// anyway so a future service process would still resolve to the same directory.
fn read_package_name() -> Option<String> {
    let raw = std::fs::read("/proc/self/cmdline").ok()?;
    let name = raw
        .split(|b| *b == 0)
        .next()
        .and_then(|s| std::str::from_utf8(s).ok())?
        .trim();
    let name = name.split(':').next().unwrap_or(name);
    // A package name always contains a dot; anything else means we read something unexpected
    // (a shell, an unpackaged test binary) and should fall back rather than build a path from it.
    if name.is_empty() || !name.contains('.') {
        return None;
    }
    Some(name.to_string())
}

/// Log the derived directory next to the one Tauri reports, once, from the setup hook.
///
/// The derivation above is the only path used at runtime — this exists so that if a device ever
/// disagrees with it, the log says so outright instead of leaving a "where did my flight log go?"
/// mystery to be reproduced.
pub fn log_resolved_dirs(app: &tauri::AppHandle) {
    use tauri::Manager;

    let derived = app_data_dir();
    match app.path().app_data_dir() {
        Ok(tauri_dir) => {
            if tauri_dir == derived {
                log::info!("[android] app data dir: {}", derived.display());
            } else {
                log::warn!(
                    "[android] app data dir: using {} but Tauri reports {} — \
                     data will be written to the former",
                    derived.display(),
                    tauri_dir.display()
                );
            }
        }
        Err(e) => log::info!(
            "[android] app data dir: {} (Tauri could not resolve one: {e})",
            derived.display()
        ),
    }
}
