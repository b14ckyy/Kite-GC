import {
  listFlights,
  getFlight,
  getFlightTrack,
  deleteFlight,
  updateFlightNotes,
  updateFlightWeather,
  geocodeFlight,
  fetchFlightWeather,
  importBlackboxLog,
  exportFlights,
  exportBlackboxFile,
  exportTrackFile,
  importKflight,
  type Flight,
  type FlightSummary,
  type TelemetryRecord,
  type KflightImportResult,
} from '$lib/stores/flightlog';
import { isValidGpsCoordinate, hasKnownLocation } from '$lib/helpers/telemetry';

/** Result of selecting / loading a flight with all its metadata. */
export interface FlightSelection {
  flight: Flight | null;
  track: TelemetryRecord[];
  trackCount: number;
  notes: string;
  weatherTempC: string;
  weatherWindMs: string;
  weatherWindDir: string;
  weatherDesc: string;
  hasGpsData: boolean;
}

/** Load the flight summary list. */
export async function loadFlights(dbPath: string): Promise<FlightSummary[]> {
  return listFlights(dbPath);
}

/**
 * Fetch flight details, track, and lazily enrich metadata (geocode + weather).
 * Returns everything needed to populate the page state in one call.
 */
export async function selectFlightData(
  flightId: number,
  dbPath: string,
  locale: string,
): Promise<FlightSelection> {
  let flight = await getFlight(flightId, dbPath);
  const track = await getFlightTrack(flightId, dbPath);
  const hasGpsData = track.some((p) => isValidGpsCoordinate(p.lat, p.lon));

  // Lazy geocode
  if (
    flight &&
    !hasKnownLocation(flight.location_name) &&
    flight.start_lat != null &&
    flight.start_lon != null
  ) {
    const geocoded = await geocodeFlight(flightId, dbPath, locale);
    if (geocoded) flight = { ...flight, location_name: geocoded };
  }

  // Lazy weather fetch for live-recorded flights
  if (
    flight &&
    flight.source !== 'blackbox' &&
    flight.weather_temp_c == null &&
    flight.start_lat != null &&
    flight.start_lon != null
  ) {
    await fetchFlightWeather(flightId, dbPath);
    flight = await getFlight(flightId, dbPath);
  }

  return {
    flight,
    track,
    trackCount: track.length,
    notes: flight?.notes ?? '',
    weatherTempC: flight?.weather_temp_c != null ? String(flight.weather_temp_c) : '',
    weatherWindMs: flight?.weather_wind_ms != null ? String(flight.weather_wind_ms) : '',
    weatherWindDir: flight?.weather_wind_deg != null ? String(flight.weather_wind_deg) : '',
    weatherDesc: flight?.weather_desc ?? '',
    hasGpsData,
  };
}

/** Save flight notes and return the updated flight. */
export async function saveNotes(
  flightId: number,
  notes: string,
  dbPath: string,
): Promise<Flight | null> {
  await updateFlightNotes(flightId, notes, dbPath);
  return getFlight(flightId, dbPath);
}

/** Parse and save weather fields, return the updated flight. */
export async function saveWeather(
  flightId: number,
  tempC: string,
  windMs: string,
  windDir: string,
  desc: string,
  dbPath: string,
): Promise<Flight | null> {
  const temp = tempC !== '' && !isNaN(Number(tempC)) ? Number(tempC) : null;
  const wind = windMs !== '' && !isNaN(Number(windMs)) ? Number(windMs) : null;
  const dir = windDir !== '' ? parseInt(windDir, 10) : null;
  const descVal = desc?.trim() || null;
  await updateFlightWeather(flightId, temp, wind, dir, descVal, dbPath);
  return getFlight(flightId, dbPath);
}

/** Delete a flight. Returns true if successful. */
export async function removeFlight(
  flightId: number,
  dbPath: string,
): Promise<boolean> {
  return deleteFlight(flightId, dbPath);
}

/**
 * Import a blackbox log file. Returns the raw result from the backend
 * (may be 'success' or 'duplicate' — caller handles the UI confirmation).
 */
export async function importBlackbox(
  filePath: string,
  dbPath: string,
  logIndex: number | undefined,
  forceImport: boolean,
  locale: string,
) {
  return importBlackboxLog(filePath, dbPath, logIndex, forceImport, locale);
}

/** Export selected flights to a .kflight file. Returns the number exported. */
export async function exportSelectedFlights(
  flightIds: number[],
  outputPath: string,
  dbPath: string,
): Promise<number> {
  return exportFlights(flightIds, outputPath, dbPath);
}

/** Import flights from a .kflight file. Returns the import result. */
export async function importFromKflight(
  filePath: string,
  dbPath: string,
): Promise<KflightImportResult> {
  return importKflight(filePath, dbPath);
}

/** Export the raw blackbox binary file. Returns the original filename. */
export async function exportBlackbox(
  flightId: number,
  outputPath: string,
  dbPath: string,
): Promise<string> {
  return exportBlackboxFile(flightId, outputPath, dbPath);
}

/** Export a flight track as KMZ/KML/GPX/CSV (format from file extension). */
export async function exportTrack(
  flightId: number,
  outputPath: string,
  dbPath: string,
): Promise<void> {
  return exportTrackFile(flightId, outputPath, dbPath);
}
