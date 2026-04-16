// Flight log command wrappers and frontend helpers

import { invoke } from '@tauri-apps/api/core';

export interface FlightSummary {
  id: number;
  start_time: string;
  duration_sec: number | null;
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
}

export type LogbookSortMode =
  | 'aircraft-location-date'
  | 'location-date-aircraft'
  | 'date-location-aircraft'
  | 'aircraft-date-location';

export interface GroupedFlights {
  key: string;
  flights: FlightSummary[];
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

export async function geocodeFlight(id: number, dbPath: string): Promise<string | null> {
  return invoke<string | null>('flightlog_geocode', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function fetchFlightWeather(id: number, dbPath: string): Promise<void> {
  await invoke('flightlog_fetch_weather', {
    flightId: id,
    dbPath: dbPath || undefined,
  });
}

export async function getDefaultFlightlogPath(): Promise<string> {
  return invoke<string>('flightlog_default_db_path');
}

function dateKey(ts: string): string {
  const d = new Date(ts);
  return d.toISOString().slice(0, 10);
}

function primaryOrUnknown(v: string | null | undefined, unknown = 'Unknown'): string {
  const s = (v ?? '').trim();
  return s.length > 0 ? s : unknown;
}

export function groupFlights(
  flights: FlightSummary[],
  mode: LogbookSortMode,
): GroupedFlights[] {
  const sorted = [...flights].sort((a, b) => {
    const aTime = new Date(a.start_time).getTime();
    const bTime = new Date(b.start_time).getTime();
    return bTime - aTime;
  });

  const groups = new Map<string, FlightSummary[]>();

  for (const f of sorted) {
    const craft = primaryOrUnknown(f.craft_name, 'Unnamed craft');
    const location = primaryOrUnknown(f.location_name, 'Unknown location');
    const day = dateKey(f.start_time);
    let key = '';

    if (mode === 'aircraft-location-date') key = `${craft} | ${location} | ${day}`;
    if (mode === 'location-date-aircraft') key = `${location} | ${day} | ${craft}`;
    if (mode === 'date-location-aircraft') key = `${day} | ${location} | ${craft}`;
    if (mode === 'aircraft-date-location') key = `${craft} | ${day} | ${location}`;

    const list = groups.get(key);
    if (list) {
      list.push(f);
    } else {
      groups.set(key, [f]);
    }
  }

  return [...groups.entries()]
    .map(([key, items]) => ({ key, flights: items }))
    .sort((a, b) => a.key.localeCompare(b.key));
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
