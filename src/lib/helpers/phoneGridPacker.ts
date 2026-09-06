// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget grid packer (Dev-Docs active/PHONE_UI.md §3) — pure, deterministic, UI-free, and
// free of runtime imports so `node --experimental-strip-types` can run a check script against it.
//
// POSITIONAL model: every active widget carries the position the user last gave it (page, row,
// col). Packing "settles" the grid:
//   1. Widgets are processed top-down (page, row, col). Each takes its own position if free.
//   2. If not (a size change, a foreign drop, a smaller raster), it slides DOWN to the first fit at
//      or below its position (next pages included); with nothing below, it takes the first fit
//      anywhere (top-down); with nothing at all it overflows (the caller deactivates it).
//   3. Gravity — from where it landed it RISES while every slot directly above its span is free.
//      Nothing rises through an occupied slot, so an S under a W stays under the W even if the W
//      left a hole beside it (Marc's example); no gap ever sits directly above a widget; no zig-zag.
//   4. A span never crosses the page boundary; rising stops at the page top.
//   5. The panel is 1 column wide iff no 2-wide widget is active and the 1×1 tiles fit on the
//      pages; else maxCols. Columns are ignored while 1 column wide.
// A widget that is (re)activated has no position yet: it is appended at the END (last page, last
// row) and therefore ends up in the earliest free slot — "wo wieder Platz ist".
//
// Geometry (rows / max cols / pages) is a parameter — the caller passes config/phoneGrid.ts. The
// raster may change to 5 × 2; nothing here assumes 4 rows.

import type { WidgetSize } from '../config/widgetRegistry';

export interface PhoneGridGeometry {
  rows: number;
  maxCols: number;
  pages: number;
}

/** One widget with the position the user last gave it. `col` is meaningful for 1×1 tiles only
 *  (2-wide spans always sit at column 0). `page`/`row` = "append at the end" when missing. */
export interface PhoneWidgetEntry {
  id: string;
  size: WidgetSize;
  col: number;
  active: boolean;
  page?: number;
  row?: number;
}

export interface Placement {
  id: string;
  size: WidgetSize;
  page: number;
  row: number;
  col: number;
  /** Span in slots. */
  w: number;
  h: number;
}

export interface PackResult {
  placements: Placement[];
  /** Columns the panel needs (1 … maxCols). */
  cols: number;
  /** Active entries that found no slot, in order. */
  overflow: string[];
  /** The input entries with the settled positions written back (inactive ones untouched). */
  settled: PhoneWidgetEntry[];
}

/** Slot span of a size state: S 1×1, L 2×2, W 2×1. */
export function spanOf(size: WidgetSize): { w: number; h: number } {
  if (size === 'L') return { w: 2, h: 2 };
  if (size === 'W') return { w: 2, h: 1 };
  return { w: 1, h: 1 };
}

/** Rule 5: how many columns the active set needs. */
export function columnsFor(entries: PhoneWidgetEntry[], geom: PhoneGridGeometry): number {
  const active = entries.filter((e) => e.active);
  const needsWide = active.some((e) => spanOf(e.size).w > 1);
  if (!needsWide && active.length <= geom.rows * geom.pages) return 1;
  return geom.maxCols;
}

export function packPhoneGrid(entries: PhoneWidgetEntry[], geom: PhoneGridGeometry): PackResult {
  const cols = columnsFor(entries, geom);
  const occ: boolean[][][] = Array.from({ length: geom.pages }, () =>
    Array.from({ length: geom.rows }, () => Array.from({ length: cols }, () => false)),
  );
  const free = (page: number, row: number, col: number, w: number, h: number): boolean => {
    if (page < 0 || page >= geom.pages || row < 0 || row + h > geom.rows || col < 0 || col + w > cols) return false;
    for (let r = row; r < row + h; r++) for (let c = col; c < col + w; c++) if (occ[page][r][c]) return false;
    return true;
  };
  const take = (page: number, row: number, col: number, w: number, h: number) => {
    for (let r = row; r < row + h; r++) for (let c = col; c < col + w; c++) occ[page][r][c] = true;
  };

  // Rule 1: top-down processing; entries without a position come last (append at the end).
  const END = geom.pages * geom.rows;
  const linear = (e: PhoneWidgetEntry) => (e.page == null || e.row == null ? END : e.page * geom.rows + e.row);
  const order = entries
    .map((e, i) => ({ e, i }))
    .filter(({ e }) => e.active)
    .sort((a, b) => linear(a.e) - linear(b.e) || a.e.col - b.e.col || a.i - b.i);

  const placements: Placement[] = [];
  const overflow: string[] = [];
  const settledById = new Map<string, Placement>();

  for (const { e } of order) {
    const span = spanOf(e.size);
    const w = Math.min(span.w, cols); // a 2-wide span can't exist in a 1-column panel (rule 5)
    const h = span.h;
    const col = w > 1 ? 0 : Math.min(Math.max(0, Math.trunc(e.col) || 0), cols - 1);
    let hit: { page: number; row: number; col: number } | null = null;
    const tryRows = (page: number, from: number, to: number, c: number) => {
      for (let row = from; row < to && !hit; row++) if (free(page, row, c, w, h)) hit = { page, row, col: c };
    };
    // Own column first; a 1×1 tile may fall back to the other column(s) — that is what turns a
    // drop onto an occupied slot into a swap (the displaced tile takes the vacated slot).
    const otherCols = w > 1 ? [] : Array.from({ length: cols }, (_, c) => c).filter((c) => c !== col);
    if (e.page == null || e.row == null) {
      // No position yet ((re)activation) → the earliest free slot anywhere, scanned from the top.
      for (let page = 0; page < geom.pages && !hit; page++) for (const c of [col, ...otherCols]) tryRows(page, 0, geom.rows, c);
    } else {
      // Rule 2: own position, else the first fit BELOW it on the same page, else anywhere on the
      // same page (own column, then the other), else the later pages, else the earlier ones.
      const ownPage = Math.min(e.page, geom.pages - 1);
      const ownRow = Math.min(e.row, geom.rows - 1);
      tryRows(ownPage, ownRow, geom.rows, col);
      tryRows(ownPage, 0, ownRow, col);
      for (const c of otherCols) tryRows(ownPage, 0, geom.rows, c);
      for (let page = ownPage + 1; page < geom.pages && !hit; page++) for (const c of [col, ...otherCols]) tryRows(page, 0, geom.rows, c);
      for (let page = ownPage - 1; page >= 0 && !hit; page--) for (const c of [col, ...otherCols]) tryRows(page, 0, geom.rows, c);
    }
    // (`hit` is written inside `tryRows`; TS's flow analysis can't see closure writes, hence the
    // re-typed alias.)
    const found = hit as { page: number; row: number; col: number } | null;
    if (!found) {
      overflow.push(e.id);
      continue;
    }
    // Rule 3: gravity — rise within the page while the row above the span is free.
    const { page, col: hitCol } = found;
    let row = found.row;
    while (row > 0 && free(page, row - 1, hitCol, w, 1)) row--;
    take(page, row, hitCol, w, h);
    const p: Placement = { id: e.id, size: e.size, page, row, col: hitCol, w, h };
    placements.push(p);
    settledById.set(e.id, p);
  }

  const settled = entries.map((e) => {
    const p = settledById.get(e.id);
    return p ? { ...e, page: p.page, row: p.row, col: p.col } : e;
  });
  return { placements, cols, overflow, settled };
}

/** Would `id` (with `size`, `col`) find a slot if activated (appended at the end)? */
export function canActivate(
  entries: PhoneWidgetEntry[],
  id: string,
  size: WidgetSize,
  col: number,
  geom: PhoneGridGeometry,
): boolean {
  const trial = entries.filter((e) => e.id !== id).map((e) => ({ ...e }));
  trial.push({ id, size, col, active: true });
  return !packPhoneGrid(trial, geom).overflow.includes(id);
}
