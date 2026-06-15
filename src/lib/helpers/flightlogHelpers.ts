// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import type {
  FlightSummary,
  FlightTree,
  FlightTreeTopLevel,
  FlightTreeSecondLevel,
  LogbookSortMode,
  LogbookSortDimension,
} from '$lib/stores/flightlogTypes';

// ── Flight-local time (ADR-048) ──────────────────────────────────────────────
// A flight's `start_time` is always true UTC; `utc_offset_min` shifts it to the wall-clock of the
// place the flight happened. We shift the epoch by the offset and then read the **UTC** components of
// the result — that yields the flight-local wall clock without involving the browser's own timezone.
// Unknown offset (null) → fall back to UTC and mark it, rather than guessing.

const pad2 = (n: number) => String(n).padStart(2, '0');

/** The flight-local `Date` (read its `getUTC*` parts), or null if the timestamp is unparseable. */
export function flightLocalDate(isoUtc: string, offsetMin: number | null | undefined): Date | null {
  const baseMs = new Date(isoUtc).getTime();
  if (!Number.isFinite(baseMs)) return null;
  return new Date(baseMs + (offsetMin ?? 0) * 60_000);
}

/** Timezone label, e.g. `UTC+02:00`, or `UTC` when the offset is unknown. */
export function flightTzLabel(offsetMin: number | null | undefined): string {
  if (offsetMin == null) return 'UTC';
  const sign = offsetMin < 0 ? '-' : '+';
  const a = Math.abs(offsetMin);
  return `UTC${sign}${pad2(Math.floor(a / 60))}:${pad2(a % 60)}`;
}

/** `YYYY-MM-DD HH:MM` in flight-local time (no timezone label — pair with {@link flightTzLabel}). */
export function formatFlightDateTime(isoUtc: string, offsetMin: number | null | undefined): string {
  const d = flightLocalDate(isoUtc, offsetMin);
  if (!d) return '-';
  return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())} ${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}`;
}

/** `HH:MM:SS` in flight-local time (for the replay clock readout). */
export function formatFlightClock(isoUtc: string, offsetMin: number | null | undefined): string | null {
  const d = flightLocalDate(isoUtc, offsetMin);
  if (!d) return null;
  return `${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}:${pad2(d.getUTCSeconds())}`;
}

/** Date-only grouping key (`YYYY-MM-DD`) in flight-local time, so a late-evening flight groups under
 *  its local date rather than the UTC one. */
function dateKey(ts: string, offsetMin?: number | null): string {
  const d = flightLocalDate(ts, offsetMin);
  if (!d) return ts.slice(0, 10);
  return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`;
}

function primaryOrUnknown(v: string | null | undefined, unknown = 'Unknown'): string {
  const s = (v ?? '').trim();
  return s.length > 0 ? s : unknown;
}

function getModeDimensions(
  mode: LogbookSortMode,
): [LogbookSortDimension, LogbookSortDimension, LogbookSortDimension] {
  if (mode === 'aircraft-location-date') return ['aircraft', 'location', 'date'];
  if (mode === 'location-date-aircraft') return ['location', 'date', 'aircraft'];
  if (mode === 'date-location-aircraft') return ['date', 'location', 'aircraft'];
  return ['aircraft', 'date', 'location'];
}

function getDimensionValue(f: FlightSummary, dim: LogbookSortDimension): string {
  if (dim === 'aircraft') return primaryOrUnknown(f.craft_name, 'Unnamed craft');
  if (dim === 'location') return primaryOrUnknown(f.location_name, 'Unknown location');
  return dateKey(f.start_time, f.utc_offset_min);
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
  const thirdCmp = compareDimensionValues(
    getDimensionValue(a, third),
    getDimensionValue(b, third),
    third,
  );
  if (thirdCmp !== 0) return thirdCmp;
  return new Date(b.start_time).getTime() - new Date(a.start_time).getTime();
}

export function buildFlightTree(flights: FlightSummary[], mode: LogbookSortMode): FlightTree {
  const dimensions = getModeDimensions(mode);
  const [topDim, secondDim, thirdDim] = dimensions;
  const topMap = new Map<string, Map<string, FlightSummary[]>>();

  // Hide linked partner flights: the later-created flight (higher id) is the secondary one.
  // If linked_flight_id < id, this flight was imported after its partner → hide it.
  const visibleFlights = flights.filter(
    (f) => !(f.linked_flight_id && f.linked_flight_id < f.id),
  );

  for (const flight of visibleFlights) {
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
        .map(([secondKey, list]) => {
          const flights = [...list].sort((a, b) => compareByThirdThenTime(a, b, thirdDim));
          return {
            key: secondKey,
            flights,
            sum_duration_sec: flights.reduce((s, f) => s + (f.duration_sec ?? 0), 0),
            sum_distance_m: flights.reduce((s, f) => s + (f.total_distance_m ?? 0), 0),
          };
        });

      return {
        key: topKey,
        children,
        flight_count: children.reduce((sum, child) => sum + child.flights.length, 0),
        sum_duration_sec: children.reduce((sum, child) => sum + child.sum_duration_sec, 0),
        sum_distance_m: children.reduce((sum, child) => sum + child.sum_distance_m, 0),
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
