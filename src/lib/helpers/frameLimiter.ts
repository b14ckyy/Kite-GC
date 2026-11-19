// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// `requestFrame`: `requestAnimationFrame` through one shared pump, capped at 60 fps (20 in power saving).
// Kite's own loops are what makes the page draw, so their rate is the UI's frame rate; one pump runs
// them in the same frame so the browser draws once.

import { lowPowerActive } from '$lib/stores/lowPower';

const NORMAL_FPS = 60;
/** Same as Map3D's low-power render rate. */
const LOW_POWER_FPS = 20;

let capFps = NORMAL_FPS;
lowPowerActive.subscribe((active) => { capFps = active ? LOW_POWER_FPS : NORMAL_FPS; });

let queue = new Map<number, FrameRequestCallback>();
let running: Map<number, FrameRequestCallback> | null = null;
let nextId = 1;
let pump: number | null = null;
let lastRun = -Infinity;
let lastDisplayFrame = 0;
let displayMs = 1000 / 60;

/** The display frame nearest to each due time runs: an even cadence at any display rate, and the phase
 *  is kept (`lastRun` advances by the interval), so 60 fps on a 75 / 90 / 144 Hz screen is 60, not 37-48. */
function onDisplayFrame(t: number): void {
  pump = null;
  const sinceFrame = t - lastDisplayFrame;
  lastDisplayFrame = t;
  if (sinceFrame > 2 && sinceFrame < 50) displayMs += (sinceFrame - displayMs) * 0.1;
  const due = lastRun + 1000 / capFps;
  if (t < due - displayMs / 2) {
    pump = requestAnimationFrame(onDisplayFrame);
    return;
  }
  lastRun = t - due < displayMs ? due : t;
  running = queue;
  queue = new Map();
  // One throwing callback must not take the rest of the pass with it: their owners would wait forever.
  for (const cb of running.values()) {
    try { cb(t); } catch (e) { console.error(e); }
  }
  running = null;
}

/** Like `requestAnimationFrame`, at most at the current cap. */
export function requestFrame(cb: FrameRequestCallback): number {
  const id = nextId++;
  queue.set(id, cb);
  if (pump == null) pump = requestAnimationFrame(onDisplayFrame);
  return id;
}

export function cancelFrame(id: number): void {
  queue.delete(id);
  running?.delete(id);
}
