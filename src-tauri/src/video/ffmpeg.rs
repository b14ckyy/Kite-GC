// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-demand ffmpeg — the RTSP video bridge's external runtime dependency. Mirrors the
//! `blackbox_decode` model (`flightlog::decoder`): not bundled in the installer, discovered next to
//! the app / on PATH / in the writable app-data `bin/` dir, and fetched on demand from
//! BtbN/FFmpeg-Builds. Windows ships a self-contained `.zip` we unpack here; other OSes install
//! manually for now (the error points at the releases page), exactly like the decoder.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

const RELEASES_API: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases";
const HTTP_USER_AGENT: &str = "Kite-GC ffmpeg-fetch";

/// Filename of ffmpeg for the current platform.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Discover ffmpeg: next to the exe → app-data install dir (where the download lands) → PATH. Same
/// search order + install dir as `blackbox::find_decoder`, so a once-downloaded ffmpeg is found later.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = binary_name();

    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        let c = dir.join(name);
        if c.is_file() {
            return Some(c);
        }
    }

    let installed = crate::flightlog::decoder::install_dir().join(name);
    if installed.is_file() {
        return Some(installed);
    }

    let path_var = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&path_var) {
        let c = d.join(name);
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// True when ffmpeg is present anywhere we look.
pub fn available() -> bool {
    find_ffmpeg().is_some()
}

/// First line of `ffmpeg -version` (e.g. "ffmpeg version n7.1 ..."), or None if absent/failed.
pub fn version() -> Option<String> {
    let ff = find_ffmpeg()?;
    let mut cmd = std::process::Command::new(&ff);
    cmd.arg("-version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW — don't flash a console
    }
    let out = cmd.output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
}

/// Download ffmpeg into the app-data `bin/` dir, reporting coarse progress (0..100). Windows-only for
/// now (self-contained GPL `.zip`); other OSes return a manual-install hint. Returns the binary path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    if !cfg!(target_os = "windows") {
        return Err(format!(
            "Automatic ffmpeg download is currently Windows-only. Install ffmpeg manually (e.g. from \
             {}) and place it next to the app or on PATH.",
            RELEASES_PAGE
        ));
    }

    report(5, "Querying latest ffmpeg release");
    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let release: Value = client
        .get(RELEASES_API)
        .send()
        .await
        .map_err(|e| format!("Release query failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Release query failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Release JSON parse failed: {e}"))?;

    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("Latest release has no downloadable assets")?;

    // Self-contained static GPL win64 build (e.g. ffmpeg-master-latest-win64-gpl.zip), NOT the
    // -shared variant (which needs separate DLLs).
    let (asset_name, url) = assets
        .iter()
        .find_map(|a| {
            let name = a.get("name").and_then(Value::as_str)?;
            let url = a.get("browser_download_url").and_then(Value::as_str)?;
            (name.contains("win64")
                && name.contains("-gpl")
                && !name.contains("shared")
                && name.ends_with(".zip"))
            .then(|| (name.to_string(), url.to_string()))
        })
        .ok_or("No win64 GPL ffmpeg .zip asset found in the latest release")?;

    report(25, "Downloading ffmpeg (~80 MB)");
    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Download read failed: {e}"))?;

    report(70, "Extracting");
    let dir = crate::flightlog::decoder::install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let target = dir.join(binary_name());
    extract_from_zip(&bytes, &target)?;

    report(100, "Done");
    log::info!("ffmpeg installed from {} -> {}", asset_name, target.display());
    Ok(target)
}

/// Extract the ffmpeg binary from the release zip by basename (BtbN nests it under `<root>/bin/`).
fn extract_from_zip(zip_bytes: &[u8], target: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Bad zip archive: {e}"))?;
    let want = binary_name();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Zip read error: {e}"))?;
        if !entry.is_file() {
            continue;
        }
        let entry_name = entry.name().replace('\\', "/");
        if entry_name.rsplit('/').next() == Some(want) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("Zip extract error: {e}"))?;
            std::fs::write(target, &buf)
                .map_err(|e| format!("Cannot write {}: {e}", target.display()))?;
            return Ok(());
        }
    }
    Err(format!("'{}' was not found inside the downloaded archive", want))
}
