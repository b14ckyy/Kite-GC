// Flight log types — data structures for flight recording and logbook

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Settings controlling flight recording behavior.
/// Passed from frontend on connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightLogSettings {
    /// Whether flight recording is enabled
    pub enabled: bool,
    /// Custom database directory (empty string = default AppData)
    pub db_path: String,
    /// Whether to also write raw text log files
    pub raw_enabled: bool,
}

impl Default for FlightLogSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: String::new(),
            raw_enabled: false,
        }
    }
}

/// A single recorded flight (row in `flights` table)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flight {
    pub id: i64,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_sec: Option<i64>,
    pub craft_name: String,
    pub fc_variant: String,
    pub fc_version: String,
    pub board_id: String,
    pub platform_type: u8,
    /// Protocol used (e.g. "MSP")
    pub protocol: String,
    /// Start position
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
    /// Reverse-geocoded location name
    pub location_name: Option<String>,
    /// Weather at flight start
    pub weather_temp_c: Option<f64>,
    pub weather_wind_ms: Option<f64>,
    pub weather_wind_deg: Option<i32>,
    pub weather_desc: Option<String>,
    /// Flight statistics
    pub max_alt_m: Option<f64>,
    pub max_speed_ms: Option<f64>,
    pub max_distance_m: Option<f64>,
    pub total_distance_m: Option<f64>,
    pub battery_used_mah: Option<u32>,
    /// User notes
    pub notes: Option<String>,
}

/// A single telemetry sample (row in `telemetry_records` table)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryRecord {
    pub id: i64,
    pub flight_id: i64,
    /// Milliseconds since flight start
    pub timestamp_ms: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt_m: Option<f64>,
    pub speed_ms: Option<f64>,
    pub heading: Option<i16>,
    pub vario_ms: Option<f64>,
    pub voltage: Option<f64>,
    pub current_a: Option<f64>,
    pub mah_drawn: Option<u32>,
    pub rssi: Option<u16>,
    pub roll: Option<f64>,
    pub pitch: Option<f64>,
    pub yaw: Option<i16>,
    pub fix_type: Option<u8>,
    pub num_sat: Option<u8>,
    pub cpu_load: Option<u16>,
}

/// Summary for the logbook list view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightSummary {
    pub id: i64,
    pub start_time: DateTime<Utc>,
    pub duration_sec: Option<i64>,
    pub craft_name: String,
    pub location_name: Option<String>,
    pub max_alt_m: Option<f64>,
    pub max_speed_ms: Option<f64>,
    pub total_distance_m: Option<f64>,
    pub platform_type: u8,
}

/// Sort mode for the logbook
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogbookSortMode {
    /// Aircraft → Location → Date
    AircraftLocationDate,
    /// Location → Date → Aircraft
    LocationDateAircraft,
    /// Date → Location → Aircraft
    DateLocationAircraft,
    /// Aircraft → Date → Location
    AircraftDateLocation,
}
