// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Android runtime support: the app-private data directory, derived from `/proc/self/cmdline` rather
//! than Tauri's `app_data_dir()` because the file logger needs it before the Tauri builder exists.

pub mod content;
pub mod device;
pub mod install_source;
pub mod jvm;
pub mod link_service;
pub mod native_video;
pub mod net;
pub mod pip;
pub mod screen;
pub mod storage;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app-private files directory `/data/user/0/<package>/files` — writable without any permission.
pub fn app_data_dir() -> PathBuf {
    DATA_DIR
        .get_or_init(|| {
            let dir = derive_data_dir();
            // Best-effort: the individual writers report more specific errors.
            let _ = std::fs::create_dir_all(&dir);
            dir
        })
        .clone()
}

/// Documents-equivalent for exports, inside app-private storage on purpose: shared `Documents/` is a
/// MediaStore insert on Android 10+, not a path. Sharing goes through the share sheet.
pub fn app_documents_dir() -> PathBuf {
    app_data_dir().join("Documents")
}

/// Package name from `/proc/self/cmdline` (no JNI needed); `/data/data/<pkg>` is the pre-multi-user
/// fallback for a device where `/data/user/0/<pkg>` is missing.
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

/// The package name: `cmdline` up to the first NUL, without a `:<process>` suffix.
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

/// Log the derived directory next to Tauri's own answer, so a device that disagrees shows up in the log.
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
