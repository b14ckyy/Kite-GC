// INAV arming flags (bit positions from INAV source)
export const ARMING_FLAG_ARMED = 2; // bit 2 = ARMED

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
