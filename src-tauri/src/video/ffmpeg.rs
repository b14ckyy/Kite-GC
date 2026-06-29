// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-demand ffmpeg — the RTSP video bridge's external runtime dependency. Mirrors the
//! `blackbox_decode` model (`flightlog::decoder`): not bundled in the installer, discovered next to
//! the app / on PATH / in the writable app-data `bin/` dir, and fetched on demand from
//! BtbN/FFmpeg-Builds. Windows ships a self-contained `.zip` we unpack here; other OSes install
//! manually for now (the error points at the releases page), exactly like the decoder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// BtbN asset selector for this OS+arch: (filename substring, archive extension), or None when we
/// don't auto-download here (manual install). Windows = self-contained `.zip`; Linux = static
/// `.tar.xz` (x86_64 / aarch64). armv7/32-bit and other platforms are intentionally unsupported.
fn asset_match() -> Option<(&'static str, &'static str)> {
    if cfg!(target_os = "windows") {
        Some(("win64", ".zip"))
    } else if cfg!(target_os = "linux") {
        match std::env::consts::ARCH {
            "x86_64" => Some(("linux64", ".tar.xz")),
            "aarch64" => Some(("linuxarm64", ".tar.xz")),
            _ => None,
        }
    } else {
        None
    }
}

/// User-facing "do it yourself" message when auto-download isn't available/possible.
fn manual_install_msg() -> String {
    format!(
        "Automatic ffmpeg download isn't available for this system. Install ffmpeg manually (e.g. from \
         {}, or your distro's package manager) and place it next to the app, on your PATH, or in {}.",
        RELEASES_PAGE,
        crate::flightlog::decoder::install_dir().display()
    )
}

/// Download ffmpeg into the app-data `bin/` dir, reporting coarse progress (0..100). Windows
/// (self-contained GPL `.zip`) + Linux x86_64/aarch64 (static GPL `.tar.xz`, unpacked via the system
/// `tar`). Other platforms/arches return a manual-install hint. Returns the binary path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    let (want_substr, want_ext) = asset_match().ok_or_else(manual_install_msg)?;

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

    // Self-contained static GPL build for this platform (e.g. ffmpeg-master-latest-win64-gpl.zip /
    // -linux64-gpl.tar.xz / -linuxarm64-gpl.tar.xz), NOT the -shared variant (needs separate libs).
    let (asset_name, url) = assets
        .iter()
        .find_map(|a| {
            let name = a.get("name").and_then(Value::as_str)?;
            let url = a.get("browser_download_url").and_then(Value::as_str)?;
            (name.contains(want_substr)
                && name.contains("-gpl")
                && !name.contains("shared")
                && name.ends_with(want_ext))
            .then(|| (name.to_string(), url.to_string()))
        })
        .ok_or_else(|| format!("No {want_substr} GPL ffmpeg {want_ext} asset found in the latest release"))?;

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
    if want_ext == ".zip" {
        extract_from_zip(&bytes, &target)?;
    } else {
        extract_from_tar_xz(&bytes, &target, &dir)?;
    }
    make_executable(&target)?;

    report(100, "Done");
    log::info!("ffmpeg installed from {} -> {}", asset_name, target.display());
    Ok(target)
}

/// Extract the ffmpeg binary from a BtbN `.tar.xz` (it nests it under `<root>/bin/ffmpeg`) using the
/// system `tar` (with xz). If `tar`/`xz` isn't available or extraction fails, returns a manual-install
/// hint — we don't bundle an xz decoder.
fn extract_from_tar_xz(bytes: &[u8], target: &Path, dir: &Path) -> Result<(), String> {
    let archive = dir.join("ffmpeg-download.tar.xz");
    let out = dir.join("ffmpeg-download-extract");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::write(&archive, bytes)
        .map_err(|e| format!("Cannot write {}: {e}", archive.display()))?;
    std::fs::create_dir_all(&out).map_err(|e| format!("Cannot create {}: {e}", out.display()))?;

    let cleanup = || {
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&out);
    };

    // `-xJf` = extract + xz-decompress. Needs `tar` (and xz support) on the system — universal on
    // desktop distros, occasionally absent on minimal images.
    let status = Command::new("tar")
        .arg("-xJf")
        .arg(&archive)
        .arg("-C")
        .arg(&out)
        .status();
    let ok = match status {
        Ok(s) if s.success() => true,
        Ok(_) => false,
        Err(_) => {
            cleanup();
            return Err(format!(
                "Could not run `tar` to unpack ffmpeg (is `tar` with xz support installed?). {}",
                manual_install_msg()
            ));
        }
    };
    if !ok {
        cleanup();
        return Err(format!("`tar` failed to unpack the ffmpeg archive. {}", manual_install_msg()));
    }

    let found = find_file(&out, binary_name());
    let result = match found {
        Some(src) => std::fs::copy(&src, target)
            .map(|_| ())
            .map_err(|e| format!("Cannot place ffmpeg at {}: {e}", target.display())),
        None => Err(format!("'{}' was not found inside the downloaded archive", binary_name())),
    };
    cleanup();
    result
}

/// Recursively find the first file named `name` under `dir`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file(&p, name) {
                return Some(found);
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}

/// Mark a freshly written binary executable (no-op on Windows).
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(|e| format!("Cannot stat {}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("Cannot chmod {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
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
