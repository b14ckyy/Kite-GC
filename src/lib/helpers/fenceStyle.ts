// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Visual styling for ArduPilot/PX4 geofences. Shared by the 2D map, the 3D view and the Airspace
// Manager panel. Colours match the INAV geozone scheme for consistency (see helpers/geozoneStyle.ts):
// Inclusion (must stay inside) = blue (the app accent), Exclusion (no-fly) = amber. Fences have no
// per-zone action/altitude (those are global params), so the style is simpler than geozones: a solid
// outline + a translucent area fill for every zone. See docs/active/GEOFENCE.md.

import { FENCE_KIND_INCLUSION, FENCE_SHAPE_CIRCLE, type FenceZone } from '$lib/stores/fence';

/** Inclusion / keep-in fence — blue (the app accent). */
export const FENCE_INCLUSION_COLOR = '#37a8db';
/** Exclusion / keep-out fence — amber. */
export const FENCE_EXCLUSION_COLOR = '#f5a623';

const LINE_OPACITY = 0.85;
const FILL_OPACITY = 0.14;

/** Outline / fill colour for a fence zone, by kind. */
export function fenceColor(zone: FenceZone): string {
  return zone.kind === FENCE_KIND_INCLUSION ? FENCE_INCLUSION_COLOR : FENCE_EXCLUSION_COLOR;
}

export interface FencePathStyle {
  color: string;
  weight: number;
  opacity: number;
  fill: boolean;
  fillColor: string;
  fillOpacity: number;
}

/** 2D (Leaflet) path style — solid outline + translucent fill. */
export function fencePathStyle(zone: FenceZone): FencePathStyle {
  const color = fenceColor(zone);
  return {
    color,
    weight: 2,
    opacity: LINE_OPACITY,
    fill: true,
    fillColor: color,
    fillOpacity: FILL_OPACITY,
  };
}

/** Radius (m) of a circular fence, or null for a polygon. (`radius_cm` is firmware cm.) */
export function fenceRadiusM(zone: FenceZone): number | null {
  return zone.shape === FENCE_SHAPE_CIRCLE && zone.radius_cm != null ? zone.radius_cm / 100 : null;
}
