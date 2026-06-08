// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// 3D radar helpers — pick a glb model per contact (rendered like the UAV model: oriented to heading,
// altitude-tinted, no flicker) and the ground-projection arrow outline. See
// docs/active/RADAR_TRACKING_PANEL_AND_MAP.md §4.5. Pure data — no Cesium here.

import type { VehicleSystem } from '$lib/stores/radarTracking';

/** Ground-projection circle radius (m) — world-sized (real perspective). */
export const GROUND_CIRCLE_RADIUS_M = 1000;

/** Radar/ADS-B 3D model classes — one glb per class (own files under /models/radar/, see its README). */
export type RadarModelClass =
  | 'light' | 'small' | 'heavy' | 'jet' | 'heli' | 'glider' | 'balloon' | 'arrow' | 'ground' | 'dot' | 'ff';

const RADAR_MODEL_FILE: Record<RadarModelClass, string> = {
  light: 'adsb-light',
  small: 'adsb-small',
  heavy: 'adsb-heavy',
  jet: 'adsb-jet',
  heli: 'adsb-heli',
  glider: 'adsb-glider',
  balloon: 'adsb-balloon',
  arrow: 'adsb-arrow',
  ground: 'adsb-ground',
  dot: 'adsb-dot',
  ff: 'ff-uav', // FormationFlight peers (paper-plane)
};

/** Whether a contact is irrelevant traffic hidden everywhere (list + map): ADS-B obstacles / reserved
 *  ground (C‑, C3–C7) and the all-reserved D‑ set. Surface VEHICLES (C1 emergency, C2 service) are kept. */
export function isHiddenCategory(category: string | null): boolean {
  if (!category) return false;
  if (category[0] === 'D') return true;
  if (category[0] === 'C') return category !== 'C1' && category !== 'C2';
  return false;
}

/** Static asset URL for a radar model class (own folder, separate from the UAV models). */
export function radarModelUri(c: RadarModelClass): string {
  return `/models/radar/${RADAR_MODEL_FILE[c]}.glb`;
}

/** Map a contact (system + ADS-B emitter category) to a model class. See the folder README for the table. */
export function contactModelClass(
  system: VehicleSystem,
  category: string | null,
  hasHeading: boolean,
): RadarModelClass {
  if (system === 'formationFlight') return 'ff'; // always the paper-plane model
  if (!hasHeading) return 'dot'; // non-directional — heading unknown
  if (system === 'radio') return 'arrow';
  switch (category) {
    case 'A0': case 'A1': return 'light'; // unspecified powered / light
    case 'A2': return 'small';
    case 'A3': case 'A4': case 'A5': return 'heavy';
    case 'A6': return 'jet'; // high performance
    case 'A7': return 'heli';
    case 'B1': return 'glider';
    case 'B2': return 'balloon'; // lighter-than-air
    case 'C1': case 'C2': return 'ground'; // surface vehicles (emergency / service)
    // B-/B3/B4/B6/B7, reserved (B5) and no category → arrow. (C‑/C3–C7/D‑ are filtered out upstream.)
    default: return 'arrow';
  }
}

/** i18n key for the spelled-out contact type word (alert banner / labels). Heading-independent — a
 *  contact without a heading is still e.g. an airliner. */
export function contactTypeKey(system: VehicleSystem, category: string | null): string {
  if (system === 'formationFlight') return 'radar.acType.formation';
  if (system === 'radio') return 'radar.acType.radio';
  const cls = contactModelClass(system, category, true);
  return `radar.acType.${cls}`;
}

/** Direction arrow drawn inside the ground circle (local units, nose = +north), heading-rotated. */
export const ARROW_POLY: [number, number][] = [
  [0, 0.5], [0.22, -0.05], [0.09, -0.05], [0.09, -0.5], [-0.09, -0.5], [-0.09, -0.05], [-0.22, -0.05],
];

/** Closed unit circle (radius 1) for the clamped alert ring — scaled to the alert radius in Map3D. */
export const CIRCLE_POLY: [number, number][] = Array.from({ length: 65 }, (_, i) => {
  const a = (i / 64) * Math.PI * 2;
  return [Math.cos(a), Math.sin(a)] as [number, number];
});
