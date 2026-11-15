// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action for a `position: fixed` dialog declared inside a side panel: the shell can be the containing
// block for fixed descendants (its blur or low-power layer) and clips them. The action moves the node next to
// the shell; outside a panel it does nothing.

export function outOfPanel(node: HTMLElement): { destroy(): void } {
  const host = node.closest('.ps')?.parentElement;
  if (host) host.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
