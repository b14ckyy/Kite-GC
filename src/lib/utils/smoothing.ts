// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Frame-rate-independent exponential smoothing — the same easing the map follow-loops use, factored
// out so HUD overlays (AHI / FPV flight-path marker) ease their values identically.

/** Exponential ease factor for a frame of `dtMs` toward a target with time constant `tauMs`.
 *  Frame-rate independent: `cur += (target - cur) * easeFactor(dt, tau)`. */
export function easeFactor(dtMs: number, tauMs: number): number {
  if (tauMs <= 0) return 1;
  return 1 - Math.exp(-dtMs / tauMs);
}

/** Ease a scalar toward `target` by `factor` (0..1). */
export function easeToward(cur: number, target: number, factor: number): number {
  return cur + (target - cur) * factor;
}

/** Ease an angle (degrees) toward `target` along the shortest path (handles 359°→1° wrap). */
export function easeAngleToward(curDeg: number, targetDeg: number, factor: number): number {
  const delta = (((targetDeg - curDeg) % 360) + 540) % 360 - 180; // -180..180
  return curDeg + delta * factor;
}
