// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action: attach a custom context menu to any element.
//   <div use:contextMenu={() => [ {label, action}, ... ]}>
// Opens on right-click (desktop) and long-press (touch), suppressing the native
// menu. The items provider is called lazily on open, so menus can be dynamic.

import { openContextMenu, type ContextMenuItem } from '$lib/stores/contextMenu';

type ItemsProvider = () => ContextMenuItem[];

const LONG_PRESS_MS = 500;
const MOVE_CANCEL_PX = 10;

export function contextMenu(node: HTMLElement, getItems: ItemsProvider) {
  let provider = getItems;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let startX = 0;
  let startY = 0;

  function clearTimer() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function onContextMenu(e: MouseEvent) {
    const items = provider();
    if (!items?.length) return;
    e.preventDefault();
    e.stopPropagation();
    openContextMenu(e.clientX, e.clientY, items);
  }

  function onTouchStart(e: TouchEvent) {
    if (e.touches.length !== 1) {
      clearTimer();
      return;
    }
    startX = e.touches[0].clientX;
    startY = e.touches[0].clientY;
    clearTimer();
    timer = setTimeout(() => {
      timer = null;
      const items = provider();
      if (items?.length) openContextMenu(startX, startY, items);
    }, LONG_PRESS_MS);
  }

  function onTouchMove(e: TouchEvent) {
    const t = e.touches[0];
    if (t && Math.hypot(t.clientX - startX, t.clientY - startY) > MOVE_CANCEL_PX) clearTimer();
  }

  node.addEventListener('contextmenu', onContextMenu);
  node.addEventListener('touchstart', onTouchStart, { passive: true });
  node.addEventListener('touchmove', onTouchMove, { passive: true });
  node.addEventListener('touchend', clearTimer);
  node.addEventListener('touchcancel', clearTimer);

  return {
    update(next: ItemsProvider) {
      provider = next;
    },
    destroy() {
      clearTimer();
      node.removeEventListener('contextmenu', onContextMenu);
      node.removeEventListener('touchstart', onTouchStart);
      node.removeEventListener('touchmove', onTouchMove);
      node.removeEventListener('touchend', clearTimer);
      node.removeEventListener('touchcancel', clearTimer);
    },
  };
}
