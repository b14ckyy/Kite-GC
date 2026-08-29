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
