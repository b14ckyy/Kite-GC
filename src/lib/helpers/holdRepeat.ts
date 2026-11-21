// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

export interface HoldRepeat {
  /** One step: a press, and a keyboard activation. */
  step: () => void;
  /** Called every `everyMs` while the button stays pressed; `step` when left out. */
  hold?: () => void;
  everyMs?: number;
}

const HOLD_DELAY_MS = 350;

/** Svelte action: a press runs `step` once, a held press then repeats. Replaces `onclick` on the button —
 *  a keyboard activation arrives as a click without a pointer press and runs `step` too. */
export function holdRepeat(node: HTMLElement, params: HoldRepeat) {
  let p = params;
  let delay: ReturnType<typeof setTimeout> | undefined;
  let repeat: ReturnType<typeof setInterval> | undefined;

  const stop = () => {
    clearTimeout(delay);
    clearInterval(repeat);
    delay = repeat = undefined;
  };
  const down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    stop();
    p.step();
    delay = setTimeout(() => {
      repeat = setInterval(() => (p.hold ?? p.step)(), p.everyMs ?? 250);
    }, HOLD_DELAY_MS);
  };
  const click = (e: MouseEvent) => {
    if (e.detail === 0) p.step();
  };

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointerup', stop);
  node.addEventListener('pointerleave', stop);
  node.addEventListener('pointercancel', stop);
  node.addEventListener('click', click);
  return {
    update(next: HoldRepeat) { p = next; },
    destroy() {
      stop();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointerup', stop);
      node.removeEventListener('pointerleave', stop);
      node.removeEventListener('pointercancel', stop);
      node.removeEventListener('click', click);
    },
  };
}
