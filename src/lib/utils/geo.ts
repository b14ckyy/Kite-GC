// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Geographic utility functions

const DEG2RAD = Math.PI / 180;
const R_EARTH = 6371000; // Earth radius in meters

/** Haversine distance between two GPS points (meters) */
export function haversineDistance(
  lat1: number, lon1: number,
  lat2: number, lon2: number,
): number {
  const dLat = (lat2 - lat1) * DEG2RAD;
  const dLon = (lon2 - lon1) * DEG2RAD;
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(lat1 * DEG2RAD) * Math.cos(lat2 * DEG2RAD) *
    Math.sin(dLon / 2) ** 2;
  return R_EARTH * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

/** Initial bearing from point 1 to point 2 (degrees, 0–360) */
export function bearing(
  lat1: number, lon1: number,
  lat2: number, lon2: number,
): number {
  const dLon = (lon2 - lon1) * DEG2RAD;
  const y = Math.sin(dLon) * Math.cos(lat2 * DEG2RAD);
  const x =
    Math.cos(lat1 * DEG2RAD) * Math.sin(lat2 * DEG2RAD) -
    Math.sin(lat1 * DEG2RAD) * Math.cos(lat2 * DEG2RAD) * Math.cos(dLon);
  return ((Math.atan2(y, x) / DEG2RAD) + 360) % 360;
}

/** Destination point reached from (lat, lon) by travelling `distanceM` along `bearingDeg`
 *  (great-circle). Returns [lat, lon] in degrees. */
export function destinationPoint(
  lat: number, lon: number,
  bearingDeg: number, distanceM: number,
): { lat: number; lon: number } {
  const d = distanceM / R_EARTH;
  const brg = bearingDeg * DEG2RAD;
  const lat1 = lat * DEG2RAD;
  const lon1 = lon * DEG2RAD;
  const lat2 = Math.asin(
    Math.sin(lat1) * Math.cos(d) + Math.cos(lat1) * Math.sin(d) * Math.cos(brg),
  );
  const lon2 = lon1 + Math.atan2(
    Math.sin(brg) * Math.sin(d) * Math.cos(lat1),
    Math.cos(d) - Math.sin(lat1) * Math.sin(lat2),
  );
  return { lat: lat2 / DEG2RAD, lon: (((lon2 / DEG2RAD) + 540) % 360) - 180 };
}

/** Format distance: meters if <1000, km otherwise */
export function formatDistance(m: number): string {
  if (m < 1000) return `${Math.round(m)} m`;
  return `${(m / 1000).toFixed(1)} km`;
}
