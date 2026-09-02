// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Exponential smoothing helpers, factored out so HUD overlays (AHI / FPV flight-path marker) ease
// their values identically — one ease step per telemetry update with a fixed factor.

/** Ease a scalar toward `target` by `factor` (0..1). */
export function easeToward(cur: number, target: number, factor: number): number {
  return cur + (target - cur) * factor;
}

/** Ease an angle (degrees) toward `target` along the shortest path (handles 359°→1° wrap).
 *  The result is normalized to -180..180: the consumers (AHI / FPV-HUD flight-path marker) use it
 *  as a LINEAR offset, and without the normalization every ±180° crossing of the target (routine
 *  on a multirotor, where course vs. yaw is unconstrained) winds the eased value up by full turns
 *  until the marker sits pinned at the dial edge for the rest of the session. */
export function easeAngleToward(curDeg: number, targetDeg: number, factor: number): number {
  const delta = (((targetDeg - curDeg) % 360) + 540) % 360 - 180; // -180..180
  return (((curDeg + delta * factor) % 360) + 540) % 360 - 180;
}
