// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget grid packer (Dev-Docs active/PHONE_UI.md §3) — pure and free of runtime imports, so
// `node --experimental-strip-types` can run a check script against it. Widgets keep their last position;
// packing settles the grid. Rules (referenced below): 1 top-down order, unpositioned last; 2 own slot,
// else the first fit below, else anywhere; 3 gravity — rise while the slot above the span is free; 4 a
// span never crosses a page; 5 one column iff no 2-wide widget is active and the 1×1 tiles fit.

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

/** The widget a drop just put on its position, and its place in the processing order: ahead of the
 *  widgets that start on its row, or (`late`) behind every widget that starts on one of the rows of
 *  its span (any column). */
export interface PackMove {
  id: string;
  late: boolean;
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

export function packPhoneGrid(entries: PhoneWidgetEntry[], geom: PhoneGridGeometry, moved?: PackMove): PackResult {
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
  const key = (e: PhoneWidgetEntry) => {
    const at = linear(e);
    if (!moved || e.id !== moved.id || at === END) return at;
    return moved.late ? at + spanOf(e.size).h - 0.5 : at - 0.5;
  };
  const order = entries
    .map((e, i) => ({ e, i }))
    .filter(({ e }) => e.active)
    .sort((a, b) => key(a.e) - key(b.e) || a.e.col - b.e.col || a.i - b.i);

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

/** A drop: put widget `id` on (page, row, col) and settle the grid; what finds no slot is deactivated. The
 *  widget keeps the dropped row wherever possible and the occupant gives way (up into the hole, else
 *  down); of the two processing orders (`PackMove`) the one achieving that wins. A 1×1 on a 1×1 swaps. */
export function packPhoneMove(
  entries: PhoneWidgetEntry[],
  geom: PhoneGridGeometry,
  id: string,
  page: number,
  row: number,
  col: number,
): PhoneWidgetEntry[] {
  const cur = entries.find((e) => e.id === id);
  if (!cur) return entries;
  const before = packPhoneGrid(entries, geom);
  const span = spanOf(cur.size);
  // A span never crosses the page end: a tall widget dropped on the last row sits one row higher.
  const p = Math.max(0, Math.min(geom.pages - 1, page));
  const r = Math.max(0, Math.min(geom.rows - span.h, row));
  const c = span.w > 1 ? 0 : Math.max(0, Math.min(geom.maxCols - 1, col));
  const own = before.placements.find((x) => x.id === id);
  const there =
    own && own.w === 1 && own.h === 1
      ? before.placements.find(
          (x) => x.id !== id && x.w === 1 && x.h === 1 && x.page === p && x.row === r && x.col === Math.min(c, before.cols - 1),
        )
      : undefined;
  const next = entries.map((e) => {
    if (e.id === id) return { ...e, active: true, page: p, row: r, col: c };
    if (own && there && e.id === there.id) return { ...e, page: own.page, row: own.row, col: own.col };
    return e;
  });
  const landed = (res: PackResult) => res.placements.some((x) => x.id === id && x.page === p && x.row === r);
  const fits = (res: PackResult) => res.overflow.length === 0;
  // A move never costs a widget its place: an order that leaves one without a slot is not taken while another
  // keeps them all. The last try is the plain order without the swap — the answer when none fits.
  const tries = [
    packPhoneGrid(next, geom, { id, late: false }),
    packPhoneGrid(next, geom, { id, late: true }),
    packPhoneGrid(next, geom),
    packPhoneGrid(entries.map((e) => (e.id === id ? { ...e, active: true, page: p, row: r, col: c } : e)), geom),
  ];
  const res = tries.find((t) => fits(t) && landed(t)) ?? tries.find(fits) ?? tries[3];
  const overflow = new Set(res.overflow);
  return res.settled.map((e) => (e.active && overflow.has(e.id) ? { ...e, active: false } : e));
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
