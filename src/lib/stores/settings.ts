// Persistent application settings
// Stores user preferences in localStorage, survives between sessions

import { writable } from 'svelte/store';

export interface MapState {
  center: [number, number];
  zoom: number;
}

export interface AppSettings {
  lastPort: string;
  lastBaud: number;
  map: MapState;
  navPanelOpen: boolean;
  activeTab: string;
}

const STORAGE_KEY = 'inav-gcs-settings';

const defaults: AppSettings = {
  lastPort: '',
  lastBaud: 115200,
  map: {
    center: [51.505, -0.09],
    zoom: 13,
  },
  navPanelOpen: false,
  activeTab: 'uav-info',
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
