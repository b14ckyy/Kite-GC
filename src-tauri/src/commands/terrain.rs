// Terrain elevation Tauri commands (Copernicus GLO-30, ≈ MSL).

use crate::terrain::{ProfileSample, TerrainProvider};
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
