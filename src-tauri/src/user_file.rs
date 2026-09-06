// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! One place to deal with the file location a *user* picked in a save/open dialog.
//!
//! On desktop that is a filesystem path and everything is trivial. On Android the dialog goes through
//! the Storage Access Framework and hands back a `content://` document URI, which `std::fs` cannot
//! open at all — so every exporter and importer that took the dialog's answer and called
//! `Path::new(&s)` silently failed on mobile.
//!
//! Rather than teach a dozen exporters to speak `ContentResolver`, these two helpers give them what
//! they already understand: a real path. When the destination is a URI the work happens in a
//! temporary file and is copied across afterwards (or copied in first, for a read). Exporters keep
//! writing zip archives and CSV through ordinary `std::io` and need no Android knowledge.
//!
//! On every non-Android target both helpers are a direct pass-through — no temp file, no copy.

use std::path::{Path, PathBuf};

/// Mirror a finished session's artefacts into user-granted shared folders, where the settings name
/// one — the Android half of the custom database / raw-log location settings. On Android those
/// settings hold SAF **tree URIs** (`content://…`): the app itself always works app-private,
/// because SQLite and the raw-log writers need real POSIX paths, which a SAF grant cannot provide.
/// This copies the results out at session end instead — which is exactly what makes them survive
/// an uninstall, the reason a user picks a folder at all.
///
/// The raw-log staging dir is mirrored file-by-file (same-size files skipped — raw logs never
/// change after close). The database is snapshotted first with `VACUUM INTO` — an atomic,
/// consistent copy even while other connections hold the live file — and the snapshot dir is then
/// mirrored the same way.
///
/// Never fails the teardown: every problem is logged and swallowed. On every non-Android target
/// the settings never hold a `content://` value and this is a no-op.
pub fn mirror_session(db_path: &Path, raw_log_dir: &Path, db_setting: &str, raw_setting: &str) {
    #[cfg(target_os = "android")]
    {
        if raw_setting.starts_with("content://") {
            match crate::android::storage::sync_dir_to_tree(
                raw_setting,
                &raw_log_dir.to_string_lossy(),
            ) {
                Ok(()) => log::info!("Raw logs mirrored to the shared folder"),
                Err(e) => log::warn!("Raw-log mirror failed: {e}"),
            }
        }
        if db_setting.starts_with("content://") {
            let snap_dir = crate::android::app_data_dir().join("db-mirror");
            if let Err(e) = std::fs::create_dir_all(&snap_dir) {
                log::warn!("Database mirror: cannot create the snapshot dir: {e}");
                return;
            }
            let snap = snap_dir.join("flights.db");
            // VACUUM INTO refuses an existing destination; the previous snapshot is disposable.
            let _ = std::fs::remove_file(&snap);
            let result = rusqlite::Connection::open(db_path)
                .map_err(|e| e.to_string())
                .and_then(|conn| {
                    conn.execute("VACUUM INTO ?1", [snap.to_string_lossy().as_ref()])
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(()) => match crate::android::storage::sync_dir_to_tree(
                    db_setting,
                    &snap_dir.to_string_lossy(),
                ) {
                    Ok(()) => log::info!("Database snapshot mirrored to the shared folder"),
                    Err(e) => log::warn!("Database mirror failed: {e}"),
                },
                Err(e) => log::warn!("Database snapshot (VACUUM INTO) failed: {e}"),
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (db_path, raw_log_dir, db_setting, raw_setting);
    }
}

/// Run `f` with a real path to write to, then deliver the result to `dest`.
///
/// `dest` is whatever the save dialog returned. `f` must have finished writing (and dropped any
/// handle) by the time it returns, because the copy happens immediately afterwards.
pub fn with_write_path<T, F>(dest: &str, f: F) -> Result<T, String>
where
    F: FnOnce(&Path) -> Result<T, String>,
{
    #[cfg(target_os = "android")]
    {
        if crate::android::content::is_content_uri(dest) {
            let tmp = temp_path("export")?;
            let out = f(&tmp);
            let delivered = match &out {
                Ok(_) => crate::android::content::write_uri(dest, &tmp),
                // The exporter already failed; its error is the useful one, so don't overwrite it
                // with a copy failure for a file that was never finished.
                Err(_) => Ok(()),
            };
            let _ = std::fs::remove_file(&tmp);
            delivered?;
            return out;
        }
    }
    f(Path::new(dest))
}

/// Run `f` with a real path to read from, materialising `src` locally first when it is a URI.
pub fn with_read_path<T, F>(src: &str, f: F) -> Result<T, String>
where
    F: FnOnce(&Path) -> Result<T, String>,
{
    #[cfg(target_os = "android")]
    {
        if crate::android::content::is_content_uri(src) {
            let tmp = temp_path("import")?;
            let fetched = crate::android::content::read_uri(src, &tmp);
            let out = match fetched {
                Ok(()) => f(&tmp),
                Err(e) => Err(e),
            };
            let _ = std::fs::remove_file(&tmp);
            return out;
        }
    }
    f(Path::new(src))
}

/// A scratch path under app-private storage, unique per call.
///
/// Named from the process id and a monotonic counter rather than a random source: two exports in the
/// same session must not collide, and pulling in a RNG for a filename would be silly. The directory
/// is app-private, so nothing else can see these while they exist.
#[cfg(target_os = "android")]
fn temp_path(kind: &str) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let dir = crate::android::app_data_dir().join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating the temp dir: {e}"))?;
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!("{kind}-{}-{n}", std::process::id())))
}

#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn temp_path(_kind: &str) -> Result<PathBuf, String> {
    unreachable!("temp files are only needed for Android content URIs")
}
