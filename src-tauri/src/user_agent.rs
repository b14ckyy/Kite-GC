// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The HTTP `User-Agent` of Kite's outgoing requests to third-party services.
//!
//! One shape for every client: `Kite-GC/<app version> <purpose> (+<project URL>)`. Community
//! services (OSM tiles, Nominatim, the ADS-B aggregators) ask for an identifying, stable user agent
//! with a way to reach the author; the purpose token tells the requests apart. The version is the
//! app version from `package.json` (baked in by `build.rs` as `KITE_APP_VERSION`), not the crate
//! version in `Cargo.toml`. Never put user or device data in here.

const PRODUCT: &str = "Kite-GC";
const CONTACT_URL: &str = "https://github.com/b14ckyy/Kite-GC";

/// `Kite-GC/<app version> <purpose> (+https://github.com/b14ckyy/Kite-GC)`. `purpose` is a short
/// fixed token naming the feature (`tiles`, `terrain`, …).
pub fn user_agent(purpose: &str) -> String {
    format!("{PRODUCT}/{} {purpose} (+{CONTACT_URL})", env!("KITE_APP_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_the_package_json_version_and_is_a_valid_header() {
        // The single version source is package.json (build.rs parses it); read it independently here.
        let pkg = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../package.json"))
            .expect("package.json next to src-tauri");
        let pkg: serde_json::Value = serde_json::from_str(&pkg).expect("package.json parses");
        let version = pkg["version"].as_str().expect("package.json has a version");

        let ua = user_agent("tiles");
        assert_eq!(ua, format!("Kite-GC/{version} tiles (+https://github.com/b14ckyy/Kite-GC)"));
        assert!(reqwest::header::HeaderValue::from_str(&ua).is_ok(), "not a valid header value: {ua}");
    }
}
