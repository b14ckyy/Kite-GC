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
const DISC =
  'width:20px;height:20px;display:flex;align-items:center;justify-content:center;border-radius:50%;' +
  'border:1.5px solid #fff;box-shadow:0 0 4px rgba(0,0,0,0.5);font-size:11px;font-weight:700;color:#fff;line-height:1;';

const WIND_TURBINE_SVG =
  '<svg viewBox="0 0 24 24" width="13" height="13" fill="#fff"><rect x="11.2" y="11" width="1.6" height="10"/>' +
  '<g transform="translate(12 9)"><circle r="1.6"/>' +
  '<path d="M0 0 L0 -8 L1.6 -2 Z"/><path d="M0 0 L7 4 L2 1 Z"/><path d="M0 0 L-7 4 L-2 1 Z"/></g></svg>';
const TOWER_SVG = '<svg viewBox="0 0 24 24" width="12" height="12" fill="#fff"><path d="M12 2 L17 21 L7 21 Z"/></svg>';
const AIRPORT_SVG =
  '<svg viewBox="0 0 24 24" width="13" height="13" fill="#fff"><path d="M12 2l2 7 7 3-7 1-2 8-2-8-7-1 7-3z"/></svg>';

/** divIcon HTML for an aero point feature. */
export function aeroPointIconHtml(p: AeroPoint): string {
  if (p.kind === 'obstacle') {
    const glyph = p.subtype === 'Wind Turbine' ? WIND_TURBINE_SVG : TOWER_SVG;
    return `<div style="${DISC}background:#e8740c;">${glyph}</div>`;
  }
  if (p.kind === 'airport') {
    return `<div style="${DISC}background:#2e6fdb;">${AIRPORT_SVG}</div>`;
  }
  // RC / model airfield
  return `<div style="${DISC}background:#59aa29;font-size:9px;">RC</div>`;
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
