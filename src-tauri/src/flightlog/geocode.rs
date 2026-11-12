// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Reverse geocoding via OSM Nominatim.
// Nominatim's usage policy: at most 1 request per second, an identifying User-Agent, cache results.
// `reverse_geocode` keeps to it for the whole app: requests are spaced by a process-wide slot
// (`next_slot`), names are remembered per ~1 km cell and language (`CACHE`) — a batch import of flights
// from one field asks once — and a request that hangs is given up after `REQUEST_TIMEOUT`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;
use tokio::time::Instant;

const NOMINATIM_URL: &str = "https://nominatim.openstreetmap.org/reverse";

/// Minimum distance between two requests (the policy's 1 s plus a margin for timer and network jitter).
const MIN_INTERVAL: Duration = Duration::from_millis(1100);
/// A request that takes longer is abandoned — callers await the name inline (imports).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound of remembered names (cleared when reached; one entry is a short string).
const CACHE_MAX: usize = 512;

/// Cache key: the position on a 0.01° grid (~1 km; the request asks for city-level names) + language.
type CellKey = (i32, i32, String);

static CACHE: OnceLock<Mutex<HashMap<CellKey, String>>> = OnceLock::new();
/// The earliest instant the next request may start.
static NEXT_SLOT: OnceLock<Mutex<Instant>> = OnceLock::new();

fn cell_key(lat: f64, lon: f64, lang: &str) -> CellKey {
    ((lat * 100.0).round() as i32, (lon * 100.0).round() as i32, lang.to_string())
}

/// Reserve the next request slot and return when it starts. Slots are `MIN_INTERVAL` apart however
/// many callers ask at once; the lock is held only to book the slot, never across the wait.
fn next_slot(now: Instant) -> Instant {
    let mut next = NEXT_SLOT
        .get_or_init(|| Mutex::new(now))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let at = (*next).max(now);
    *next = at + MIN_INTERVAL;
    at
}

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
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let key = cell_key(lat, lon, lang);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(name) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(name.clone());
    }

    tokio::time::sleep_until(next_slot(Instant::now())).await;
    // Another caller may have fetched this cell while we waited for the slot.
    if let Some(name) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Some(name.clone());
    }
    let name = request_name(lat, lon, lang).await?;

    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= CACHE_MAX {
        map.clear();
    }
    map.insert(key, name.clone());
    Some(name)
}

/// One Nominatim request (no spacing, no cache — `reverse_geocode` adds both).
async fn request_name(lat: f64, lon: f64, lang: &str) -> Option<String> {
    // One client for the app's lifetime: its kept-alive connection makes the time from `send()` to the
    // request's arrival nearly constant, so the slot spacing also holds on Nominatim's side.
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(crate::user_agent::user_agent("flight-logging"))
                .timeout(REQUEST_TIMEOUT)
                .build()
                .ok()
        })
        .as_ref()?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_spaced_even_when_booked_at_once() {
        let now = Instant::now() + Duration::from_secs(3600); // later than any slot another test booked
        let a = next_slot(now);
        let b = next_slot(now);
        let c = next_slot(now);
        assert_eq!(a, now);
        assert_eq!(b, a + MIN_INTERVAL);
        assert_eq!(c, b + MIN_INTERVAL);
        // After a quiet period the next request starts at once.
        let later = c + Duration::from_secs(30);
        assert_eq!(next_slot(later), later);
        // Hand the shared slot back, so a later test in this binary does not wait for an hour.
        *NEXT_SLOT.get().unwrap().lock().unwrap() = Instant::now();
    }

    #[test]
    fn nearby_positions_share_a_cell_and_languages_do_not() {
        assert_eq!(cell_key(51.4821, 11.9698, "de"), cell_key(51.4849, 11.9702, "de"));
        assert_ne!(cell_key(51.4821, 11.9698, "de"), cell_key(51.4821, 11.9698, "en"));
        assert_ne!(cell_key(51.48, 11.97, "de"), cell_key(51.52, 11.97, "de"));
        assert_ne!(cell_key(0.004, 0.0, "en"), cell_key(-0.006, 0.0, "en"));
    }
}
