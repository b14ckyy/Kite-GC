// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Log Commands — Tauri commands for logbook CRUD operations

use tauri::Emitter;

use crate::flightlog::db;
use crate::flightlog::exchange;
use crate::flightlog::types::{
    BatteryAggregate, BatteryFile, BatteryPack, BatteryPackInput, BlackboxImportProgress,
    BlackboxImportStatus, Flight, FlightSummary, Mission, MissionInput, TelemetryRecord,
};

/// Resolve the database path and open a connection.
/// Uses the provided custom path, or falls back to defaults.
fn open_db(custom_path: &str) -> Result<rusqlite::Connection, String> {
    let portable = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
        .unwrap_or(false);
    let path = db::resolve_db_path(custom_path, portable);
    db::open_database(&path).map_err(|e| format!("Database error: {}", e))
}

#[inline]
fn is_valid_gps_coord(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

fn resolve_flight_coords(
    conn: &rusqlite::Connection,
    flight_id: i64,
    start_lat: Option<f64>,
    start_lon: Option<f64>,
) -> Result<Option<(f64, f64)>, String> {
    if let (Some(lat), Some(lon)) = (start_lat, start_lon) {
        if is_valid_gps_coord(lat, lon) {
            return Ok(Some((lat, lon)));
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT lat, lon
             FROM telemetry_records
             WHERE flight_id = ?1
               AND lat IS NOT NULL
               AND lon IS NOT NULL
             ORDER BY timestamp_ms ASC",
        )
        .map_err(|e| format!("Query error: {}", e))?;

    let mut rows = stmt
        .query_map(rusqlite::params![flight_id], |row| {
            Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|e| format!("Query error: {}", e))?;

    let mut fallback: Option<(f64, f64)> = None;
    while let Some(next) = rows.next().transpose().map_err(|e| format!("Query error: {}", e))? {
        let (lat, lon) = next;
        if is_valid_gps_coord(lat, lon) {
            fallback = Some((lat, lon));
            break;
        }
    }

    if let Some((lat, lon)) = fallback {
        conn.execute(
            "UPDATE flights SET start_lat = ?1, start_lon = ?2 WHERE id = ?3",
            rusqlite::params![lat, lon, flight_id],
        )
        .map_err(|e| format!("Update error: {}", e))?;
        return Ok(Some((lat, lon)));
    }

    Ok(None)
}

/// List all flights (summaries) for the logbook
#[tauri::command]
pub fn flightlog_list(db_path: Option<String>) -> Result<Vec<FlightSummary>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::list_flights(&conn).map_err(|e| format!("Query error: {}", e))
}

/// Get a single flight with full details
#[tauri::command]
pub fn flightlog_get(flight_id: i64, db_path: Option<String>) -> Result<Option<Flight>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_flight(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}

/// Get the GPS track / telemetry data for a flight
#[tauri::command]
pub fn flightlog_get_track(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<Vec<TelemetryRecord>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_flight_track(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}

// ── Mission library ─────────────────────────────────────────────────

/// Save a mission to the library (dedup by content hash). Returns the mission id.
#[tauri::command]
pub fn mission_db_save(mission: MissionInput, db_path: Option<String>) -> Result<i64, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::upsert_mission(&conn, &mission).map_err(|e| format!("Save error: {}", e))
}

/// Fetch a library mission by id.
#[tauri::command]
pub fn mission_db_get(id: i64, db_path: Option<String>) -> Result<Option<Mission>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_mission(&conn, id).map_err(|e| format!("Query error: {}", e))
}

/// List all library missions (newest first) — for the Mission Manager.
#[tauri::command]
pub fn mission_db_list(db_path: Option<String>) -> Result<Vec<Mission>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::list_missions(&conn).map_err(|e| format!("Query error: {}", e))
}

/// Update a mission's name + notes (Manager rename / notes edit).
#[tauri::command]
pub fn mission_db_set_meta(
    id: i64,
    name: String,
    notes: Option<String>,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_mission_meta(&conn, id, &name, notes.as_deref())
        .map_err(|e| format!("Update error: {}", e))
}

/// Find a library mission by content hash (import dedup-match / save NEW-vs-OVERWRITE check).
#[tauri::command]
pub fn mission_db_find_by_hash(
    content_hash: String,
    db_path: Option<String>,
) -> Result<Option<Mission>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::find_mission_by_hash(&conn, &content_hash).map_err(|e| format!("Query error: {}", e))
}

/// Overwrite an existing library mission in place (OVERWRITE on save).
#[tauri::command]
pub fn mission_db_update(
    id: i64,
    mission: MissionInput,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_mission(&conn, id, &mission).map_err(|e| format!("Update error: {}", e))
}

/// Fetch the mission linked to a flight (if any).
#[tauri::command]
pub fn mission_db_for_flight(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<Option<Mission>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_mission_for_flight(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}

/// Link a recorded flight to a library mission.
#[tauri::command]
pub fn flight_link_mission(
    flight_id: i64,
    mission_id: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::link_flight_mission(&conn, flight_id, mission_id)
        .map_err(|e| format!("Link error: {}", e))
}

/// Unlink a flight from its mission (Logbook unlink).
#[tauri::command]
pub fn flight_unlink_mission(flight_id: i64, db_path: Option<String>) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::unlink_flight_mission(&conn, flight_id).map_err(|e| format!("Unlink error: {}", e))
}

/// Delete a library mission (unlinks referencing flights first).
#[tauri::command]
pub fn mission_db_delete(id: i64, db_path: Option<String>) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::delete_mission(&conn, id).map_err(|e| format!("Delete error: {}", e))
}

/// List the flights that link a given mission (reverse lookup + delete warning).
#[tauri::command]
pub fn mission_db_flights(
    mission_id: i64,
    db_path: Option<String>,
) -> Result<Vec<FlightSummary>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::list_flights_for_mission(&conn, mission_id).map_err(|e| format!("Query error: {}", e))
}

/// Read the Blackbox-header waypoint count for a flight (replay `WP N/X` fallback).
#[tauri::command]
pub fn flight_logged_wp_count(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<Option<i64>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_flight_logged_wp_count(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
}

// ── Battery library ─────────────────────────────────────────────────

/// Create a new battery pack (serial is UNIQUE; a duplicate surfaces as an error). Returns the id.
#[tauri::command]
pub fn battery_db_create(battery: BatteryPackInput, db_path: Option<String>) -> Result<i64, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::create_battery(&conn, &battery).map_err(|e| format!("Create error: {}", e))
}

/// Update a pack's identity/spec fields (not serial, not the baseline).
#[tauri::command]
pub fn battery_db_update(
    id: i64,
    battery: BatteryPackInput,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_battery(&conn, id, &battery).map_err(|e| format!("Update error: {}", e))
}

/// List all battery packs (newest first) — for the Battery Manager.
#[tauri::command]
pub fn battery_db_list(db_path: Option<String>) -> Result<Vec<BatteryPack>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::list_batteries(&conn).map_err(|e| format!("Query error: {}", e))
}

/// Fetch a pack by id.
#[tauri::command]
pub fn battery_db_get(id: i64, db_path: Option<String>) -> Result<Option<BatteryPack>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_battery(&conn, id).map_err(|e| format!("Query error: {}", e))
}

/// Find a pack by serial (link resolution / unknown-serial check).
#[tauri::command]
pub fn battery_db_find_by_serial(
    serial: String,
    db_path: Option<String>,
) -> Result<Option<BatteryPack>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::find_battery_by_serial(&conn, &serial).map_err(|e| format!("Query error: {}", e))
}

/// Delete a pack (flights keep their serial → "not in library").
#[tauri::command]
pub fn battery_db_delete(id: i64, db_path: Option<String>) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::delete_battery(&conn, id).map_err(|e| format!("Delete error: {}", e))
}

/// Add consumption to a pack's persistent baseline (additive only).
#[tauri::command]
pub fn battery_db_add_usage(
    id: i64,
    flight_seconds: i64,
    mah: i64,
    cycles: f64,
    charges: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::add_battery_usage(&conn, id, flight_seconds, mah, cycles, charges)
        .map_err(|e| format!("Update error: {}", e))
}

/// Aggregate the flights linked to a serial (dynamic part of the lifetime).
#[tauri::command]
pub fn battery_db_aggregate(
    serial: String,
    db_path: Option<String>,
) -> Result<BatteryAggregate, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::battery_aggregate(&conn, &serial).map_err(|e| format!("Query error: {}", e))
}

/// List the flights linked to a serial (Manager detail + delete warning).
#[tauri::command]
pub fn battery_db_flights(
    serial: String,
    db_path: Option<String>,
) -> Result<Vec<FlightSummary>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::list_flights_for_serial(&conn, &serial).map_err(|e| format!("Query error: {}", e))
}

/// Set (or clear, with an empty string) the soft battery-serial link on a flight.
#[tauri::command]
pub fn flight_set_battery_serial(
    flight_id: i64,
    serial: String,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let s = serial.trim();
    db::set_flight_battery_serial(&conn, flight_id, if s.is_empty() { None } else { Some(s) })
        .map_err(|e| format!("Link error: {}", e))
}

/// Set a pack's baseline to absolute values (import "new" / "overwrite").
#[tauri::command]
pub fn battery_db_set_baseline(
    id: i64,
    flight_seconds: i64,
    mah: i64,
    cycles: f64,
    charges: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::set_battery_baseline(&conn, id, flight_seconds, mah, cycles, charges)
        .map_err(|e| format!("Update error: {}", e))
}

/// Write a battery pack to a `.kbatt` file (one pack per file).
#[tauri::command]
pub fn battery_file_write(path: String, file: BatteryFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("Serialize error: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Write error: {}", e))
}

/// Read + validate a `.kbatt` file (for the import preview).
#[tauri::command]
pub fn battery_file_read(path: String) -> Result<BatteryFile, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Read error: {}", e))?;
    let file: BatteryFile =
        serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;
    if file.format != "kbatt" {
        return Err(format!("Not a battery file (format: {})", file.format));
    }
    Ok(file)
}

/// Reverse-geocode a mission's location (bounding-box centroid) and store it, reusing the
/// same Nominatim helper as the flight log. Skips the network call if the mission already has
/// a location name (dedup means each mission is geocoded once). Returns the location name.
#[tauri::command]
pub async fn mission_db_geocode(
    id: i64,
    lang: Option<String>,
    db_path: Option<String>,
) -> Result<Option<String>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let mission = db::get_mission(&conn, id)
        .map_err(|e| format!("Query error: {}", e))?
        .ok_or("Mission not found")?;

    if mission.location_name.is_some() {
        return Ok(mission.location_name);
    }

    let (lat, lon) = match (
        mission.bndbox_min_lat,
        mission.bndbox_min_lon,
        mission.bndbox_max_lat,
        mission.bndbox_max_lon,
    ) {
        (Some(min_lat), Some(min_lon), Some(max_lat), Some(max_lon)) => {
            ((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0)
        }
        _ => return Ok(None), // no geo waypoints → nothing to geocode
    };

    let lang_str = lang.as_deref().unwrap_or("en");
    let location = crate::flightlog::geocode::reverse_geocode(lat, lon, lang_str).await;

    if let Some(ref name) = location {
        conn.execute(
            "UPDATE missions SET location_name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(|e| format!("Update error: {}", e))?;
    }

    Ok(location)
}

/// Delete a flight and its telemetry data
#[tauri::command]
pub fn flightlog_delete(flight_id: i64, db_path: Option<String>) -> Result<bool, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::delete_flight(&conn, flight_id).map_err(|e| format!("Delete error: {}", e))
}

/// Update notes on a flight
#[tauri::command]
pub fn flightlog_update_notes(
    flight_id: i64,
    notes: String,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_flight_notes(&conn, flight_id, &notes).map_err(|e| format!("Update error: {}", e))
}

/// Update craft name on a flight
#[tauri::command]
pub fn flightlog_update_craft_name(
    flight_id: i64,
    craft_name: String,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_flight_craft_name(&conn, flight_id, &craft_name).map_err(|e| format!("Update error: {}", e))
}

/// Update the UAV platform type on a flight (manual override; drives the map replay symbol).
#[tauri::command]
pub fn flightlog_update_platform_type(
    flight_id: i64,
    platform_type: u8,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::update_flight_platform_type(&conn, flight_id, platform_type)
        .map_err(|e| format!("Update error: {}", e))
}

/// Update pilot metadata (name + id) on a flight. Empty strings → NULL.
#[tauri::command]
pub fn flightlog_update_pilot(
    flight_id: i64,
    pilot_name: String,
    pilot_id: String,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let name = pilot_name.trim();
    let pid = pilot_id.trim();
    db::update_flight_pilot(
        &conn,
        flight_id,
        if name.is_empty() { None } else { Some(name) },
        if pid.is_empty() { None } else { Some(pid) },
    )
    .map_err(|e| format!("Update error: {}", e))
}

/// Manually update weather data for a flight
#[tauri::command]
pub fn flightlog_update_weather(
    flight_id: i64,
    temp_c: Option<f64>,
    wind_ms: Option<f64>,
    wind_deg: Option<i32>,
    description: Option<String>,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    conn.execute(
        "UPDATE flights SET weather_temp_c = ?1, weather_wind_ms = ?2, weather_wind_deg = ?3, weather_desc = ?4 WHERE id = ?5",
        rusqlite::params![temp_c, wind_ms, wind_deg, description, flight_id],
    ).map_err(|e| format!("Update error: {}", e))?;
    Ok(())
}

/// Trigger reverse geocoding for a flight (async, fetches from Nominatim)
#[tauri::command]
pub async fn flightlog_geocode(
    flight_id: i64,
    db_path: Option<String>,
    lang: Option<String>,
) -> Result<Option<String>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let flight = db::get_flight(&conn, flight_id)
        .map_err(|e| format!("Query error: {}", e))?
        .ok_or("Flight not found")?;

    let (lat, lon) = match resolve_flight_coords(&conn, flight_id, flight.start_lat, flight.start_lon)? {
        Some(coords) => coords,
        None => return Ok(None),
    };

    let lang_str = lang.as_deref().unwrap_or("en");
    let location = crate::flightlog::geocode::reverse_geocode(lat, lon, lang_str).await;

    if let Some(ref name) = location {
        // Update the flight record with the location name
        conn.execute(
            "UPDATE flights SET location_name = ?1 WHERE id = ?2",
            rusqlite::params![name, flight_id],
        )
        .map_err(|e| format!("Update error: {}", e))?;
    }

    Ok(location)
}

/// Fetch weather data for a flight's start position
#[tauri::command]
pub async fn flightlog_fetch_weather(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let flight = db::get_flight(&conn, flight_id)
        .map_err(|e| format!("Query error: {}", e))?
        .ok_or("Flight not found")?;

    let (lat, lon) = match resolve_flight_coords(&conn, flight_id, flight.start_lat, flight.start_lon)? {
        Some(coords) => coords,
        None => return Ok(()),
    };

    // Current weather is only meaningful for MSP-recorded flights.
    // Pure Blackbox imports should not be backfilled with "now" conditions.
    if flight.source == "blackbox" {
        return Ok(());
    }

    if let Some(weather) = crate::flightlog::weather::fetch_weather(lat, lon).await {
        conn.execute(
            "UPDATE flights SET weather_temp_c = ?1, weather_wind_ms = ?2, weather_wind_deg = ?3, weather_desc = ?4 WHERE id = ?5",
            rusqlite::params![weather.temp_c, weather.wind_ms, weather.wind_deg, weather.description, flight_id],
        ).map_err(|e| format!("Update error: {}", e))?;
    }

    Ok(())
}

/// Get the default database path (for display in settings)
#[tauri::command]
pub fn flightlog_default_db_path() -> String {
    let portable = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
        .unwrap_or(false);
    let path = db::resolve_db_path("", portable);
    path.parent()
        .unwrap_or(std::path::Path::new("."))
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
pub async fn flightlog_import_blackbox(
    file_path: String,
    db_path: Option<String>,
    log_index: Option<u32>,
    force_import: bool,
    lang: Option<String>,
    app_handle: tauri::AppHandle,
) -> Result<BlackboxImportStatus, String> {
    let db_path = db_path.unwrap_or_default();
    let db_path_for_import = db_path.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let emit_progress = |progress: u8, stage: &str, message: &str| {
            let _ = app_handle.emit(
                "flightlog-import-progress",
                BlackboxImportProgress {
                    stage: stage.to_string(),
                    progress,
                    message: message.to_string(),
                },
            );
        };

        emit_progress(0, "start", "Preparing Blackbox import...");
        let conn = open_db(&db_path_for_import)?;
        crate::flightlog::blackbox::import_blackbox_log_with_progress(
            &conn,
            std::path::Path::new(&file_path),
            log_index,
            force_import,
            emit_progress,
        )
    })
    .await
    .map_err(|e| format!("Blackbox import task failed: {}", e))??;

    // Only enrich imported Blackbox flights immediately with location if import was successful
    if let BlackboxImportStatus::Success { flight_id, rows_imported } = &result {
        let conn = open_db(&db_path)?;
        if let Some(flight) = db::get_flight(&conn, *flight_id)
            .map_err(|e| format!("Query error: {}", e))?
        {
            if flight.location_name.as_deref().unwrap_or("").trim().is_empty() {
                if let (Some(lat), Some(lon)) = (flight.start_lat, flight.start_lon) {
                    if let Some(name) = crate::flightlog::geocode::reverse_geocode(lat, lon, lang.as_deref().unwrap_or("en")).await {
                        conn.execute(
                            "UPDATE flights SET location_name = ?1 WHERE id = ?2",
                            rusqlite::params![name, flight_id],
                        )
                        .map_err(|e| format!("Update error: {}", e))?;
                    }
                }
            }

            // Check if a linkable live flight exists (same craft, ±60s)
            if let Ok(Some(linkable)) = db::find_linkable_live_flight(&conn, &flight.craft_name, flight.start_time) {
                eprintln!("[LINK-AUTO] Found linkable live flight {} for blackbox import {}", linkable.id, flight_id);
                return Ok(BlackboxImportStatus::SuccessLinkable {
                    flight_id: *flight_id,
                    rows_imported: *rows_imported,
                    linkable_flight_id: linkable.id,
                });
            }
        }
    }

    Ok(result)
}

// ── Export / Import / Offline replay ────────────────────────────────

/// Export a flight track as KMZ/KML/GPX/CSV (format detected from file extension)
#[tauri::command]
pub fn flightlog_export_track(
    flight_id: i64,
    output_path: String,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let flight = db::get_flight(&conn, flight_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "Flight not found".to_string())?;
    let track = db::get_flight_track(&conn, flight_id)
        .map_err(|e| format!("DB error: {}", e))?;
    crate::flightlog::track_export::export_track(
        &flight,
        &track,
        std::path::Path::new(&output_path),
    )
}

/// Export the raw blackbox binary file for a flight
#[tauri::command]
pub fn flightlog_export_blackbox(
    flight_id: i64,
    output_path: String,
    db_path: Option<String>,
) -> Result<String, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let (filename, data) = db::get_blackbox_file(&conn, flight_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "No blackbox file attached to this flight".to_string())?;
    std::fs::write(&output_path, &data)
        .map_err(|e| format!("Failed to write file: {}", e))?;
    Ok(filename)
}

/// Export selected flights to a .kflight file
#[tauri::command]
pub fn flightlog_export(
    flight_ids: Vec<i64>,
    output_path: String,
    db_path: Option<String>,
) -> Result<usize, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    exchange::export_flights(&conn, &flight_ids, std::path::Path::new(&output_path))
}

/// Import flights from a .kflight file into the main database
#[tauri::command]
pub fn flightlog_import_kflight(
    file_path: String,
    db_path: Option<String>,
) -> Result<exchange::ImportResult, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    exchange::import_flights(&conn, std::path::Path::new(&file_path))
}

/// List flights contained in a .kflight file (for preview / offline replay)
#[tauri::command]
pub fn flightlog_kflight_list(file_path: String) -> Result<Vec<FlightSummary>, String> {
    exchange::list_flights_in_file(std::path::Path::new(&file_path))
}

/// Get a single flight from a .kflight file (offline replay)
#[tauri::command]
pub fn flightlog_kflight_get(
    file_path: String,
    flight_id: i64,
) -> Result<Option<Flight>, String> {
    exchange::get_flight_from_file(std::path::Path::new(&file_path), flight_id)
}

/// Get telemetry track from a .kflight file (offline replay)
#[tauri::command]
pub fn flightlog_kflight_track(
    file_path: String,
    flight_id: i64,
) -> Result<Vec<TelemetryRecord>, String> {
    exchange::get_track_from_file(std::path::Path::new(&file_path), flight_id)
}

// ─── ArduPilot DataFlash commands ────────────────────────────────────────────

use crate::flightlog::ardupilot;
use serde::Serialize;

/// Inventory the FMT records in an ArduPilot .bin file.
/// Returns the list of registered message type names so the frontend can
/// display what data is available before committing to a full import.
#[tauri::command]
pub fn flightlog_probe_ardupilot(file_path: String) -> Result<Vec<ArdupilotMsgTypeInfo>, String> {
    let data = std::fs::read(&file_path)
        .map_err(|e| format!("Failed to read file '{}': {}", file_path, e))?;

    let defs = ardupilot::probe_message_types(&data);
    let result = defs
        .into_iter()
        .map(|d| ArdupilotMsgTypeInfo {
            type_id: d.type_id,
            name: d.name,
            format: d.format,
            columns: d.columns,
        })
        .collect();
    Ok(result)
}

#[derive(Debug, Clone, Serialize)]
pub struct ArdupilotMsgTypeInfo {
    pub type_id: u8,
    pub name: String,
    pub format: String,
    pub columns: Vec<String>,
}

/// Decode an ArduPilot .bin file and write a normalized CSV to `out_csv_path`.
/// Returns a summary of what was decoded for verification.
#[tauri::command]
pub fn flightlog_decode_ardupilot_csv(
    file_path: String,
    out_csv_path: String,
) -> Result<ArdupilotDecodeStats, String> {
    let data = std::fs::read(&file_path)
        .map_err(|e| format!("Failed to read file '{}': {}", file_path, e))?;

    let stats = ardupilot::decode_to_normalized_csv(&data, std::path::Path::new(&out_csv_path))?;

    Ok(ArdupilotDecodeStats {
        total_records: stats.total_records,
        gps_rows: stats.gps_rows,
        vehicle_type: stats.vehicle_type,
        fw_version: stats.fw_version,
        first_fix_time: stats.first_fix_time.map(|t| t.to_rfc3339()),
        last_fix_time: stats.last_fix_time.map(|t| t.to_rfc3339()),
        arm_count: stats.arm_count,
        disarm_count: stats.disarm_count,
        message_type_counts: stats.message_type_counts,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ArdupilotDecodeStats {
    pub total_records: usize,
    pub gps_rows: usize,
    pub vehicle_type: Option<String>,
    pub fw_version: Option<String>,
    pub first_fix_time: Option<String>,
    pub last_fix_time: Option<String>,
    pub arm_count: usize,
    pub disarm_count: usize,
    pub message_type_counts: std::collections::HashMap<String, usize>,
}

/// Import an ArduPilot DataFlash .bin file into the logbook database.
/// Emits `flightlog-import-progress` events during processing.
#[tauri::command]
pub async fn flightlog_import_ardupilot(
    file_path: String,
    db_path: Option<String>,
    force_import: bool,
    lang: Option<String>,
    app_handle: tauri::AppHandle,
) -> Result<BlackboxImportStatus, String> {
    let db_path = db_path.unwrap_or_default();
    let db_path_for_import = db_path.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let emit_progress = |progress: u8, stage: &str, message: &str| {
            let _ = app_handle.emit(
                "flightlog-import-progress",
                BlackboxImportProgress {
                    stage: stage.to_string(),
                    progress,
                    message: message.to_string(),
                },
            );
        };

        emit_progress(0, "start", "Preparing ArduPilot import...");
        let conn = open_db(&db_path_for_import)?;
        ardupilot::import_ardupilot_log_with_progress(
            &conn,
            std::path::Path::new(&file_path),
            force_import,
            emit_progress,
        )
    })
    .await
    .map_err(|e| format!("ArduPilot import task failed: {}", e))??;

    // Auto-geocode on successful import
    if let BlackboxImportStatus::Success { flight_id, rows_imported } = &result {
        let conn = open_db(&db_path)?;
        if let Some(flight) = db::get_flight(&conn, *flight_id)
            .map_err(|e| format!("Query error: {}", e))?
        {
            if flight.location_name.as_deref().unwrap_or("").trim().is_empty() {
                if let (Some(lat), Some(lon)) = (flight.start_lat, flight.start_lon) {
                    if let Some(name) = crate::flightlog::geocode::reverse_geocode(
                        lat, lon,
                        lang.as_deref().unwrap_or("en"),
                    ).await {
                        conn.execute(
                            "UPDATE flights SET location_name = ?1 WHERE id = ?2",
                            rusqlite::params![name, flight_id],
                        )
                        .map_err(|e| format!("Update error: {}", e))?;
                    }
                }
            }

            // Check if a linkable live flight exists (same craft, ±60s)
            if let Ok(Some(linkable)) = db::find_linkable_live_flight(&conn, &flight.craft_name, flight.start_time) {
                eprintln!("[LINK-AUTO] Found linkable live flight {} for ArduPilot import {}", linkable.id, flight_id);
                return Ok(BlackboxImportStatus::SuccessLinkable {
                    flight_id: *flight_id,
                    rows_imported: *rows_imported,
                    linkable_flight_id: linkable.id,
                });
            }
        }
    }

    Ok(result)
}

// --- Flight Linking Commands -------------------------------------------------

/// Link two flights together (live recording ? blackbox import).
/// Both flights get `linked_flight_id` pointing at each other, source becomes "both".
#[tauri::command]
pub fn flightlog_link_flights(
    flight_a: i64,
    flight_b: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::link_flights(&conn, flight_a, flight_b).map_err(|e| format!("Link error: {}", e))
}

/// Remove the link between two flights. Resets source to "live" or "blackbox".
#[tauri::command]
pub fn flightlog_unlink_flight(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<(), String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::unlink_flight(&conn, flight_id).map_err(|e| format!("Unlink error: {}", e))
}

/// Find a live flight that could be auto-linked to a blackbox import.
#[tauri::command]
pub fn flightlog_find_linkable(
    craft_name: String,
    start_time: String,
    db_path: Option<String>,
) -> Result<Option<FlightSummary>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let dt = chrono::DateTime::parse_from_rfc3339(&start_time)
        .map(|t| t.with_timezone(&chrono::Utc))
        .map_err(|e| format!("Invalid timestamp: {}", e))?;
    db::find_linkable_live_flight(&conn, &craft_name, dt)
        .map_err(|e| format!("Query error: {}", e))
}

// ── Deferred-commit: resolve the pending live-recording session (ADR-041) ──────────────

/// Commit the pending live-recording session into the main DB (the End-Flight dialog's **Save**).
/// Returns the new flight id so the frontend can link the flown mission + battery/notes.
#[tauri::command]
pub fn flightlog_commit_pending_session(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<i64, String> {
    let session = state
        .pending_session
        .lock()
        .map_err(|_| "Pending-session lock poisoned".to_string())?
        .take()
        .ok_or_else(|| "No pending recording session to commit".to_string())?;
    crate::flightlog::recorder::commit_pending_session(session)
}

/// Discard the pending live-recording session (the End-Flight dialog's **Discard Recording**) —
/// the temp file is deleted and nothing reaches the main DB.
#[tauri::command]
pub fn flightlog_discard_pending_session(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let session = state
        .pending_session
        .lock()
        .map_err(|_| "Pending-session lock poisoned".to_string())?
        .take();
    if let Some(session) = session {
        crate::flightlog::recorder::discard_pending_session(session);
    }
    Ok(())
}

/// Continue-on-reconnect for a session interrupted by a disconnect while armed (ADR-042): move the
/// pending session into the resume slot so the next connection's recorder resumes/finalizes it.
#[tauri::command]
pub fn flightlog_continue_pending_session(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let session = state
        .pending_session
        .lock()
        .map_err(|_| "Pending-session lock poisoned".to_string())?
        .take();
    if let Some(session) = session {
        *state
            .resume_pending
            .lock()
            .map_err(|_| "Resume-session lock poisoned".to_string())? = Some(session);
    }
    Ok(())
}

// ── Recovery of an orphan temp session left by a crash/close (ADR-042) ──────────────────

/// Summary of an orphan temp `.ktmp` for the recovery prompt.
#[derive(serde::Serialize)]
pub struct OrphanInfo {
    pub temp_path: String,
    pub craft_name: String,
    pub start_time: String,
    pub duration_sec: i64,
    pub sample_count: i64,
}

/// Resolve the main DB path (custom dir or default/portable).
fn resolve_main_db_path(custom: &str) -> std::path::PathBuf {
    let portable = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
        .unwrap_or(false);
    db::resolve_db_path(custom, portable)
}

fn sessions_dir(custom: &str) -> std::path::PathBuf {
    resolve_main_db_path(custom)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("sessions")
}

/// Scan `<db_dir>/sessions/*.ktmp` for an orphan session left by a crash/close. Empty temp files
/// (no telemetry) are deleted in passing; the newest non-empty one is returned for the recovery
/// prompt. There should be at most one (the single-temp invariant); a straggler is simply offered
/// on a later launch.
#[tauri::command]
pub fn flightlog_scan_orphan_sessions(db_path: Option<String>) -> Result<Option<OrphanInfo>, String> {
    let dir = sessions_dir(&db_path.unwrap_or_default());
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(None), // no sessions dir yet → nothing to recover
    };
    let mut best: Option<OrphanInfo> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ktmp") {
            continue;
        }
        let conn = match db::open_temp_session(&path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Orphan scan: cannot open {}: {}", path.display(), e);
                continue;
            }
        };
        let count = db::temp_session_row_count(&conn).unwrap_or(0);
        if count == 0 {
            drop(conn);
            db::remove_temp_session(&path); // empty → worthless
            continue;
        }
        let meta = db::read_session_meta(&conn).ok().flatten();
        let max_ts: i64 = conn
            .query_row("SELECT COALESCE(MAX(timestamp_ms), 0) FROM telemetry_records", [], |r| r.get(0))
            .unwrap_or(0);
        let (craft_name, start_time) = match meta {
            Some(m) => (m.craft_name, m.start_time),
            None => (String::new(), String::new()),
        };
        let info = OrphanInfo {
            temp_path: path.to_string_lossy().to_string(),
            craft_name,
            start_time,
            duration_sec: max_ts / 1000,
            sample_count: count,
        };
        // Keep the newest by start_time (RFC3339 sorts lexicographically).
        best = match best {
            Some(b) if b.start_time >= info.start_time => Some(b),
            _ => Some(info),
        };
    }
    Ok(best)
}

/// Recovery prompt → **Discard**: delete the orphan temp file; nothing reaches the main DB.
#[tauri::command]
pub fn flightlog_recover_discard(temp_path: String) -> Result<(), String> {
    db::remove_temp_session(std::path::Path::new(&temp_path));
    Ok(())
}

/// Recovery prompt → **Save Incomplete**: reconstruct the flight from the orphan and commit it to
/// the main DB (finalized with `end_time` = last sample). Returns the new flight id.
#[tauri::command]
pub fn flightlog_recover_save_incomplete(
    temp_path: String,
    db_path: Option<String>,
) -> Result<i64, String> {
    let main_db = resolve_main_db_path(&db_path.unwrap_or_default());
    let (session, _count) = crate::flightlog::recorder::summarize_temp_session(
        std::path::PathBuf::from(&temp_path),
        main_db,
    )?;
    crate::flightlog::recorder::commit_pending_session(session)
}

/// Recovery prompt → **Continue on Reconnect**: load the orphan into the shared resume slot; the
/// next connection's recorder resumes it (armed) or finalizes it (disarmed) on its first poll.
#[tauri::command]
pub fn flightlog_recover_continue(
    temp_path: String,
    db_path: Option<String>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let main_db = resolve_main_db_path(&db_path.unwrap_or_default());
    let (session, _count) = crate::flightlog::recorder::summarize_temp_session(
        std::path::PathBuf::from(&temp_path),
        main_db,
    )?;
    *state
        .resume_pending
        .lock()
        .map_err(|_| "Resume-session lock poisoned".to_string())? = Some(session);
    Ok(())
}