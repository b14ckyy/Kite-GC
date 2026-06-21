// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FC RC-channel sync (docs/archive/MSP_RC_CONTROL.md §10 Phase 4a). We seed our internal channel state
// from the FC's CURRENT channel values at the moment we engage — so a stateful method (toggle/adjust)
// continues from where the FC already is (no jump / unexpected mode change at handover). The source
// differs by platform:
//   • INAV / MSP → a one-shot MSP_RC read (we avoid continuous polling: the ~80-byte reply is an RX spike);
//   • ArduPilot / MAVLink → the FC already BROADCASTS RC_CHANNELS, so we cache the last one (no request)
//     and seed from it. The handler emits it as `telemetry-rc-channels`.
// `fcChannels` keeps the last synced snapshot for the debug monitor.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { seedFromFc } from './rcEngine';
import { rcPlatform } from './rcPlatform';

/** FC's channel values (µs) from the last sync; index 0 = CH1. For the debug monitor only. */
export const fcChannels = writable<number[]>([]);

/** Last RC_CHANNELS broadcast from a MAVLink FC (µs, CH1..). Updated continuously; read at engage. */
let liveMavChannels: number[] = [];
void listen<number[]>('telemetry-rc-channels', (e) => {
  liveMavChannels = e.payload;
});

/** Read/seed the FC's current channel values once and seed our channel state from them. Returns true on
 *  success. Called at engage time — never on a timer. */
export async function syncFromFc(): Promise<boolean> {
  if (get(rcPlatform) === 'inav') {
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
  // MAVLink (ArduPilot): seed from the last RC_CHANNELS broadcast. If none seen yet, seed with what we
  // have (possibly empty → method defaults) rather than blocking engage.
  fcChannels.set(liveMavChannels);
  seedFromFc(liveMavChannels);
  return true;
}
