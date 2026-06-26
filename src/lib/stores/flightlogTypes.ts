// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// All shared types and interfaces for the flight logbook system.
// Imported by flightlog.ts (store), logbookController.ts, and UI components.

/** A reusable mission in the DB library (mirrors Rust flightlog::types::Mission). */
export interface LibraryMission {
  id: number;
  content_hash: string;
  name: string;
  format: string;
  waypoints_json: string;
  source_xml: string | null;
  wp_count: number;
  total_distance_m: number | null;
  alt_diff_m: number | null;
  max_alt_m: number | null;
  min_alt_m: number | null;
  bndbox_min_lat: number | null;
  bndbox_min_lon: number | null;
  bndbox_max_lat: number | null;
  bndbox_max_lon: number | null;
  location_name: string | null;
  created_at: string;
  notes: string | null;
  /** Planned launch/home reference (degrees) — base for REL altitudes + 3D preview height. */
  home_lat: number | null;
  home_lon: number | null;
}

/** Payload for saving a mission to the library (mirrors Rust flightlog::types::MissionInput). */
export interface LibraryMissionInput {
  content_hash: string;
  name: string;
  format: string;
  waypoints_json: string;
  source_xml: string | null;
  wp_count: number;
  total_distance_m: number | null;
  alt_diff_m: number | null;
  max_alt_m: number | null;
  min_alt_m: number | null;
  bndbox_min_lat: number | null;
  bndbox_min_lon: number | null;
  bndbox_max_lat: number | null;
  bndbox_max_lon: number | null;
  notes: string | null;
  home_lat: number | null;
  home_lon: number | null;
}

/** A battery pack in the library (mirrors the Rust BatteryPack). Identity = serial. */
export interface BatteryPack {
  id: number;
  serial: string;
  label: string | null;
  manufacturer: string | null;
  model: string | null;
  chemistry: string | null; // lipo | liion | life | lihv
  cell_count: number | null;
  capacity_mah: number | null;
  c_rating_discharge: number | null;
  c_rating_charge: number | null;
  connector: string | null;
  in_service_date: string | null;
  status: string; // active | storage | retired | damaged
  notes: string | null;
  created_at: string;
  // Persistent consumption baseline (additive only).
  base_flight_seconds: number;
  base_mah: number;
  base_cycles: number;
  base_charges: number;
}

/** Create/update payload for a pack's identity/spec fields (no id/created_at, no baseline). */
export interface BatteryPackInput {
  serial: string;
  label: string | null;
  manufacturer: string | null;
  model: string | null;
  chemistry: string | null;
  cell_count: number | null;
  capacity_mah: number | null;
  c_rating_discharge: number | null;
  c_rating_charge: number | null;
  connector: string | null;
  in_service_date: string | null;
  status: string;
  notes: string | null;
}

/** A `.kbatt` export file (one pack + a baseline snapshot). */
export interface BatteryFile {
  format: string; // "kbatt"
  version: number;
  exported_at: string;
  consolidated: boolean;
  flight_count: number;
  pack: BatteryPackInput;
  base_flight_seconds: number;
  base_mah: number;
  base_cycles: number;
  base_charges: number;
}

/** Aggregated contribution of the flights linked to a pack (combined with baseline on display). */
export interface BatteryAggregate {
  flight_count: number;
  sum_duration_sec: number;
  sum_mah: number;
  first_used: string | null;
  last_used: string | null;
}

/** A vehicle/aircraft in the library (mirrors the Rust Vehicle). Flights soft-link by craft_name. */
export interface Vehicle {
  id: number;
  name: string;
  craft_name: string | null;
  vehicle_type: string; // fixed_wing | flying_wing | vtol | multirotor | helicopter | rover | boat | other
  status: string; // active | storage | retired | damaged | crashed
  image: string | null; // base64 data URI
  notes: string | null;
  // Airframe
  model: string | null;
  wingspan_mm: number | null;
  length_mm: number | null;
  weight_auw_g: number | null;
  weight_dry_g: number | null;
  // Propulsion (freetext)
  motors: string | null;
  props: string | null;
  esc: string | null;
  // Power recommendation
  recommended_cells: string | null;
  recommended_capacity_mah: number | null;
  // Radio / FPV / Link (freetext)
  rx: string | null;
  vtx: string | null;
  camera: string | null;
  gimbal_camera: string | null;
  datalink: string | null;
  // Sensors
  sensor_airspeed: boolean;
  sensor_rangefinder: boolean;
  sensor_optical_flow: boolean;
  sensor_gps: boolean;
  sensor_rtk: boolean;
  sensor_compass: boolean;
  // Flight controller
  fc_model: string | null;
  fc_manufacturer: string | null;
  fc_firmware: string | null;
  fc_firmware_version: string | null;
  blackbox_available: boolean;
  // Persistent lifetime baseline (adopted on request from the INAV FC `stats` feature).
  base_flight_count: number;
  base_total_time_s: number;
  base_total_dist_m: number;
  base_total_energy: number;
  created_at: string;
  updated_at: string;
}

/** Create/update payload for a vehicle (no id/timestamps, no baseline — baseline is set separately). */
export type VehicleInput = Omit<
  Vehicle,
  'id' | 'created_at' | 'updated_at' | 'base_flight_count' | 'base_total_time_s' | 'base_total_dist_m' | 'base_total_energy'
>;

/** A `.kvehicle` export file (one vehicle; self-contained incl. image + lifetime baseline). */
export interface VehicleFile {
  format: string; // "kvehicle"
  version: number;
  exported_at: string;
  vehicle: VehicleInput;
  base_flight_count: number;
  base_total_time_s: number;
  base_total_dist_m: number;
  base_total_energy: number;
}

/** INAV lifetime flight statistics read from the FC `stats` settings. */
export interface InavStats {
  enabled: boolean;
  flight_count: number;
  total_time_s: number;
  total_dist_m: number;
  total_energy: number;
}

/** Aggregated flights linked to a vehicle (totals + per-flight records). */
export interface VehicleAggregate {
  flight_count: number;
  sum_duration_sec: number;
  sum_distance_m: number;
  first_used: string | null;
  last_used: string | null;
  max_flight_time_sec: number | null;
  max_flight_time_flight_id: number | null;
  max_distance_m: number | null;
  max_distance_flight_id: number | null;
  max_altitude_m: number | null;
  max_altitude_flight_id: number | null;
}

export interface FlightSummary {
  id: number;
  start_time: string;
  duration_sec: number | null;
  source: string;
  craft_name: string;
  location_name: string | null;
  max_alt_m: number | null;
  max_speed_ms: number | null;
  total_distance_m: number | null;
  platform_type: number;
  linked_flight_id: number | null;
  notes: string | null;
  /** Local UTC offset (minutes, east-positive) at the flight location; null → display in UTC (ADR-048). */
  utc_offset_min: number | null;
}

export interface Flight {
  id: number;
  start_time: string;
  end_time: string | null;
  duration_sec: number | null;
  source: string;
  craft_name: string;
  fc_variant: string;
  fc_version: string;
  board_id: string;
  platform_type: number;
  protocol: string;
  start_lat: number | null;
  start_lon: number | null;
  location_name: string | null;
  weather_temp_c: number | null;
  weather_wind_ms: number | null;
  weather_wind_deg: number | null;
  weather_desc: string | null;
  max_alt_m: number | null;
  max_speed_ms: number | null;
  max_distance_m: number | null;
  total_distance_m: number | null;
  battery_used_mah: number | null;
  notes: string | null;
  linked_flight_id: number | null;
  pilot_name: string | null;
  pilot_id: string | null;
  battery_serial: string | null;
  /** Local UTC offset (minutes, east-positive) at the flight location; null → display in UTC (ADR-048). */
  utc_offset_min: number | null;
}

export interface TelemetryRecord {
  id: number;
  flight_id: number;
  timestamp_ms: number;
  lat: number | null;
  lon: number | null;
  alt_m: number | null;
  speed_ms: number | null;
  airspeed_ms: number | null;
  throttle_pct: number | null;
  heading: number | null;
  vario_ms: number | null;
  voltage: number | null;
  current_a: number | null;
  mah_drawn: number | null;
  rssi: number | null;
  battery_percentage: number | null;
  roll: number | null;
  pitch: number | null;
  yaw: number | null;
  fix_type: number | null;
  num_sat: number | null;
  cpu_load: number | null;
  link_quality: number | null;
  baro_alt_m: number | null;
  gps_hdop: number | null;
  gps_eph: number | null;
  gps_epv: number | null;
  active_wp_number: number | null;
  active_flight_mode_flags: number | null;
  state_flags: number | null;
  nav_state: number | null;
  nav_flags: number | null;
  rx_signal_received: number | null;
  hw_health_status: number | null;
  baro_temperature: number | null;
  wind_n_ms: number | null;
  wind_e_ms: number | null;
  wind_d_ms: number | null;
  rc_data_json: string | null;
  rc_command_json: string | null;
  nav_lat: number | null;
  nav_lon: number | null;
  nav_alt_m: number | null;
  /** Canonical flight mode (protocol-agnostic, see flightModeRegistry). `mode_modifiers` is a
   *  comma-separated list of modifier ids (null when none). */
  mode_primary: string | null;
  mode_modifiers: string | null;
  /** RC-link metrics (unified link-stats pipeline). `link_quality` (above) = LQ 0–100; these add SNR
   *  (dB) and raw uplink RSSI (dBm) when the protocol provides them (CRSF / INAV 9.1+). */
  link_snr: number | null;
  link_rssi_dbm: number | null;
}

export type LogbookSortMode =
  | 'aircraft-location-date'
  | 'location-date-aircraft'
  | 'date-location-aircraft'
  | 'aircraft-date-location';

export type LogbookSortDimension = 'aircraft' | 'location' | 'date';

export interface FlightTreeSecondLevel {
  key: string;
  flights: FlightSummary[];
  /** Σ of the group's flight durations (seconds) and distances (metres) — for the header stats. */
  sum_duration_sec: number;
  sum_distance_m: number;
}

export interface FlightTreeTopLevel {
  key: string;
  children: FlightTreeSecondLevel[];
  flight_count: number;
  /** Σ over all child flights — for the header stats. */
  sum_duration_sec: number;
  sum_distance_m: number;
}

export interface FlightTree {
  dimensions: [LogbookSortDimension, LogbookSortDimension, LogbookSortDimension];
  groups: FlightTreeTopLevel[];
}

export interface BlackboxImportSuccess {
  type: 'success';
  flight_id: number;
  rows_imported: number;
}

export interface BlackboxImportSuccessLinkable {
  type: 'success_linkable';
  flight_id: number;
  rows_imported: number;
  linkable_flight_id: number;
}

export interface BlackboxImportDuplicate {
  type: 'duplicate';
  existing_flight: Flight;
  duplicate_craft_name: string;
  duplicate_start_time: string;
  duplicate_duration_sec: number | null;
  duplicate_lat: number | null;
  duplicate_lon: number | null;
}

export type BlackboxImportStatus =
  | BlackboxImportSuccess
  | BlackboxImportSuccessLinkable
  | BlackboxImportDuplicate;

export interface BlackboxImportProgress {
  stage: string;
  progress: number;
  message: string;
}

export interface KflightImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}
