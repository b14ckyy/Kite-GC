// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Terrain elevation Tauri commands (Copernicus GLO-30, ≈ MSL).

use crate::terrain::{ProfileSample, TerrainFan, TerrainProvider};
use serde::Serialize;
use tauri::State;

/// On-disk terrain tile cache size for the settings readout.
#[derive(Serialize)]
pub struct TerrainCacheStats {
    pub bytes: u64,
    pub count: usize,
}

/// Current size + tile count of the on-disk terrain cache.
#[tauri::command]
pub async fn terrain_cache_stats(
    provider: State<'_, TerrainProvider>,
) -> Result<TerrainCacheStats, String> {
    let (bytes, count) = provider.cache_stats();
    Ok(TerrainCacheStats { bytes, count })
}

/// Delete every cached terrain tile from disk. Returns the number of files removed.
#[tauri::command]
pub async fn terrain_cache_clear(provider: State<'_, TerrainProvider>) -> Result<usize, String> {
    Ok(provider.clear_cache())
}

/// Terrain elevation (metres ≈ MSL) at a single lat/lon. `null` if unavailable.
#[tauri::command]
pub async fn terrain_elevation(
    lat: f64,
    lon: f64,
    provider: State<'_, TerrainProvider>,
) -> Result<Option<f32>, String> {
    Ok(provider.elevation(lat, lon).await)
}

/// Terrain elevation (metres ≈ MSL) at each lat/lon, in input order. `null`
/// where unavailable. Batched so a caller resolving many points pays one IPC
/// round-trip instead of one per point.
#[tauri::command]
pub async fn terrain_elevations(
    points: Vec<(f64, f64)>,
    provider: State<'_, TerrainProvider>,
) -> Result<Vec<Option<f32>>, String> {
    Ok(provider.elevations(&points).await)
}

/// Terrain profile along a polyline of `[lat, lon]` points, sampled every
/// `spacing_m` metres (plus the exact waypoint positions).
#[tauri::command]
pub async fn terrain_profile(
    points: Vec<(f64, f64)>,
    spacing_m: f64,
    provider: State<'_, TerrainProvider>,
) -> Result<Vec<ProfileSample>, String> {
    Ok(provider.profile(&points, spacing_m).await)
}

/// Terrain sampled over a forward fan (polar grid) for the terrain-radar widget.
/// `heading_deg` true bearing, fan spans ±`half_angle_deg`, out to `range_m`,
/// `ang_cells` × `rad_cells` cells sampled at their centres.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub async fn terrain_fan(
    lat: f64,
    lon: f64,
    heading_deg: f64,
    half_angle_deg: f64,
    range_m: f64,
    ang_cells: usize,
    rad_cells: usize,
    provider: State<'_, TerrainProvider>,
) -> Result<TerrainFan, String> {
    Ok(provider
        .fan(lat, lon, heading_deg, half_angle_deg, range_m, ang_cells, rad_cells)
        .await)
}
