// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action: a touch long-press on an element.
//   <button use:longPress={() => startSelecting(id)}>
// Touch only — a mouse has modifier keys. The click that follows the lifted finger is swallowed.

const LONG_PRESS_MS = 500;
const MOVE_CANCEL_PX = 10;

export function longPress(node: HTMLElement, onLongPress: () => void) {
  let handler = onLongPress;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let fired = false;
  let startX = 0;
  let startY = 0;

  function clearTimer() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function onTouchStart(e: TouchEvent) {
    clearTimer();
    fired = false;
    if (e.touches.length !== 1) return;
    startX = e.touches[0].clientX;
    startY = e.touches[0].clientY;
    timer = setTimeout(() => {
      timer = null;
      fired = true;
      handler();
    }, LONG_PRESS_MS);
  }

  function onTouchMove(e: TouchEvent) {
    const t = e.touches[0];
    if (t && Math.hypot(t.clientX - startX, t.clientY - startY) > MOVE_CANCEL_PX) clearTimer();
  }

  // Capture phase on the node: ahead of the element's own (delegated) click handler.
  function onClick(e: MouseEvent) {
    if (!fired) return;
    fired = false;
    e.preventDefault();
    e.stopPropagation();
  }

  function onContextMenu(e: MouseEvent) {
    if (fired || timer) e.preventDefault();
  }

  node.addEventListener('touchstart', onTouchStart, { passive: true });
  node.addEventListener('touchmove', onTouchMove, { passive: true });
  node.addEventListener('touchend', clearTimer);
  node.addEventListener('touchcancel', clearTimer);
  node.addEventListener('click', onClick, true);
  node.addEventListener('contextmenu', onContextMenu);

  return {
    update(next: () => void) {
      handler = next;
    },
    destroy() {
      clearTimer();
      node.removeEventListener('touchstart', onTouchStart);
      node.removeEventListener('touchmove', onTouchMove);
      node.removeEventListener('touchend', clearTimer);
      node.removeEventListener('touchcancel', clearTimer);
      node.removeEventListener('click', onClick, true);
      node.removeEventListener('contextmenu', onContextMenu);
    },
  };
}
