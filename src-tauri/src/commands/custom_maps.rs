// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Tauri commands for custom (offline) maps — desktop only (see `custom_maps`). Every command refuses
//! on mobile; the frontend hides the feature there too (`hasCustomMaps`). All file work runs on a
//! blocking thread, never on the async runtime or the UI thread.

use crate::custom_maps::{self, coverage, MapInfo, TilePlan};
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;

/// Cap on concurrent tile reads / existence checks. Each one occupies a blocking-pool thread; on a
/// dead network share a stat or read can hang for a long time, and without a cap one pan of the map
/// (dozens of tiles) could fill the whole blocking pool and stall every other blocking task.
const MAX_CONCURRENT_TILE_IO: usize = 8;

fn tile_io_gate() -> Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_TILE_IO))).clone()
}

fn refuse_mobile() -> Result<(), String> {
    if cfg!(mobile) {
        return Err("Custom maps are not available on this platform".into());
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("custom maps task failed: {e}"))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMapsListing {
    /// The folder that was scanned.
    pub root: String,
    /// The default folder (shown when the setting is empty).
    pub default_root: String,
    pub maps: Vec<MapInfo>,
}

/// List the maps under `root` (`""` = the default `<app data>/custom_maps`, created if missing) and
/// register them for tile reads. An unreadable root clears the registered set and returns the error.
#[tauri::command]
pub async fn custom_maps_list(root: String) -> Result<CustomMapsListing, String> {
    refuse_mobile()?;
    blocking(move || {
        let default_root = custom_maps::default_root().to_string_lossy().into_owned();
        let dir = match custom_maps::resolve_root(&root) {
            Ok(d) => d,
            Err(e) => {
                custom_maps::clear_registry();
                log::warn!("custom maps: {e}");
                return Err(e);
            }
        };
        let maps = custom_maps::list_maps(&dir).inspect_err(|e| {
            custom_maps::clear_registry();
            log::warn!("custom maps: {e}");
        })?;
        Ok(CustomMapsListing { root: dir.to_string_lossy().into_owned(), default_root, maps })
    })
    .await
}

/// One tile's raw bytes (exact level, no fallback) — the custom-map leg of the shared tile loader.
/// Missing → `Err("tile not found")`; the caller decides what to draw instead.
#[tauri::command]
pub async fn custom_map_tile(map: String, z: u8, x: u32, y: u32) -> Result<tauri::ipc::Response, String> {
    refuse_mobile()?;
    if !custom_maps::valid_tile(z, x, y) {
        return Err(format!("invalid tile {z}/{x}/{y}"));
    }
    let entry = custom_maps::lookup(&map)?;
    // Held until the blocking read has finished (the permit is dropped after the await).
    let _permit = tile_io_gate()
        .acquire_owned()
        .await
        .map_err(|e| format!("tile gate closed: {e}"))?;
    blocking(move || match entry.source.read(z, x, y) {
        Ok(Some(bytes)) => Ok(tauri::ipc::Response::new(bytes)),
        Ok(None) => Err("tile not found".into()),
        Err(e) => {
            log::debug!("custom maps: read {map}/{z}/{x}/{y} failed: {e}");
            Err(format!("tile read failed: {e}"))
        }
    })
    .await
}

/// What the 2D map draws for a tile that is not on disk: its nearest existing ancestor, or (one level
/// below the lowest level of an area) its existing children, or nothing. Existence checks only.
#[tauri::command]
pub async fn custom_map_resolve(map: String, z: u8, x: u32, y: u32) -> Result<TilePlan, String> {
    refuse_mobile()?;
    if !custom_maps::valid_tile(z, x, y) {
        return Err(format!("invalid tile {z}/{x}/{y}"));
    }
    let entry = custom_maps::lookup(&map)?;
    let _permit = tile_io_gate()
        .acquire_owned()
        .await
        .map_err(|e| format!("tile gate closed: {e}"))?;
    blocking(move || {
        Ok(custom_maps::resolve_tile(entry.source.as_ref(), z, x, y, entry.min_zoom, entry.max_zoom))
    })
    .await
}

/// Coverage of a map (its covered areas: frames + "Show map area") from its index, or a fresh background scan
/// when the index is missing / stale or `force` is set (the Settings "Rescan").
#[tauri::command]
pub async fn custom_map_coverage(map: String, force: bool) -> Result<coverage::CoverageReport, String> {
    refuse_mobile()?;
    let entry = custom_maps::lookup(&map)?;
    blocking(move || {
        coverage::load_or_scan(&entry.dir, force).map_err(|e| {
            log::warn!("custom maps: coverage scan of '{map}' failed: {e}");
            format!("coverage scan failed: {e}")
        })
    })
    .await
}
