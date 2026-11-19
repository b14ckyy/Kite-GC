// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Frame pacing for Kite's own animation loops: `requestFrame` is `requestAnimationFrame` with a rate cap.
// - The page only draws when one of these loops changes something, so their rate IS the UI's frame rate.
// - `requestAnimationFrame` follows the display: uncapped, a 120 Hz screen runs every loop at 120 Hz.
// - All callbacks share one pump, so a capped frame runs them together and the browser draws once.

import { lowPowerActive } from '$lib/stores/lowPower';

const NORMAL_FPS = 60;
/** Same as Map3D's low-power render rate. */
const LOW_POWER_FPS = 20;
/** A frame that arrives this much early still counts: display frames jitter around their period. */
const EARLY_MS = 2;

let capFps = NORMAL_FPS;
lowPowerActive.subscribe((active) => { capFps = active ? LOW_POWER_FPS : NORMAL_FPS; });

let queue = new Map<number, FrameRequestCallback>();
let nextId = 1;
let pump: number | null = null;
let lastRun = -Infinity;

function onDisplayFrame(t: number): void {
  pump = null;
  if (t - lastRun < 1000 / capFps - EARLY_MS) {
    pump = requestAnimationFrame(onDisplayFrame);
    return;
  }
  lastRun = t;
  const run = queue;
  queue = new Map();
  for (const cb of run.values()) cb(t);
}

/** Like `requestAnimationFrame`, at most at the current cap. The callback gets the frame's timestamp. */
export function requestFrame(cb: FrameRequestCallback): number {
  const id = nextId++;
  queue.set(id, cb);
  if (pump == null) pump = requestAnimationFrame(onDisplayFrame);
  return id;
}

export function cancelFrame(id: number): void {
  queue.delete(id);
}
