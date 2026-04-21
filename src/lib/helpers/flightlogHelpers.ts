import type {
  FlightSummary,
  FlightTree,
  FlightTreeTopLevel,
  FlightTreeSecondLevel,
  LogbookSortMode,
  LogbookSortDimension,
} from '$lib/stores/flightlogTypes';

function dateKey(ts: string): string {
  const d = new Date(ts);
  return d.toISOString().slice(0, 10);
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
