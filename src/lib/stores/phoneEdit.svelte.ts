// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget edit mode + the drag session, shared by the widget column (PhoneWidgetPanel) and
// the bottom slots (PhoneBottomBar) — Dev-Docs active/PHONE_BOTTOM_WIDGETS.md B6. One edit mode
// for both surfaces, one drag at a time; whoever picked the widget up owns the pointer stream and
// resolves the drop target, the other surface previews it (and the column turns its pages for
// either one's drag). The ghost is drawn at the root (PhoneDragGhost) because the column clips its
// own overflow.

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

/** A drag takes the touch it started from away from the browser. The long-press that ARMS edit
 *  mode begins as an ordinary touch (the column's page scroller is touch-action pan-y, a bottom
 *  tile has none set — the value is fixed at touch start, edit mode's `none` comes too late for
 *  it): the first move after the pickup would be read as a native scroll, the browser then cancels
 *  the pointer and the widget snaps back. The listener sits on the touch's OWN target: touch events
 *  keep arriving there, and only there, once the preview has taken that element out of the
 *  document — a listener on the window stops hearing them at exactly that moment. */
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

/** The bottom slot under a viewport point for the running drag — looks THROUGH layers above it (the
 *  docked video window may cover a slot; the drag ghost is pointer-transparent). A slot stays the
 *  target while the finger is inside the box it had when it was hit: the preview of that very drop
 *  can change the column's width, the bar moves with it and the tile slides out from under a finger
 *  that did not move — without the memory the target would flip between the slot and nothing on
 *  every move. The memory belongs to one drag session. */
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
