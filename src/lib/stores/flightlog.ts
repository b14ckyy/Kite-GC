// Flight log Tauri command wrappers.
// Types live in ./flightlogTypes, helpers in ../helpers/flightlogHelpers.
// Both are re-exported here so existing imports continue to work unchanged.

import { invoke } from '@tauri-apps/api/core';

export type {
  FlightSummary,
  Flight,
  TelemetryRecord,
  LogbookSortMode,
  LogbookSortDimension,
  FlightTreeSecondLevel,
  FlightTreeTopLevel,
  FlightTree,
  BlackboxImportSuccess,
  BlackboxImportSuccessLinkable,
  BlackboxImportDuplicate,
  BlackboxImportStatus,
  BlackboxImportProgress,
  KflightImportResult,
} from './flightlogTypes';

export { buildFlightTree, formatDurationSec } from '../helpers/flightlogHelpers';

// ── Tauri command wrappers ───────────────────────────────────────────

import type { FlightSummary, Flight, TelemetryRecord, BlackboxImportStatus, KflightImportResult } from './flightlogTypes';

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

export async function updateFlightCraftName(id: number, craftName: string, dbPath: string): Promise<void> {
  return invoke('flightlog_update_craft_name', {
    flightId: id,
    craftName,
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

export async function importArdupilotLog(
  filePath: string,
  dbPath: string,
  forceImport: boolean = false,
  lang?: string,
): Promise<BlackboxImportStatus> {
  return invoke<BlackboxImportStatus>('flightlog_import_ardupilot', {
    filePath,
    dbPath: dbPath || undefined,
    forceImport,
    lang,
  });
}

export async function linkFlights(flightA: number, flightB: number, dbPath: string): Promise<void> {
  await invoke('flightlog_link_flights', {
    flightA,
    flightB,
    dbPath: dbPath || undefined,
  });
}

export async function unlinkFlight(flightId: number, dbPath: string): Promise<void> {
  await invoke('flightlog_unlink_flight', {
    flightId,
    dbPath: dbPath || undefined,
  });
}

export async function findLinkableFlight(
  craftName: string,
  startTime: string,
  dbPath: string,
): Promise<FlightSummary | null> {
  return invoke<FlightSummary | null>('flightlog_find_linkable', {
    craftName,
    startTime,
    dbPath: dbPath || undefined,
  });
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

export async function exportTrackFile(
  flightId: number,
  outputPath: string,
  dbPath: string,
): Promise<void> {
  return invoke<void>('flightlog_export_track', {
    flightId,
    outputPath,
    dbPath: dbPath || undefined,
  });
}

export async function importKflight(filePath: string, dbPath: string): Promise<KflightImportResult> {
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
