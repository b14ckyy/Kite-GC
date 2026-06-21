// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC engine — runs the input methods (helpers/rcMethods.ts) against the live HID frame to produce the
// channel µs values (docs/active/RC_CONTROL.md §7). Drives the live channel-state view now; the MSP
// stream later. Stateful methods (adjust integrators, toggle/step) keep per-channel state across frames;
// state is reset when a channel's config changes. Subscribes to the ~50 Hz hid-input stream so rate
// integrators advance even while an input is held steady.

import { writable, get } from 'svelte/store';
import { hidSnapshot, type HidSnapshot } from './hid';
import { currentChannels } from './rcProfiles';
import {
  initState, seedState, stepMethod, toUs,
  type InputFrame, type MethodState, type RcMethod,
} from '$lib/helpers/rcMethods';

/** RC channel number → current µs value. */
export const channelValues = writable<Record<number, number>>({});

/** Build the A/B/H input frame from a raw HID snapshot. Inputs are 1-based per type (sorted order). */
function makeFrame(snap: HidSnapshot): InputFrame {
  return {
    axis(label) {
      const n = Number(label.slice(1));
      return snap.axes[n - 1]?.value ?? 0;
    },
    button(label) {
      const n = Number(label.slice(1));
      if (label[0] === 'B') return snap.buttons[n - 1]?.pressed ?? false;
      if (label[0] === 'H') {
        // Hat directions as buttons: 4 per hat — up / right / down / left.
        const hat = snap.hats[Math.floor((n - 1) / 4)];
        if (!hat) return false;
        switch ((n - 1) % 4) {
          case 0: return hat.y > 0; // up
          case 1: return hat.x > 0; // right
          case 2: return hat.y < 0; // down
          default: return hat.x < 0; // left
        }
      }
      return false;
    },
  };
}

const states = new Map<number, { cfg: unknown; state: MethodState }>();
let lastTime = 0;

/** Seed the per-channel runtime state from the FC's current channel values (µs, index 0 = CH1), so on
 *  engage every stateful method continues from where the FC already is — no jump at handover
 *  (docs/active/RC_CONTROL.md §10 Phase 4). Passthrough / hold follow live input and are left untouched.
 *  The next HID frame recomputes from these seeds; we also push the seeded values to `channelValues`
 *  immediately so the RC-Out view reflects the handover without waiting for a frame. */
export function seedFromFc(channelsUs: number[]): void {
  const channels = get(currentChannels);
  const seeded: Record<number, number> = {};
  for (const [key, cfg] of Object.entries(channels)) {
    const ch = Number(key);
    const us = channelsUs[ch - 1];
    if (us == null) continue;
    const m = cfg as RcMethod;
    if (m.kind === 'passthrough' || m.kind === 'hold') continue; // live input — nothing to latch
    if (m.kind === 'dualAxis' && m.mode === 'absolute') continue;
    const state = seedState(m, us);
    states.set(ch, { cfg, state });
    seeded[ch] = toUs(state.value);
  }
  if (Object.keys(seeded).length) channelValues.update((cur) => ({ ...cur, ...seeded }));
}

hidSnapshot.subscribe((snap) => {
  if (!snap) return;
  const now = performance.now();
  // Clamp dt so a paused/resumed stream (panel reopened) can't make integrators jump.
  const dt = lastTime ? Math.min(now - lastTime, 100) : 20;
  lastTime = now;

  const channels = get(currentChannels);
  const frame = makeFrame(snap);
  const out: Record<number, number> = {};

  for (const [key, cfg] of Object.entries(channels)) {
    const ch = Number(key);
    let entry = states.get(ch);
    if (!entry || entry.cfg !== cfg) {
      entry = { cfg, state: initState(cfg) }; // config changed (immutable edit) → reset runtime state
      states.set(ch, entry);
    }
    const result = stepMethod(cfg, entry.state, frame, dt);
    entry.state = result.state;
    out[ch] = toUs(result.value);
  }

  // Drop state for removed channels.
  for (const ch of [...states.keys()]) if (!(ch in channels)) states.delete(ch);

  channelValues.set(out);
});
