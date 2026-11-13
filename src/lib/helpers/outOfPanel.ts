// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action for a full-screen overlay (a `position: fixed` dialog) that is declared inside a side
// panel's body. A panel shell can be the containing block for fixed descendants — its blur, or the
// compositor layer it keeps in low-power mode — and clips them with its `overflow: hidden`; the dialog
// would then sit inside the panel instead of over the app. The action moves the node next to the panel
// shell, where it is positioned against the app like every dialog rendered outside the shells. Outside
// a panel it does nothing.

export function outOfPanel(node: HTMLElement): { destroy(): void } {
  const host = node.closest('.ps')?.parentElement;
  if (host) host.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
