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

export interface InterfaceSettings {
  speedUnit: SpeedUnit;
  altitudeUnit: AltitudeUnit;
  distanceUnit: DistanceUnit;
  verticalSpeedUnit: VerticalSpeedUnit;
  temperatureUnit: TemperatureUnit;
}

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
  // 3D Map
  cesiumIonToken: string;
  /** Show the vertical altitude curtain (wall down to ground) under the 3D track. */
  altitudeCurtain3D: boolean;
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
  cesiumIonToken: '',
  altitudeCurtain3D: true,
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
