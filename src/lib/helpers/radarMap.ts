// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar map rendering helpers — relative-altitude colour scale + map visibility / distance relevance.
// See docs/active/RADAR_TRACKING_PANEL_AND_MAP.md §4. Colour depends on altitude only, never distance.

import type { RadarMapSettings } from '$lib/stores/settings';
import type { TrackedVehicle } from '$lib/stores/radarTracking';

export interface ContactColor {
  /** CSS `rgb()` fill for the silhouette body. */
  fill: string;
  /** Fill opacity (0–1) — translucent for contacts far above us. */
  fillOpacity: number;
  /** Outline colour (always a dark contrast for legibility). */
  outline: string;
}

type RGB = [number, number, number];

const BLUE: RGB = [21, 101, 255];
const VIOLET: RGB = [155, 48, 255];
const RED: RGB = [255, 42, 42];
const YELLOW: RGB = [255, 224, 0];
const GREEN: RGB = [46, 204, 64];
const WHITE: RGB = [255, 255, 255];
const OUTLINE = '#000000';
const NO_ALT = '#aaaaaa';

/** > +2000 m: translucent white, faded out (irrelevant). */
const ABOVE_OPACITY = 0.35;
/** Top of the relative colour scale — also the "+2000 m always show" override threshold (§4.0). */
export const REL_OVERRIDE_M = 2000;

interface Stop {
  d: number;
  c: RGB;
  a: number;
}
// Relative-altitude (Δ = contact − reference, metres) stops. Violet at our level = max danger; cools
// steeply going up (red→yellow→green→white) and goes hard blue going down.
const REL_STOPS: Stop[] = [
  { d: -500, c: BLUE, a: 1 },
  { d: 0, c: VIOLET, a: 1 },
  { d: 500, c: RED, a: 1 },
  { d: 1000, c: YELLOW, a: 1 },
  { d: 1500, c: GREEN, a: 1 },
  { d: 2000, c: WHITE, a: 1 },
];

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const lerpRGB = (a: RGB, b: RGB, t: number): RGB => [
  Math.round(lerp(a[0], b[0], t)),
  Math.round(lerp(a[1], b[1], t)),
  Math.round(lerp(a[2], b[2], t)),
];
const rgbCss = (c: RGB) => `rgb(${c[0]}, ${c[1]}, ${c[2]})`;

/** Colour for a RELATIVE altitude difference Δ (m). ≤ −500 = constant blue; ≥ +2000 = translucent white. */
export function relativeAltColor(deltaM: number): ContactColor {
  if (deltaM <= REL_STOPS[0].d) return { fill: rgbCss(BLUE), fillOpacity: 1, outline: OUTLINE };
  const last = REL_STOPS[REL_STOPS.length - 1];
  if (deltaM >= last.d) return { fill: rgbCss(WHITE), fillOpacity: ABOVE_OPACITY, outline: OUTLINE };
  for (let i = 0; i < REL_STOPS.length - 1; i++) {
    const s0 = REL_STOPS[i];
    const s1 = REL_STOPS[i + 1];
    if (deltaM >= s0.d && deltaM <= s1.d) {
      const t = (deltaM - s0.d) / (s1.d - s0.d);
      return { fill: rgbCss(lerpRGB(s0.c, s1.c, t)), fillOpacity: lerp(s0.a, s1.a, t), outline: OUTLINE };
    }
  }
  return { fill: rgbCss(WHITE), fillOpacity: 1, outline: OUTLINE };
}

function hsvToRgb(h: number, s: number, v: number): RGB {
  const i = Math.floor(h * 6);
  const f = h * 6 - i;
  const p = v * (1 - s);
  const q = v * (1 - f * s);
  const t = v * (1 - (1 - f) * s);
  let r = 0;
  let g = 0;
  let b = 0;
  switch (i % 6) {
    case 0: r = v; g = t; b = p; break;
    case 1: r = q; g = v; b = p; break;
    case 2: r = p; g = v; b = t; break;
    case 3: r = p; g = q; b = v; break;
    case 4: r = t; g = p; b = v; break;
    default: r = v; g = p; b = q; break;
  }
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
}

/** MWP-style ABSOLUTE-altitude fallback (no reference altitude): HSV hue 0.05→0.85 over 0–12 km. */
export function absoluteAltColor(altM: number): ContactColor {
  const a = Math.max(0, Math.min(12000, altM));
  const h = 0.05 + 0.8 * (a / 12000);
  return { fill: rgbCss(hsvToRgb(h, 0.8, 1)), fillOpacity: 1, outline: OUTLINE };
}

/** FormationFlight contact colour by state (NOT altitude-based): armed = dark blue + white outline,
 *  disarmed = grey + white outline, lost = grey dimmed + red outline. State from `extra.ffState`. */
export function ffContactColor(state: string | undefined): ContactColor {
  switch (state) {
    case 'lost':
      return { fill: '#6b7280', fillOpacity: 0.6, outline: '#ff2a2a' };
    case 'disarmed':
      return { fill: '#6b7280', fillOpacity: 0.92, outline: '#ffffff' };
    default: // armed (or unknown)
      return { fill: '#1e40af', fillOpacity: 0.95, outline: '#ffffff' };
  }
}

/** Colour for a contact: relative when a reference altitude exists, else absolute fallback. */
export function contactColor(altM: number | null, refAltM: number | null): ContactColor {
  if (altM == null) return { fill: NO_ALT, fillOpacity: 1, outline: OUTLINE };
  if (refAltM == null) return absoluteAltColor(altM);
  return relativeAltColor(altM - refAltM);
}

// Horizontal legend gradient stops, left (below) → right (above), positioned linearly over the Δ range
// −500 … +2000 m. So Δ = 0 (our level, violet) lands at 500/2500 = 20% — the UAV-altitude marker.
/** Percentage position of "our level" (Δ = 0) on the legend bar. */
export const LEGEND_LEVEL_PCT = 20;
/** Legend gradient stops (colour + % position), for the panel Map-tab legend bar. */
export const RELATIVE_LEGEND_STOPS: { color: string; pct: number }[] = [
  { color: rgbCss(BLUE), pct: 0 },
  { color: rgbCss(VIOLET), pct: LEGEND_LEVEL_PCT },
  { color: rgbCss(RED), pct: 40 },
  { color: rgbCss(YELLOW), pct: 60 },
  { color: rgbCss(GREEN), pct: 80 },
  { color: rgbCss(WHITE), pct: 100 },
];

/**
 * Should this contact render on the map? The altitude cutoff only affects the map — the contact always
 * stays in the panel list. (§4.0)
 *  - Hidden ⟺ ADS-B AND absolute alt > ceiling AND Δ > +2000 m (no relative override).
 *  - Distance never hides (only dims, see {@link relevanceFactor}).
 */
export function contactVisibleOnMap(
  v: TrackedVehicle,
  refAltM: number | null,
  map: RadarMapSettings,
): boolean {
  if (!v.validPos) return false;
  if (!map.visible[v.system]) return false;
  if (map.showAll) return true;
  if (v.system === 'adsb' && v.altM != null && v.altM > map.maxAltM) {
    const delta = refAltM != null ? v.altM - refAltM : null;
    return delta != null && delta <= REL_OVERRIDE_M; // +2000 m relative override
  }
  return true;
}

/** Soft dim/scale factor (0.4–1) for contacts beyond the radius. 1 = full strength. */
export function relevanceFactor(distanceM: number | null, radiusKm: number, showAll: boolean): number {
  if (showAll || distanceM == null) return 1;
  const r = radiusKm * 1000;
  if (distanceM <= r) return 1;
  return Math.max(0.4, 1 - (distanceM - r) / r);
}
