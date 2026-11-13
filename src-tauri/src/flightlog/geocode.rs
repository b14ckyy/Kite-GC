// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Geocoding via OSM Nominatim: reverse (position → place name, flight logs and missions) and the map
// search (place name → positions, `search_places`).
// Nominatim's usage policy: at most 1 request per second, an identifying User-Agent, cache results,
// no search-as-you-type. Both requests keep to it for the whole app: they are spaced by one
// process-wide slot (`next_slot`) and go through one client (`client`), names are remembered per
// ~1 km cell and language (`CACHE`) — a batch import of flights from one field asks once — searches
// per normalised query and language (`SEARCH_CACHE`), and a request that hangs is given up after
// `REQUEST_TIMEOUT`. Searching on Enter only is the frontend's part (MapSearch.svelte).
// The search text is never logged: it can be someone's home address.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

const NOMINATIM_URL: &str = "https://nominatim.openstreetmap.org/reverse";
const NOMINATIM_SEARCH_URL: &str = "https://nominatim.openstreetmap.org/search";

/// Minimum distance between two requests (the policy's 1 s plus a margin for timer and network jitter).
const MIN_INTERVAL: Duration = Duration::from_millis(1100);
/// A request that takes longer is abandoned — callers await the name inline (imports).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// A refused request is reported at most this often.
const WARN_INTERVAL: Duration = Duration::from_secs(600);
/// Upper bound of remembered names (cleared when reached; one entry is a short string).
const CACHE_MAX: usize = 512;
/// Hits per search (the map search shows a short list, not an address finder).
const SEARCH_LIMIT: &str = "5";
/// Upper bound of remembered searches (cleared when reached; one entry is up to `SEARCH_LIMIT` hits).
const SEARCH_CACHE_MAX: usize = 64;

/// Cache key: the position on a 0.01° grid (~1 km; the request asks for city-level names) + language.
type CellKey = (i32, i32, String);
/// Search cache key: the normalised query (`search_key`) + language.
type SearchKey = (String, String);

static CACHE: OnceLock<Mutex<HashMap<CellKey, String>>> = OnceLock::new();
static SEARCH_CACHE: OnceLock<Mutex<HashMap<SearchKey, Vec<PlaceHit>>>> = OnceLock::new();
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

/// Report a refused request at the default log level — at most once per `WARN_INTERVAL` for both
/// request kinds together (it is the same service refusing), so a block that hits every flight of an
/// import does not fill the log. `request` names the request, `consequence` what the user sees.
fn warn_refused(status: u16, request: &str, consequence: &str) {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let mut last = LAST.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if last.is_none_or(|t| now.duration_since(t) >= WARN_INTERVAL) {
        *last = Some(now);
        log::warn!("[geocode] Nominatim refused {request} (HTTP {status}); {consequence}");
    }
}

/// One client for the app's lifetime, shared by both request kinds: its kept-alive connection makes
/// the time from `send()` to the request's arrival nearly constant, so the slot spacing also holds on
/// Nominatim's side. Its default User-Agent is the flight-logging one; the search sets its own per
/// request (a request header replaces the client's default).
fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(crate::user_agent::user_agent("flight-logging"))
                .timeout(REQUEST_TIMEOUT)
                .build()
                .ok()
        })
        .as_ref()
}

/// One Nominatim request (no spacing, no cache — `reverse_geocode` adds both).
async fn request_name(lat: f64, lon: f64, lang: &str) -> Option<String> {
    let client = client()?;

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
        .ok()?; // no network / timeout: the normal offline case, nothing to report

    // The service answered but refused (403 / 429 = blocked or rate-limited, 5xx = its own trouble).
    // Without this line a block of Kite's User-Agent is indistinguishable from "offline" in a tester's
    // log. Never logs the position.
    let status = resp.status();
    if !status.is_success() {
        warn_refused(
            status.as_u16(),
            "the location-name request",
            "flights keep an empty location until it answers again",
        );
        return None;
    }

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

// ── Map search (place name → positions) ─────────────────────────────────────────────────────────

/// One search hit for the map search (MapSearch.svelte).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceHit {
    /// Nominatim's display name (in the requested language where OSM has one).
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// `[west, south, east, north]` in degrees — the frontend fits it (a city wide, a street close).
    /// None when Nominatim sent none or a degenerate one. West > east = the box crosses 180°.
    pub bbox: Option<[f64; 4]>,
    /// Short type ("city", "village", "peak" …), when Nominatim sent a useful one.
    pub kind: Option<String>,
}

/// Why a search has no hit list. `token` is what the frontend receives — stable, never translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchError {
    /// Nothing to search for (blank text).
    EmptyQuery,
    /// No answer: no network, DNS, timeout — the normal offline case.
    Offline,
    /// The service answered with an error status (blocked, rate-limited, or its own trouble).
    Refused,
    /// The service answered, but not with a readable hit list.
    BadResponse,
}

impl SearchError {
    pub fn token(self) -> &'static str {
        match self {
            SearchError::EmptyQuery => "empty_query",
            SearchError::Offline => "offline",
            SearchError::Refused => "refused",
            SearchError::BadResponse => "bad_response",
        }
    }
}

/// Nominatim sends coordinates as strings; a number is accepted too.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StrOrNum {
    Num(f64),
    Str(String),
}

impl StrOrNum {
    fn value(&self) -> Option<f64> {
        let v = match self {
            StrOrNum::Num(n) => *n,
            StrOrNum::Str(s) => s.trim().parse().ok()?,
        };
        v.is_finite().then_some(v)
    }
}

/// One element of Nominatim's `/search?format=jsonv2` array — only the fields the map search uses.
#[derive(Debug, Deserialize)]
struct RawSearchHit {
    lat: Option<StrOrNum>,
    lon: Option<StrOrNum>,
    display_name: Option<String>,
    name: Option<String>,
    /// `[south, north, west, east]`.
    boundingbox: Option<Vec<StrOrNum>>,
    addresstype: Option<String>,
    #[serde(rename = "type")]
    osm_type: Option<String>,
}

/// The search text as it is sent and cached: whitespace collapsed; the cache key is also lower case
/// (Nominatim does not tell the two apart), together with the language.
fn search_key(query: &str, lang: &str) -> SearchKey {
    (collapse_whitespace(query).to_lowercase(), lang.to_string())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Turn a Nominatim `/search` body into hits. Hits without a usable position are dropped; a bounding
/// box that is incomplete, out of range or a single point is dropped from its hit.
fn parse_search(body: &str) -> Result<Vec<PlaceHit>, serde_json::Error> {
    let raw: Vec<RawSearchHit> = serde_json::from_str(body)?;
    Ok(raw.into_iter().filter_map(place_hit).collect())
}

fn place_hit(raw: RawSearchHit) -> Option<PlaceHit> {
    let lat = raw.lat.as_ref()?.value()?;
    let lon = raw.lon.as_ref()?.value()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let name = raw
        .display_name
        .filter(|s| !s.trim().is_empty())
        .or(raw.name.filter(|s| !s.trim().is_empty()))?;
    let bbox = raw.boundingbox.as_deref().and_then(|b| {
        let v: Vec<f64> = b.iter().filter_map(StrOrNum::value).collect();
        let [s, n, w, e] = <[f64; 4]>::try_from(v).ok()?;
        let lat_ok = |x: f64| (-90.0..=90.0).contains(&x);
        let lon_ok = |x: f64| (-180.0..=180.0).contains(&x);
        if !(lat_ok(s) && lat_ok(n) && lon_ok(w) && lon_ok(e)) || s > n || (s == n && w == e) {
            return None;
        }
        Some([w, s, e, n])
    });
    // "yes" is OSM's untyped tag value ("building=yes") — no use as a label.
    let kind = raw
        .addresstype
        .filter(|s| !s.is_empty() && s != "yes")
        .or(raw.osm_type.filter(|s| !s.is_empty() && s != "yes"));
    Some(PlaceHit { name, lat, lon, bbox, kind })
}

/// Search places by name for the map search: up to `SEARCH_LIMIT` hits, names in `lang` (a BCP 47
/// tag, Nominatim's `accept-language`). An empty list means "no hits". Shares the reverse geocoder's
/// request slot and client; answers are cached per normalised query and language.
pub async fn search_places(query: &str, lang: &str) -> Result<Vec<PlaceHit>, SearchError> {
    let key = search_key(query, lang);
    if key.0.is_empty() {
        return Err(SearchError::EmptyQuery);
    }
    let cache = SEARCH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hits) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        log::debug!("[geocode] place search answered from the cache ({} hit(s))", hits.len());
        return Ok(hits.clone());
    }

    tokio::time::sleep_until(next_slot(Instant::now())).await;
    // The same search may have been answered while we waited for the slot (a double Enter).
    if let Some(hits) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(hits.clone());
    }
    let hits = request_search(&collapse_whitespace(query), lang).await?;

    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= SEARCH_CACHE_MAX {
        map.clear();
    }
    map.insert(key, hits.clone());
    Ok(hits)
}

/// One Nominatim `/search` request (no spacing, no cache — `search_places` adds both). Logs the
/// outcome only — never the query, nor an error's text (a reqwest error carries the URL, and with it
/// the query).
async fn request_search(q: &str, lang: &str) -> Result<Vec<PlaceHit>, SearchError> {
    let Some(client) = client() else {
        log::warn!("[geocode] place search: the HTTP client could not be built");
        return Err(SearchError::Offline);
    };
    let resp = client
        .get(NOMINATIM_SEARCH_URL)
        .header(reqwest::header::USER_AGENT, crate::user_agent::user_agent("map-search"))
        .query(&[
            ("q", q),
            ("format", "jsonv2"),
            ("limit", SEARCH_LIMIT),
            ("accept-language", lang),
        ])
        .send()
        .await
        .map_err(|e| {
            log::debug!(
                "[geocode] place search: no answer from Nominatim (timeout: {}, connect: {})",
                e.is_timeout(),
                e.is_connect()
            );
            SearchError::Offline
        })?;

    let status = resp.status();
    if !status.is_success() {
        warn_refused(
            status.as_u16(),
            "the map search",
            "place names cannot be searched until it answers again (coordinates still work)",
        );
        return Err(SearchError::Refused);
    }

    let body = resp.text().await.map_err(|e| {
        log::debug!("[geocode] place search: the answer broke off (timeout: {})", e.is_timeout());
        SearchError::Offline
    })?;
    let hits = parse_search(&body).map_err(|e| {
        // Position and category only: the message itself may quote the answer.
        log::warn!(
            "[geocode] place search: unreadable Nominatim answer ({:?} at line {}, column {})",
            e.classify(),
            e.line(),
            e.column()
        );
        SearchError::BadResponse
    })?;
    log::debug!("[geocode] place search: {} hit(s)", hits.len());
    Ok(hits)
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

    #[test]
    fn search_hit_with_bounding_box() {
        // Shape of a `/search?format=jsonv2` element (coordinates and box as strings).
        let body = r#"[{
            "place_id": 1, "osm_type": "relation", "osm_id": 62637,
            "lat": "51.4825041", "lon": "11.9705452",
            "category": "boundary", "type": "administrative", "addresstype": "city",
            "name": "Halle (Saale)", "display_name": "Halle (Saale), Sachsen-Anhalt, Deutschland",
            "boundingbox": ["51.3911600", "51.5428600", "11.8604300", "12.0868400"]
        }]"#;
        let hits = parse_search(body).unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!(h.name, "Halle (Saale), Sachsen-Anhalt, Deutschland");
        assert!((h.lat - 51.4825041).abs() < 1e-9 && (h.lon - 11.9705452).abs() < 1e-9);
        // [south, north, west, east] in, [west, south, east, north] out.
        assert_eq!(h.bbox, Some([11.86043, 51.39116, 12.08684, 51.54286]));
        assert_eq!(h.kind.as_deref(), Some("city"));
    }

    #[test]
    fn search_hit_without_a_usable_bounding_box() {
        let body = r#"[
            {"lat": 47.0, "lon": "11.5", "type": "peak", "display_name": "Somewhere"},
            {"lat": "47.1", "lon": "11.6", "type": "yes", "name": "Point box",
             "boundingbox": ["47.1", "47.1", "11.6", "11.6"]},
            {"lat": "47.2", "lon": "11.7", "display_name": "Short box", "boundingbox": ["47.2", "47.3"]},
            {"lat": "95.0", "lon": "11.7", "display_name": "Off the globe"},
            {"lon": "11.7", "display_name": "No latitude"}
        ]"#;
        let hits = parse_search(body).unwrap();
        assert_eq!(hits.len(), 3, "hits without a usable position are dropped");
        assert_eq!(hits[0].bbox, None);
        assert_eq!(hits[0].kind.as_deref(), Some("peak"));
        assert_eq!(hits[1].name, "Point box", "name stands in for a missing display name");
        assert_eq!(hits[1].bbox, None, "a single-point box is no box");
        assert_eq!(hits[1].kind, None, "OSM's untyped 'yes' is no label");
        assert_eq!(hits[2].bbox, None, "an incomplete box is dropped");
    }

    #[test]
    fn search_with_no_hits_and_unreadable_answers() {
        assert_eq!(parse_search("[]").unwrap(), Vec::<PlaceHit>::new());
        assert!(parse_search(r#"{"error": "Unable to geocode"}"#).is_err());
        assert!(parse_search("<html>blocked</html>").is_err());
    }

    #[test]
    fn search_key_normalises_case_and_whitespace_but_keeps_the_language() {
        assert_eq!(search_key("  Halle   (Saale)\t", "de"), search_key("halle (saale)", "de"));
        assert_ne!(search_key("Halle", "de"), search_key("Halle", "en"));
        assert_ne!(search_key("Halle", "de"), search_key("Hall e", "de"));
        assert!(search_key(" \t ", "de").0.is_empty());
    }

    #[test]
    fn search_error_tokens_are_stable() {
        assert_eq!(SearchError::EmptyQuery.token(), "empty_query");
        assert_eq!(SearchError::Offline.token(), "offline");
        assert_eq!(SearchError::Refused.token(), "refused");
        assert_eq!(SearchError::BadResponse.token(), "bad_response");
    }
}
