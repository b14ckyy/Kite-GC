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

export function isValidGpsCoordinate(
  lat: number | null | undefined,
  lon: number | null | undefined,
): boolean {
  if (lat == null || lon == null) return false;
  if (!Number.isFinite(lat) || !Number.isFinite(lon)) return false;
  if (lat < -90 || lat > 90 || lon < -180 || lon > 180) return false;
  return !(lat === 0 && lon === 0);
}
