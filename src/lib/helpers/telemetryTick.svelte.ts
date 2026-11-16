// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { untrack } from 'svelte';

/**
 * Newest value of `source`, handed on at most once per `ms`. Telemetry arrives as separate messages in
 * separate frames and every widget change repaints its whole dock, so widgets fed from one snapshot per
 * tick repaint together. Call during component initialisation.
 */
export function tickSnapshot<T>(source: () => T, ms: number): () => T {
  let shown = $state.raw(untrack(source));
  let dirty = false;
  $effect(() => {
    source();
    dirty = true;
  });
  $effect(() => {
    const id = setInterval(() => {
      if (!dirty) return;
      dirty = false;
      shown = untrack(source);
    }, ms);
    return () => clearInterval(id);
  });
  return () => shown;
}
