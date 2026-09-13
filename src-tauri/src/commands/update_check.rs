// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-startup update check — query the GitHub Releases API for a newer published version. This command
//! only decides WHICH release is relevant for the user's channel; the frontend owns the version
//! comparison, the per-version "skip" state and the prompt, and `commands::updater` does the install
//! when the user asks for it. Mirrors the GitHub-fetch style of `flightlog::decoder`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RELEASES_LATEST: &str = "https://api.github.com/repos/b14ckyy/Kite-GC/releases/latest";
const RELEASES_LIST: &str = "https://api.github.com/repos/b14ckyy/Kite-GC/releases?per_page=20";

/// The user's channel (Settings → Updates). Mirrors `UpdateCheckMode` minus `disabled`, which never
/// reaches the backend.
#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// The latest stable release (GitHub's `/releases/latest`, which excludes drafts + pre-releases).
    Release,
    /// The latest stable release of the RUNNING minor only: 1.1.0 is told about 1.1.1, never 1.2.0.
    Patch,
    /// The newest published release of any kind.
    Prerelease,
}

/// A published release as the frontend needs it.
#[derive(Serialize)]
pub struct UpdateInfo {
    /// Release tag with any leading `v` stripped (e.g. `1.1.0` or `1.0.0-b2`) — compared frontend-side.
    pub version: String,
    /// Raw tag name (`v1.1.0`) — also the `releases/download/<tag>/…` path segment the installer uses.
    pub tag: String,
    /// The release's web page (opened in the system browser on the user's request).
    pub url: String,
    /// Release title (falls back to the tag).
    pub name: String,
    /// Whether GitHub flagged it a pre-release.
    pub prerelease: bool,
    /// The release notes as written on GitHub (markdown; rendered inside the update dialog).
    pub body: String,
}

fn to_info(r: &Value) -> Option<UpdateInfo> {
    let tag = r.get("tag_name")?.as_str()?.to_string();
    let url = r.get("html_url")?.as_str()?.to_string();
    let name = r.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&tag).to_string();
    let prerelease = r.get("prerelease").and_then(Value::as_bool).unwrap_or(false);
    let body = r.get("body").and_then(Value::as_str).unwrap_or("").to_string();
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    Some(UpdateInfo { version, tag, url, name, prerelease, body })
}

fn is_draft(r: &Value) -> bool {
    r.get("draft").and_then(Value::as_bool).unwrap_or(false)
}

fn is_prerelease(r: &Value) -> bool {
    r.get("prerelease").and_then(Value::as_bool).unwrap_or(false)
}

/// `MAJOR.MINOR` of a version string (`1.1.0-rc1` → `(1, 1)`); a leading `v` is ignored. `None` when
/// the string does not start with two dot-separated numbers.
fn major_minor(version: &str) -> Option<(u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split('-').next().unwrap_or(core);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Pick the release the channel wants out of GitHub's newest-first list. Pure — unit-tested.
fn pick<'a>(releases: &'a [Value], channel: UpdateChannel, current: &str) -> Option<&'a Value> {
    let running = major_minor(current);
    releases.iter().filter(|r| !is_draft(r)).find(|r| match channel {
        UpdateChannel::Prerelease => true,
        UpdateChannel::Release => !is_prerelease(r),
        UpdateChannel::Patch => {
            !is_prerelease(r)
                && r.get("tag_name").and_then(Value::as_str).and_then(major_minor) == running
                && running.is_some()
        }
    })
}

/// Fetch the release to compare against. Returns `Ok(None)` when there's nothing to compare (e.g. no
/// stable release exists yet → the `/latest` endpoint 404s; no patch of the running minor published).
/// Any network/parse failure is an `Err` the frontend logs and ignores — an update check never
/// disrupts use.
#[tauri::command(async)]
pub async fn check_for_update(app: tauri::AppHandle, channel: UpdateChannel) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent(format!("Kite-GC/{current} update-check"))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    if channel == UpdateChannel::Release {
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
        return Ok(to_info(&release));
    }

    // Patch + pre-release channels walk the newest-first list (drafts skipped inside `pick`).
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
    let list = releases.as_array().map(Vec::as_slice).unwrap_or(&[]);
    Ok(pick(list, channel, &current).and_then(to_info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, prerelease: bool, draft: bool) -> Value {
        json!({ "tag_name": tag, "html_url": "u", "prerelease": prerelease, "draft": draft })
    }

    #[test]
    fn major_minor_parses_the_tag_scheme() {
        assert_eq!(major_minor("1.1.0"), Some((1, 1)));
        assert_eq!(major_minor("v1.2.3-rc1"), Some((1, 2)));
        assert_eq!(major_minor("1.1.0-dev"), Some((1, 1)));
        assert_eq!(major_minor("nightly"), None);
    }

    #[test]
    fn patch_channel_stays_inside_the_running_minor() {
        let list = vec![
            release("v1.2.0-rc1", true, false),
            release("v1.2.0", false, true), // draft: never offered
            release("v1.1.2", false, false),
            release("v1.1.1", false, false),
            release("v1.0.3", false, false),
        ];
        let tag = |r: Option<&Value>| r.and_then(|r| r["tag_name"].as_str()).map(str::to_string);
        assert_eq!(tag(pick(&list, UpdateChannel::Patch, "1.1.0")), Some("v1.1.2".into()));
        assert_eq!(tag(pick(&list, UpdateChannel::Patch, "1.0.0")), Some("v1.0.3".into()));
        assert_eq!(tag(pick(&list, UpdateChannel::Patch, "1.3.0")), None);
        assert_eq!(tag(pick(&list, UpdateChannel::Release, "1.0.0")), Some("v1.1.2".into()));
        assert_eq!(tag(pick(&list, UpdateChannel::Prerelease, "1.0.0")), Some("v1.2.0-rc1".into()));
    }
}
