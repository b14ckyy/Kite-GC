// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget edit mode + drag session, shared by the column and the bottom slots (Dev-Docs
// PHONE_BOTTOM_WIDGETS.md B6): one drag at a time; whoever picked the widget up owns the pointer stream and the
// drop target, the other surface previews it. The ghost is drawn at the root (the column clips its overflow).

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
  /** The dragged tile is in its `S` state. */
  small: boolean;
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

/** A drag takes its touch away from the browser: touch-action is fixed at touch start, so the first move after
 *  the pickup would be read as a native scroll and cancel the pointer. The listener sits on the touch's OWN
 *  target — once the preview removes it from the document, touch events still arrive only there. */
export function claimTouch(target: EventTarget | null): void {
  if (!target) return;
  let warned = false;
  const block = (e: Event) => {
    if (e.cancelable) e.preventDefault();
    else if (!warned) {
      warned = true;
      console.warn('[phoneEdit] touchmove not cancelable — a native gesture may take the drag');
    }
  };
  const done = () => {
    target.removeEventListener('touchmove', block);
    target.removeEventListener('touchend', done);
    target.removeEventListener('touchcancel', done);
  };
  target.addEventListener('touchmove', block, { passive: false });
  target.addEventListener('touchend', done);
  target.addEventListener('touchcancel', done);
}

/** The bottom slot under a viewport point for the running drag, looking THROUGH layers above it. A slot stays
 *  the target while the finger is inside its box at hit time — the drop preview can move the bar under a still
 *  finger, which would otherwise flip the target every move. The memory is per drag session. */
let heldSlot: { slot: PhoneBottomSlot; box: DOMRect; drag: PhoneDragSession | null } | null = null;
export function dragSlotAt(x: number, y: number): PhoneBottomSlot | null {
  const el = document.elementsFromPoint(x, y).find((e) => (e as HTMLElement).dataset?.bottomSlot) as
    | HTMLElement
    | undefined;
  const slot = el?.dataset.bottomSlot as PhoneBottomSlot | undefined;
  if (el && slot) {
    heldSlot = { slot, box: el.getBoundingClientRect(), drag: phoneEdit.drag };
    return slot;
  }
  const b = heldSlot?.box;
  if (heldSlot && b && heldSlot.drag === phoneEdit.drag && x >= b.left && x <= b.right && y >= b.top && y <= b.bottom) {
    return heldSlot.slot;
  }
  heldSlot = null;
  return null;
}

export function sameTarget(a: PhoneDragTarget | null, b: PhoneDragTarget | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.kind !== b.kind) return false;
  if (a.kind === 'bottom' && b.kind === 'bottom') return a.slot === b.slot;
  if (a.kind === 'column' && b.kind === 'column') return a.page === b.page && a.row === b.row && a.col === b.col;
  return false;
}
