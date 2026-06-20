// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV arming flags (bit positions from INAV source)
export const ARMING_FLAG_ARMED = 2; // bit 2 = ARMED

// Minimum satellite count before we trust a 3D fix enough to jump the camera to the UAV. The FC can
// briefly report fixType 3 with garbage/near-0,0 coordinates while the fix is still settling; a sat
// count gate is the extra safety so the camera never snaps to a bogus early position.
export const MIN_FIX_SATELLITES = 6;

export function isArmed(armingFlags: number, lastUpdate: number): boolean {
  return lastUpdate > 0 && (armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
}

export function hasKnownLocation(name: string | null | undefined): boolean {
  const n = (name ?? '').trim();
  if (!n) return false;
  return n !== 'Unknown location' && n !== 'Unbekannter Ort';
}

/** Reject null island AND its immediate neighbourhood. A GPS receiver / EKF reports a position at (or
 *  very near) 0°,0° before it has a real origin/fix — ArduPilot in particular emits a brief near-0,0
 *  frame on acquisition, which otherwise leaks into the pre-arm track and the go-to-UAV camera jump.
 *  There is no land within ~100 m of 0,0 (mid-Atlantic), and a real flight on the equator OR the prime
 *  meridian has only ONE component near zero — so requiring BOTH to be ~0 is safe. */
const NULL_ISLAND_EPS = 0.001; // ≈ 111 m
export function isValidGpsCoordinate(
  lat: number | null | undefined,
  lon: number | null | undefined,
): boolean {
  if (lat == null || lon == null) return false;
  if (!Number.isFinite(lat) || !Number.isFinite(lon)) return false;
  if (lat < -90 || lat > 90 || lon < -180 || lon > 180) return false;
  return !(Math.abs(lat) < NULL_ISLAND_EPS && Math.abs(lon) < NULL_ISLAND_EPS);
}
