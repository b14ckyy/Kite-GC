// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget edit mode + the drag session, shared by the widget column (PhoneWidgetPanel) and
// the bottom slots (PhoneBottomBar) — Dev-Docs active/PHONE_BOTTOM_WIDGETS.md B6. One edit mode
// for both surfaces, one drag at a time; whoever picked the widget up owns the pointer stream and
// resolves the drop target, the other surface only previews it. The ghost is drawn at the root
// (PhoneDragGhost) because the column clips its own overflow.

import type { PhoneBottomSlot } from '$lib/controllers/phoneWidgetController';

export type PhoneDragTarget =
  | { kind: 'column'; page: number; row: number; col: number }
  | { kind: 'bottom'; slot: PhoneBottomSlot };

export interface PhoneDragSession {
  id: string;
  /** Which surface picked the widget up (and owns the window pointer listeners). */
  owner: 'column' | 'bottom';
  target: PhoneDragTarget | null;
  /** Ghost box, viewport css px. */
  x: number;
  y: number;
  w: number;
  h: number;
}

export const phoneEdit = $state({
  editing: false,
  drag: null as PhoneDragSession | null,
});

/** Column hit test, registered by PhoneWidgetPanel while mounted: viewport point → grid cell when
 *  the point is over the column, else null. */
export const phoneHitTests: {
  column: ((x: number, y: number) => { page: number; row: number; col: number } | null) | null;
} = { column: null };

/** The bottom slot under a viewport point — looks THROUGH layers above it (the docked video
 *  window may cover a slot; the drag ghost is pointer-transparent). */
export function bottomSlotAt(x: number, y: number): PhoneBottomSlot | null {
  const el = document.elementsFromPoint(x, y).find((e) => (e as HTMLElement).dataset?.bottomSlot) as
    | HTMLElement
    | undefined;
  return (el?.dataset.bottomSlot as PhoneBottomSlot | undefined) ?? null;
}

export function sameTarget(a: PhoneDragTarget | null, b: PhoneDragTarget | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === 'bottom' && b.kind === 'bottom') return a.slot === b.slot;
  if (a.kind === 'column' && b.kind === 'column') return a.page === b.page && a.row === b.row && a.col === b.col;
  return false;
}
