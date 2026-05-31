// Flight log types — data structures for flight recording and logbook

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Settings controlling flight recording behavior.
/// Passed from frontend on connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightLogSettings {
    /// Whether flight recording is enabled
    pub enabled: bool,
    /// Whether to write flights/telemetry to the database
    pub db_enabled: bool,
    /// Custom database directory (empty string = default AppData)
    pub db_path: String,
    /// Whether to also write raw text log files
    pub raw_enabled: bool,
    /// Continuous raw logging: start recording on connect, not just on arm.
    /// Raw logs include pre-arm data; DB still only records armed segments.
    pub raw_always: bool,
}

impl Default for FlightLogSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            db_enabled: false,
            db_path: String::new(),
            raw_enabled: false,
            raw_always: false,
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
    pub source: String,
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
    /// ID of the linked flight (e.g. live ↔ blackbox association)
    pub linked_flight_id: Option<i64>,
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
    /// Battery state of charge in percent (from MSP_BATTERY_STATE byte 21).
    /// None when not available (e.g. blackbox imports where the FC doesn't log it).
    pub battery_percentage: Option<u8>,
    pub roll: Option<f64>,
    pub pitch: Option<f64>,
    pub yaw: Option<i16>,
    pub fix_type: Option<u8>,
    pub num_sat: Option<u8>,
    pub cpu_load: Option<u16>,
    /// Link Quality 0–100 % (ELRS/CRSF — from blackbox `lq` column; None for MSP)
    pub link_quality: Option<u8>,
    /// Barometric altitude in meters (`BaroAlt`)
    pub baro_alt_m: Option<f64>,
    /// GPS quality/accuracy metrics
    pub gps_hdop: Option<f64>,
    pub gps_eph: Option<f64>,
    pub gps_epv: Option<f64>,
    /// Mission/navigation context
    pub active_wp_number: Option<i32>,
    pub active_flight_mode_flags: Option<i64>,
    pub state_flags: Option<i64>,
    pub nav_state: Option<i32>,
    pub nav_flags: Option<i64>,
    /// RX / hardware health context
    pub rx_signal_received: Option<u8>,
    pub hw_health_status: Option<i64>,
    pub baro_temperature: Option<f64>,
    /// Wind estimator output (NED axes in m/s)
    pub wind_n_ms: Option<f64>,
    pub wind_e_ms: Option<f64>,
    pub wind_d_ms: Option<f64>,
    /// Raw/processed RC channel arrays encoded as JSON
    pub rc_data_json: Option<String>,
    pub rc_command_json: Option<String>,
    /// INAV navigation filter fused position (navPos[0..2])
    /// These provide sensor-fused lat/lon/alt from the EKF,
    /// smoother than raw GPS, used for track display & export.
    pub nav_lat: Option<f64>,
    pub nav_lon: Option<f64>,
    pub nav_alt_m: Option<f64>,
}

/// Summary for the logbook list view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightSummary {
    pub id: i64,
    pub start_time: DateTime<Utc>,
    pub duration_sec: Option<i64>,
    pub source: String,
    pub craft_name: String,
    pub location_name: Option<String>,
    pub max_alt_m: Option<f64>,
    pub max_speed_ms: Option<f64>,
    pub total_distance_m: Option<f64>,
    pub platform_type: u8,
    pub linked_flight_id: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BlackboxImportStatus {
    /// Import successful
    #[serde(rename = "success")]
    Success {
        flight_id: i64,
        rows_imported: usize,
    },
    /// Import successful AND a linkable live flight was found
    #[serde(rename = "success_linkable")]
    SuccessLinkable {
        flight_id: i64,
        rows_imported: usize,
        linkable_flight_id: i64,
    },
    /// Duplicate flight detected — user must confirm override
    #[serde(rename = "duplicate")]
    DuplicateDetected {
        existing_flight: Flight,
        duplicate_craft_name: String,
        duplicate_start_time: DateTime<Utc>,
        duplicate_duration_sec: Option<i64>,
        duplicate_lat: Option<f64>,
        duplicate_lon: Option<f64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboxImportProgress {
    pub stage: String,
    pub progress: u8,
    pub message: String,
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
