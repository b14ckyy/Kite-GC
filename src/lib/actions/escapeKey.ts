// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Escape as the keyboard's cancel for dialogs that close through their buttons only. Window listener in the
// capture phase: works wherever the focus is and swallows the key; only the dialog opened last answers.

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
