// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-startup update check: which GitHub release is relevant for the user's channel. Version
//! comparison, "skip" and the prompt live in the frontend, the install in `commands::updater`.

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
    /// The highest published version of any kind.
    Prerelease,
}

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

/// One chunk of a pre-release suffix: `rc10` is `[Text("rc"), Num(10)]`, so `b2` sorts below `b10`.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreChunk {
    Num(u64),
    Text(String),
}

/// Sort key of a release tag — must keep the order of the frontend's `compareVersions`
/// (utils/version.ts): a stable version sorts above every pre-release of the same core.
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

/// The HIGHEST published version the channel accepts — not the first match: GitHub documents no
/// order for the list, and an older line's patch can stand in front of the next version's beta.
fn pick<'a>(releases: &'a [Value], channel: UpdateChannel, current: &str) -> Option<&'a Value> {
    let running = major_minor(current);
    releases
        .iter()
        .filter(|r| !is_draft(r))
        .filter(|r| match channel {
            UpdateChannel::Prerelease => true,
            UpdateChannel::Release => !is_prerelease(r),
            UpdateChannel::Patch => {
                !is_prerelease(r)
                    && r.get("tag_name").and_then(Value::as_str).and_then(major_minor) == running
                    && running.is_some()
            }
        })
        .filter_map(|r| Some((version_key(r.get("tag_name")?.as_str()?), r)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}

/// Fetch the release to compare against; `Ok(None)` when there is none (no stable release yet → 404,
/// no patch of the running minor). Failures are logged and ignored by the frontend.
#[tauri::command(async)]
pub async fn check_for_update(app: tauri::AppHandle, channel: UpdateChannel) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent(crate::user_agent::user_agent("update-check"))
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

    // Patch + pre-release channels take the highest matching version of the list (see `pick`).
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

    #[test]
    fn version_order_matches_the_frontend() {
        let ordered = ["1.0.0-b2", "1.0.0-b10", "1.0.0-rc1", "1.0.0", "1.0.1", "1.0.2", "2.0.0-b1", "2.0.0-b2", "2.0.0-rc1", "2.0.0", "10.0.0"];
        for pair in ordered.windows(2) {
            assert!(version_key(pair[0]) < version_key(pair[1]), "{} must sort below {}", pair[0], pair[1]);
        }
        assert!(version_key("v2.0.0-b1") == version_key("2.0.0-b1"));
    }

    #[test]
    fn the_list_order_does_not_decide() {
        let tag = |r: Option<&Value>| r.and_then(|r| r["tag_name"].as_str()).map(str::to_string);
        // A patch of the older line stands first (published after the beta of the next version).
        let list = vec![
            release("v1.0.3", false, false),
            release("v2.0.0-b2", true, false),
            release("v2.0.0-b3", true, true), // draft: never offered
            release("v1.0.2", false, false),
            release("v2.0.0-b1", true, false),
        ];
        assert_eq!(tag(pick(&list, UpdateChannel::Prerelease, "2.0.0-b1")), Some("v2.0.0-b2".into()));
        assert_eq!(tag(pick(&list, UpdateChannel::Prerelease, "1.0.2")), Some("v2.0.0-b2".into()));
        // Patch channel: the highest patch of the running minor, also when it is not the first one listed.
        let list = vec![
            release("v1.0.4-rc1", true, false), // a pre-release of the running minor is not a patch
            release("v1.0.2", false, false),
            release("v1.0.3", false, false),
            release("v1.1.0", false, false),
        ];
        assert_eq!(tag(pick(&list, UpdateChannel::Patch, "1.0.1")), Some("v1.0.3".into()));
        assert_eq!(tag(pick(&[], UpdateChannel::Prerelease, "1.0.0")), None);
    }
}
