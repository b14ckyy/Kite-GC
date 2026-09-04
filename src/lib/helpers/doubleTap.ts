// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/** When the last touch double-tap fired (performance.now()). A touch double-tap is followed by a
 *  browser-synthesized `dblclick` on whatever element sits under the finger AFTER the second
 *  release — after a map ⇄ video swap that is the OTHER surface, whose own double-click handler
 *  then swapped straight back (Marc: "only stays if I keep the finger down"). */
let lastTouchDoubleTap = 0;
const SYNTH_WINDOW_MS = 700;

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
 *  the window after a touch double-tap) is dropped — the `doubleTap` action already handled it. */
export function mouseDoubleClick(cb: () => void): (e: MouseEvent) => void {
  return (e) => {
    const caps = (e as MouseEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } }).sourceCapabilities;
    if (caps?.firesTouchEvents) return;
    if (performance.now() - lastTouchDoubleTap < SYNTH_WINDOW_MS) return;
    cb();
  };
}
