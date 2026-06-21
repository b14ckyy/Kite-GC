// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-control engage gate (docs/archive/MSP_RC_CONTROL.md §10 Phase 4b). "Engaged" = we have taken over and
// (later phase) stream RC to the FC. Engaging always SYNCS first: read the FC's current channels once
// and seed our state, so there's no jump at handover. Two ways to engage:
//   • serial RX → automatic: the panel watches the FC's MSP RC OVERRIDE box and engages while it's on
//     (the pilot arms the takeover with a switch on the TX);
//   • MSP RX    → manual: an explicit long-press toggle in the panel (default OFF at app start).
// This phase only seeds + reflects the handover in the UI; no RC is transmitted yet.

import { writable, get } from 'svelte/store';
import { syncFromFc } from './rcMirror';

export type EngageMode = 'serial' | 'msp';

/** Current engage state. `on` false = idle (preview only). */
export const rcEngaged = writable<{ on: boolean; mode: EngageMode | null }>({ on: false, mode: null });

let busy = false;

/** Engage: sync from the FC (seed) first, then mark engaged. No-op if already engaged or mid-sync.
 *  Returns true if we became engaged. */
export async function engage(mode: EngageMode): Promise<boolean> {
  if (busy || get(rcEngaged).on) return false;
  busy = true;
  try {
    const ok = await syncFromFc();
    if (!ok) return false;
    rcEngaged.set({ on: true, mode });
    return true;
  } finally {
    busy = false;
  }
}

/** Disengage (link lost, switch off, or manual toggle off). */
export function disengage(): void {
  if (get(rcEngaged).on) rcEngaged.set({ on: false, mode: null });
}
