// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Reverse geocoding via OSM Nominatim
// Respects Nominatim usage policy: max 1 request per second, custom User-Agent.

use serde::Deserialize;

const NOMINATIM_URL: &str = "https://nominatim.openstreetmap.org/reverse";
const USER_AGENT: &str = "KiteGC/0.3.0 (flight-logging)";

#[derive(Debug, Deserialize)]
struct NominatimResponse {
    display_name: Option<String>,
    address: Option<NominatimAddress>,
}

#[derive(Debug, Deserialize)]
struct NominatimAddress {
    village: Option<String>,
    town: Option<String>,
    city: Option<String>,
    municipality: Option<String>,
    county: Option<String>,
    state: Option<String>,
    country: Option<String>,
}

/// Reverse geocode a lat/lon to a human-readable location name.
/// Returns a short name like "Garching, Bayern, Germany".
/// `lang` is a BCP 47 language tag (e.g. "en", "de") used for Nominatim's
/// `accept-language` parameter so names are returned in the UI language.
/// Returns None on error (network, parse, etc.) — never blocks the recorder.
pub async fn reverse_geocode(lat: f64, lon: f64, lang: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .ok()?;

    let resp = client
        .get(NOMINATIM_URL)
        .query(&[
            ("lat", lat.to_string()),
            ("lon", lon.to_string()),
            ("format", "json".to_string()),
            ("zoom", "10".to_string()),
            ("addressdetails", "1".to_string()),
            ("accept-language", lang.to_string()),
        ])
        .send()
        .await
        .ok()?;

    let data: NominatimResponse = resp.json().await.ok()?;

    // Build a short location name from address parts
    if let Some(addr) = data.address {
        let locality = addr
            .village
            .or(addr.town)
            .or(addr.city)
            .or(addr.municipality)
            .unwrap_or_default();
        let region = addr.county.or(addr.state).unwrap_or_default();
        let country = addr.country.unwrap_or_default();

        let parts: Vec<&str> = [locality.as_str(), region.as_str(), country.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();

        if !parts.is_empty() {
            return Some(parts.join(", "));
        }
    }

    // Fallback to display_name
    data.display_name
}
