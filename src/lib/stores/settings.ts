// Persistent application settings
// Stores user preferences in localStorage, survives between sessions

import { writable } from 'svelte/store';

export interface MapState {
  center: [number, number];
  zoom: number;
}

export interface PanelConfig {
  bottom: string[]; // widget IDs in display order
  right: string[];  // widget IDs in display order
  /** Remembers last panel assignment per widget so toggle off/on restores position */
  positions?: Record<string, 'bottom' | 'right'>;
}

export type SpeedUnit = 'kmh' | 'mph' | 'ms' | 'fts' | 'kt';
export type AltitudeUnit = 'm' | 'ft';
export type DistanceUnit = 'metric' | 'imperial';
export type VerticalSpeedUnit = 'ms' | 'fts';
export type TemperatureUnit = 'c' | 'f';
export type NightMode = 'off' | 'auto' | 'on';

export interface InterfaceSettings {
  speedUnit: SpeedUnit;
  altitudeUnit: AltitudeUnit;
  distanceUnit: DistanceUnit;
  verticalSpeedUnit: VerticalSpeedUnit;
  temperatureUnit: TemperatureUnit;
}

/** One online ADS-B provider. `url` is a template with `{lat}`/`{lon}`/`{dist}` placeholders
 *  (`dist` filled in NM by the backend). */
export interface AdsbOnlineProvider {
  name: string;
  url: string;
  apiKey?: string;
  enabled: boolean;
}

/** Fixed, always-present ADS-B providers (URL defined in code, not editable/deletable). Only their
 *  on/off state is persisted (in `radar.adsb.builtins`). */
export const BUILTIN_ADSB_PROVIDERS: { name: string; url: string }[] = [
  { name: 'adsb.lol', url: 'https://api.adsb.lol/v2/point/{lat}/{lon}/{dist}' },
  { name: 'adsb.one', url: 'https://api.adsb.one/v2/point/{lat}/{lon}/{dist}' },
];

/** Radar (foreign-vehicle tracking) settings: master switch + per-system enables + per-system
 *  source config (ADS-B online from Phase 1; more added per phase). */
export interface RadarSettings {
  /** Master switch — off hides the whole Radar panel/feature. */
  enabled: boolean;
  adsb: {
    enabled: boolean;
    /** On/off state for the fixed built-in providers, keyed by name (URL lives in code). */
    builtins: Record<string, boolean>;
    /** User-editable custom providers (e.g. adsb.fi example). Merged with the built-ins by ICAO. */
    online: AdsbOnlineProvider[];
    /** Query radius in km — dropdown 10/25/50/75/100, capped at 100. */
    radiusKm: number;
    /** Poll interval in seconds (provider limit ≈ 1 req/s, so ≥2 s). */
    pollSec: number;
  };
  formationFlight: { enabled: boolean };
  radio: { enabled: boolean };
  /** Dev-only synthetic source (ignored by release backend). */
  sim: boolean;
}

/** Default radar settings — built-ins (adsb.lol/.one) on; adsb.fi as a custom example (off). */
export const DEFAULT_RADAR: RadarSettings = {
  enabled: false,
  adsb: {
    enabled: false,
    builtins: { 'adsb.lol': true, 'adsb.one': true },
    online: [
      { name: 'adsb.fi', url: 'https://opendata.adsb.fi/api/v3/lat/{lat}/lon/{lon}/dist/{dist}', enabled: false },
    ],
    radiusKm: 25,
    pollSec: 5,
  },
  formationFlight: { enabled: false },
  radio: { enabled: false },
  sim: false,
};

export interface AppSettings {
  lastPort: string;
  lastBaud: number;
  lastProtocol: string;
  map: MapState;
  mapProvider: string;
  mapCacheMaxMB: number;
  navPanelOpen: boolean;
  activeTab: string;
  // Telemetry poll rates
  attitudeRateHz: number;
  positionRateHz: number;
  airspeedEnabled: boolean;
  // Flight logging
  flightLoggingEnabled: boolean;
  flightRecordingEnabled: boolean;
  flightLogDbPath: string;
  flightLogRawEnabled: boolean;
  flightLogRawAlways: boolean;
  // Mission Control
  defaultWpAltitudeM: number;
  defaultPhTimeSec: number;
  lastAutopilotSystem: string;
  // Alerts
  warnAltitudeM: number;
  // Global UI options (display-only conversions)
  interface: InterfaceSettings;
  // Widget panel layout
  panels: PanelConfig;
  // Locale / language
  locale: string;
  /** Global UI scale factor (1 = 100%, up to 2 = 200%); zooms the chrome, not the map. */
  uiScale: number;
  // 3D Map
  cesiumIonToken: string;
  /** Show the vertical altitude curtain (wall down to ground) under the 3D track. */
  altitudeCurtain3D: boolean;
  /** Light the 3D globe with the real sun position (day/night terminator + shading). */
  realLighting3D: boolean;
  /** During replay, drive the 3D sun clock from the log's recorded timestamp (not wall-clock now). */
  logReplayTime: boolean;
  /** Dim the 2D Leaflet imagery for night: off / auto (sun below horizon) / on. */
  nightMode2D: NightMode;
  /** Last known physical user location (for Night-Mode auto sunset timing); persisted across sessions. */
  userLocation: { lat: number; lon: number } | null;
  /** Radar (foreign-vehicle tracking) subsystem settings. */
  radar: RadarSettings;
}

const STORAGE_KEY = 'kite-gc-settings';

const defaults: AppSettings = {
  lastPort: '',
  lastBaud: 115200,
  lastProtocol: 'msp',
  map: {
    center: [51.505, -0.09],
    zoom: 13,
  },
  mapProvider: 'osm',
  mapCacheMaxMB: 200,
  navPanelOpen: false,
  activeTab: 'uav-info',
  attitudeRateHz: 5,
  positionRateHz: 2,
  airspeedEnabled: false,
  flightLoggingEnabled: false,
  flightRecordingEnabled: false,
  flightLogDbPath: '',
  flightLogRawEnabled: false,
  flightLogRawAlways: false,
  defaultWpAltitudeM: 50,
  defaultPhTimeSec: 30,
  lastAutopilotSystem: 'inav',
  warnAltitudeM: 120,
  interface: {
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  },
  panels: {
    bottom: ['home', 'battery', 'speed', 'ahi', 'altitude', 'gps', 'compass'],
    right: ['rawTelemetry'],
  },
  locale: 'en',
  uiScale: 1,
  cesiumIonToken: '',
  altitudeCurtain3D: true,
  realLighting3D: false,
  logReplayTime: false,
  nightMode2D: 'off',
  userLocation: null,
  radar: DEFAULT_RADAR,
};

function load(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<AppSettings>;
      return {
        ...defaults,
        ...parsed,
        interface: {
          ...defaults.interface,
          ...(parsed.interface ?? {}),
        },
        radar: (() => {
          const dr = defaults.radar;
          const pr = (parsed.radar ?? {}) as Partial<RadarSettings>;
          const pa = (pr.adsb ?? {}) as Partial<RadarSettings['adsb']>;
          const builtinNames = new Set(BUILTIN_ADSB_PROVIDERS.map((p) => p.name));
          return {
            ...dr,
            ...pr,
            adsb: {
              ...dr.adsb,
              ...pa,
              builtins: { ...dr.adsb.builtins, ...(pa.builtins ?? {}) },
              // Strip any built-in-named entries from the custom list (migration from the old flat
              // `online` array, where adsb.lol/.one lived as custom rows).
              online: (pa.online ?? dr.adsb.online).filter((p) => !builtinNames.has(p.name)),
            },
            formationFlight: { ...dr.formationFlight, ...(pr.formationFlight ?? {}) },
            radio: { ...dr.radio, ...(pr.radio ?? {}) },
          };
        })(),
      };
    }
  } catch {
    // Ignore parse errors, use defaults
  }
  return { ...defaults };
}

function createSettingsStore() {
  const initial = load();
  const { subscribe, set, update } = writable<AppSettings>(initial);

  return {
    subscribe,
    set(value: AppSettings) {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
      set(value);
    },
    update(updater: (s: AppSettings) => AppSettings) {
      update((current) => {
        const next = updater(current);
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        return next;
      });
    },
    /** Update a single field without replacing the whole object */
    patch(partial: Partial<AppSettings>) {
      update((current) => {
        const next = { ...current, ...partial };
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        return next;
      });
    },
  };
}

export const settings = createSettingsStore();
