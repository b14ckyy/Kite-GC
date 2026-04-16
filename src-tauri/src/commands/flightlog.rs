// Flight Log Commands — Tauri commands for logbook CRUD operations

use crate::flightlog::db;
use crate::flightlog::types::{Flight, FlightSummary, TelemetryRecord};

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

/// Trigger reverse geocoding for a flight (async, fetches from Nominatim)
#[tauri::command]
pub async fn flightlog_geocode(
    flight_id: i64,
    db_path: Option<String>,
) -> Result<Option<String>, String> {
    let conn = open_db(&db_path.unwrap_or_default())?;
    let flight = db::get_flight(&conn, flight_id)
        .map_err(|e| format!("Query error: {}", e))?
        .ok_or("Flight not found")?;

    let (lat, lon) = match (flight.start_lat, flight.start_lon) {
        (Some(lat), Some(lon)) => (lat, lon),
        _ => return Ok(None),
    };

    let location = crate::flightlog::geocode::reverse_geocode(lat, lon).await;

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

    let (lat, lon) = match (flight.start_lat, flight.start_lon) {
        (Some(lat), Some(lon)) => (lat, lon),
        _ => return Ok(()),
    };

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
