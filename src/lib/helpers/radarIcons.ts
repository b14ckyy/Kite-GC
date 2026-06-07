// Foreign-vehicle silhouette icons — top-down, pointing north (heading 0), recoloured by altitude.
// Inspired by MWPTools' per-category SVG set (A0–C3) but drawn ourselves, not copied.
// See docs/active/RADAR_TRACKING_PANEL_AND_MAP.md §4.2.

import type { VehicleSystem } from '$lib/stores/radarTracking';
import type { ContactColor } from '$lib/helpers/radarMap';

export type Shape = 'jet' | 'prop' | 'heli' | 'glider' | 'drone' | 'generic' | 'dot';

// All shapes drawn on a 24×24 viewBox, centred at (12,12), nose pointing up (north).
const SHAPES: Record<Shape, string> = {
  // Swept-wing airliner.
  jet: '<path d="M12 2 L13 9 L22 15 L22 17 L13 13.5 L13 19 L15.5 21 L15.5 22 L12 20.8 L8.5 22 L8.5 21 L11 19 L11 13.5 L2 17 L2 15 L11 9 Z"/>',
  // Straight-wing light/GA aircraft.
  prop: '<path d="M12 3 L12.9 10 L21 12 L21 13.6 L12.9 13 L12.9 18 L15 20 L15 20.8 L12 19.8 L9 20.8 L9 20 L11.1 18 L11.1 13 L3 13.6 L3 12 L11.1 10 Z"/>',
  // Long, thin wings.
  glider: '<path d="M12 4 L12.6 11 L23 12 L23 13 L12.6 12.8 L12.6 19 L13.8 20.5 L13.8 21.2 L12 20.4 L10.2 21.2 L10.2 20.5 L11.4 19 L11.4 12.8 L1 13 L1 12 L11.4 11 Z"/>',
  // Helicopter: fuselage + rotor disc (symmetric, rotation harmless).
  heli: '<circle cx="12" cy="12" r="8.5" fill="none" stroke-width="1.1"/><rect x="10.6" y="5" width="2.8" height="14" rx="1.2"/>',
  // Quad: X frame + four rotor discs.
  drone: '<path d="M5 5 L7 5 L12 10 L17 5 L19 5 L19 7 L14 12 L19 17 L19 19 L17 19 L12 14 L7 19 L5 19 L5 17 L10 12 L5 7 Z"/><circle cx="6" cy="6" r="2.3"/><circle cx="18" cy="6" r="2.3"/><circle cx="6" cy="18" r="2.3"/><circle cx="18" cy="18" r="2.3"/>',
  // Generic directional delta (category unknown but heading available).
  generic: '<path d="M12 3 L20 20 L12 16 L4 20 Z"/>',
  // Non-directional dot (no heading).
  dot: '<circle cx="12" cy="12" r="6"/>',
};

// ADS-B emitter category code → silhouette.
const CATEGORY_SHAPE: Record<string, Shape> = {
  A1: 'prop', A2: 'prop', A3: 'jet', A4: 'jet', A5: 'jet', A6: 'jet', A7: 'heli',
  B1: 'glider', B4: 'prop', B6: 'drone',
};

/** Pick a silhouette from system + ADS-B category + whether a heading is known. */
export function pickShape(
  system: VehicleSystem,
  category: string | null,
  hasHeading: boolean,
): Shape {
  if (!hasHeading) return 'dot';
  if (system === 'formationFlight') return 'drone';
  if (system === 'radio') return 'generic';
  return (category && CATEGORY_SHAPE[category]) || 'generic';
}

export interface ContactIconOpts {
  shape: Shape;
  /** Heading in degrees (0 = north), or null for a non-directional dot. */
  heading: number | null;
  color: ContactColor;
  /** Icon edge length in px (already scaled for relevance). */
  sizePx: number;
  /** Overall opacity (age/relevance dim), 0–1. */
  opacity: number;
  /** Draw the selection ring. */
  selected: boolean;
  /** Optional callsign label under the icon. */
  label?: string;
  /** Conflict-alert highlight: a pulsing ring around the icon (yellow caution / red warning). */
  alertLevel?: 'caution' | 'warning' | null;
}

/** Build the inner HTML for a Leaflet `divIcon` (SVG silhouette + optional callsign label). */
export function buildContactIconHtml(o: ContactIconOpts): string {
  const rot = o.heading ?? 0;
  const ring = o.selected
    ? '<circle cx="12" cy="12" r="11" fill="none" stroke="#37a8db" stroke-width="1.6"/>'
    : '';
  const body = `<g fill="${o.color.fill}" fill-opacity="${o.color.fillOpacity}" stroke="${o.color.outline}" stroke-width="0.8" stroke-linejoin="round" transform="rotate(${rot} 12 12)">${SHAPES[o.shape]}</g>`;
  const svg = `<svg width="${o.sizePx}" height="${o.sizePx}" viewBox="0 0 24 24" style="overflow:visible">${ring}${body}</svg>`;
  const label = o.label
    ? `<span class="radar-icon-label">${o.label}</span>`
    : '';
  // Conflict-alert ring around the icon, pulsing via CSS (alerting contacts are always near, so the
  // icon's distance-dim is ~1 anyway).
  const alertRing = o.alertLevel
    ? `<span class="radar-alert-ring ${o.alertLevel}"></span>`
    : '';
  return `<div class="radar-icon" style="opacity:${o.opacity}">${alertRing}${svg}${label}</div>`;
}
