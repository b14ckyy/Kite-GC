// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-demand download of INAV's `blackbox_decode` — Kite's one external runtime dependency.
//!
//! The decoder is intentionally NOT bundled, so it can be updated independently when a new INAV
//! version changes the log format. When it's missing, this module fetches it from the
//! iNavFlight/blackbox-tools GitHub releases and installs it into a writable app-data `bin/` dir
//! (which `blackbox::find_decoder` also searches).
//!
//! Windows releases ship a `.zip` containing `bin/blackbox_decode.exe`, which we unpack with the
//! `zip` crate. macOS/Linux/FreeBSD releases ship `.tar.zst`, which we don't unpack here — those
//! users install manually (the error points them at the releases page).

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

const RELEASES_API: &str = "https://api.github.com/repos/iNavFlight/blackbox-tools/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/iNavFlight/blackbox-tools/releases";
const HTTP_USER_AGENT: &str = "Kite-GC blackbox-decode-fetch";

/// Filename of the decoder for the current platform.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "blackbox_decode.exe"
    } else {
        "blackbox_decode"
    }
}

/// Writable directory we install the downloaded decoder into. Portable → `<exe>/data/bin`, else the
/// platform AppData (`<AppData>/kite-gc/bin` on Windows). `blackbox::find_decoder` searches this too,
/// so a once-downloaded decoder is found on every later run.
pub fn install_dir() -> PathBuf {
    if crate::is_portable() {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        {
            return dir.join("data").join("bin");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("kite-gc").join("bin");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("kite-gc")
                .join("bin");
        }
    }
    PathBuf::from("bin")
}

/// True when the decoder is already present anywhere we look (PATH, exe dir, or our install dir).
pub fn available() -> bool {
    super::blackbox::find_decoder().is_some()
}

/// Query the installed decoder's version by running `blackbox_decode --version`. Returns the trimmed
/// first output line (e.g. `"9.0.0 INAV 1918a75"`), or `None` if the decoder is absent or the call
/// fails. Works for both the auto-downloaded binary and one the user placed themselves — the only
/// reliable way to know the version, since `blackbox_decode` carries no Windows file-version resource.
pub fn version() -> Option<String> {
    let decoder = super::blackbox::find_decoder()?;
    let mut cmd = std::process::Command::new(&decoder);
    cmd.arg("--version");
    // Don't flash a console window on Windows (this runs whenever Settings is opened).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        String::from_utf8_lossy(&output.stdout)
    };
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// Download + extract `blackbox_decode` from the latest GitHub release into `install_dir()`, reporting
/// coarse progress (0..100) via the callback. Returns the installed binary path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    // Per-OS release packaging: Windows ships a .zip (bin/blackbox_decode.exe); Linux/macOS/FreeBSD
    // ship a .tar.zst (bin/blackbox_decode). Android isn't supported (no blackbox import there).
    let (os_key, ext, is_zip) = if cfg!(target_os = "windows") {
        ("windows", ".zip", true)
    } else if cfg!(target_os = "macos") {
        ("macos", ".tar.zst", false)
    } else if cfg!(target_os = "linux") {
        ("linux", ".tar.zst", false)
    } else {
        return Err(format!(
            "Automatic download isn't supported on this platform. Please install blackbox_decode manually from {}",
            RELEASES_PAGE
        ));
    };

    report(5, "Querying latest release");
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

    // Match the asset by OS + arch + extension. Arch needs aliases: the release names the Linux x64
    // build "x64_64" (not "x86_64") and ARM as "aarch64". Prefer an arch match, then any OS asset.
    let arch_aliases: Vec<&str> = match std::env::consts::ARCH {
        "x86_64" => vec!["x86_64", "x86-64", "x64", "amd64"],
        "aarch64" => vec!["aarch64", "arm64"],
        other => vec![other],
    };
    let pick = |pred: &dyn Fn(&str) -> bool| -> Option<(String, String)> {
        assets.iter().find_map(|a| {
            let name = a.get("name").and_then(Value::as_str)?;
            let url = a.get("browser_download_url").and_then(Value::as_str)?;
            pred(name).then(|| (name.to_string(), url.to_string()))
        })
    };
    let (asset_name, url) = pick(&|n| {
        n.contains(os_key) && n.ends_with(ext) && arch_aliases.iter().any(|a| n.contains(a))
    })
    .or_else(|| pick(&|n| n.contains(os_key) && n.ends_with(ext)))
    .ok_or_else(|| format!("No blackbox_decode {os_key} asset found in the latest release"))?;

    report(25, "Downloading blackbox_decode");
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
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let target = dir.join(binary_name());
    if is_zip {
        extract_from_zip(&bytes, &target)?;
    } else {
        extract_from_tar_zst(&bytes, &target)?;
    }

    report(100, "Done");
    log::info!("blackbox_decode installed from {} -> {}", asset_name, target.display());
    eprintln!("[BBX-DECODE] installed {} from {}", target.display(), asset_name);
    Ok(target)
}

/// On Unix, make the freshly written decoder executable (the tar/zip entry mode isn't preserved).
#[cfg(unix)]
fn make_executable(target: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_executable(_target: &Path) {}

/// Pull the `blackbox_decode.exe` entry out of the downloaded Windows zip and write it to `target`.
/// The archive nests it under `bin/`, so match on the file name regardless of its folder.
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
            make_executable(target);
            return Ok(());
        }
    }
    Err(format!("'{}' was not found inside the downloaded archive", want))
}

/// Pull `blackbox_decode` out of the downloaded Linux/macOS `.tar.zst` (a zstd-compressed tarball) and
/// write it to `target`. Matches on the file name (the archive nests it under `bin/`).
fn extract_from_tar_zst(archive_bytes: &[u8], target: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(archive_bytes);
    let decoder = zstd::Decoder::new(cursor).map_err(|e| format!("zstd init error: {e}"))?;
    let mut archive = tar::Archive::new(decoder);
    let want = binary_name();

    for entry in archive.entries().map_err(|e| format!("Bad tar archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("Tar read error: {e}"))?;
        let is_match = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .as_deref()
            == Some(want);
        if is_match {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("Tar extract error: {e}"))?;
            std::fs::write(target, &buf)
                .map_err(|e| format!("Cannot write {}: {e}", target.display()))?;
            make_executable(target);
            return Ok(());
        }
    }
    Err(format!("'{}' was not found inside the downloaded archive", want))
}
