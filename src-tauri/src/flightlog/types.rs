// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight log types — data structures for flight recording and logbook

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Settings controlling flight recording, passed from the frontend on connect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct FlightLogSettings {
    pub enabled: bool,
    pub db_enabled: bool,
    /// Custom database directory (empty string = default AppData)
    pub db_path: String,
    /// Custom raw-log directory (empty string = default Documents/KiteGC). Separate from `db_path`.
    pub raw_log_path: String,
    pub raw_enabled: bool,
    /// Continuous raw logging: start recording on connect, not just on arm.
    /// Raw logs include pre-arm data; DB still only records armed segments.
    pub raw_always: bool,
}


/// One answer of the weather service as stored with a flight (row in `weather_records`).
#[derive(Debug, Clone, Serialize)]
pub struct WeatherRecord {
    /// Session time the answer reached the recorder.
    pub timestamp_ms: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub source: Option<String>,
    /// The widget's `WindAloft`, as JSON text.
    pub data_json: String,
}

/// One raw sample of one GPS receiver as stored with a flight (row in `gps_records`).
#[derive(Debug, Clone, Serialize)]
pub struct GpsRecord {
    pub timestamp_ms: i64,
    /// 0 = GPS_RAW_INT, 1 = GPS2_RAW.
    pub instance: i64,
    pub fix_type: Option<i64>,
    pub fix_raw: Option<i64>,
    pub num_sat: Option<i64>,
    pub hdop: Option<f64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt_msl: Option<f64>,
    pub vdop: Option<f64>,
    pub h_acc_m: Option<f64>,
    pub v_acc_m: Option<f64>,
    pub speed_ms: Option<f64>,
    pub course_deg: Option<f64>,
    pub yaw_deg: Option<f64>,
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
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
    pub location_name: Option<String>,
    pub weather_temp_c: Option<f64>,
    pub weather_wind_ms: Option<f64>,
    pub weather_wind_deg: Option<i32>,
    pub weather_desc: Option<String>,
    pub max_alt_m: Option<f64>,
    pub max_speed_ms: Option<f64>,
    pub max_distance_m: Option<f64>,
    pub total_distance_m: Option<f64>,
    pub battery_used_mah: Option<u32>,
    pub notes: Option<String>,
    /// ID of the linked flight (e.g. live ↔ blackbox association)
    pub linked_flight_id: Option<i64>,
    /// Pilot / operator name (manually editable)
    pub pilot_name: Option<String>,
    /// Pilot / operator ID (manually editable)
    pub pilot_id: Option<String>,
    /// Serial of the battery pack flown (soft link — resolved to a `battery_packs` row by
    /// serial match at read time; may reference a serial with no pack row → "not in library")
    pub battery_serial: Option<String>,
    /// Local UTC offset (minutes, east-positive) at the flight location, DST-aware (ADR-048).
    /// `start_time` is always true UTC; this offset shifts it to flight-local time for display.
    /// `None` (old rows / no GPS) → display in UTC.
    pub utc_offset_min: Option<i32>,
    /// Hardware id of the FC that flew (MSP_UID / MAVLink AUTOPILOT_VERSION uid2), as flown — never
    /// rewritten. `None` for imports, passive telemetry and the known synthetic ids (INAV SITL, an all-zero
    /// id — see `msp::hardware_uid`).
    #[serde(default)]
    pub fc_uid: Option<String>,
    /// Group flight this flight is a member of (`flight_groups.id`, schema v20). `None` = a single
    /// flight. Changeable (unlink / relink); the group id itself never changes.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Library vehicle of the flight (`vehicles.id`, `ON DELETE SET NULL`), resolved on store unless set
    /// explicitly (`db::resolve_vehicle_id`); changeable afterwards. `None` = not in the library.
    #[serde(default)]
    pub vehicle_id: Option<i64>,
}

/// A group flight (row in `flight_groups`, schema v20): the header that ties the member flights of a
/// multi-vehicle flight together — time span, place, timezone, weather, notes; no track, no telemetry,
/// no battery, no vehicle. Aggregates (member count, sums) are derived on query, never stored.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightGroup {
    /// Immutable id (16 hex chars, a hash of the anchor's start time + position).
    pub id: String,
    /// Group start = the earliest arm among the member recordings (true UTC, like `Flight`).
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    /// Local UTC offset (minutes, east-positive) at the flight location (ADR-048).
    pub utc_offset_min: Option<i32>,
    /// Anchor model's position at its arm.
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
    /// Anchor model's GPS altitude (MSL) at its arm.
    pub start_alt_m: Option<f64>,
    pub location_name: Option<String>,
    pub weather_temp_c: Option<f64>,
    pub weather_wind_ms: Option<f64>,
    pub weather_wind_deg: Option<i32>,
    pub weather_desc: Option<String>,
    pub notes: Option<String>,
    /// Set by the database on insert (ignored by `insert_flight_group`).
    #[serde(default)]
    pub created_at: String,
}

/// A timeline event of a flight (row in `flight_events`, schema v20) — arm / disarm in Phase 1; the
/// column set leaves room for STATUSTEXT, disarm reasons, … from later producers.
#[allow(dead_code)] // group flights: callers arrive with GROUP_FLIGHTS.md steps 3-7
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlightEvent {
    pub id: i64,
    pub flight_id: i64,
    /// Same time base as `telemetry_records.timestamp_ms` (ms since the flight start).
    pub timestamp_ms: i64,
    /// Wall-clock UTC epoch ms, when known.
    pub wall_ms: Option<i64>,
    /// `arm` | `disarm` | … .
    pub kind: String,
    pub code: Option<i64>,
    pub text: Option<String>,
    /// `live` | the importer's name.
    pub source: Option<String>,
}

/// A reusable mission in the library (row in `missions`). Identity is `content_hash` (the frontend's
/// provenance hash), so a mission is stored once; geometry metadata is computed by the frontend.
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
    /// Vehicle class the mission was planned for (ArduPilot / PX4: `plane`, `copter`, …); None = not set.
    pub vehicle_class: Option<String>,
    /// A JSON object of data this version may not know. Never parsed here: stored and handed on as it
    /// came, or a newer version's data would be lost.
    pub extra_json: Option<String>,
}

/// Payload for saving a mission to the library (no `id` — assigned by the DB).
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
    #[serde(default)]
    pub vehicle_class: Option<String>,
    /// See `Mission::extra_json`. On an overwrite `None` keeps what the row holds.
    #[serde(default)]
    pub extra_json: Option<String>,
    /// Only an import from another library (`.kmission`, `.kflight`) sets these two; else the DB's own
    /// time and the geocoder fill them.
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub location_name: Option<String>,
}

/// A battery pack in the library (row in `battery_packs`), identified by the user's `serial`; flights
/// soft-link by serial. `base_*` is a persistent baseline, never auto-updated, only added to; lifetime =
/// baseline + Σ(linked flights), computed on read (BATTERY_MANAGEMENT.md).
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

/// A vehicle/aircraft stored in the library (row in `vehicles` table); flights link by
/// `flights.vehicle_id`. Records (max flight time / distance / altitude) are NOT stored here — they
/// are derived live from the linked flights (`VehicleAggregate`), so they stay correct after relink/delete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vehicle {
    pub id: i64,
    pub name: String,
    /// Fallback resolution key against `flights.craft_name` (trimmed; case-insensitive match) for
    /// flights without a matching `fc_uid`. May differ from `name`.
    pub craft_name: Option<String>,
    /// `fixed_wing` | `flying_wing` | `vtol` | `multirotor` | `tricopter` | `helicopter` | `rover` | `boat` | `other`
    pub vehicle_type: String,
    /// `active` | `storage` | `retired` | `damaged` | `crashed`
    pub status: String,
    /// Optional image as a base64 data URI (self-contained → travels with `.kvehicle` export).
    pub image: Option<String>,
    pub notes: Option<String>,
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
    /// FC hardware id (see `Flight::fc_uid`) — prefilled from the connected FC, freely editable (an FC swap
    /// = a new UID). The first key when a flight is resolved to its vehicle.
    pub fc_uid: Option<String>,
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
    #[serde(default)]
    pub fc_uid: Option<String>,
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

/// How a flight's identity resolves to a library vehicle (the End-Flight screen's vehicle pick).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VehicleResolution {
    /// The resolved vehicle, `None` when nothing matches.
    pub vehicle: Option<Vehicle>,
    /// `"uid"` | `"craft"` | `None` (no match).
    pub matched_by: Option<String>,
    /// Matched by craft name, but the vehicle stores a different hardware FC id while the flight has
    /// one: either the FC was swapped or a second aircraft carries the same craft name — the UI asks
    /// which. The automatic link (`db::resolve_vehicle_id`) leaves such a flight unlinked.
    pub uid_conflict: bool,
}

/// Aggregated contribution of the flights linked to a vehicle (`flights.vehicle_id`). Totals + the
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
    /// Throttle output (0–100%): MSP2_INAV_MISC2 / VFR_HUD live; INAV blackbox RC throttle channel;
    /// ArduPilot CTUN throttle-out on import. None when not available.
    pub throttle_pct: Option<f64>,
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
    /// GPS fix as received: live recordings use `GpsData::fix_type`'s scale (0 none / 1 2D / 2 3D / 3 RTK),
    /// except older passive-link recordings (0 / 2 2D / 3 3D); DataFlash and ULog keep the raw status, the
    /// INAV blackbox importer writes 3 whenever sats are logged.
    pub fix_type: Option<u8>,
    pub num_sat: Option<u8>,
    pub cpu_load: Option<u16>,
    /// Link Quality 0–100 % (ELRS/CRSF — from blackbox `lq` column; None for MSP)
    pub link_quality: Option<u8>,
    /// Barometric altitude in meters (`BaroAlt`)
    pub baro_alt_m: Option<f64>,
    pub gps_hdop: Option<f64>,
    pub gps_eph: Option<f64>,
    pub gps_epv: Option<f64>,
    pub active_wp_number: Option<i32>,
    pub active_flight_mode_flags: Option<i64>,
    pub state_flags: Option<i64>,
    pub nav_state: Option<i32>,
    pub nav_flags: Option<i64>,
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
    /// INAV nav-filter fused position (navPos) — smoother than raw GPS, for track display & export.
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
    /// Wall-clock UTC epoch ms of the sample (live recordings from schema v20; `None` for imports and
    /// older rows). Not sent to the frontend until a reader needs it (group replay).
    #[serde(default, skip_serializing)]
    pub wall_ms: Option<i64>,
}

/// A per-instance battery sample (row in `battery_records`): ArduPilot/PX4 can run several monitors
/// (`instance` 0..9). INAV is single-battery and uses the columns on `telemetry_records` instead
/// (docs/active/MULTI_BATTERY.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryRecord {
    pub id: i64,
    pub flight_id: i64,
    /// Milliseconds since flight start (shares the exact `telemetry_records.timestamp_ms` grid so the
    /// frontend can align batteries to a track frame by timestamp).
    pub timestamp_ms: i64,
    pub instance: u8,
    pub voltage: Option<f64>,
    pub current_a: Option<f64>,
    pub mah_drawn: Option<u32>,
    pub battery_percentage: Option<u8>,
    pub cell_count: Option<u8>,
    pub temperature: Option<f64>,
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
    /// Group flight membership (see `Flight::group_id`).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Library vehicle (see `Flight::vehicle_id`).
    #[serde(default)]
    pub vehicle_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
// DuplicateDetected carries a full Flight; boxing would ripple through all match/construction sites
#[allow(clippy::large_enum_variant)]
pub enum BlackboxImportStatus {
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

/// A library mission exported to a `.kmission` file (one mission, or one INAV mission set, per file):
/// the row as it is, so another library rebuilds it — statistics excepted, the importer computes them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionFile {
    /// Always "kmission" (validated on import).
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub app_version: String,
    pub mission: MissionFileBody,
}

/// `waypoints` and `extra` stay raw JSON text: parsed into a `Value` and written again, a float can come
/// back one bit off — a new content hash for an ArduPilot / PX4 mission, changed data of a newer Kite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionFileBody {
    pub name: String,
    /// `inav` | `ardupilot` | `px4`
    pub format: String,
    /// Informational: the importer derives the identity from the waypoints.
    #[serde(default)]
    pub content_hash: String,
    pub waypoints: Box<serde_json::value::RawValue>,
    #[serde(default)]
    pub source_xml: Option<String>,
    #[serde(default)]
    pub home_lat: Option<f64>,
    #[serde(default)]
    pub home_lon: Option<f64>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub location_name: Option<String>,
    #[serde(default)]
    pub vehicle_class: Option<String>,
    /// `Mission::extra_json`, embedded.
    #[serde(default)]
    pub extra: Option<Box<serde_json::value::RawValue>>,
}
