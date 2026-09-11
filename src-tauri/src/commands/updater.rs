// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! "Update and Restart" — the install half of the update check. `update_check.rs` decides WHICH release
//! the user is told about; this module installs the one the user accepted and relaunches Kite.
//!
//! Two install paths, chosen by how the running copy was deployed:
//!
//! * **Installed** (NSIS on Windows, the `.app` bundle on macOS, `.deb` / `.rpm` / AppImage on Linux)
//!   → the Tauri updater plugin. It is pointed at the accepted release's `latest.json` manifest (not at
//!   `releases/latest`), so the release the dialog shows and the release the plugin installs can never
//!   differ — channel, patch-only and "skip" are decided in one place. The manifest's `.sig` entries are
//!   verified against the `pubkey` in tauri.conf.json; the private key lives in the Dev-Docs repo.
//! * **Portable** (the `.portable` marker beside the executable) → our own path: download the portable
//!   ZIP of the release, take the binary out of it and swap it for the running one. Windows cannot
//!   overwrite a running executable but allows renaming it, so the old one is moved to `<exe>.old`
//!   first and removed on the next start (`remove_stale_executable`). Linux overwrites in place.
//!
//! Mobile has no in-app updating on either platform; the dialog links to the store instead.

use serde::Serialize;
use tauri::AppHandle;

const PROGRESS_EVENT: &str = "update-progress";
const REPO: &str = "b14ckyy/Kite-GC";

/// How the running copy can be updated — drives which button the update dialog shows.
#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)] // each build constructs only its own variant
pub enum UpdateKind {
    /// Installed via a package the Tauri updater handles.
    Installer,
    /// Portable ZIP — own binary swap.
    Portable,
    /// Android / iOS — store link only.
    Mobile,
}

/// Download / install progress, emitted as `update-progress` while `install_update` runs.
#[derive(Serialize, Clone)]
pub struct UpdateProgress {
    pub downloaded: u64,
    /// Content length when the server sent one.
    pub total: Option<u64>,
    /// `download` while bytes arrive, `install` once the package is being applied.
    pub phase: &'static str,
}

#[tauri::command]
pub fn update_kind() -> UpdateKind {
    #[cfg(mobile)]
    {
        UpdateKind::Mobile
    }
    #[cfg(desktop)]
    {
        if crate::is_portable() {
            UpdateKind::Portable
        } else {
            UpdateKind::Installer
        }
    }
}

/// Install release `tag` (version `version`, without the `v`) and relaunch. Does not return on success:
/// the Tauri updater exits the process itself on Windows (the installer relaunches Kite), everywhere
/// else `AppHandle::restart` takes over once the swap is done. Errors come back as plain text for the
/// dialog.
#[tauri::command(async)]
pub async fn install_update(app: AppHandle, tag: String, version: String) -> Result<(), String> {
    #[cfg(mobile)]
    {
        let _ = (app, tag, version);
        Err("In-app updates are not available on this platform".to_string())
    }
    #[cfg(desktop)]
    {
        log::info!("Update: installing {tag} ({})", if crate::is_portable() { "portable" } else { "installer" });
        if crate::is_portable() {
            install_portable(&app, &tag, &version).await
        } else {
            install_bundle(&app, &tag).await
        }
    }
}

#[cfg(desktop)]
fn emit_progress(app: &AppHandle, downloaded: u64, total: Option<u64>, phase: &'static str) {
    use tauri::Emitter;
    let _ = app.emit(PROGRESS_EVENT, UpdateProgress { downloaded, total, phase });
}

/// Installed copy → the Tauri updater against the release's own manifest.
#[cfg(desktop)]
async fn install_bundle(app: &AppHandle, tag: &str) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let manifest = tauri::Url::parse(&format!("https://github.com/{REPO}/releases/download/{tag}/latest.json"))
        .map_err(|e| format!("Bad manifest URL: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![manifest])
        .map_err(|e| format!("Updater setup failed: {e}"))?
        // TEST ONLY: accept the release even when it is not newer than the running app, so the whole
        // path can be exercised against the current release. Remove before the final commit.
        .version_comparator(|_current, _remote| true)
        .build()
        .map_err(|e| format!("Updater setup failed: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Update manifest failed: {e}"))?
        .ok_or_else(|| format!("Release {tag} carries no update package for this platform"))?;

    let mut downloaded = 0u64;
    let on_chunk = {
        let app = app.clone();
        move |chunk: usize, total: Option<u64>| {
            downloaded += chunk as u64;
            emit_progress(&app, downloaded, total, "download");
        }
    };
    let on_finished = {
        let app = app.clone();
        move || emit_progress(&app, 0, None, "install")
    };
    update
        .download_and_install(on_chunk, on_finished)
        .await
        .map_err(|e| format!("Update failed: {e}"))?;
    log::info!("Update: {tag} installed, restarting");
    app.restart()
}

/// Portable copy → download the portable ZIP and swap the binary.
#[cfg(desktop)]
async fn install_portable(app: &AppHandle, tag: &str, version: &str) -> Result<(), String> {
    use std::io::Read;

    let (os, bin) = if cfg!(windows) {
        ("Windows", "kite-gc.exe")
    } else if cfg!(target_os = "linux") {
        ("Linux", "kite-gc")
    } else {
        return Err("Portable updates are not available on this platform".to_string());
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(format!("No portable build for {other}")),
    };
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate the running executable: {e}"))?;
    let dir = exe.parent().ok_or("Cannot locate the running executable")?.to_path_buf();

    // Same unified name the collect scripts produce (KiteGC_<OS>_<Arch>_<Version>_portable.zip).
    let asset = format!("KiteGC_{os}_{arch}_{version}_portable.zip");
    let url = crate::github_release::asset_url(REPO, tag, &asset);
    let client = reqwest::Client::builder()
        .user_agent(format!("Kite-GC/{} update", app.package_info().version))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length();
    let mut archive = Vec::with_capacity(total.unwrap_or(0) as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download failed: {e}"))? {
        archive.extend_from_slice(&chunk);
        emit_progress(app, archive.len() as u64, total, "download");
    }
    emit_progress(app, archive.len() as u64, total, "install");

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| format!("Bad portable archive: {e}"))?;
    let mut entry = zip.by_name(bin).map_err(|e| format!("Bad portable archive: {e}"))?;
    let mut binary = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut binary).map_err(|e| format!("Bad portable archive: {e}"))?;
    drop(entry);

    // Write beside the running file, then swap by rename (atomic on both platforms; the only way to
    // replace a running executable on Windows).
    let fresh = dir.join(format!("{bin}.new"));
    std::fs::write(&fresh, &binary).map_err(|e| format!("Cannot write the new executable: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(windows)]
    {
        let stale = stale_path(&exe);
        let _ = std::fs::remove_file(&stale);
        std::fs::rename(&exe, &stale).map_err(|e| format!("Cannot move the running executable aside: {e}"))?;
    }
    std::fs::rename(&fresh, &exe).map_err(|e| format!("Cannot replace the executable: {e}"))?;
    log::info!("Update: portable {tag} in place, restarting");
    app.restart()
}

/// `<exe>.old` — where the Windows portable swap parks the previous executable.
#[cfg(desktop)]
fn stale_path(exe: &std::path::Path) -> std::path::PathBuf {
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    exe.with_file_name(format!("{name}.old"))
}

/// Delete the previous executable a portable update left behind. Called at startup; if the old process
/// has not fully exited yet the delete fails silently and the next start gets it.
#[cfg(desktop)]
pub fn remove_stale_executable() {
    if let Ok(exe) = std::env::current_exe() {
        let stale = stale_path(&exe);
        if stale.exists() {
            match std::fs::remove_file(&stale) {
                Ok(()) => log::info!("Update: removed {}", stale.display()),
                Err(e) => log::debug!("Update: {} not removed yet: {e}", stale.display()),
            }
        }
    }
}
