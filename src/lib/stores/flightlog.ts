// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

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

export {
  buildFlightTree,
  formatDurationSec,
  formatFlightDateTime,
  formatFlightClock,
  flightTzLabel,
} from '../helpers/flightlogHelpers';

// ── Tauri command wrappers ───────────────────────────────────────────

import type { FlightSummary, Flight, TelemetryRecord, BlackboxImportStatus, KflightImportResult, LibraryMission, LibraryMissionInput, BatteryPack, BatteryPackInput, BatteryAggregate, BatteryFile } from './flightlogTypes';

/** Save a mission to the library (dedup by content hash). Returns the mission id. */
export async function missionDbSave(mission: LibraryMissionInput, dbPath: string): Promise<number> {
  return invoke<number>('mission_db_save', {
    mission,
    dbPath: dbPath || undefined,
  });
}

/** Link a recorded flight to a library mission. */
export async function flightLinkMission(flightId: number, missionId: number, dbPath: string): Promise<void> {
  return invoke<void>('flight_link_mission', {
    flightId,
    missionId,
    dbPath: dbPath || undefined,
  });
}

/** Unlink a flight from its mission (Logbook unlink). */
export async function flightUnlinkMission(flightId: number, dbPath: string): Promise<void> {
  return invoke<void>('flight_unlink_mission', {
    flightId,
    dbPath: dbPath || undefined,
  });
}

/** Delete a library mission (unlinks referencing flights first). */
export async function missionDbDelete(id: number, dbPath: string): Promise<void> {
  return invoke<void>('mission_db_delete', {
    id,
    dbPath: dbPath || undefined,
  });
}

/** List the flights that link a given mission (reverse lookup + delete warning). */
export async function missionDbFlights(missionId: number, dbPath: string): Promise<FlightSummary[]> {
  return invoke<FlightSummary[]>('mission_db_flights', {
    missionId,
    dbPath: dbPath || undefined,
  });
}

/** List all library missions (newest first). */
export async function missionDbList(dbPath: string): Promise<LibraryMission[]> {
  return invoke<LibraryMission[]>('mission_db_list', {
    dbPath: dbPath || undefined,
  });
}

/** Update a mission's name + notes (Manager rename / notes edit). */
export async function missionDbSetMeta(id: number, name: string, notes: string | null, dbPath: string): Promise<void> {
  return invoke<void>('mission_db_set_meta', {
    id,
    name,
    notes,
    dbPath: dbPath || undefined,
  });
}

/** Export a library mission (its waypoints JSON) to a .mission file (INAV). */
export async function missionExportFileFromJson(path: string, waypointsJson: string): Promise<void> {
  return invoke<void>('mission_save_file_from_json', {
    path,
    waypointsJson,
  });
}

/** Fetch a library mission by id. */
export async function missionDbGet(id: number, dbPath: string): Promise<LibraryMission | null> {
  return invoke<LibraryMission | null>('mission_db_get', {
    id,
    dbPath: dbPath || undefined,
  });
}

/** Find a library mission by content hash (import dedup-match / save NEW-vs-OVERWRITE check). */
export async function missionDbFindByHash(contentHash: string, dbPath: string): Promise<LibraryMission | null> {
  return invoke<LibraryMission | null>('mission_db_find_by_hash', {
    contentHash,
    dbPath: dbPath || undefined,
  });
}

/** Overwrite an existing library mission in place (OVERWRITE on save). */
export async function missionDbUpdate(id: number, mission: LibraryMissionInput, dbPath: string): Promise<void> {
  return invoke<void>('mission_db_update', {
    id,
    mission,
    dbPath: dbPath || undefined,
  });
}

/** Reverse-geocode a mission (bbox centroid) and store its location_name. Fire-and-forget. */
export async function missionDbGeocode(id: number, lang: string, dbPath: string): Promise<string | null> {
  return invoke<string | null>('mission_db_geocode', {
    id,
    lang: lang || undefined,
    dbPath: dbPath || undefined,
  });
}

/** The mission linked to a recorded flight (replay `WP N/X` source), or null. */
export async function missionDbForFlight(flightId: number, dbPath: string): Promise<LibraryMission | null> {
  return invoke<LibraryMission | null>('mission_db_for_flight', {
    flightId,
    dbPath: dbPath || undefined,
  });
}

/** Blackbox-header waypoint count for a flight (replay `WP N/X` fallback), or null. */
export async function flightLoggedWpCount(flightId: number, dbPath: string): Promise<number | null> {
  return invoke<number | null>('flight_logged_wp_count', {
    flightId,
    dbPath: dbPath || undefined,
  });
}

// ── Battery library ─────────────────────────────────────────────────

/** Create a new battery pack (serial UNIQUE → duplicate rejects). Returns the new id. */
export async function batteryDbCreate(battery: BatteryPackInput, dbPath: string): Promise<number> {
  return invoke<number>('battery_db_create', { battery, dbPath: dbPath || undefined });
}

/** Update a pack's identity/spec fields (not serial, not baseline). */
export async function batteryDbUpdate(id: number, battery: BatteryPackInput, dbPath: string): Promise<void> {
  return invoke<void>('battery_db_update', { id, battery, dbPath: dbPath || undefined });
}

/** List all battery packs (newest first). */
export async function batteryDbList(dbPath: string): Promise<BatteryPack[]> {
  return invoke<BatteryPack[]>('battery_db_list', { dbPath: dbPath || undefined });
}

/** Fetch a pack by id. */
export async function batteryDbGet(id: number, dbPath: string): Promise<BatteryPack | null> {
  return invoke<BatteryPack | null>('battery_db_get', { id, dbPath: dbPath || undefined });
}

/** Find a pack by serial (link resolution / unknown-serial check), or null. */
export async function batteryDbFindBySerial(serial: string, dbPath: string): Promise<BatteryPack | null> {
  return invoke<BatteryPack | null>('battery_db_find_by_serial', { serial, dbPath: dbPath || undefined });
}

/** Delete a pack (flights keep their serial → "not in library"). */
export async function batteryDbDelete(id: number, dbPath: string): Promise<void> {
  return invoke<void>('battery_db_delete', { id, dbPath: dbPath || undefined });
}

/** Add consumption to a pack's persistent baseline (additive only). */
export async function batteryDbAddUsage(
  id: number,
  flightSeconds: number,
  mah: number,
  cycles: number,
  charges: number,
  dbPath: string,
): Promise<void> {
  return invoke<void>('battery_db_add_usage', {
    id, flightSeconds, mah, cycles, charges, dbPath: dbPath || undefined,
  });
}

/** Aggregate the flights linked to a serial (dynamic part of the lifetime). */
export async function batteryDbAggregate(serial: string, dbPath: string): Promise<BatteryAggregate> {
  return invoke<BatteryAggregate>('battery_db_aggregate', { serial, dbPath: dbPath || undefined });
}

/** List the flights linked to a serial (Manager detail + delete warning). */
export async function batteryDbFlights(serial: string, dbPath: string): Promise<FlightSummary[]> {
  return invoke<FlightSummary[]>('battery_db_flights', { serial, dbPath: dbPath || undefined });
}

/** Set (or clear, with an empty string) the soft battery-serial link on a flight. */
export async function flightSetBatterySerial(flightId: number, serial: string, dbPath: string): Promise<void> {
  return invoke<void>('flight_set_battery_serial', { flightId, serial, dbPath: dbPath || undefined });
}

/** Set a pack's baseline to absolute values (import new / overwrite). */
export async function batteryDbSetBaseline(
  id: number,
  flightSeconds: number,
  mah: number,
  cycles: number,
  charges: number,
  dbPath: string,
): Promise<void> {
  return invoke<void>('battery_db_set_baseline', {
    id, flightSeconds, mah, cycles, charges, dbPath: dbPath || undefined,
  });
}

/** Write a battery pack to a `.kbatt` file. */
export async function batteryFileWrite(path: string, file: BatteryFile): Promise<void> {
  return invoke<void>('battery_file_write', { path, file });
}

/** Read + validate a `.kbatt` file (import preview). */
export async function batteryFileRead(path: string): Promise<BatteryFile> {
  return invoke<BatteryFile>('battery_file_read', { path });
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

/** Commit the pending live-recording session into the main DB (End-Flight dialog Save).
 *  Returns the new flight id. Deferred commit — see ADR-041. */
export async function flightlogCommitPending(): Promise<number> {
  return invoke<number>('flightlog_commit_pending_session');
}

/** Discard the pending live-recording session (End-Flight dialog Discard) — drops the temp file. */
export async function flightlogDiscardPending(): Promise<void> {
  await invoke('flightlog_discard_pending_session');
}

/** Continue-on-reconnect for a session interrupted by a disconnect while armed: move the pending
 *  session into the resume slot so the next connection resumes/finalizes it (ADR-042). */
export async function flightlogContinuePending(): Promise<void> {
  await invoke('flightlog_continue_pending_session');
}

/** An orphan temp recording session found at startup (crash/close recovery, ADR-042). */
export interface OrphanSession {
  temp_path: string;
  craft_name: string;
  start_time: string;
  duration_sec: number;
  sample_count: number;
}

/** Scan for an orphan temp session left by a crash/close; null if none. */
export async function scanOrphanSessions(dbPath: string): Promise<OrphanSession | null> {
  return invoke<OrphanSession | null>('flightlog_scan_orphan_sessions', { dbPath: dbPath || undefined });
}

/** Recovery → Discard: delete the orphan temp file. */
export async function recoverDiscard(tempPath: string): Promise<void> {
  await invoke('flightlog_recover_discard', { tempPath });
}

/** Recovery → Save Incomplete: commit the orphan to the DB as a finished flight; returns its id. */
export async function recoverSaveIncomplete(tempPath: string, dbPath: string): Promise<number> {
  return invoke<number>('flightlog_recover_save_incomplete', { tempPath, dbPath: dbPath || undefined });
}

/** Recovery → Continue on Reconnect: arm the orphan for resumption on the next connection. */
export async function recoverContinue(tempPath: string, dbPath: string): Promise<void> {
  await invoke('flightlog_recover_continue', { tempPath, dbPath: dbPath || undefined });
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

export async function updateFlightPlatformType(id: number, platformType: number, dbPath: string): Promise<void> {
  return invoke('flightlog_update_platform_type', {
    flightId: id,
    platformType,
    dbPath: dbPath || undefined,
  });
}

export async function updateFlightPilot(id: number, pilotName: string, pilotId: string, dbPath: string): Promise<void> {
  return invoke('flightlog_update_pilot', {
    flightId: id,
    pilotName,
    pilotId,
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

export async function getDefaultRawLogPath(): Promise<string> {
  return invoke<string>('flightlog_default_raw_log_path');
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

/** Result of parsing a recorded raw log (.rawmsp / .tlog) into the logbook (ADR-049). */
export interface RawImportResult {
  imported: number;
  skipped: number;
  flightIds: number[];
}

/** Parse a recorded raw serial log into the logbook as LIVE flights (split at arm/disarm). */
export async function importRawLog(filePath: string, dbPath: string): Promise<RawImportResult> {
  return invoke<RawImportResult>('flightlog_import_raw', {
    filePath,
    dbPath: dbPath || undefined,
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
