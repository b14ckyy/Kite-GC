// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/** When the last touch double-tap fired (performance.now()). A touch double-tap is followed by a
 *  browser-synthesized `dblclick` on whatever element sits under the finger AFTER the second
 *  release — after a map ⇄ video swap that is the OTHER surface, whose own double-click handler
 *  then swapped straight back (Marc: "only stays if I keep the finger down"). */
let lastTouchDoubleTap = 0;
const SYNTH_WINDOW_MS = 700;

/** Ghost events: the browser follows a touch double-tap with a synthesized `click` (on the second
 *  release) and a `dblclick`, both hit-tested FRESH against whatever lies under the finger by then.
 *  The `doubleTap` action fires on the second pointerdown and its callback swaps the surfaces at
 *  once, so the ghosts land on the surface that has just moved in — the main map, whose Leaflet
 *  double-click zoom is live there (Marc, 2026-09-09: double-tap on the full-screen phone video
 *  zoomed the returning map in) and whose click handlers (guided "fly here", deselect) are too.
 *  Swallow one of each at the window in the capture phase, before any element sees them. Armed
 *  until the tapping pointer has lifted plus a grace period (a held finger delays the ghosts), with
 *  a hard cap in case the release never reaches us. */
const GHOST_GRACE_MS = 400;
const GHOST_CAP_MS = 5000;
let ghostPending = new Set<string>();
let ghostPointerId = -1;
let ghostTimer: ReturnType<typeof setTimeout> | undefined;

function ghostStop(): void {
  if (ghostTimer !== undefined) clearTimeout(ghostTimer);
  ghostTimer = undefined;
  ghostPending = new Set();
  window.removeEventListener('click', onGhost, true);
  window.removeEventListener('dblclick', onGhost, true);
  window.removeEventListener('pointerup', onGhostRelease, true);
  window.removeEventListener('pointercancel', onGhostRelease, true);
}

function onGhost(e: Event): void {
  if (!ghostPending.has(e.type)) return;
  const caps = (e as MouseEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } }).sourceCapabilities;
  if (caps && !caps.firesTouchEvents) return; // a real mouse click in the window — not ours
  ghostPending.delete(e.type);
  e.stopImmediatePropagation();
  if (ghostPending.size === 0) ghostStop();
}

function onGhostRelease(e: PointerEvent): void {
  if (e.pointerId !== ghostPointerId) return;
  if (ghostTimer !== undefined) clearTimeout(ghostTimer);
  ghostTimer = setTimeout(ghostStop, GHOST_GRACE_MS);
}

function ghostArm(pointerId: number): void {
  ghostStop();
  ghostPointerId = pointerId;
  ghostPending = new Set(['click', 'dblclick']);
  window.addEventListener('click', onGhost, true);
  window.addEventListener('dblclick', onGhost, true);
  window.addEventListener('pointerup', onGhostRelease, true);
  window.addEventListener('pointercancel', onGhostRelease, true);
  ghostTimer = setTimeout(ghostStop, GHOST_CAP_MS);
}

/** Svelte action: a touch / pen double-tap → `cb`. `dblclick` is the mouse's job (see
 *  `mouseDoubleClick`); Android's WebView does not synthesize it reliably for touch, and the
 *  phone grid's tiles run `touch-action: none` on top of that. Two pointerdowns within 350 ms
 *  and 24 px count; mouse pointers are ignored so a desktop double-click never fires twice. */
export function doubleTap(node: HTMLElement, cb: () => void): { destroy(): void } {
  let last = 0;
  let lx = 0;
  let ly = 0;
  const onDown = (e: PointerEvent) => {
    if (e.pointerType === 'mouse') return;
    const now = performance.now();
    if (now - last < 350 && Math.hypot(e.clientX - lx, e.clientY - ly) < 24) {
      last = 0;
      lastTouchDoubleTap = now;
      ghostArm(e.pointerId);
      cb();
      return;
    }
    last = now;
    lx = e.clientX;
    ly = e.clientY;
  };
  node.addEventListener('pointerdown', onDown);
  return { destroy: () => node.removeEventListener('pointerdown', onDown) };
}

/** Wrap a `dblclick` handler so it only serves the MOUSE: a double-click synthesized from touch
 *  (Chromium marks it via `sourceCapabilities.firesTouchEvents`; as a fallback, anything within
 *  the window after a touch double-tap) is dropped — the `doubleTap` action already handled it,
 *  and normally swallowed the ghost at the window before it got here. */
export function mouseDoubleClick(cb: () => void): (e: MouseEvent) => void {
  return (e) => {
    const caps = (e as MouseEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } }).sourceCapabilities;
    if (caps?.firesTouchEvents) return;
    if (performance.now() - lastTouchDoubleTap < SYNTH_WINDOW_MS) return;
    cb();
  };
}
