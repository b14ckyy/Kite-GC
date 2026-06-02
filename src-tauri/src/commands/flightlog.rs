// Flight Log Commands — Tauri commands for logbook CRUD operations

use tauri::Emitter;

use crate::flightlog::db;
use crate::flightlog::exchange;
use crate::flightlog::types::{
    BlackboxImportProgress, BlackboxImportStatus, Flight, FlightSummary,
    Mission, MissionInput, TelemetryRecord,
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

/// Read the Blackbox-header waypoint count for a flight (replay `WP N/X` fallback).
#[tauri::command]
pub fn flight_logged_wp_count(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<Option<i64>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    db::get_flight_logged_wp_count(&conn, flight_id).map_err(|e| format!("Query error: {}", e))
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