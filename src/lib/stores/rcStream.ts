// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC injection pump (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c). While engaged, this pushes the live
// channel frame to the backend at a fixed rate (the scheduler thread does the actual MSP streaming):
//   • RAW-RC every 30 ms — steady heartbeat (drives the backend deadman) + latest values, so the FC
//     holds the last command even when nothing changes;
//   • AUX-RC only when the controlled AUX values change (latched overlay; the scheduler re-sends until
//     the FC ACKs). One frame is forced right after engage to establish our overlay at the seeded state.
// Self-initialising: subscribing to `rcEngaged` starts/stops the pump. Import it once (RcControlPanel).

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { channelValues } from './rcEngine';
import { rcEngaged } from './rcEngage';
import { rcLayout } from './rcLayout';
import { currentChannels } from './rcProfiles';
import { settings } from './settings';

/** Push rate (ms) — faster than the backend's 20 Hz send so it always has a fresh frame. */
const PUSH_INTERVAL_MS = 30;

/** Build the RAW-RC frame as CH1..CHmax µs, where CHmax is the highest *controlled* RAW channel —
 *  MSP_SET_RAW_RC is positional from CH1, so we can trim the tail but not the head. Controlled channels
 *  use the mapping output; gaps below CHmax are sent as 0 (INAV ≥8.0 treats 0 as "no override", and they
 *  aren't in the override bitmask either — double safety). Returns [] when no RAW channel is controlled
 *  → no SET_RAW_RC. Trimming to CHmax saves TX bandwidth (e.g. CH1–6 = 6 channels, not 16). */
function buildRaw(): number[] {
  const layout = get(rcLayout);
  const controlled = get(currentChannels);
  const rawNums = Object.keys(controlled).map(Number).filter((c) => c <= layout.rawMax);
  if (!rawNums.length) return [];
  const maxCh = Math.max(...rawNums);
  const ch = get(channelValues);
  const out: number[] = [];
  for (let c = 1; c <= maxCh; c++) {
    out.push(controlled[c] ? Math.round(ch[c] ?? 1500) : 0); // 0 = skip
  }
  return out;
}

/** AUX-RC change detection rate (ms). ≥ the backend's 5 Hz send so it never adds latency, but slower
 *  than the RAW tick to keep IPC down. The backend accumulates pushes, so this can't lose changes. */
const AUX_EVAL_MS = 100;

let timer: ReturnType<typeof setInterval> | null = null;
let lastAuxEval = 0;
/** Last µs we pushed per AUX channel (1-based) — only channels that differ are re-pushed. */
let lastAux: Record<number, number> = {};

/** Push only the AUX channels whose value changed since the last push. The backend packs the minimal
 *  16-bit run covering them — so moving one gimbal axis sends one channel, two axes send two, etc. */
function evalAux(): void {
  const layout = get(rcLayout);
  const controlled = get(currentChannels);
  const auxChs = Object.keys(controlled).map(Number).filter((c) => c > layout.rawMax);
  if (!auxChs.length) return;
  const ch = get(channelValues);
  const channels: number[] = []; // 0-based
  const values: number[] = [];
  for (const c of auxChs) {
    const v = Math.round(ch[c] ?? 1500);
    if (v !== lastAux[c]) {
      channels.push(c - 1);
      values.push(v);
      lastAux[c] = v;
    }
  }
  if (channels.length) void invoke('rc_stream_set_aux', { channels, values });
}

function tick(): void {
  void invoke('rc_stream_update', { channels: buildRaw() });
  const now = performance.now();
  if (now - lastAuxEval >= AUX_EVAL_MS) {
    lastAuxEval = now;
    evalAux();
  }
}

rcEngaged.subscribe((e) => {
  if (e.on && !timer) {
    lastAux = {}; // force a full AUX push on engage (establish the overlay at the seeded state)
    lastAuxEval = 0;
    void invoke('rc_stream_set_rate', { hz: get(settings).rcControl.rawRateHz });
    void invoke('rc_stream_enable', { enabled: true });
    tick(); // send an immediate frame so handover is instant
    timer = setInterval(tick, PUSH_INTERVAL_MS);
  } else if (!e.on && timer) {
    clearInterval(timer);
    timer = null;
    void invoke('rc_stream_enable', { enabled: false });
  }
});
