// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! API-free GitHub release resolution for the on-demand helper binaries.
//!
//! All three sideloads (`blackbox_decode`, `ffmpeg`, `go2rtc`) used to resolve their download through
//! `api.github.com/repos/<repo>/releases/latest`. Unauthenticated, that endpoint allows **60 requests
//! per hour per IP** and answers **HTTP 403** once the budget is gone — and the budget is shared by
//! everyone behind the same NAT/CGNAT address. A Raspberry Pi on an LTE link can therefore fail on its
//! very first attempt, through no fault of its own (observed 2026-07-25 on the UAV-Link Pi; reproduced
//! from the dev machine minutes later).
//!
//! The plain `github.com/<repo>/releases/…` paths are CDN-backed and not rate-limited, so we use those:
//!
//! * **fixed asset names** (go2rtc, BtbN ffmpeg) → `releases/latest/download/<name>` redirects straight
//!   to that asset of the newest release. One request, nothing to parse.
//! * **versioned asset names** (blackbox-tools ships `blackbox-tools-9.0.0_linux-aarch64.tar.zst`) →
//!   `releases/latest` answers `302` with the tag in the `Location` header, and
//!   `releases/expanded_assets/<tag>` returns the asset list — the same fragment the release page
//!   itself lazy-loads.
//!
//! The same approach is already used by `scripts/fetch-ffmpeg-macos.sh` for the CI sidecar, for exactly
//! the same reason.

use std::time::Duration;

/// Bound every metadata request: these run inside the guided download, and a hung connection would
/// leave the progress bar sitting at 5 % forever.
const META_TIMEOUT: Duration = Duration::from_secs(20);

/// Download URL for an asset whose file name is identical in every release.
pub fn latest_asset_url(repo: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/latest/download/{asset}")
}

/// Download URL for a named asset of a specific release tag.
pub fn asset_url(repo: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset}")
}

/// The tag of the latest release, read from the redirect `github.com/<repo>/releases/latest` issues
/// towards `…/releases/tag/<tag>`. Needs a client that does **not** follow redirects, so it builds its
/// own rather than borrowing the caller's.
pub async fn latest_tag(user_agent: &str, repo: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(META_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let resp = client
        .get(format!("https://github.com/{repo}/releases/latest"))
        .send()
        .await
        .map_err(|e| format!("Release lookup failed: {e}"))?;
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or("Release lookup failed: no redirect to the latest tag")?;
    tag_from_location(location)
        .ok_or_else(|| format!("Release lookup failed: unexpected redirect '{location}'"))
}

/// Asset file names of a release, from the release page's own `expanded_assets` fragment.
pub async fn release_assets(user_agent: &str, repo: &str, tag: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(META_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let html = client
        .get(format!("https://github.com/{repo}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .map_err(|e| format!("Asset listing failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Asset listing failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Asset listing read failed: {e}"))?;
    let assets = parse_expanded_assets(&html, tag);
    if assets.is_empty() {
        return Err(format!("No downloadable assets found for {repo} {tag}"));
    }
    Ok(assets)
}

/// Pull the tag out of a `…/releases/tag/<tag>` redirect target (absolute or relative).
fn tag_from_location(location: &str) -> Option<String> {
    let tag = location.rsplit_once("/releases/tag/")?.1;
    let tag = tag.split(['?', '#']).next().unwrap_or(tag).trim_end_matches('/');
    (!tag.is_empty()).then(|| tag.to_string())
}

/// Extract asset file names from an `expanded_assets` fragment: each one appears as an
/// `href="/<owner>/<repo>/releases/download/<tag>/<name>"`. Pure — unit-tested.
fn parse_expanded_assets(html: &str, tag: &str) -> Vec<String> {
    let needle = format!("/releases/download/{tag}/");
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(rel) = html[from..].find(&needle) {
        let start = from + rel + needle.len();
        let end = html[start..]
            .find(|c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c.is_whitespace())
            .map(|i| start + i)
            .unwrap_or(html.len());
        if end > start {
            let name = &html[start..end];
            if !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
        from = (start + 1).max(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_from_redirect_location() {
        assert_eq!(
            tag_from_location("https://github.com/iNavFlight/blackbox-tools/releases/tag/v9.0.0"),
            Some("v9.0.0".to_string())
        );
        assert_eq!(
            tag_from_location("/AlexxIT/go2rtc/releases/tag/v1.9.9/"),
            Some("v1.9.9".to_string())
        );
        assert_eq!(tag_from_location("https://github.com/login"), None);
    }

    #[test]
    fn assets_from_expanded_fragment() {
        // Shape of the real fragment (attributes trimmed); the same asset appears twice in GitHub's
        // markup, so duplicates must collapse.
        let html = r#"
<ul class="Box">
  <li><a href="/iNavFlight/blackbox-tools/releases/download/v9.0.0/blackbox-tools-9.0.0_linux-aarch64.tar.zst" rel="nofollow">
    <span>blackbox-tools-9.0.0_linux-aarch64.tar.zst</span></a></li>
  <li><a href="/iNavFlight/blackbox-tools/releases/download/v9.0.0/blackbox-tools-9.0.0_windows-x86_64.zip">x</a></li>
  <li><a href="/iNavFlight/blackbox-tools/releases/download/v9.0.0/blackbox-tools-9.0.0_linux-aarch64.tar.zst">dup</a></li>
  <li><a href="/iNavFlight/blackbox-tools/archive/refs/tags/v9.0.0.tar.gz">source</a></li>
</ul>"#;
        let assets = parse_expanded_assets(html, "v9.0.0");
        assert_eq!(
            assets,
            vec![
                "blackbox-tools-9.0.0_linux-aarch64.tar.zst".to_string(),
                "blackbox-tools-9.0.0_windows-x86_64.zip".to_string(),
            ]
        );
    }

    #[test]
    fn fixed_asset_urls() {
        assert_eq!(
            latest_asset_url("AlexxIT/go2rtc", "go2rtc_linux_arm64"),
            "https://github.com/AlexxIT/go2rtc/releases/latest/download/go2rtc_linux_arm64"
        );
        assert_eq!(
            asset_url("iNavFlight/blackbox-tools", "v9.0.0", "x.zip"),
            "https://github.com/iNavFlight/blackbox-tools/releases/download/v9.0.0/x.zip"
        );
    }
}
