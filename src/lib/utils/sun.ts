// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Low-precision solar position (NOAA-style approximation, ±~0.5°). Enough to decide
// day vs. night for the 2D map's auto night dimming and to mirror Cesium's terminator.

const RAD = Math.PI / 180;

/** Julian Date from a JS Date (UTC epoch). */
function toJulian(date: Date): number {
  return date.getTime() / 86_400_000 + 2_440_587.5;
}

/**
 * Sun altitude (degrees above the horizon) for a given instant and location.
 * Negative = below the horizon (night). 0 ≈ sunrise/sunset.
 */
export function sunAltitudeDeg(date: Date, latDeg: number, lonDeg: number): number {
  const n = toJulian(date) - 2_451_545.0; // days since J2000.0

  const meanLong = (280.46 + 0.985_647_4 * n) % 360;          // mean longitude (deg)
  const meanAnom = ((357.528 + 0.985_600_3 * n) % 360) * RAD; // mean anomaly (rad)

  // Ecliptic longitude (rad) with the two largest equation-of-center terms.
  const eclLong = (meanLong + 1.915 * Math.sin(meanAnom) + 0.02 * Math.sin(2 * meanAnom)) * RAD;
  const obliquity = (23.439 - 0.000_000_4 * n) * RAD;

  const declination = Math.asin(Math.sin(obliquity) * Math.sin(eclLong));
  const rightAsc = Math.atan2(Math.cos(obliquity) * Math.sin(eclLong), Math.cos(eclLong));

  // Greenwich mean sidereal time → local hour angle.
  const gmstHours = (18.697_374_558 + 24.065_709_824_419_08 * n) % 24;
  const lmstRad = (gmstHours * 15 + lonDeg) * RAD;
  const hourAngle = lmstRad - rightAsc;

  const lat = latDeg * RAD;
  const altitude = Math.asin(
    Math.sin(lat) * Math.sin(declination) +
      Math.cos(lat) * Math.cos(declination) * Math.cos(hourAngle),
  );
  return altitude / RAD;
}

/** True when the sun is below the horizon (night) at the given instant/location. */
export function isNightAt(date: Date, latDeg: number, lonDeg: number): boolean {
  return sunAltitudeDeg(date, latDeg, lonDeg) < 0;
}

/**
 * Continuous brightness factor (0.3…1.0) for a given sun altitude, mirroring Cesium's globe
 * day/night shading (GlobeFS: `clamp(lambert·0.9 + 0.3, 0.3, 1.0)`, lambert = max(sin α, 0)).
 * Smoothly fades from full daylight (1.0) down to the night floor (0.3) as the sun sets.
 */
export function cesiumLikeBrightness(sunAltDeg: number): number {
  const lambert = Math.max(Math.sin(sunAltDeg * RAD), 0);
  return Math.min(Math.max(lambert * 0.9 + 0.3, 0.3), 1.0);
}
