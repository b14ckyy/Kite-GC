// Flight Log Commands — Tauri commands for logbook CRUD operations

use tauri::{AppHandle, Emitter};

use crate::flightlog::db;
use crate::flightlog::types::{
    BlackboxImportProgress, BlackboxImportStatus, Flight, FlightSummary,
    TelemetryRecord,
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
    if let BlackboxImportStatus::Success { flight_id, .. } = &result {
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
        }
    }

    Ok(result)
}
