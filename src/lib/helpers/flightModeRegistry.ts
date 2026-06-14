// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Flight-mode output registry — the single presentation source for the unified, protocol-agnostic
 * flight-mode model (see docs/active/FLIGHT_MODE_UNIFIED.md).
 *
 * Protocol input adapters (backend) classify raw mode data into a canonical `FlightModeState`
 * (string ids). This registry maps each id → a display label + a semantic **category**. The category
 * is the shared colour axis (drives the widget badge colour + track-coloring); the label stays exact
 * per mode, so nothing is lost. Adding a protocol = a new adapter + a few ids here — no widget/pipeline
 * changes. Mode names are intentionally kept in English (universal pilot terminology — FBW-A, RTL, …).
 */

/** Canonical, protocol-agnostic flight mode (mirrors the backend struct). */
export interface FlightModeState {
  primary: string;
  modifiers: string[];
}

export type ModeCategory =
  | 'manual' | 'acro' | 'stabilized' | 'althold' | 'poshold' | 'cruise'
  | 'mission' | 'guided' | 'rth' | 'launch' | 'land' | 'failsafe' | 'autotune' | 'other';

/** Category → colour (the only place flight-mode colours are defined). */
export const CATEGORY_COLOR: Record<ModeCategory, string> = {
  manual:     '#808080',
  acro:       '#c0c0c0',
  stabilized: '#59aa29',
  althold:    '#e8c820',
  poshold:    '#00bcd4',
  cruise:     '#ff8c00',
  mission:    '#37a8db',
  guided:     '#2980b9',
  rth:        '#9b59b6',
  launch:     '#e91e9c',
  land:       '#e67e22',
  failsafe:   '#e60000',
  autotune:   '#e8c820',
  other:      '#808080',
};

interface ModeDef {
  label: string;          // exact mode name (primary badge)
  category: ModeCategory; // shared semantic bucket (colour)
  short?: string;         // compact label for modifier chips (INAV stacking)
}

/**
 * Canonical id → definition. Ids come 1:1 from the protocol adapters; cross-protocol-identical modes
 * (acro/manual/poshold/althold/autotune/cruise) share one entry.
 */
export const MODE_REGISTRY: Record<string, ModeDef> = {
  // ── INAV primary ──
  failsafe_rth: { label: 'Failsafe RTH', category: 'failsafe' },
  failsafe:     { label: 'Failsafe',     category: 'failsafe' },
  mission:      { label: 'Mission',      category: 'mission' },
  rth:          { label: 'RTH',          category: 'rth' },
  launch:       { label: 'Launch',       category: 'launch' },
  poshold:      { label: 'PosHold',      category: 'poshold' },
  cruise:       { label: 'Cruise',       category: 'cruise' },
  angle:        { label: 'Angle',        category: 'stabilized' },
  horizon:      { label: 'Horizon',      category: 'stabilized' },
  manual:       { label: 'Manual',       category: 'manual' },
  acro:         { label: 'Acro',         category: 'acro' },

  // ── INAV modifiers (chips) ──
  althold:      { label: 'AltHold',  category: 'althold',  short: 'ALT' },
  headinghold:  { label: 'Heading',  category: 'other',    short: 'HDG' },
  headfree:     { label: 'HeadFree', category: 'other',    short: 'HFREE' },
  soaring:      { label: 'Soaring',  category: 'other',    short: 'SOAR' },
  autotune:     { label: 'AutoTune', category: 'autotune', short: 'TUNE' },
  flaperon:     { label: 'Flaperon', category: 'other',    short: 'FLAP' },
  autoland:     { label: 'Autoland', category: 'land',     short: 'LAND' },

  // ── ArduPilot (Plane + Copter; shared ids reuse the entries above) ──
  ardu_auto:    { label: 'Auto',        category: 'mission' },
  rtl:          { label: 'RTL',         category: 'rth' },
  guided:       { label: 'Guided',      category: 'guided' },
  guided_nogps: { label: 'Guided NoGPS', category: 'guided' },
  loiter:       { label: 'Loiter',      category: 'poshold' },
  circle:       { label: 'Circle',      category: 'cruise' },
  land:         { label: 'Land',        category: 'land' },
  stabilize:    { label: 'Stabilize',   category: 'stabilized' },
  training:     { label: 'Training',    category: 'stabilized' },
  fbwa:         { label: 'FBW-A',       category: 'stabilized' },
  fbwb:         { label: 'FBW-B',       category: 'althold' },
  takeoff:      { label: 'Takeoff',     category: 'launch' },
  avoid_adsb:   { label: 'Avoid ADSB',  category: 'failsafe' },
  drift:        { label: 'Drift',       category: 'other' },
  sport:        { label: 'Sport',       category: 'stabilized' },
  flip:         { label: 'Flip',        category: 'acro' },
  brake:        { label: 'Brake',       category: 'other' },
  throw:        { label: 'Throw',       category: 'launch' },
  smartrtl:     { label: 'SmartRTL',    category: 'rth' },
  flowhold:     { label: 'FlowHold',    category: 'poshold' },
  follow:       { label: 'Follow',      category: 'guided' },
  zigzag:       { label: 'ZigZag',      category: 'mission' },
  systemid:     { label: 'SystemID',    category: 'other' },
  autorotate:   { label: 'Autorotate',  category: 'failsafe' },
  autortl:      { label: 'AutoRTL',     category: 'rth' },
  qstabilize:   { label: 'QStabilize',  category: 'stabilized' },
  qhover:       { label: 'QHover',      category: 'althold' },
  qloiter:      { label: 'QLoiter',     category: 'poshold' },
  qland:        { label: 'QLand',       category: 'land' },
  qrtl:         { label: 'QRTL',        category: 'rth' },
  qautotune:    { label: 'QAutoTune',   category: 'autotune' },
  qacro:        { label: 'QAcro',       category: 'acro' },
  thermal:      { label: 'Thermal',     category: 'other' },
  loiter_qland: { label: 'Loiter QLand', category: 'land' },
};

// Shown when no FC is connected (empty primary id). Kept short so the badge stays inside the
// widget bounds (the full "Unknown" overflowed the fixed-size card).
const UNKNOWN: ModeDef = { label: 'N/A', category: 'other' };

function def(id: string | null | undefined): ModeDef {
  if (!id) return UNKNOWN;
  return MODE_REGISTRY[id] ?? { label: id, category: 'other' };
}

/** Display label for a canonical mode id (primary badge). */
export function modeLabel(id: string | null | undefined): string {
  return def(id).label;
}

/** Compact label for a modifier chip (falls back to the full label). */
export function modeShort(id: string): string {
  const d = def(id);
  return d.short ?? d.label;
}

/** Semantic category for a canonical mode id. */
export function modeCategory(id: string | null | undefined): ModeCategory {
  return def(id).category;
}

/** Colour for a canonical mode id (via its category). */
export function modeColor(id: string | null | undefined): string {
  return CATEGORY_COLOR[def(id).category];
}
