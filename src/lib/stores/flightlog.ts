// Flight log command wrappers and frontend helpers

import { invoke } from '@tauri-apps/api/core';

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
}

export type LogbookSortMode =
  | 'aircraft-location-date'
  | 'location-date-aircraft'
  | 'date-location-aircraft'
  | 'aircraft-date-location';

type LogbookSortDimension = 'aircraft' | 'location' | 'date';

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

export interface BlackboxImportDuplicate {
  type: 'duplicate';
  existing_flight: Flight;
  duplicate_craft_name: string;
  duplicate_start_time: string;
  duplicate_duration_sec: number | null;
  duplicate_lat: number | null;
  duplicate_lon: number | null;
}

export type BlackboxImportStatus = BlackboxImportSuccess | BlackboxImportDuplicate;

export interface BlackboxImportProgress {
  stage: string;
  progress: number;
  message: string;
}

export async function listFlights(dbPath: string): Promise<FlightSummary[]> {
  return invoke<FlightSummary[]>('flightlog_list', {
    dbPath: dbPath || undefined,
  });
}

export async function getFlight(id: number, dbPath: string): Promise<Flight | null> {
  return invoke<Flight | null>('flightlog_get', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function getFlightTrack(id: number, dbPath: string): Promise<TelemetryRecord[]> {
  return invoke<TelemetryRecord[]>('flightlog_get_track', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function deleteFlight(id: number, dbPath: string): Promise<boolean> {
  return invoke<boolean>('flightlog_delete', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function updateFlightNotes(id: number, notes: string, dbPath: string): Promise<void> {
  return invoke('flightlog_update_notes', {
    flightId: id,
    notes,
    dbPath: dbPath || undefined,
  });
}

export async function geocodeFlight(id: number, dbPath: string, lang: string): Promise<string | null> {
  return invoke<string | null>('flightlog_geocode', {
    flightId: id,
    dbPath: dbPath || undefined,
    lang,
  });
}

export async function fetchFlightWeather(id: number, dbPath: string): Promise<void> {
  await invoke('flightlog_fetch_weather', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function updateFlightWeather(
  id: number,
  tempC: number | null,
  windMs: number | null,
  windDeg: number | null,
  description: string | null,
  dbPath: string,
): Promise<void> {
  await invoke('flightlog_update_weather', {
    flightId: id,
    tempC,
    windMs,
    windDeg,
    description: description || null,
    dbPath: dbPath || undefined,
  });
}

export async function getDefaultFlightlogPath(): Promise<string> {
  return invoke<string>('flightlog_default_db_path');
}

export async function importBlackboxLog(
  filePath: string,
  dbPath: string,
  logIndex?: number,
  forceImport: boolean = false,
  lang?: string,
): Promise<BlackboxImportStatus> {
  return invoke<BlackboxImportStatus>('flightlog_import_blackbox', {
    filePath,
    dbPath: dbPath || undefined,
    logIndex,
    forceImport,
    lang,
  });
}

// ── Export / Import / Offline replay ────────────────────────────────

export interface KflightImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}

export async function exportFlights(
  flightIds: number[],
  outputPath: string,
  dbPath: string,
): Promise<number> {
  return invoke<number>('flightlog_export', {
    flightIds,
    outputPath,
    dbPath: dbPath || undefined,
  });
}

export async function exportBlackboxFile(
  flightId: number,
  outputPath: string,
  dbPath: string,
): Promise<string> {
  return invoke<string>('flightlog_export_blackbox', {
    flightId,
    outputPath,
    dbPath: dbPath || undefined,
  });
}

export async function importKflight(
  filePath: string,
  dbPath: string,
): Promise<KflightImportResult> {
  return invoke<KflightImportResult>('flightlog_import_kflight', {
    filePath,
    dbPath: dbPath || undefined,
  });
}

export async function listKflightFlights(filePath: string): Promise<FlightSummary[]> {
  return invoke<FlightSummary[]>('flightlog_kflight_list', { filePath });
}

export async function getKflightFlight(filePath: string, flightId: number): Promise<Flight | null> {
  return invoke<Flight | null>('flightlog_kflight_get', { filePath, flightId });
}

export async function getKflightTrack(filePath: string, flightId: number): Promise<TelemetryRecord[]> {
  return invoke<TelemetryRecord[]>('flightlog_kflight_track', { filePath, flightId });
}

function dateKey(ts: string): string {
  const d = new Date(ts);
  return d.toISOString().slice(0, 10);
}

function primaryOrUnknown(v: string | null | undefined, unknown = 'Unknown'): string {
  const s = (v ?? '').trim();
  return s.length > 0 ? s : unknown;
}

function getModeDimensions(mode: LogbookSortMode): [LogbookSortDimension, LogbookSortDimension, LogbookSortDimension] {
  if (mode === 'aircraft-location-date') return ['aircraft', 'location', 'date'];
  if (mode === 'location-date-aircraft') return ['location', 'date', 'aircraft'];
  if (mode === 'date-location-aircraft') return ['date', 'location', 'aircraft'];
  return ['aircraft', 'date', 'location'];
}

function getDimensionValue(f: FlightSummary, dim: LogbookSortDimension): string {
  if (dim === 'aircraft') return primaryOrUnknown(f.craft_name, 'Unnamed craft');
  if (dim === 'location') return primaryOrUnknown(f.location_name, 'Unknown location');
  return dateKey(f.start_time);
}

function compareDimensionValues(a: string, b: string, dim: LogbookSortDimension): number {
  if (dim === 'date') return b.localeCompare(a);
  return a.localeCompare(b, undefined, { sensitivity: 'base' });
}

function compareByThirdThenTime(
  a: FlightSummary,
  b: FlightSummary,
  third: LogbookSortDimension,
): number {
  const thirdCmp = compareDimensionValues(getDimensionValue(a, third), getDimensionValue(b, third), third);
  if (thirdCmp !== 0) return thirdCmp;
  const aTime = new Date(a.start_time).getTime();
  const bTime = new Date(b.start_time).getTime();
  return bTime - aTime;
}

export function buildFlightTree(
  flights: FlightSummary[],
  mode: LogbookSortMode,
): FlightTree {
  const dimensions = getModeDimensions(mode);
  const [topDim, secondDim, thirdDim] = dimensions;
  const topMap = new Map<string, Map<string, FlightSummary[]>>();

  for (const flight of flights) {
    const topKey = getDimensionValue(flight, topDim);
    const secondKey = getDimensionValue(flight, secondDim);
    let secondMap = topMap.get(topKey);
    if (!secondMap) {
      secondMap = new Map<string, FlightSummary[]>();
      topMap.set(topKey, secondMap);
    }
    const list = secondMap.get(secondKey);
    if (list) {
      list.push(flight);
    } else {
      secondMap.set(secondKey, [flight]);
    }
  }

  const groups: FlightTreeTopLevel[] = [...topMap.entries()]
    .sort(([a], [b]) => compareDimensionValues(a, b, topDim))
    .map(([topKey, secondMap]) => {
      const children: FlightTreeSecondLevel[] = [...secondMap.entries()]
        .sort(([a], [b]) => compareDimensionValues(a, b, secondDim))
        .map(([secondKey, list]) => ({
          key: secondKey,
          flights: [...list].sort((a, b) => compareByThirdThenTime(a, b, thirdDim)),
        }));

      return {
        key: topKey,
        children,
        flight_count: children.reduce((sum, child) => sum + child.flights.length, 0),
      };
    });

  return { dimensions, groups };
}

export function formatDurationSec(durationSec: number | null): string {
  if (durationSec == null || durationSec <= 0) return '0m';
  const h = Math.floor(durationSec / 3600);
  const m = Math.floor((durationSec % 3600) / 60);
  const s = durationSec % 60;

  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}
