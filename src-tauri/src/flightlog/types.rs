// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

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
    /// Custom raw-log directory (empty string = default Documents/KiteGC). Separate from `db_path`.
    pub raw_log_path: String,
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
            raw_log_path: String::new(),
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
    /// Pilot / operator name (manually editable; future login system can prefill)
    pub pilot_name: Option<String>,
    /// Pilot / operator ID (manually editable; future login system can prefill)
    pub pilot_id: Option<String>,
    /// Serial of the battery pack flown (soft link — resolved to a `battery_packs` row by
    /// serial match at read time; may reference a serial with no pack row → "not in library")
    pub battery_serial: Option<String>,
    /// Local UTC offset (minutes, east-positive) at the flight location, DST-aware (ADR-048).
    /// `start_time` is always true UTC; this offset shifts it to flight-local time for display.
    /// `None` (old rows / no GPS) → display in UTC.
    pub utc_offset_min: Option<i32>,
}

/// A reusable mission stored in the library (row in `missions` table).
/// Identity is `content_hash` (the same hash the frontend provenance uses), so the
/// same mission is stored once and shared across any number of flights.
/// All geometry metadata is computed by the frontend at save time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: i64,
    pub content_hash: String,
    pub name: String,
    /// `inav` | `ardupilot` (forward-looking)
    pub format: String,
    /// Canonical waypoints as JSON (the planner's `Waypoint[]`).
    pub waypoints_json: String,
    /// Optional original `.mission` XML, for round-trip fidelity.
    pub source_xml: Option<String>,
    pub wp_count: i64,
    pub total_distance_m: Option<f64>,
    /// Highest − lowest waypoint altitude.
    pub alt_diff_m: Option<f64>,
    pub max_alt_m: Option<f64>,
    pub min_alt_m: Option<f64>,
    pub bndbox_min_lat: Option<f64>,
    pub bndbox_min_lon: Option<f64>,
    pub bndbox_max_lat: Option<f64>,
    pub bndbox_max_lon: Option<f64>,
    /// Reverse-geocoded location (centroid of the bounding box), like the flight log.
    pub location_name: Option<String>,
    pub created_at: String,
    pub notes: Option<String>,
    /// Planned launch/home reference (degrees) — the base for REL waypoint altitudes and the
    /// 3D mission preview's height. Mirrors the `.mission` `<mwp>` meta written on file export.
    pub home_lat: Option<f64>,
    pub home_lon: Option<f64>,
}

/// Payload for saving a mission to the library (no `id` / `created_at` — assigned by the DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionInput {
    pub content_hash: String,
    pub name: String,
    pub format: String,
    pub waypoints_json: String,
    pub source_xml: Option<String>,
    pub wp_count: i64,
    pub total_distance_m: Option<f64>,
    pub alt_diff_m: Option<f64>,
    pub max_alt_m: Option<f64>,
    pub min_alt_m: Option<f64>,
    pub bndbox_min_lat: Option<f64>,
    pub bndbox_min_lon: Option<f64>,
    pub bndbox_max_lat: Option<f64>,
    pub bndbox_max_lon: Option<f64>,
    pub notes: Option<String>,
    pub home_lat: Option<f64>,
    pub home_lon: Option<f64>,
}

/// A reusable battery pack stored in the library (row in `battery_packs` table).
/// Identity is the user-defined `serial`. Flights soft-link by serial (no FK).
/// The `base_*` fields are a persistent consumption baseline that is never auto-updated —
/// only ever *added to* (manual usage editor / flight-deletion transfer). The displayed
/// lifetime = baseline + Σ(linked flights), computed on read (see BATTERY_MANAGEMENT.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryPack {
    pub id: i64,
    pub serial: String,
    pub label: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    /// `lipo` | `liion` | `life` | `lihv`
    pub chemistry: Option<String>,
    pub cell_count: Option<i64>,
    pub capacity_mah: Option<i64>,
    pub c_rating_discharge: Option<i64>,
    pub c_rating_charge: Option<i64>,
    pub connector: Option<String>,
    pub in_service_date: Option<String>,
    /// `active` | `storage` | `retired` | `damaged`
    pub status: String,
    pub notes: Option<String>,
    pub created_at: String,
    // Persistent consumption baseline (additive only).
    pub base_flight_seconds: i64,
    pub base_mah: i64,
    pub base_cycles: f64,
    pub base_charges: i64,
}

/// Payload for creating/updating a pack's identity/spec fields (no `id` / `created_at` and no
/// `base_*` — the baseline is mutated only via the additive usage path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryPackInput {
    pub serial: String,
    pub label: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub chemistry: Option<String>,
    pub cell_count: Option<i64>,
    pub capacity_mah: Option<i64>,
    pub c_rating_discharge: Option<i64>,
    pub c_rating_charge: Option<i64>,
    pub connector: Option<String>,
    pub in_service_date: Option<String>,
    pub status: String,
    pub notes: Option<String>,
}

/// A single battery pack exported to a `.kbatt` file (one pack per file). Carries the identity/spec
/// plus a baseline snapshot (post-consolidation if the user chose to fold in the linked flights).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryFile {
    /// Always "kbatt" (validated on import).
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    /// Whether the linked flights' usage was folded into the baseline at export.
    pub consolidated: bool,
    /// How many flights were folded in (informational; 0 for a base export).
    pub flight_count: i64,
    pub pack: BatteryPackInput,
    pub base_flight_seconds: i64,
    pub base_mah: i64,
    pub base_cycles: f64,
    pub base_charges: i64,
}

/// Aggregated contribution of the flights linked to a pack (by serial). Combined with the pack's
/// `base_*` baseline on the frontend to produce the displayed lifetime figures.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatteryAggregate {
    pub flight_count: i64,
    pub sum_duration_sec: i64,
    pub sum_mah: i64,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
}

// ── Vehicle library ─────────────────────────────────────────────────

/// A vehicle/aircraft stored in the library (row in `vehicles` table). Flights soft-link by
/// `craft_name` (no FK) — the same craft name already recorded per flight. INAV provides the
/// craft name automatically; ArduPilot/PX4 are linked manually post-flight.
/// Records (max flight time / distance / altitude) are NOT stored here — they are derived live
/// from the linked flights (see `VehicleAggregate`), so they stay correct after relink/delete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: i64,
    pub name: String,
    /// Soft-link key to `flights.craft_name` (trimmed; case-insensitive match). May differ from `name`.
    pub craft_name: Option<String>,
    /// `fixed_wing` | `flying_wing` | `vtol` | `multirotor` | `helicopter` | `rover` | `boat` | `other`
    pub vehicle_type: String,
    /// `active` | `storage` | `retired` | `damaged` | `crashed`
    pub status: String,
    /// Optional image as a base64 data URI (self-contained → travels with `.kvehicle` export).
    pub image: Option<String>,
    pub notes: Option<String>,
    // Airframe
    pub model: Option<String>,
    pub wingspan_mm: Option<i64>,
    pub length_mm: Option<i64>,
    pub weight_auw_g: Option<i64>,
    pub weight_dry_g: Option<i64>,
    // Propulsion (freetext)
    pub motors: Option<String>,
    pub props: Option<String>,
    pub esc: Option<String>,
    // Power recommendation (documentation only — no battery link)
    pub recommended_cells: Option<String>,
    pub recommended_capacity_mah: Option<i64>,
    // Radio / FPV / Link (freetext)
    pub rx: Option<String>,
    pub vtx: Option<String>,
    pub camera: Option<String>,
    pub gimbal_camera: Option<String>,
    pub datalink: Option<String>,
    // Sensors (present/absent)
    pub sensor_airspeed: bool,
    pub sensor_rangefinder: bool,
    pub sensor_optical_flow: bool,
    pub sensor_gps: bool,
    pub sensor_rtk: bool,
    pub sensor_compass: bool,
    // Flight controller (can be prefilled from the latest linked flight, editable)
    pub fc_model: Option<String>,
    pub fc_manufacturer: Option<String>,
    pub fc_firmware: Option<String>,
    pub fc_firmware_version: Option<String>,
    pub blackbox_available: bool,
    // Persistent lifetime baseline (adopted on request from the INAV FC `stats` feature). Additive to
    // the logged flights — the displayed lifetime = baseline + Σ(linked flights). Never auto-updated.
    pub base_flight_count: i64,
    pub base_total_time_s: i64,
    pub base_total_dist_m: i64,
    pub base_total_energy: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Payload for creating/updating a vehicle (no `id` / timestamps and no `base_*` — the baseline is
/// set only via the explicit `set_vehicle_baseline` path, mirroring the battery pack baseline).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleInput {
    pub name: String,
    pub craft_name: Option<String>,
    pub vehicle_type: String,
    pub status: String,
    pub image: Option<String>,
    pub notes: Option<String>,
    pub model: Option<String>,
    pub wingspan_mm: Option<i64>,
    pub length_mm: Option<i64>,
    pub weight_auw_g: Option<i64>,
    pub weight_dry_g: Option<i64>,
    pub motors: Option<String>,
    pub props: Option<String>,
    pub esc: Option<String>,
    pub recommended_cells: Option<String>,
    pub recommended_capacity_mah: Option<i64>,
    pub rx: Option<String>,
    pub vtx: Option<String>,
    pub camera: Option<String>,
    pub gimbal_camera: Option<String>,
    pub datalink: Option<String>,
    pub sensor_airspeed: bool,
    pub sensor_rangefinder: bool,
    pub sensor_optical_flow: bool,
    pub sensor_gps: bool,
    pub sensor_rtk: bool,
    pub sensor_compass: bool,
    pub fc_model: Option<String>,
    pub fc_manufacturer: Option<String>,
    pub fc_firmware: Option<String>,
    pub fc_firmware_version: Option<String>,
    pub blackbox_available: bool,
}

/// A single vehicle exported to a `.kvehicle` file (one vehicle per file). Self-contained
/// (the image data URI + the lifetime baseline travel inside the JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleFile {
    /// Always "kvehicle" (validated on import).
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub vehicle: VehicleInput,
    #[serde(default)]
    pub base_flight_count: i64,
    #[serde(default)]
    pub base_total_time_s: i64,
    #[serde(default)]
    pub base_total_dist_m: i64,
    #[serde(default)]
    pub base_total_energy: i64,
}

/// INAV lifetime flight statistics read from the FC `stats` settings (MSP2_COMMON_SETTING by name).
/// `enabled` reflects the `stats` toggle; the totals are only meaningful when it is on.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InavStats {
    pub enabled: bool,
    pub flight_count: i64,
    pub total_time_s: i64,
    pub total_dist_m: i64,
    pub total_energy: i64,
}

/// Aggregated contribution of the flights linked to a vehicle (by craft name). Totals + the
/// per-flight records (max flight time / distance / altitude, each with the achieving flight id).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VehicleAggregate {
    pub flight_count: i64,
    pub sum_duration_sec: i64,
    pub sum_distance_m: f64,
    pub first_used: Option<String>,
    pub last_used: Option<String>,
    pub max_flight_time_sec: Option<i64>,
    pub max_flight_time_flight_id: Option<i64>,
    pub max_distance_m: Option<f64>,
    pub max_distance_flight_id: Option<i64>,
    pub max_altitude_m: Option<f64>,
    pub max_altitude_flight_id: Option<i64>,
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
    /// Airspeed (m/s): VFR_HUD / MSP2_INAV_AIR_SPEED live; ARSP sensor (ArduPilot) / `airspeed`
    /// column (INAV blackbox) on import. None when not available.
    pub airspeed_ms: Option<f64>,
    pub heading: Option<f64>, // course over ground (degrees, decimals preserved)
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
    pub yaw: Option<f64>, // FC fused heading (degrees, decimals preserved)
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
    /// Canonical flight mode (protocol-agnostic, see docs/active/FLIGHT_MODE_UNIFIED.md):
    /// `mode_primary` = canonical primary id; `mode_modifiers` = comma-separated modifier ids
    /// (None when none). The raw `active_flight_mode_flags` stays as a forensic field.
    pub mode_primary: Option<String>,
    pub mode_modifiers: Option<String>,
    /// RC-link metrics (unified link-stats pipeline). `link_quality` (above) = LQ 0–100; these add the
    /// SNR (dB) and raw uplink RSSI (dBm) when the protocol provides them (CRSF / INAV 9.1+).
    pub link_snr: Option<i8>,
    pub link_rssi_dbm: Option<i16>,
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
    /// Local UTC offset (minutes, east-positive) at the flight location, DST-aware (ADR-048).
    pub utc_offset_min: Option<i32>,
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
