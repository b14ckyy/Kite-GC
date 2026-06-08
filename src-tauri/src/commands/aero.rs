// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Airspace Manager (aeronautical data) commands — on-demand region fetch + RAM-cache stats/clear.

use serde::Serialize;
use tauri::State;

use crate::aero::{now_ms, openaip, AeroCache, AeroData};
use crate::state::AppState;

/// Fetch the enabled aero layers for a region (cached; only re-fetched when the requested circle no
/// longer fits the cached one or the provider/key/layers change).
#[tauri::command]
pub async fn aero_fetch(
    provider: String,
    api_key: String,
    lat: f64,
    lon: f64,
    radius_km: f64,
    layers: Vec<String>,
    state: State<'_, AppState>,
) -> Result<AeroData, String> {
    if provider == "none" || provider.is_empty() || layers.is_empty() {
        return Ok(AeroData::default());
    }
    // Serve from cache when it covers the request (lock released before the network call).
    {
        let guard = state.aero.lock().map_err(|e| e.to_string())?;
        if let Some(c) = guard.as_ref() {
            if c.covers(&provider, &api_key, &layers, lat, lon) {
                return Ok(c.data.clone());
            }
        }
    }
    let data = match provider.as_str() {
        "openaip" => openaip::fetch(&api_key, lat, lon, radius_km, &layers).await?,
        other => return Err(format!("unknown aero provider: {other}")),
    };
    {
        let mut guard = state.aero.lock().map_err(|e| e.to_string())?;
        *guard = Some(AeroCache {
            provider,
            key: api_key,
            layers,
            center: (lat, lon),
            radius_km,
            data: data.clone(),
            fetched_at_ms: now_ms(),
        });
    }
    Ok(data)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AeroCacheStats {
    pub airspaces: usize,
    pub obstacles: usize,
    pub airports: usize,
    pub rc_airfields: usize,
    pub total: usize,
    pub approx_bytes: usize,
    pub age_ms: u128,
}

#[tauri::command]
pub fn aero_cache_stats(state: State<'_, AppState>) -> Result<AeroCacheStats, String> {
    let guard = state.aero.lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(c) => {
            let d = &c.data;
            let total = d.airspaces.len() + d.obstacles.len() + d.airports.len() + d.rc_airfields.len();
            let approx_bytes = serde_json::to_vec(d).map(|v| v.len()).unwrap_or(0);
            Ok(AeroCacheStats {
                airspaces: d.airspaces.len(),
                obstacles: d.obstacles.len(),
                airports: d.airports.len(),
                rc_airfields: d.rc_airfields.len(),
                total,
                approx_bytes,
                age_ms: now_ms().saturating_sub(c.fetched_at_ms),
            })
        }
        None => Ok(AeroCacheStats {
            airspaces: 0, obstacles: 0, airports: 0, rc_airfields: 0, total: 0, approx_bytes: 0, age_ms: 0,
        }),
    }
}

#[tauri::command]
pub fn aero_cache_clear(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.aero.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}
