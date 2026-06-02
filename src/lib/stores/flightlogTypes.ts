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
}

export interface TelemetryRecord {
  id: number;
  flight_id: number;
  timestamp_ms: number;
  lat: number | null;
  lon: number | null;
  alt_m: number | null;
  speed_ms: number | null;
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
}

export interface FlightTreeTopLevel {
  key: string;
  children: FlightTreeSecondLevel[];
  flight_count: number;
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
