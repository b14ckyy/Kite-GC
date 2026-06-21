// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FC RC-channel sync (docs/active/RC_CONTROL.md §10 Phase 4a). We do NOT poll the FC continuously
// (MSP_RC is a ~80-byte reply — a recurring RX spike): instead we read the FC's current channel values
// ONCE, at the moment we engage, and seed our internal channel state from it (helpers/rcMethods.ts
// seedState). That way a stateful method (toggle/adjust) continues from where the FC already is — no
// jump / unexpected mode change at handover. `fcChannels` keeps the last synced snapshot for the
// debug monitor.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { seedFromFc } from './rcEngine';

/** FC's channel values (µs) from the last sync; index 0 = CH1. For the debug monitor only. */
export const fcChannels = writable<number[]>([]);

/** Read the FC's current channel values once (MSP_RC) and seed our channel state from them.
 *  Returns true on success. Called at engage time — never on a timer. */
export async function syncFromFc(): Promise<boolean> {
  try {
    const ch = await invoke<number[]>('rc_read_channels');
    fcChannels.set(ch);
    seedFromFc(ch);
    return true;
  } catch (e) {
    console.warn('[rc] syncFromFc failed', e);
    return false;
  }
}
