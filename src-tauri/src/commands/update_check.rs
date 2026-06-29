// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-startup update check — query the GitHub Releases API for a newer published version. Deliberately
//! NO download / auto-updater: we only point the user at the release page so they read the notes. The
//! frontend owns the version comparison and the per-version "skip" state; this command just fetches the
//! relevant release. Mirrors the GitHub-fetch style of `flightlog::decoder`.

use serde::Serialize;
use serde_json::Value;

const RELEASES_LATEST: &str = "https://api.github.com/repos/b14ckyy/Kite-GC/releases/latest";
const RELEASES_LIST: &str = "https://api.github.com/repos/b14ckyy/Kite-GC/releases?per_page=10";
const HTTP_USER_AGENT: &str = "Kite-GC update-check";

/// A published release as the frontend needs it.
#[derive(Serialize)]
pub struct UpdateInfo {
    /// Release tag with any leading `v` stripped (e.g. `1.1.0` or `1.0.0-b2`) — compared frontend-side.
    pub version: String,
    /// Raw tag name (`v1.1.0`).
    pub tag: String,
    /// The release's web page (opened in the system browser on the user's request).
    pub url: String,
    /// Release title (falls back to the tag).
    pub name: String,
    /// Whether GitHub flagged it a pre-release.
    pub prerelease: bool,
}

fn to_info(r: &Value) -> Option<UpdateInfo> {
    let tag = r.get("tag_name")?.as_str()?.to_string();
    let url = r.get("html_url")?.as_str()?.to_string();
    let name = r.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&tag).to_string();
    let prerelease = r.get("prerelease").and_then(Value::as_bool).unwrap_or(false);
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    Some(UpdateInfo { version, tag, url, name, prerelease })
}

/// Fetch the release to compare against. `include_prerelease` = the user's "Pre-Release" channel choice:
/// when true we take the newest published release of any kind; when false only the latest **stable**
/// release (GitHub's `/releases/latest`, which excludes drafts + pre-releases). Returns `Ok(None)` when
/// there's nothing to compare (e.g. no stable release exists yet → the `/latest` endpoint 404s). Any
/// network/parse failure is an `Err` the frontend logs and ignores — an update check never disrupts use.
#[tauri::command(async)]
pub async fn check_for_update(include_prerelease: bool) -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    if include_prerelease {
        // Newest published release of any kind; the list is newest-first, skip drafts.
        let releases: Value = client
            .get(RELEASES_LIST)
            .send()
            .await
            .map_err(|e| format!("Release query failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Release query failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Release JSON parse failed: {e}"))?;
        let first = releases
            .as_array()
            .and_then(|a| a.iter().find(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false)));
        Ok(first.and_then(to_info))
    } else {
        // Latest stable (non-prerelease, non-draft). No stable release yet → 404 → nothing to compare.
        let resp = client
            .get(RELEASES_LATEST)
            .send()
            .await
            .map_err(|e| format!("Release query failed: {e}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let release: Value = resp
            .error_for_status()
            .map_err(|e| format!("Release query failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Release JSON parse failed: {e}"))?;
        Ok(to_info(&release))
    }
}
