// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Visual styling for the Airspace Manager layers: airspace polygon colours (by OpenAIP type) + point
// marker icon HTML (obstacles incl. wind turbines, airports, RC fields). Shared by the 2D map (and
// later the 3D view + legend). See docs/active/AIRSPACE_MANAGER.md.

import type { Airspace, AeroPoint } from '$lib/stores/airspace';

export interface AirspacePathStyle {
  color: string;
  fillColor: string;
  fillOpacity: number;
  weight: number;
  dashArray?: string;
}

/** Polygon style for an airspace, grouped by OpenAIP `type` into a small palette. */
export function airspaceStyle(a: Airspace): AirspacePathStyle {
  const t = a.typeId;
  if (t === 3) return { color: '#d40000', fillColor: '#d40000', fillOpacity: 0.12, weight: 2, dashArray: '6 4' }; // Prohibited
  if (t === 1 || t === 2) return { color: '#e8740c', fillColor: '#e8740c', fillOpacity: 0.10, weight: 2, dashArray: '6 4' }; // Restricted / Danger
  if (t === 4 || t === 13 || t === 14 || t === 36) return { color: '#c0392b', fillColor: '#c0392b', fillOpacity: 0.08, weight: 2 }; // CTR / ATZ / MATZ
  if (t === 7 || t === 26 || t === 24 || t === 34 || t === 35) return { color: '#2e6fdb', fillColor: '#2e6fdb', fillOpacity: 0.07, weight: 1.6 }; // TMA / CTA / TIA / LTA / UTA
  if (t === 5 || t === 6) return { color: '#8e44ad', fillColor: '#8e44ad', fillOpacity: 0.06, weight: 1.6 }; // TMZ / RMZ
  if (t === 21 || t === 28) return { color: '#59aa29', fillColor: '#59aa29', fillOpacity: 0.06, weight: 1.4 }; // Gliding / sporting
  return { color: '#37a8db', fillColor: '#37a8db', fillOpacity: 0.05, weight: 1.2 };
}

// ── Point marker icons (inline-styled, self-contained divIcon HTML) ──
// Obstacles: subtle black silhouettes with a white outline (no coloured disc) — `paint-order:stroke`
// draws the white stroke behind the black fill so it reads as an outline, legible on dark *and* light
// tiles. Airports/RC keep a small coloured badge, colour-coded by category.

// White outline applied to all child shapes (stroke/paint-order inherit through the SVG).
const OUTLINE = 'paint-order:stroke;stroke:#fff;stroke-width:1.6;stroke-linejoin:round;';
const OBST_WRAP = 'display:flex;align-items:center;justify-content:center;width:18px;height:18px;';

const WIND_TURBINE_SVG =
  `<svg viewBox="0 0 24 24" width="17" height="17" fill="#161616" style="${OUTLINE}">` +
  '<rect x="11.2" y="10" width="1.6" height="12"/>' +
  '<g transform="translate(12 8.5)"><circle r="1.8"/>' +
  '<path d="M0 0 L0 -8 L1.7 -2 Z"/><path d="M0 0 L7 4 L2 1 Z"/><path d="M0 0 L-7 4 L-2 1 Z"/></g></svg>';
// Simple lattice mast/tower silhouette (chimney, building, tower, generic obstacle).
const TOWER_SVG =
  `<svg viewBox="0 0 16 16" width="15" height="15" fill="#161616" style="${OUTLINE}">` +
  '<path d="M8 1.5 L11 14 L5 14 Z"/></svg>';

const STAR = '<path d="M12 2l2 7 7 3-7 1-2 8-2-8-7-1 7-3z"/>'; // OpenAIP-style airport star glyph
/** A small round badge: coloured fill, white border, the given inner content style. */
function badge(bg: string, inner: string, content: string): string {
  return (
    `<div style="width:18px;height:18px;display:flex;align-items:center;justify-content:center;` +
    `border-radius:50%;border:1.5px solid #fff;box-shadow:0 0 3px rgba(0,0,0,0.55);background:${bg};` +
    `${inner}">${content}</div>`
  );
}

/** Airport badge colour-coded by OpenAIP `type`: intl=red, airport=blue, airfield=green, heliport="H". */
function airportIconHtml(p: AeroPoint): string {
  const t = p.typeId;
  if (t === 4 || t === 7) // Heliport (mil) / Heliport → white "H"
    return badge('#5b6b7a', 'font-size:12px;font-weight:800;color:#fff;line-height:1;', 'H');
  let bg = '#59aa29'; // small airfields / glider / ultralight / strips / water / altiport → green
  if (t === 3) bg = '#d40000'; // International Airport → red
  else if (t === 0 || t === 5 || t === 9) bg = '#2e6fdb'; // Airport / Military Aerodrome / Airfield IFR → blue
  else if (t === 8) bg = '#7a7a7a'; // Aerodrome (closed) → grey
  const star = `<svg viewBox="0 0 24 24" width="12" height="12" fill="#fff">${STAR}</svg>`;
  return badge(bg, 'color:#fff;', star);
}

/** divIcon HTML for an aero point feature. */
export function aeroPointIconHtml(p: AeroPoint): string {
  if (p.kind === 'obstacle') {
    const glyph = p.subtype === 'Wind Turbine' ? WIND_TURBINE_SVG : TOWER_SVG;
    return `<div style="${OBST_WRAP}">${glyph}</div>`;
  }
  if (p.kind === 'airport') return airportIconHtml(p);
  // RC / model airfield → green "RC" badge
  return badge('#59aa29', 'font-size:9px;font-weight:700;color:#fff;line-height:1;', 'RC');
}

// ── Point-in-polygon (for the 2D click → "all airspaces here" list) ──
function pointInRing(lat: number, lon: number, ring: [number, number][]): boolean {
  // Ray casting; ring vertices are [lon, lat] → x = lon, y = lat.
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const xi = ring[i][0], yi = ring[i][1];
    const xj = ring[j][0], yj = ring[j][1];
    if ((yi > lat) !== (yj > lat) && lon < ((xj - xi) * (lat - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** True if the point falls inside any of the airspace's outer rings. */
export function airspaceContainsPoint(a: Airspace, lat: number, lon: number): boolean {
  for (const ring of a.outlines) if (pointInRing(lat, lon, ring)) return true;
  return false;
}

// ── Click-list relevance ─────────────────────────────────────────────
// Unclassified airspace (FIR/UIR, FIS/ACC sectors, classless airways) is effectively free-to-use for
// UAVs — pure clutter when a *classified* airspace covers the same spot. Drop it in that case, but
// never drop no-fly / mandatory zones (Restricted/Danger/Prohibited, TMZ, RMZ, …) even when classless.
const CLASSIFIED_ICAO = new Set(['A', 'B', 'C', 'D', 'E', 'F', 'G']);
const ALWAYS_SHOW_TYPES = new Set([1, 2, 3, 5, 6, 12, 17, 18, 19, 36]); // R/D/P · TMZ · RMZ · ADIZ · Alert · Warning · Protected · MCTR

/** Worth keeping even when classified airspaces overlap (has an ICAO class, or is a no-fly/mandatory type). */
export function airspaceIsRelevant(a: Airspace): boolean {
  return CLASSIFIED_ICAO.has(a.icaoClassName) || ALWAYS_SHOW_TYPES.has(a.typeId);
}

// ── Zoom-density management ─────────────────────────────────────────
// Each feature appears only at/above a minimum 2D (Leaflet) zoom, by importance/size — so zoomed out
// you see only the big/important features and detail fills in as you zoom in (à la the OpenAIP map).
// Leaflet zoom ≈ 2 world · 6 country · 8 region · 11 area · 13 local. Tunable. See AIRSPACE_MANAGER.md.

// Airspaces grouped into importance tiers by OpenAIP `type`.
const AIRSPACE_TIER_A = new Set([1, 2, 3, 4, 7, 12, 26, 36]); // Prohibited/Restricted/Danger/CTR/TMA/ADIZ/CTA/MCTR
const AIRSPACE_TIER_B = new Set([5, 6, 13, 14, 17, 18, 19, 23, 24, 25]); // RMZ/TMZ/ATZ/MATZ/Alert/Warning/Protected/TIZ/TIA/MTA

export function airspaceMinZoom(a: Airspace): number {
  const t = a.typeId;
  if (t === 10 || t === 11) return 6; // FIR / UIR — huge
  if (AIRSPACE_TIER_A.has(t)) return 7;
  if (AIRSPACE_TIER_B.has(t)) return 9;
  return 11; // gliding / sporting / VFR-FIS sectors / airways / etc.
}

export function airportMinZoom(p: AeroPoint): number {
  switch (p.typeId) {
    case 3: return 6; // International
    case 0: case 9: case 5: return 8; // Airport / IFR / Military aerodrome
    case 2: case 7: case 4: return 9; // Airfield civil / Heliport / Heliport military
    default: return 11; // glider / ultralight / water / strips / altiport / closed
  }
}

export const RC_MIN_ZOOM = 12;
const OBSTACLE_MIN_ZOOM = 12;
const OBSTACLE_TALL_MIN_ZOOM = 10; // height ≥ 150 m (e.g. tall masts) — show a bit earlier

export function obstacleMinZoom(p: AeroPoint): number {
  return p.heightM != null && p.heightM >= 150 ? OBSTACLE_TALL_MIN_ZOOM : OBSTACLE_MIN_ZOOM;
}

/** Short info line for a point feature (popup / nearby list). */
export function aeroPointInfo(p: AeroPoint): string {
  const parts: string[] = [];
  if (p.kind === 'obstacle' && p.heightM != null) parts.push(`${Math.round(p.heightM)} m AGL`);
  if (p.subtype) parts.push(p.subtype);
  if (p.kind === 'rc' && p.extra.permittedAltitude) parts.push(`max ${p.extra.permittedAltitude} m`);
  if (p.kind === 'rc' && p.extra.operator) parts.push(p.extra.operator);
  return parts.join(' · ');
}
