// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Live geofence breach check (ArduPilot/PX4) for the in-flight breach toast. ArduPilot already emits a
// STATUSTEXT on a fence breach (shown via stores/statusText.ts) — this is the GCS-side geometric
// fallback so a breach is flagged even with system messages off / on a link that doesn't relay them.
// Lateral: inside any exclusion fence, or outside the inclusion-fence union; vertical: above the global
// FENCE_ALT_MAX / GF_MAX_VER_DIST. See docs/active/GEOFENCE.md.

import { FENCE_KIND_INCLUSION, FENCE_SHAPE_CIRCLE, type FenceConfig, type FenceZone } from '$lib/stores/fence';
import { haversineDistance } from '$lib/utils/geo';

/** Ray-casting point-in-polygon (lon = x, lat = y); ring vertices in degrees. */
function pointInPolygon(lat: number, lon: number, ring: [number, number][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [yi, xi] = ring[i];
    const [yj, xj] = ring[j];
    if ((xi > lon) !== (xj > lon) && lat < ((yj - yi) * (lon - xi)) / (xj - xi) + yi) inside = !inside;
  }
  return inside;
}

/** Lateral containment of a point (degrees) in a fence zone's footprint. */
function fenceContainsLatLon(z: FenceZone, lat: number, lon: number): boolean {
  if (z.vertices.length === 0) return false;
  if (z.shape === FENCE_SHAPE_CIRCLE) {
    const c = z.vertices[0];
    const r = (z.radius_cm ?? 0) / 100;
    return r > 0 && haversineDistance(c.lat / 1e7, c.lon / 1e7, lat, lon) <= r;
  }
  if (z.vertices.length < 3) return false;
  return pointInPolygon(lat, lon, z.vertices.map((v) => [v.lat / 1e7, v.lon / 1e7] as [number, number]));
}

/** True when the live point breaches the fence (inside an exclusion / outside all inclusions / above the
 *  global vertical limit). `point.relM` is altitude above home. */
export function checkLiveFenceBreach(cfg: FenceConfig, point: { lat: number; lon: number; relM: number }): boolean {
  const incl = cfg.zones.filter((z) => z.kind === FENCE_KIND_INCLUSION);
  const excl = cfg.zones.filter((z) => z.kind !== FENCE_KIND_INCLUSION);

  if (excl.some((z) => fenceContainsLatLon(z, point.lat, point.lon))) return true;
  if (incl.length && !incl.some((z) => fenceContainsLatLon(z, point.lat, point.lon))) return true;

  const altMax = cfg.params.find((p) => p.name === 'FENCE_ALT_MAX' || p.name === 'GF_MAX_VER_DIST');
  if (altMax && altMax.value > 0 && point.relM > altMax.value) return true;
  return false;
}
