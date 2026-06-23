// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Visual styling for INAV geozones. Shared by the 2D map, the 3D view and the Airspace Manager panel
// list. Colours are intentionally our own (not the INAV-configurator green/red): Inclusive (Flight-Zone)
// = blue (the app accent), Exclusive (No-Flight-Zone) = amber. The line-style / fill scheme by FENCE
// ACTION is adopted from MWPTools' geozone manager (mwp-geozonemgr.vala, the scheme worked out with the
// INAV author): None = dashed thin no-fill; Avoid = solid thin; Pos-Hold/RTH = solid thick. A
// translucent area fill is added for every exclusive zone with a real action, and only for an inclusive
// RTH zone. See docs/active/GEOZONES.md.

import {
  GEOZONE_TYPE_INCLUSIVE, GEOZONE_SHAPE_CIRCULAR,
  GEOZONE_ACTION_NONE, GEOZONE_ACTION_POSHOLD, GEOZONE_ACTION_RTH, type GeoZone,
} from '$lib/stores/geozone';

/** Inclusive / Flight-Zone — blue (the app accent). */
export const GEOZONE_INCLUSIVE_COLOR = '#37a8db';
/** Exclusive / No-Flight-Zone — amber. */
export const GEOZONE_EXCLUSIVE_COLOR = '#f5a623';

/** Line / fill opacities (mwp: 0.625 lines, 0.125 fills). */
const LINE_OPACITY = 0.85;
const FILL_OPACITY = 0.14;

/** Outline / fill colour for a zone, by type. */
export function geozoneColor(zone: GeoZone): string {
  return zone.zone_type === GEOZONE_TYPE_INCLUSIVE ? GEOZONE_INCLUSIVE_COLOR : GEOZONE_EXCLUSIVE_COLOR;
}

/** Whether this (type, action) combination gets a translucent area fill (the mwp scheme): every
 *  exclusive zone with a real action (Avoid/Pos-Hold/RTH), and an inclusive zone only when its action
 *  is RTH. "None" never fills. */
export function geozoneFilled(zone: GeoZone): boolean {
  if (zone.zone_type === GEOZONE_TYPE_INCLUSIVE) return zone.fence_action === GEOZONE_ACTION_RTH;
  return zone.fence_action !== GEOZONE_ACTION_NONE;
}

export interface GeozonePathStyle {
  color: string;
  weight: number;
  opacity: number;
  dashArray?: string;
  fill: boolean;
  fillColor: string;
  fillOpacity: number;
}

/** 2D (Leaflet) path style by type + action. None → dashed thin; Pos-Hold/RTH → thick; fill per
 *  `geozoneFilled`. */
export function geozonePathStyle(zone: GeoZone): GeozonePathStyle {
  const color = geozoneColor(zone);
  const none = zone.fence_action === GEOZONE_ACTION_NONE;
  const thick = zone.fence_action === GEOZONE_ACTION_POSHOLD || zone.fence_action === GEOZONE_ACTION_RTH;
  const filled = geozoneFilled(zone);
  return {
    color,
    weight: thick ? 4 : 2,
    opacity: LINE_OPACITY,
    dashArray: none ? '4 4' : undefined,
    fill: filled,
    fillColor: color,
    fillOpacity: filled ? FILL_OPACITY : 0,
  };
}

/** Radius (m) of a circular zone, or null for a polygon. (`radius_cm` is firmware cm.) */
export function geozoneRadiusM(zone: GeoZone): number | null {
  return zone.shape === GEOZONE_SHAPE_CIRCULAR && zone.radius_cm != null ? zone.radius_cm / 100 : null;
}
