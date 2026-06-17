// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight-path marker (velocity vector) geometry, shared by the AHI widget + the 3D FPV HUD.
// Body-frame, pilot's view: where the aircraft is actually going relative to where the nose points.
//  • gamma = flight-path angle (climb/descent) from vertical vs ground speed
//  • crab  = course-over-ground minus heading (lateral drift; the wind/no-mag crab)
// Both derive purely from existing telemetry — no wind estimate needed (we have the track directly).

/** Below this ground speed COG/vario are noise → hide the marker. Matches the compass COG gate. */
export const FPM_MIN_SPEED_MS = 1.5;

export interface FlightPathVector {
  /** Flight-path (climb) angle in degrees, + = climbing. */
  gamma: number;
  /** Crab angle in degrees, + = track to the right of the nose. */
  crab: number;
  /** False when too slow for COG/vario to be meaningful. */
  shown: boolean;
}

/** Compute the flight-path vector from telemetry (all SI / degrees). */
export function flightPathVector(
  groundSpeedMs: number,
  varioMs: number,
  courseDeg: number,
  yawDeg: number,
): FlightPathVector {
  const shown = groundSpeedMs >= FPM_MIN_SPEED_MS;
  const gamma = (Math.atan2(varioMs, Math.max(groundSpeedMs, 0.01)) * 180) / Math.PI;
  const crab = (((courseDeg - yawDeg) % 360) + 540) % 360 - 180; // -180..180
  return { gamma, crab, shown };
}
