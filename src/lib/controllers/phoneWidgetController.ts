// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget layout — pure config operations over `PhoneWidgetsConfig` (persisted in settings,
// separate from the desktop PanelConfig; Dev-Docs active/PHONE_UI.md D13). The packer
// (helpers/phoneGridPacker.ts) turns the config into slot placements; this module keeps the
// config consistent: registry-sanitised, overflowed widgets deactivated, sizes valid for the shape.

import { WIDGET_MAP, effectiveWidgetSize, nextWidgetSize, type WidgetSize } from '$lib/config/widgetRegistry';
import { PHONE_GRID_MAX_COLS, PHONE_GRID_PAGES, PHONE_GRID_ROWS } from '$lib/config/phoneGrid';
import {
  canActivate,
  packPhoneGrid,
  type PackResult,
  type PhoneGridGeometry,
  type PhoneWidgetEntry,
} from '$lib/helpers/phoneGridPacker';

export interface PhoneWidgetsConfig {
  /** The user's order; inactive entries keep their place for when they come back. */
  entries: PhoneWidgetEntry[];
}

/** The raster from config/phoneGrid.ts (D15 — may become 5 × 2). */
export const PHONE_GEOMETRY: PhoneGridGeometry = {
  rows: PHONE_GRID_ROWS,
  maxCols: PHONE_GRID_MAX_COLS,
  pages: PHONE_GRID_PAGES,
};

/** Marc's default layout (PHONE_UI.md §6): page 1 AHI (L), Home · Altitude, GPS · Compass;
 *  page 2 Live AGL (W), Terrain Radar (L), Flight Mode · Battery. Speed, RC Link, Battery 2 off. */
export const DEFAULT_PHONE_WIDGETS: PhoneWidgetsConfig = {
  entries: [
    { id: 'ahi', size: 'L', col: 0, active: true, page: 0, row: 0 },
    { id: 'home', size: 'S', col: 0, active: true, page: 0, row: 2 },
    { id: 'altitude', size: 'S', col: 1, active: true, page: 0, row: 2 },
    { id: 'gps', size: 'S', col: 0, active: true, page: 0, row: 3 },
    { id: 'compass', size: 'S', col: 1, active: true, page: 0, row: 3 },
    { id: 'liveAgl', size: 'W', col: 0, active: true, page: 1, row: 0 },
    { id: 'terrainRadar', size: 'L', col: 0, active: true, page: 1, row: 1 },
    { id: 'flightMode', size: 'S', col: 0, active: true, page: 1, row: 3 },
    { id: 'battery', size: 'S', col: 1, active: true, page: 1, row: 3 },
    { id: 'speed', size: 'S', col: 0, active: false },
    { id: 'rcLink', size: 'S', col: 1, active: false },
    { id: 'battery2', size: 'S', col: 0, active: false },
  ],
};

export function packPhone(cfg: PhoneWidgetsConfig): PackResult {
  return packPhoneGrid(cfg.entries, PHONE_GEOMETRY);
}

/** Drop ids the registry doesn't know, coerce sizes to the widget's shape, append registry
 *  widgets the config has never seen (inactive), deactivate whatever overflows. Returns the same
 *  object when nothing changed. */
export function normalizePhoneWidgets(cfg: PhoneWidgetsConfig): PhoneWidgetsConfig {
  const seen = new Set<string>();
  let changed = false;
  const entries: PhoneWidgetEntry[] = [];
  for (const e of cfg.entries ?? []) {
    if (!WIDGET_MAP.has(e.id) || seen.has(e.id)) {
      changed = true;
      continue;
    }
    seen.add(e.id);
    const size = effectiveWidgetSize(e.id, { [e.id]: e.size });
    const col = e.col === 1 ? 1 : 0;
    if (size !== e.size || col !== e.col) changed = true;
    entries.push({ id: e.id, size, col, active: !!e.active, page: e.page, row: e.row });
  }
  for (const def of WIDGET_MAP.values()) {
    if (!seen.has(def.id)) {
      entries.push({ id: def.id, size: def.defaultSize, col: 0, active: false });
      changed = true;
    }
  }
  const packed = packPhoneGrid(entries, PHONE_GEOMETRY);
  const overflow = new Set(packed.overflow);
  const settled = packed.settled.map((e) => (e.active && overflow.has(e.id) ? { ...e, active: false } : e));
  // Settled positions are the persisted truth (a stale position would re-settle identically, but
  // the stored state should read like the screen).
  for (let i = 0; i < settled.length; i++) {
    const a = settled[i];
    const b = entries[i];
    if (a.active !== b.active || a.page !== b.page || a.row !== b.row || a.col !== b.col) changed = true;
  }
  return changed ? { entries: settled } : cfg;
}

export function isPhoneWidgetActive(cfg: PhoneWidgetsConfig, id: string): boolean {
  return cfg.entries.some((e) => e.id === id && e.active);
}

/** Page (0-based) the widget sits on, or null when inactive/unplaced. */
export function phoneWidgetPage(cfg: PhoneWidgetsConfig, id: string): number | null {
  const p = packPhone(cfg).placements.find((x) => x.id === id);
  return p ? p.page : null;
}

/** Toggle a widget. Activation is refused (returns null) when the grid has no slot for it. */
export function togglePhoneWidget(cfg: PhoneWidgetsConfig, id: string): PhoneWidgetsConfig | null {
  const cur = cfg.entries.find((e) => e.id === id);
  if (!cur) return cfg;
  if (cur.active) {
    return normalizePhoneWidgets({ entries: cfg.entries.map((e) => (e.id === id ? { ...e, active: false } : e)) });
  }
  if (!canActivate(cfg.entries, id, cur.size, cur.col, PHONE_GEOMETRY)) return null;
  // Re-activated widgets go to the END of the order (they get the next free slot, never shove
  // others around).
  const rest = cfg.entries.filter((e) => e.id !== id);
  return normalizePhoneWidgets({ entries: [...rest, { ...cur, active: true, page: undefined, row: undefined }] });
}

/** Step a widget to its next size state (S↔L for squares, W→L→S for wide tiles). A size that
 *  no longer fits deactivates what overflows (normalize) — the caller may want to warn. */
export function cyclePhoneWidgetSize(cfg: PhoneWidgetsConfig, id: string): PhoneWidgetsConfig {
  const def = WIDGET_MAP.get(id);
  const cur = cfg.entries.find((e) => e.id === id);
  if (!def || !cur) return cfg;
  const next: WidgetSize = nextWidgetSize(def.shape, cur.size);
  return normalizePhoneWidgets({ entries: cfg.entries.map((e) => (e.id === id ? { ...e, size: next } : e)) });
}

/** Put a widget at a position (page, row, col — the user's drop); the packer settles it. */
export function movePhoneWidget(cfg: PhoneWidgetsConfig, id: string, page: number, row: number, col: number): PhoneWidgetsConfig {
  const cur = cfg.entries.find((e) => e.id === id);
  if (!cur) return cfg;
  return normalizePhoneWidgets({
    entries: cfg.entries.map((e) => (e.id === id ? { ...e, page, row, col: col === 1 ? 1 : 0 } : e)),
  });
}
