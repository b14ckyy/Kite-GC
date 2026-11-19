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
  // Armed by a change only: a quiet source (no link) must not wake the page ten times a second.
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    source();
    timer ??= setTimeout(() => {
      timer = undefined;
      shown = untrack(source);
    }, ms);
  });
  $effect(() => () => clearTimeout(timer));
  return () => shown;
}
