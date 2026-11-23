// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Canned in-flight state for the dev widget gallery (routes/dev/widgets) and the mocked backend it needs.
// Dev-only: imported solely behind `import.meta.env.DEV`, so none of it reaches a production build.

import { mockIPC } from '@tauri-apps/api/mocks';
import type { TelemetryData } from '$lib/stores/telemetry';
import type { InterfaceSettings } from '$lib/stores/settings';
import type { HomePosition } from '$lib/stores/home';
import type { WindAloft, WindLevel, WindStep } from '$lib/helpers/windAloft';
import type { WindAloftState } from '$lib/controllers/windAloft';

const EARTH_R = 6371000;
const D2R = Math.PI / 180;

export const FIXTURE_UNITS: InterfaceSettings = {
  speedUnit: 'kmh',
  altitudeUnit: 'm',
  distanceUnit: 'metric',
  verticalSpeedUnit: 'ms',
  temperatureUnit: 'c',
};

const HOME_LAT = 47.58312;
const HOME_LON = 9.72645;
const HOME_GROUND_MSL = 412;

export const FIXTURE_HOME: HomePosition = { lat: HOME_LAT, lon: HOME_LON, alt: HOME_GROUND_MSL, set: true, source: 'fc' };

/** Local metres (x east, y north) from home → lat/lon. */
function toLatLon(x: number, y: number): [number, number] {
  return [HOME_LAT + y / EARTH_R / D2R, HOME_LON + x / (EARTH_R * Math.cos(HOME_LAT * D2R)) / D2R];
}
function toLocal(lat: number, lon: number): [number, number] {
  return [(lon - HOME_LON) * D2R * EARTH_R * Math.cos(HOME_LAT * D2R), (lat - HOME_LAT) * D2R * EARTH_R];
}

const HEADING = 57;
const COURSE = 61;
const GROUND_SPEED = 18.1;
const VARIO = 0.12;
const FINAL_ALT = 121.4;
const FRAMES = 45;

// Final position 850 m from home; the flown track leads up to it on a straight course, one frame per second.
const FWD: [number, number] = [Math.sin(COURSE * D2R), Math.cos(COURSE * D2R)];
const RIGHT: [number, number] = [FWD[1], -FWD[0]];
const FINAL: [number, number] = [850 * Math.sin(20 * D2R), 850 * Math.cos(20 * D2R)];
const at = (fwd: number, right: number): [number, number] => [
  FINAL[0] + FWD[0] * fwd + RIGHT[0] * right,
  FINAL[1] + FWD[1] * fwd + RIGHT[1] * right,
];

// Rolling ground around 404 m, a ridge ahead (peak above the aircraft) and a lower hill ahead-left.
const RIDGE = at(2000, 350);
const HILL = at(1300, -1100);
function bump(x: number, y: number, c: [number, number], h: number, sAlong: number, sCross: number): number {
  const dx = x - c[0];
  const dy = y - c[1];
  const a = dx * FWD[0] + dy * FWD[1];
  const r = dx * RIGHT[0] + dy * RIGHT[1];
  return h * Math.exp(-(a * a) / (2 * sAlong * sAlong) - (r * r) / (2 * sCross * sCross));
}
function elevation(lat: number, lon: number): number {
  const [x, y] = toLocal(lat, lon);
  const rolling = 9 * Math.sin(x / 260 + 0.7) * Math.cos(y / 330 - 0.4) + 5 * Math.sin((x + y) / 140);
  return 404 + rolling + bump(x, y, RIDGE, 125, 380, 650) + bump(x, y, HILL, 70, 300, 300);
}

function frame(i: number, t0: number): TelemetryData {
  const back = FRAMES - 1 - i;
  const [x, y] = at(-back * GROUND_SPEED, 0);
  const [lat, lon] = toLatLon(x, y);
  const altitude = FINAL_ALT - VARIO * back;
  return {
    lat, lon, altMsl: HOME_GROUND_MSL + altitude, groundSpeed: GROUND_SPEED, numSat: 17, fixType: 2, gpsHdop: 0.8,
    course: COURSE,
    roll: 6, pitch: -4, yaw: HEADING,
    altitude, vario: VARIO,
    // Tailwind from the left rear; picks up 0.5 m/s² over the last frames, so the acceleration bar shows.
    airspeed: back < 4 ? 16.4 - 0.5 * back : 14.9,
    windDirFrom: 300, windSpeedMs: 4.1,
    voltage: 15.4, current: 14.6, mAhDrawn: 1870, rssi: 0, power: 225, batteryPercentage: 62, cellCount: 4,
    throttle: 52,
    batteries: [],
    gpsReceivers: [],
    link: { rssiPercent: 82, rssiDbm: -71, lq: 98, snrDb: 9 },
    armingFlags: 1 << 2, statusSeen: true, cpuLoad: 18, sensorStatus: 0, flightModeFlags: 0, mspRcOverride: false,
    sensorGyro: 1, sensorAcc: 1, sensorMag: 1, sensorBaro: 1,
    sensorGps: 1, sensorRangefinder: 0, sensorPitot: 1, sensorOpflow: 0, sensorRcReceiver: 0, prearmHealthy: 0,
    ekfStatus: 0, ekfType: 0,
    flightMode: { primary: 'cruise', modifiers: ['althold'] }, navState: 0, activeWpNumber: 0, fcMissionItems: 0,
    fcVariant: 'INAV',
    lastUpdate: t0 + i * 1000,
  };
}

/** The flight up to the shown moment; the last frame is the state every widget shows. */
export function fixtureFrames(): TelemetryData[] {
  const t0 = Date.now() - FRAMES * 1000;
  return Array.from({ length: FRAMES }, (_, i) => frame(i, t0));
}

function haversineM(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const h = Math.sin(((bLat - aLat) * D2R) / 2) ** 2
    + Math.cos(aLat * D2R) * Math.cos(bLat * D2R) * Math.sin(((bLon - aLon) * D2R) / 2) ** 2;
  return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
}
function destPoint(lat: number, lon: number, brngDeg: number, distM: number): [number, number] {
  const b = brngDeg * D2R;
  const f1 = lat * D2R;
  const dr = distM / EARTH_R;
  const f2 = Math.asin(Math.sin(f1) * Math.cos(dr) + Math.cos(f1) * Math.sin(dr) * Math.cos(b));
  const l2 = lon * D2R + Math.atan2(Math.sin(b) * Math.sin(dr) * Math.cos(f1), Math.cos(dr) - Math.sin(f1) * Math.sin(f2));
  return [f2 / D2R, l2 / D2R];
}

// Same sampling as the backend's `terrain::profile` / `terrain::fan`, over the synthetic ground.
function terrainProfile(points: [number, number][], spacingM: number) {
  const spacing = Math.max(1, spacingM);
  const sample = (dist: number, lat: number, lon: number) => ({ dist_m: dist, lat, lon, elev_m: elevation(lat, lon) });
  if (points.length === 0) return [];
  const out = [sample(0, points[0][0], points[0][1])];
  let cum = 0;
  for (let i = 1; i < points.length; i++) {
    const [lat0, lon0] = points[i - 1];
    const [lat1, lon1] = points[i];
    const seg = haversineM(lat0, lon0, lat1, lon1);
    for (let s = 1; s <= Math.floor(seg / spacing); s++) {
      const t = (s * spacing) / seg;
      if (t >= 1) break;
      out.push(sample(cum + s * spacing, lat0 + (lat1 - lat0) * t, lon0 + (lon1 - lon0) * t));
    }
    cum += seg;
    out.push(sample(cum, lat1, lon1));
  }
  return out;
}

interface FanArgs { lat: number; lon: number; headingDeg: number; halfAngleDeg: number; rangeM: number; angCells: number; radCells: number }
function terrainFan(a: FanArgs) {
  const elev: number[] = [];
  for (let i = 0; i < a.angCells; i++) {
    const brg = a.headingDeg - a.halfAngleDeg + (2 * a.halfAngleDeg * (i + 0.5)) / a.angCells;
    for (let j = 0; j < a.radCells; j++) {
      const [lat, lon] = destPoint(a.lat, a.lon, brg, (a.rangeM * (j + 0.5)) / a.radCells);
      elev.push(elevation(lat, lon));
    }
  }
  return { ang_cells: a.angCells, rad_cells: a.radCells, range_m: a.rangeM, elev };
}

// Model wind by height, freshening and veering over the next 90 min (15-minute steps).
function windAloft(): WindAloftState {
  const t0 = Math.floor(Date.now() / 900_000) * 900;
  const base: WindLevel[] = [
    { heightAglM: 10, speedMs: 3.1, dirDeg: 290 },
    { heightAglM: 80, speedMs: 4.6, dirDeg: 296 },
    { heightAglM: 120, speedMs: 5.2, dirDeg: 300 },
    { heightAglM: 180, speedMs: 6.0, dirDeg: 304 },
  ];
  const steps: WindStep[] = Array.from({ length: 8 }, (_, i) => ({
    time: t0 + i * 900,
    weatherCode: 2,
    isDay: true,
    gustMs: 8.3 + i * 0.3,
    tempC: 14,
    levels: base.map((l) => ({ ...l, speedMs: l.speedMs * (1 + i * 0.06), dirDeg: l.dirDeg + i * 3 })),
  }));
  const data: WindAloft = { elevationM: HOME_GROUND_MSL, steps };
  return { data, fetchedAt: Date.now(), failed: false, noPosition: false };
}

/** A static test-card picture for the Video widget (a neutral sky / ground gradient, no footage). */
export function videoTestCard(): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720" viewBox="0 0 1280 720">
<defs>
<linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3f6f9e"/><stop offset="1" stop-color="#b9cfdf"/></linearGradient>
<linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#6d7c4f"/><stop offset="1" stop-color="#3b4429"/></linearGradient>
</defs>
<g transform="rotate(-6 640 360)">
<rect x="-400" y="-400" width="2080" height="790" fill="url(#s)"/>
<path d="M-400 390 L120 330 L330 365 L560 300 L820 352 L1040 318 L1680 372 L1680 1120 L-400 1120 Z" fill="#59684a" opacity="0.8"/>
<rect x="-400" y="388" width="2080" height="740" fill="url(#g)"/>
</g>
</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/** Answers the backend commands the widgets reach; everything else resolves to null. */
export function installWidgetMocks(): void {
  mockIPC(
    (cmd, payload) => {
      switch (cmd) {
        case 'terrain_profile': {
          const p = payload as { points: [number, number][]; spacingM: number };
          return terrainProfile(p.points, p.spacingM);
        }
        case 'terrain_fan':
          return terrainFan(payload as unknown as FanArgs);
        case 'weather_watch':
          return (payload as { on: boolean }).on ? windAloft() : null;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
