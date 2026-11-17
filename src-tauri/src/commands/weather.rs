// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

use crate::weather::{fetch_wind_aloft, WindAloft};

/// Wind by height and the next 90 minutes at a position, from the forecast model.
#[tauri::command]
pub async fn weather_wind_aloft(lat: f64, lon: f64) -> Result<WindAloft, String> {
    fetch_wind_aloft(lat, lon).await
}
