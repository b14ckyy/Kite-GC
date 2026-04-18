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

export interface AppSettings {
  lastPort: string;
  lastBaud: number;
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
  flightLogDbPath: string;
  flightLogRawEnabled: boolean;
  // Mission Control
  defaultWpAltitudeM: number;
  defaultPhTimeSec: number;
  // Alerts
  warnAltitudeM: number;
  // Widget panel layout
  panels: PanelConfig;
  // Locale / language
  locale: string;
}

const STORAGE_KEY = 'kite-gc-settings';

const defaults: AppSettings = {
  lastPort: '',
  lastBaud: 115200,
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
  flightLogDbPath: '',
  flightLogRawEnabled: false,
  defaultWpAltitudeM: 50,
  defaultPhTimeSec: 30,
  warnAltitudeM: 120,
  panels: {
    bottom: ['home', 'battery', 'speed', 'ahi', 'altitude', 'gps', 'compass'],
    right: ['rawTelemetry'],
  },
  locale: 'en',
};

function load(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      return { ...defaults, ...JSON.parse(raw) };
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
