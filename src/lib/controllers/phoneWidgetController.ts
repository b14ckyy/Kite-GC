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
  /** The three bottom slots (Dev-Docs active/PHONE_BOTTOM_WIDGETS.md). Missing = all empty. */
  bottom?: PhoneBottomSlots;
}

export type PhoneBottomSlot = 'left' | 'centre' | 'right';
export const PHONE_BOTTOM_SLOTS: readonly PhoneBottomSlot[] = ['left', 'centre', 'right'];

/** Widget id per slot (null = empty). A slotted widget is `active: false` in `entries` — it lives
 *  in ONE place (B6); its column position stays in the entry for the way back. `centreWide`: the
 *  centre tile is 2:1 instead of square (meaningful for the wide widget family only, B1). */
export interface PhoneBottomSlots {
  left: string | null;
  centre: string | null;
  right: string | null;
  centreWide: boolean;
}

export const EMPTY_BOTTOM: PhoneBottomSlots = { left: null, centre: null, right: null, centreWide: false };

/** Widgets that can go into a bottom slot: everything but the video widget (B9 — the native
 *  sink's hole is cut through the column's glass, the bar has none). */
export function canGoToBottom(id: string): boolean {
  return WIDGET_MAP.has(id) && id !== 'videoFeed';
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
  // Bottom slots first: an unknown / excluded / duplicated id empties the slot, and whatever sits
  // in a slot is not active in the column.
  const b = cfg.bottom ?? EMPTY_BOTTOM;
  const slotted = new Set<string>();
  const bottom: PhoneBottomSlots = { left: null, centre: null, right: null, centreWide: !!b.centreWide };
  for (const slot of PHONE_BOTTOM_SLOTS) {
    const id = b[slot];
    if (id && canGoToBottom(id) && !slotted.has(id)) {
      bottom[slot] = id;
      slotted.add(id);
    }
  }
  if (!cfg.bottom || PHONE_BOTTOM_SLOTS.some((s) => bottom[s] !== b[s]) || bottom.centreWide !== b.centreWide) changed = true;
  const entries: PhoneWidgetEntry[] = [];
  for (const e of cfg.entries ?? []) {
    if (!WIDGET_MAP.has(e.id) || seen.has(e.id)) {
      changed = true;
      continue;
    }
    seen.add(e.id);
    const size = effectiveWidgetSize(e.id, { [e.id]: e.size });
    const col = e.col === 1 ? 1 : 0;
    const active = !!e.active && !slotted.has(e.id);
    if (size !== e.size || col !== e.col || active !== !!e.active) changed = true;
    entries.push({ id: e.id, size, col, active, page: e.page, row: e.row });
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
  return changed ? { entries: settled, bottom } : cfg;
}

/** Active anywhere: in the column or in a bottom slot. */
export function isPhoneWidgetActive(cfg: PhoneWidgetsConfig, id: string): boolean {
  return cfg.entries.some((e) => e.id === id && e.active) || bottomSlotOf(cfg, id) !== null;
}

/** The bottom slot a widget sits in, or null. */
export function bottomSlotOf(cfg: PhoneWidgetsConfig, id: string): PhoneBottomSlot | null {
  const b = cfg.bottom;
  if (!b) return null;
  return PHONE_BOTTOM_SLOTS.find((s) => b[s] === id) ?? null;
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
  const slot = bottomSlotOf(cfg, id);
  if (slot) return normalizePhoneWidgets({ ...cfg, bottom: { ...(cfg.bottom ?? EMPTY_BOTTOM), [slot]: null } });
  if (cur.active) {
    return normalizePhoneWidgets({ ...cfg, entries: cfg.entries.map((e) => (e.id === id ? { ...e, active: false } : e)) });
  }
  if (!canActivate(cfg.entries, id, cur.size, cur.col, PHONE_GEOMETRY)) return null;
  // Re-activated widgets go to the END of the order (they get the next free slot, never shove
  // others around).
  const rest = cfg.entries.filter((e) => e.id !== id);
  return normalizePhoneWidgets({ ...cfg, entries: [...rest, { ...cur, active: true, page: undefined, row: undefined }] });
}

/** Step a widget to its next size state (S↔L for squares, W→L→S for wide tiles). A size that
 *  no longer fits deactivates what overflows (normalize) — the caller may want to warn. */
export function cyclePhoneWidgetSize(cfg: PhoneWidgetsConfig, id: string): PhoneWidgetsConfig {
  const def = WIDGET_MAP.get(id);
  const cur = cfg.entries.find((e) => e.id === id);
  if (!def || !cur) return cfg;
  const next: WidgetSize = nextWidgetSize(def.shape, cur.size);
  return normalizePhoneWidgets({ ...cfg, entries: cfg.entries.map((e) => (e.id === id ? { ...e, size: next } : e)) });
}

/** Put a widget at a position (page, row, col — the user's drop) in the COLUMN; the packer settles
 *  it. A widget coming out of a bottom slot frees the slot and is activated at the drop. Refused
 *  (returns null) when the column has no room for it. */
export function movePhoneWidget(cfg: PhoneWidgetsConfig, id: string, page: number, row: number, col: number): PhoneWidgetsConfig | null {
  const cur = cfg.entries.find((e) => e.id === id);
  if (!cur) return cfg;
  const slot = bottomSlotOf(cfg, id);
  const bottom = slot ? { ...(cfg.bottom ?? EMPTY_BOTTOM), [slot]: null } : cfg.bottom;
  const c = col === 1 ? 1 : 0;
  if (slot && !canActivate(cfg.entries.filter((e) => e.id !== id), id, cur.size, c, PHONE_GEOMETRY)) return null;
  return normalizePhoneWidgets({
    ...cfg,
    entries: cfg.entries.map((e) => (e.id === id ? { ...e, active: true, page, row, col: c } : e)),
    bottom,
  });
}

/** Put a widget into a bottom slot (B6). Coming from the column it is deactivated there (its
 *  position is kept for the way back); coming from another slot the slots swap; a widget already
 *  in the target slot goes back to where the moved one came from — the column, at the end of the
 *  order (the next free slot), or its own old slot. */
export function movePhoneWidgetToBottom(cfg: PhoneWidgetsConfig, id: string, slot: PhoneBottomSlot): PhoneWidgetsConfig | null {
  if (!canGoToBottom(id) || !cfg.entries.some((e) => e.id === id)) return cfg;
  const b = { ...(cfg.bottom ?? EMPTY_BOTTOM) };
  const from = bottomSlotOf(cfg, id);
  if (from === slot) return cfg;
  const displaced = b[slot];
  b[slot] = id;
  let entries = cfg.entries;
  if (from) {
    b[from] = displaced; // swap (or the old slot just empties)
  } else if (displaced) {
    // The displaced widget returns to the column at the end of the order; refused when there is
    // no room for it — the column cannot take a widget the user cannot see.
    const disp = cfg.entries.find((e) => e.id === displaced);
    if (!disp) return cfg;
    const rest = cfg.entries.filter((e) => e.id !== displaced && e.id !== id);
    if (!canActivate(rest, displaced, disp.size, disp.col, PHONE_GEOMETRY)) return null;
    entries = [...rest, { ...disp, active: true, page: undefined, row: undefined }, ...cfg.entries.filter((e) => e.id === id)];
  }
  return normalizePhoneWidgets({ ...cfg, entries, bottom: b });
}

/** The centre tile: 2:1 ↔ square (B1). */
export function togglePhoneBottomWide(cfg: PhoneWidgetsConfig): PhoneWidgetsConfig {
  const b = cfg.bottom ?? EMPTY_BOTTOM;
  return normalizePhoneWidgets({ ...cfg, bottom: { ...b, centreWide: !b.centreWide } });
}
