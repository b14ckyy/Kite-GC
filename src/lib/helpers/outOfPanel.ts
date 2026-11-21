// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action for a `position: fixed` dialog declared inside a side panel or the fleet command bar: a blurred
// or transformed ancestor is the containing block for fixed descendants (the panel also clips them). The action moves the
// node into the panels layer; elsewhere it does nothing.

export function outOfPanel(node: HTMLElement): { destroy(): void } {
  const host = node.closest('.ps')?.parentElement ?? (node.closest('.gcb') ? document.querySelector('.panels-host') : null);
  if (host) host.appendChild(node);
  return {
    destroy() {
      node.remove();
    },
  };
}
