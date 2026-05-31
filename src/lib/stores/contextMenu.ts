// Global custom context-menu state. A single <ContextMenu> mounted at the app
// root renders whatever the active opener requested; `use:contextMenu` (action)
// or openContextMenu() drives it. Replaces the native WebView menu.

import { writable } from 'svelte/store';

export interface ContextMenuItem {
  label?: string;
  /** Run on click (ignored for separators / submenu parents). */
  action?: () => void;
  /** Optional leading glyph. */
  icon?: string;
  disabled?: boolean;
  /** Red styling for destructive actions. */
  danger?: boolean;
  /** Render a divider instead of an item. */
  separator?: boolean;
  /** Nested items — opens a flyout to the side. */
  submenu?: ContextMenuItem[];
}

interface ContextMenuState {
  open: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
}

export const contextMenu = writable<ContextMenuState>({ open: false, x: 0, y: 0, items: [] });

export function openContextMenu(x: number, y: number, items: ContextMenuItem[]): void {
  if (!items?.length) return;
  contextMenu.set({ open: true, x, y, items });
}

export function closeContextMenu(): void {
  contextMenu.update((s) => (s.open ? { ...s, open: false } : s));
}
