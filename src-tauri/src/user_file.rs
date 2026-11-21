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

/// Raw logs a logger is writing right now: these must not be moved away under it.
static OPEN_RAW_LOGS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

pub fn raw_log_opened(path: &Path) {
    OPEN_RAW_LOGS.lock().unwrap_or_else(|e| e.into_inner()).push(path.to_path_buf());
}

pub fn raw_log_closed(path: &Path) {
    OPEN_RAW_LOGS.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| p != path);
}

/// Android: move the closed raw logs from app-private staging into the SAF tree the raw-log setting
/// names (the loggers need a real path). Never fails its caller: problems are logged and the files
/// stay; a no-op elsewhere.
pub fn move_raw_logs(raw_log_dir: &Path, raw_setting: &str) {
    #[cfg(target_os = "android")]
    {
        // One move at a time: two listing the same files would write each of them twice.
        static MOVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        if !raw_setting.starts_with("content://") {
            return;
        }
        let _moving = MOVING.lock().unwrap_or_else(|e| e.into_inner());
        let listed: Vec<PathBuf> = std::fs::read_dir(raw_log_dir.join("raw_logs"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        // Read after the listing: a logger registers before it creates its file.
        let open = OPEN_RAW_LOGS.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let files: Vec<PathBuf> = listed.into_iter().filter(|p| !open.contains(p)).collect();
        if files.is_empty() {
            return;
        }
        match crate::android::storage::move_files_to_tree(raw_setting, &files) {
            Ok(()) => log::info!("{} raw log(s) moved to the shared folder", files.len()),
            Err(e) => log::warn!("Raw logs not moved to the shared folder: {e}"),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (raw_log_dir, raw_setting);
    }
}

/// Android: the leftover `db-mirror` folder holds a full second copy of the database.
pub fn remove_db_mirror_snapshot() {
    #[cfg(target_os = "android")]
    {
        let _ = std::fs::remove_dir_all(crate::android::app_data_dir().join("db-mirror"));
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

/// Where an export to `dest` takes its space from: the scratch folder for a document URI (the file is
/// built there first), otherwise the folder of `dest` itself.
pub fn write_volume(dest: &str) -> PathBuf {
    #[cfg(target_os = "android")]
    if crate::android::content::is_content_uri(dest) {
        return crate::android::app_data_dir();
    }
    Path::new(dest).parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Bytes the user can still write on the volume of the existing folder `dir`; `None` where the
/// system does not say.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `path` is a NUL-terminated string and `stat` is written only on success (0).
        let stat = unsafe {
            if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
                return None;
            }
            stat.assume_init()
        };
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` is NUL-terminated and outlives the call; `free` is a valid out pointer.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
        Some(free)
    }
}

/// A scratch path under app-private storage (Android) or the OS temp dir (desktop), unique per call.
///
/// Named from the process id and a monotonic counter rather than a random source: two exports in the
/// same session must not collide, and pulling in a RNG for a filename would be silly. On Android the
/// directory is app-private, so nothing else can see these while they exist.
pub(crate) fn temp_path(kind: &str) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    #[cfg(target_os = "android")]
    let dir = crate::android::app_data_dir().join("tmp");
    #[cfg(not(target_os = "android"))]
    let dir = std::env::temp_dir().join("kite-gc");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating the temp dir: {e}"))?;
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!("{kind}-{}-{n}", std::process::id())))
}
