// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Mission-marker icon primitives — the SINGLE source of the mission waypoint visual schema, shared by
 * INAV (missionIcons.ts) and ArduPilot (missionIconsArdupilot.ts).
 *
 * Each platform has its own type→primitive mapper (their waypoint models differ), but the *shapes*
 * (teardrop, orbit ring, house, …) and the *colour palette* live here once. Change a shared marker
 * (e.g. the loiter ring) in one place and both platforms update; platform-specific types just pick a
 * primitive + a palette entry. SVGs carry `xmlns` so the same spec renders both as a Leaflet divIcon
 * (2D) and a Cesium billboard (3D).
 */

import L from 'leaflet';

/** Marker visual spec, shared by the 2D map (Leaflet divIcon) and the 3D map (Cesium billboard) so
 *  both render the exact same SVG. `anchorX/anchorY` are image pixels from the top-left (the point
 *  that sits on the coordinate). */
export interface WpIconSpec {
  svg: string;
  width: number;
  height: number;
  anchorX: number;
  anchorY: number;
}

/** Fill + stroke pair for a marker. */
export interface WpColor {
  fill: string;
  stroke: string;
}

/** Selected state — overrides any semantic colour (both platforms). */
export const WP_SELECTED: WpColor = { fill: '#ff4444', stroke: '#cc0000' };

export type WpSemantic = 'waypoint' | 'loiter' | 'land' | 'takeoff' | 'rth' | 'poi' | 'generic';

/**
 * Canonical per-semantic mission-marker palette — the single colour source for both platforms.
 * Set to INAV's established values so the ArduPilot layer *conforms* to them (its hand-copied icons
 * had drifted: loiter was cyan, ROI teal). Flip a value here once to recolour a type everywhere.
 */
export const WP_COLOR: Record<WpSemantic, WpColor> = {
  waypoint: { fill: '#37a8db', stroke: '#1a5276' },
  loiter:   { fill: '#f39c12', stroke: '#d68910' },
  land:     { fill: '#f39c12', stroke: '#d68910' },
  takeoff:  { fill: '#27ae60', stroke: '#1e8449' },
  rth:      { fill: '#e67e22', stroke: '#7e5109' },
  poi:      { fill: '#8e44ad', stroke: '#6c3483' },
  generic:  { fill: '#7f8c8d', stroke: '#2c3e50' },
};

/** Resolve a semantic colour, with the selected-state override. */
export function wpColor(semantic: WpSemantic, selected: boolean): WpColor {
  return selected ? WP_SELECTED : WP_COLOR[semantic];
}

// ── Shape primitives ────────────────────────────────────────────────────────

/** Optional small corner badge (e.g. "V" for VTOL, "S" for spline) for the teardrop specs. Rendered
 *  top-right in the 32×44 teardrop viewBox; empty string when no badge. */
function badgeSvg(badge?: string): string {
  if (!badge) return '';
  return `<circle cx="25" cy="9" r="7" fill="#2c3e50" stroke="white" stroke-width="1.5"/>
      <text x="25" y="12" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">${badge}</text>`;
}

/** Optional 4-slot indicator row (INAV User Actions UA1–4) inside the teardrop bulb. `mask` is a 4-bit
 *  flag set (bit0 = slot 1 … bit3 = slot 4); active slots are bright amber, inactive faint. Empty when
 *  no slot is set, so unflagged waypoints stay clean. */
function uaDotsSvg(mask?: number): string {
  if (!mask) return '';
  const xs = [11.5, 14.7, 17.9, 21.1];
  let s = '';
  for (let i = 0; i < 4; i++) {
    const on = (mask >> i) & 1;
    s += `<circle cx="${xs[i]}" cy="29" r="1.6" fill="${on ? '#ffd000' : '#ffffff'}"${on ? '' : ' opacity="0.25"'} stroke="#000" stroke-width="0.5"/>`;
  }
  return s;
}

/** Upside-down teardrop with a waypoint number (48×66, bottom-anchored). Optional corner `badge`
 *  marks a variant (e.g. "S" spline, "P" payload-place); optional `uaMask` draws the UA1–4 indicator. */
export function teardropNumberSpec(num: number, c: WpColor, badge?: string, uaMask?: number): WpIconSpec {
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${c.fill}" stroke="${c.stroke}" stroke-width="2"/>
      <text x="16" y="20" text-anchor="middle" fill="white" font-size="12" font-weight="bold"
            font-family="sans-serif">${num}</text>
      ${uaDotsSvg(uaMask)}
      ${badgeSvg(badge)}
    </svg>`,
    width: 48, height: 66, anchorX: 24, anchorY: 66,
  };
}

/** Teardrop with an up/down arrow (Takeoff / Land) (48×66, bottom-anchored). Optional corner `badge`
 *  distinguishes VTOL takeoff/land ("V") from the fixed-wing/multirotor variants. */
export function teardropArrowSpec(dir: 'up' | 'down', c: WpColor, badge?: string): WpIconSpec {
  const arrow = dir === 'down'
    ? 'M16 10 L16 25 M11 20 L16 25 L21 20'
    : 'M16 26 L16 11 M11 16 L16 11 L21 16';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${c.fill}" stroke="${c.stroke}" stroke-width="2"/>
      <path d="${arrow}" fill="none" stroke="white"
            stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
      ${badgeSvg(badge)}
    </svg>`,
    width: 48, height: 66, anchorX: 24, anchorY: 66,
  };
}

/** Orbit ring + inner circle with number and a small label (PosHold / Loiter) (88×88, centre-anchored).
 *  `label` is the hold descriptor, e.g. "30s", "∞", "×3". */
export function orbitSpec(num: number, label: string, c: WpColor): WpIconSpec {
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40" width="88" height="88">
      <circle cx="20" cy="20" r="17" fill="none" stroke="${c.stroke}" stroke-width="2" stroke-dasharray="4 2"/>
      <circle cx="20" cy="20" r="11" fill="${c.fill}" stroke="${c.stroke}" stroke-width="1.5"/>
      <text x="20" y="18" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
      <text x="20" y="26" text-anchor="middle" fill="white" font-size="7"
            font-family="sans-serif">${label}</text>
    </svg>`,
    width: 88, height: 88, anchorX: 44, anchorY: 44,
  };
}

/** House icon with a label (RTH / RTL) (42×42, centre-anchored). */
export function houseSpec(label: string, c: WpColor): WpIconSpec {
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="42" height="42">
      <path d="M16 4 L4 16 L8 16 L8 28 L24 28 L24 16 L28 16 Z" fill="${c.fill}" stroke="${c.stroke}" stroke-width="1.5"/>
      <text x="16" y="22" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">${label}</text>
    </svg>`,
    width: 42, height: 42, anchorX: 21, anchorY: 21,
  };
}

/** Circle with an eye glyph and number (Set POI / ROI) (48×48, centre-anchored). */
export function eyeSpec(num: number, c: WpColor): WpIconSpec {
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${c.fill}" stroke="${c.stroke}" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="10"
            font-family="sans-serif">👁</text>
      <text x="16" y="25" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 48, height: 48, anchorX: 24, anchorY: 24,
  };
}

/** Generic fallback: circle with a short label and number (48×48, centre-anchored). */
export function genericSpec(num: number, label: string, c: WpColor): WpIconSpec {
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${c.fill}" stroke="${c.stroke}" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="8"
            font-family="sans-serif">${label}</text>
      <text x="16" y="24" text-anchor="middle" fill="white" font-size="10" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 48, height: 48, anchorX: 24, anchorY: 24,
  };
}

// ── 2D wrapper ──────────────────────────────────────────────────────────────

/** Build a 2D Leaflet divIcon from a spec. The anchor class drives the CSS transform-origin so the
 *  marker scales around the point on the coordinate (bottom-centre for teardrops, centre otherwise);
 *  `active` adds the pulsing-glow class for the FC's current target waypoint. */
export function divIconFromSpec(spec: WpIconSpec, active = false): L.DivIcon {
  const anchorCls = spec.anchorY >= spec.height ? 'wp-anchor-bottom' : 'wp-anchor-center';
  return L.divIcon({
    className: `mission-wp-icon ${anchorCls}${active ? ' mission-wp-active' : ''}`,
    html: spec.svg,
    iconSize: [spec.width, spec.height],
    iconAnchor: [spec.anchorX, spec.anchorY],
  });
}
