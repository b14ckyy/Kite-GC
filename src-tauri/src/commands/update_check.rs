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

/// One chunk of a pre-release suffix: `rc10` is `[Text("rc"), Num(10)]`, so `b2` sorts below `b10`.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreChunk {
    Num(u64),
    Text(String),
}

/// Sort key of a release tag, the same order as the frontend's `compareVersions` (utils/version.ts):
/// numeric MAJOR.MINOR.PATCH, then a stable version above every pre-release of the same core, then
/// the pre-release suffix compared naturally. A leading `v` is ignored; missing or unreadable numbers
/// count as 0.
fn version_key(tag: &str) -> ([u64; 3], bool, Vec<PreChunk>) {
    let s = tag.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = s.split_once('-').unwrap_or((s, ""));
    let mut nums = [0u64; 3];
    for (slot, part) in nums.iter_mut().zip(core.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    let mut chunks = Vec::new();
    let mut rest = pre;
    while let Some(first) = rest.chars().next() {
        let digits = first.is_ascii_digit();
        let end = rest.find(|c: char| c.is_ascii_digit() != digits).unwrap_or(rest.len());
        let (chunk, tail) = rest.split_at(end);
        chunks.push(if digits { PreChunk::Num(chunk.parse().unwrap_or(0)) } else { PreChunk::Text(chunk.to_string()) });
        rest = tail;
    }
    (nums, pre.is_empty(), chunks)
}

/// The published release with the highest version. GitHub documents no order for the release list,
/// and a patch of an older line published after a beta of the next version can stand in front of it,
/// so the first entry is not the one to compare against. Pure — unit-tested.
fn highest_published(releases: &[Value]) -> Option<&Value> {
    releases
        .iter()
        .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|r| Some((version_key(r.get("tag_name")?.as_str()?), r)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}

/// Fetch the release to compare against. `include_prerelease` = the user's "Pre-Release" channel choice:
/// when true we take the highest published release of any kind; when false only the latest **stable**
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
        // Highest published release of any kind (drafts skipped), whatever order the list is in.
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
        Ok(highest_published(list).and_then(to_info))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rel(tag: &str, draft: bool) -> Value {
        json!({ "tag_name": tag, "html_url": "https://example.invalid", "draft": draft, "prerelease": tag.contains('-') })
    }

    fn picked(releases: &[Value]) -> Option<String> {
        highest_published(releases).and_then(to_info).map(|i| i.version)
    }

    #[test]
    fn version_order_matches_the_frontend() {
        let ordered = ["1.0.0-b2", "1.0.0-b10", "1.0.0-rc1", "1.0.0", "1.0.1", "1.0.2", "2.0.0-b1", "2.0.0-b2", "2.0.0-rc1", "2.0.0", "10.0.0"];
        for pair in ordered.windows(2) {
            assert!(version_key(pair[0]) < version_key(pair[1]), "{} must sort below {}", pair[0], pair[1]);
        }
        assert!(version_key("v2.0.0-b1") == version_key("2.0.0-b1"));
    }

    #[test]
    fn a_later_patch_of_an_older_line_does_not_hide_the_beta() {
        // The patch stands first in the list (published after the beta).
        let list = [rel("v1.0.2", false), rel("v2.0.0-b1", false), rel("v1.0.1", false)];
        assert_eq!(picked(&list).as_deref(), Some("2.0.0-b1"));
    }

    #[test]
    fn drafts_are_never_picked() {
        let list = [rel("v2.0.0-b2", true), rel("v1.0.2", false), rel("v2.0.0-b1", false)];
        assert_eq!(picked(&list).as_deref(), Some("2.0.0-b1"));
        assert_eq!(picked(&[rel("v2.0.0", true)]), None);
        assert_eq!(picked(&[]), None);
    }
}
