// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Per-panel UI view state (which tab is open, compact vs advanced size) — kept in a module-level
// store so it survives switching between nav-rail panels (each panel is unmounted when inactive,
// which would otherwise reset its local `$state`). Persisted to localStorage so it also survives a
// restart, matching the behaviour of the Airspace panel's `compact` (which lives in `settings`).
// This is UI view state, not data settings — hence its own lightweight store.

import { writable } from 'svelte/store';

const SETTINGS_TABS = ['interface', 'map', 'data'] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];

export interface PanelState {
  /** Radar panel: compact (info) vs advanced (full) view. */
  radarCompact: boolean;
  /** Settings panel: which tab was last open. */
  settingsTab: SettingsTab;
}

const DEFAULTS: PanelState = {
  radarCompact: false,
  settingsTab: 'interface',
};

const STORAGE_KEY = 'kite-gc-panelstate';

function load(): PanelState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const merged = { ...DEFAULTS, ...(JSON.parse(raw) as Partial<PanelState>) };
      // A tab that no longer exists (or a hand-edited value) falls back to the first one.
      if (!SETTINGS_TABS.includes(merged.settingsTab)) merged.settingsTab = DEFAULTS.settingsTab;
      return merged;
    }
  } catch {
    // ignore parse errors → defaults
  }
  return { ...DEFAULTS };
}

function createPanelState() {
  const { subscribe, update } = writable<PanelState>(load());
  return {
    subscribe,
    /** Patch one or more fields and persist. */
    patch(partial: Partial<PanelState>) {
      update((current) => {
        const next = { ...current, ...partial };
        try { localStorage.setItem(STORAGE_KEY, JSON.stringify(next)); } catch { /* ignore */ }
        return next;
      });
    },
  };
}

export const panelState = createPanelState();
