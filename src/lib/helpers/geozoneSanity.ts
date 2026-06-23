// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Geozone sanity checks — the same constraints the INAV configurator enforces, so a config saved from
// Kite is valid on the FC. Hard errors block "Save to FC"; CCW winding is auto-fixed at save. See
// docs/active/GEOZONES.md.

import {
  GEOZONE_SHAPE_CIRCULAR, MAX_GEOZONES, MAX_VERTICES_TOTAL,
  type GeoZone, type GeozoneConfig, type GeoZoneVertex,
} from '$lib/stores/geozone';

/** Vertices a zone consumes in the shared FC pool: a circle uses 2 (centre + radius), a polygon uses N. */
export function zoneVertexUsage(z: GeoZone): number {
  return z.shape === GEOZONE_SHAPE_CIRCULAR ? 2 : z.vertices.length;
}

/** Total vertices used across all zones (against `MAX_VERTICES_TOTAL`). */
export function totalVertexUsage(zones: GeoZone[]): number {
  return zones.reduce((sum, z) => sum + zoneVertexUsage(z), 0);
}

/** Signed area of a polygon (shoelace) in local coordinates relative to the first vertex, so the e7
 *  integer products stay well within double precision. > 0 = counter-clockwise (the INAV requirement). */
function signedArea(vertices: GeoZoneVertex[]): number {
  if (vertices.length < 3) return 0;
  const lat0 = vertices[0].lat, lon0 = vertices[0].lon;
  let a = 0;
  for (let i = 0; i < vertices.length; i++) {
    const j = (i + 1) % vertices.length;
    const xi = (vertices[i].lon - lon0) / 1e7, yi = (vertices[i].lat - lat0) / 1e7;
    const xj = (vertices[j].lon - lon0) / 1e7, yj = (vertices[j].lat - lat0) / 1e7;
    a += xi * yj - xj * yi;
  }
  return a / 2;
}

/** True when a polygon's vertices wind counter-clockwise (lon = x, lat = y). */
export function isCCW(vertices: GeoZoneVertex[]): boolean {
  return signedArea(vertices) > 0;
}

/** Do two segments p1→p2 and p3→p4 properly cross? (Endpoints touching is not counted.) */
function segmentsCross(p1: GeoZoneVertex, p2: GeoZoneVertex, p3: GeoZoneVertex, p4: GeoZoneVertex): boolean {
  const d = (a: GeoZoneVertex, b: GeoZoneVertex, c: GeoZoneVertex) =>
    (b.lon - a.lon) * (c.lat - a.lat) - (b.lat - a.lat) * (c.lon - a.lon);
  const d1 = d(p3, p4, p1), d2 = d(p3, p4, p2), d3 = d(p1, p2, p3), d4 = d(p1, p2, p4);
  return ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0));
}

/** True when a polygon's edges cross one another (a self-intersecting / invalid ring). */
export function polygonSelfIntersects(vertices: GeoZoneVertex[]): boolean {
  const n = vertices.length;
  if (n < 4) return false;
  for (let i = 0; i < n; i++) {
    const a1 = vertices[i], a2 = vertices[(i + 1) % n];
    for (let j = i + 1; j < n; j++) {
      // Skip shared-vertex / adjacent edges (incl. the wrap-around pair).
      if (j === i || (i + 1) % n === j || (j + 1) % n === i) continue;
      if (segmentsCross(a1, a2, vertices[j], vertices[(j + 1) % n])) return true;
    }
  }
  return false;
}

/** Return a copy of the config with every polygon wound counter-clockwise (the INAV requirement);
 *  circles are untouched. Applied at save time so winding is always correct regardless of edit order. */
export function ensureCCWConfig(cfg: GeozoneConfig): GeozoneConfig {
  return {
    ...cfg,
    zones: cfg.zones.map((z) =>
      z.shape !== GEOZONE_SHAPE_CIRCULAR && z.vertices.length >= 3 && !isCCW(z.vertices)
        ? { ...z, vertices: [...z.vertices].reverse() }
        : z,
    ),
  };
}

export interface GeozoneIssue {
  level: 'error' | 'warning';
  key: string;                              // i18n key under `geozone.*`
  values?: Record<string, string | number>; // ICU params
  zoneId?: number;
}

/** Validate the whole config against the FC constraints. Errors must block "Save to FC". */
export function validateGeozones(zones: GeoZone[]): GeozoneIssue[] {
  const issues: GeozoneIssue[] = [];
  if (zones.length > MAX_GEOZONES) {
    issues.push({ level: 'error', key: 'errTooManyZones', values: { max: MAX_GEOZONES } });
  }
  const total = totalVertexUsage(zones);
  if (total > MAX_VERTICES_TOTAL) {
    issues.push({ level: 'error', key: 'errTooManyVertices', values: { n: total, max: MAX_VERTICES_TOTAL } });
  }
  for (const z of zones) {
    const n = z.id + 1;
    if (z.shape === GEOZONE_SHAPE_CIRCULAR) {
      if (z.radius_cm == null || z.radius_cm <= 0) {
        issues.push({ level: 'error', key: 'errCircleRadius', values: { n }, zoneId: z.id });
      }
    } else {
      if (z.vertices.length < 3) {
        issues.push({ level: 'error', key: 'errPolyMinVertices', values: { n }, zoneId: z.id });
      } else if (polygonSelfIntersects(z.vertices)) {
        issues.push({ level: 'error', key: 'errSelfIntersect', values: { n }, zoneId: z.id });
      }
    }
    if (z.max_alt_cm > 0 && z.max_alt_cm <= z.min_alt_cm) {
      issues.push({ level: 'error', key: 'errAltBand', values: { n }, zoneId: z.id });
    }
  }
  return issues;
}
