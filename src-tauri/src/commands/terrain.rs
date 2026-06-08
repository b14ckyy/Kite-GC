// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Terrain elevation Tauri commands (Copernicus GLO-30, ≈ MSL).

use crate::terrain::{ProfileSample, TerrainFan, TerrainProvider};
use tauri::State;

/// Terrain elevation (metres ≈ MSL) at a single lat/lon. `null` if unavailable.
#[tauri::command]
pub async fn terrain_elevation(
    lat: f64,
    lon: f64,
    provider: State<'_, TerrainProvider>,
) -> Result<Option<f32>, String> {
    Ok(provider.elevation(lat, lon).await)
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
