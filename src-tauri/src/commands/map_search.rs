// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Map search: a place name → up to five hits (OSM Nominatim, through the shared geocoder in
//! `flightlog::geocode` — same request slot, client and refusal log). Coordinates never come here:
//! the frontend recognises them itself, offline.

use crate::flightlog::geocode::{search_places, PlaceHit};

/// Search a place by name. `Ok(vec![])` = no hits. The error is a stable token for the frontend:
/// `empty_query`, `offline` (no answer), `refused` (an error status), `bad_response` (unreadable).
/// `lang` is the UI language (Nominatim's `accept-language`).
#[tauri::command]
pub async fn map_search_places(query: String, lang: Option<String>) -> Result<Vec<PlaceHit>, String> {
    search_places(&query, lang.as_deref().unwrap_or("en"))
        .await
        .map_err(|e| e.token().to_string())
}
