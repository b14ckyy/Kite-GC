// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Svelte action for a dialog that closes through its buttons only: Escape is the keyboard's cancel.
//   <div class="dialog-backdrop" use:escapeKey={() => close(null)}>
// Listens on the window, so it works wherever the focus is. Of several open dialogs only the one
// opened last answers.

const stack: { run: () => void }[] = [];

function onKeydown(e: KeyboardEvent) {
  if (e.key !== 'Escape' || stack.length === 0) return;
  e.preventDefault();
  e.stopPropagation();
  stack[stack.length - 1].run();
}

export function escapeKey(_node: HTMLElement, onEscape: () => void) {
  const entry = { run: onEscape };
  if (stack.length === 0) window.addEventListener('keydown', onKeydown, true);
  stack.push(entry);
  return {
    update(next: () => void) {
      entry.run = next;
    },
    destroy() {
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
      if (stack.length === 0) window.removeEventListener('keydown', onKeydown, true);
    },
  };
}
