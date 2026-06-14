// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Track color helpers — flight-mode (via the canonical registry) + gradient coloring.
// Flight-mode classification lives in the protocol input adapters (backend); here we only map the
// canonical `mode_primary` id → colour/label via flightModeRegistry. See FLIGHT_MODE_UNIFIED.md.

import type { TelemetryRecord } from '$lib/stores/flightlog';
import { modeColor, modeLabel } from './flightModeRegistry';

// ── Types ───────────────────────────────────────────────────────────

export type TrackColorMode = 'flightmode' | 'altitude' | 'speed' | 'signal' | 'none';

export interface FlightModeInfo {
  id: string;
  label: string;
  color: string;
}

export interface TrackSegment {
  points: [number, number][];   // [lat, lon] pairs
  color: string;
  modeInfo?: FlightModeInfo;    // only for flightmode mode
}

/** Result from gradient segmentation, includes metadata for the legend. */
export interface GradientResult {
  segments: TrackSegment[];
  min: number;
  max: number;
  fieldLabel: string;           // e.g. "Altitude", "Speed", "LQ", "RSSI"
  unit: string;                 // e.g. "m", "m/s", "%", ""
}

// ── Nav State → UAV Icon Color ──────────────────────────────────────
// INAV MW_NAV_STATE values from navigation.h — used for UAV icon coloring.

const DEFAULT_UAV_COLOR = '#37a8db';    // INAV blue

const NAV_STATE_COLORS: Record<number, string> = {
  // 0: NONE — idle / no nav
  0:  DEFAULT_UAV_COLOR,
  // 1: RTH_START
  1:  '#9b59b6',
  // 2: RTH_ENROUTE
  2:  '#9b59b6',
  // 3: HOLD_INFINIT (PosHold)
  3:  '#00bcd4',
  // 4: HOLD_TIMED (PosHold)
  4:  '#00bcd4',
  // 5: WP_ENROUTE (Mission)
  5:  '#37a8db',
  // 6: PROCESS_NEXT (Mission)
  6:  '#37a8db',
  // 7: DO_JUMP (Mission)
  7:  '#37a8db',
  // 8: LAND_START
  8:  '#ff8c00',
  // 9: LAND_IN_PROGRESS
  9:  '#ff8c00',
  // 10: LANDED
  10: '#59aa29',
  // 11: LAND_SETTLE
  11: '#ff8c00',
  // 12: LAND_START_DESCENT
  12: '#ff8c00',
  // 13: HOVER_ABOVE_HOME (RTH loiter)
  13: '#9b59b6',
  // 14: EMERGENCY_LANDING
  14: '#e60000',
  // 15: RTH_CLIMB
  15: '#9b59b6',
};

/** Get UAV icon fill color based on INAV nav_state. */
export function getNavStateColor(navState: number): string {
  return NAV_STATE_COLORS[navState] ?? DEFAULT_UAV_COLOR;
}

// ── Gradient Color ──────────────────────────────────────────────────
// Maps a value in [min, max] to a color on a blue→green→yellow→red gradient.
// Uses HSL hue rotation: 240° (blue) → 120° (green) → 60° (yellow) → 0° (red).

const GRADIENT_STEPS = 20;

export function getGradientColor(value: number, min: number, max: number): string {
  if (max <= min) return 'hsl(240, 80%, 55%)'; // blue fallback
  const t = Math.max(0, Math.min(1, (value - min) / (max - min)));
  // Hue: 240 (blue) at t=0 → 0 (red) at t=1
  const hue = 240 * (1 - t);
  return `hsl(${Math.round(hue)}, 80%, 50%)`;
}

/** Inverted gradient for signal quality: green(high) → red(low). */
export function getSignalGradientColor(value: number, min: number, max: number): string {
  if (max <= min) return 'hsl(120, 80%, 45%)'; // green fallback
  const t = Math.max(0, Math.min(1, (value - min) / (max - min)));
  // Hue: 0 (red) at t=0 → 120 (green) at t=1
  const hue = 120 * t;
  return `hsl(${Math.round(hue)}, 80%, 45%)`;
}

/** Quantize a value to one of N steps for segment merging. */
function quantize(value: number, min: number, max: number, steps: number): number {
  if (max <= min) return 0;
  const t = Math.max(0, Math.min(1, (value - min) / (max - min)));
  return Math.min(Math.floor(t * steps), steps - 1);
}

// ── Track Segmentation ──────────────────────────────────────────────

/** Canonical-mode info for a record (its primary mode id → label + colour). */
function modeInfoFor(r: TelemetryRecord): FlightModeInfo {
  const id = r.mode_primary ?? '';
  return { id, label: modeLabel(id), color: modeColor(id) };
}

/** Segment a track by flight mode. Consecutive points with same mode = one segment. */
export function segmentTrackByFlightMode(track: TelemetryRecord[]): TrackSegment[] {
  const segments: TrackSegment[] = [];
  let current: TrackSegment | null = null;

  for (const r of track) {
    const lat = r.lat;
    const lon = r.lon;
    if (lat == null || lon == null) continue;

    const mode = modeInfoFor(r);

    if (!current || current.modeInfo!.id !== mode.id) {
      // Bridge: duplicate last point of previous segment as first of new
      if (current && current.points.length > 0) {
        const lastPt: [number, number] = current.points[current.points.length - 1];
        current = { points: [lastPt], color: mode.color, modeInfo: mode };
      } else {
        current = { points: [], color: mode.color, modeInfo: mode };
      }
      segments.push(current);
    }
    current.points.push([lat, lon]);
  }

  return segments;
}

/** Segment a track by a numeric value using gradient coloring. */
export function segmentTrackByGradient(
  track: TelemetryRecord[],
  getValue: (r: TelemetryRecord) => number | null,
  min: number,
  max: number,
  inverted = false,
): TrackSegment[] {
  const segments: TrackSegment[] = [];
  let current: TrackSegment | null = null;
  let currentStep = -1;

  for (const r of track) {
    const lat = r.lat;
    const lon = r.lon;
    if (lat == null || lon == null) continue;

    const raw = getValue(r);
    const val = raw ?? min;
    const step = quantize(val, min, max, GRADIENT_STEPS);
    const color = inverted
      ? getSignalGradientColor(val, min, max)
      : getGradientColor(val, min, max);

    if (!current || step !== currentStep) {
      if (current && current.points.length > 0) {
        const lastPt: [number, number] = current.points[current.points.length - 1];
        current = { points: [lastPt], color };
      } else {
        current = { points: [], color };
      }
      segments.push(current);
      currentStep = step;
    }
    current.points.push([lat, lon]);
  }

  return segments;
}

/** Collect the set of unique flight modes actually used in a track. */
export function getUsedFlightModes(track: TelemetryRecord[]): FlightModeInfo[] {
  const seen = new Set<string>();
  const result: FlightModeInfo[] = [];
  for (const r of track) {
    const mode = modeInfoFor(r);
    if (!seen.has(mode.id)) {
      seen.add(mode.id);
      result.push(mode);
    }
  }
  return result;
}

// ── Gradient Segmentation Helpers ───────────────────────────────────

/** Compute min/max of a numeric field across a track. Returns [min, max]. */
function fieldRange(
  track: TelemetryRecord[],
  getValue: (r: TelemetryRecord) => number | null,
): [number, number] {
  let lo = Infinity;
  let hi = -Infinity;
  for (const r of track) {
    const v = getValue(r);
    if (v != null) {
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
  }
  return lo <= hi ? [lo, hi] : [0, 0];
}

/**
 * Segment by altitude gradient (blue=low → red=high).
 * Uses baro_alt_m (relative altitude) with fallback to alt_m (GPS MSL).
 * If warnAltitudeM > 0, it is used as the max reference;
 * otherwise the track's maximum altitude is used.
 */
export function segmentTrackByAltitude(
  track: TelemetryRecord[],
  warnAltitudeM: number,
): GradientResult {
  const getValue = (r: TelemetryRecord) => r.baro_alt_m ?? r.alt_m;
  const [, trackMax] = fieldRange(track, getValue);
  const max = warnAltitudeM > 0 ? warnAltitudeM : trackMax;
  return {
    segments: segmentTrackByGradient(track, getValue, 0, max),
    min: 0,
    max,
    fieldLabel: 'Altitude',
    unit: 'm',
  };
}

/**
 * Segment by ground speed gradient (blue=slow → red=fast).
 * Range is always 0 … track-max.
 */
export function segmentTrackBySpeed(track: TelemetryRecord[]): GradientResult {
  const getValue = (r: TelemetryRecord) => r.speed_ms;
  const [, trackMax] = fieldRange(track, getValue);
  return {
    segments: segmentTrackByGradient(track, getValue, 0, trackMax),
    min: 0,
    max: trackMax,
    fieldLabel: 'Speed',
    unit: 'm/s',
  };
}

/**
 * Build a per-point color function for a given color mode, calibrated against
 * the whole track (so gradient ranges match the segmented line). Used by the 3D
 * map's progressive curtain/shadow, which colors points incrementally rather
 * than re-segmenting on every frame.
 */
export function trackPointColorizer(
  track: TelemetryRecord[],
  colorMode: TrackColorMode,
  warnAltitudeM = 0,
): (r: TelemetryRecord) => string {
  if (colorMode === 'flightmode') {
    return (r) => modeColor(r.mode_primary ?? '');
  }
  if (colorMode === 'altitude') {
    const getV = (r: TelemetryRecord) => r.baro_alt_m ?? r.alt_m;
    const [, trackMax] = fieldRange(track, getV);
    const max = warnAltitudeM > 0 ? warnAltitudeM : trackMax;
    return (r) => getGradientColor(getV(r) ?? 0, 0, max);
  }
  if (colorMode === 'speed') {
    const getV = (r: TelemetryRecord) => r.speed_ms;
    const [, trackMax] = fieldRange(track, getV);
    return (r) => getGradientColor(getV(r) ?? 0, 0, trackMax);
  }
  if (colorMode === 'signal') {
    const hasLQ = track.some((r) => r.link_quality != null && r.link_quality > 0);
    const getV: (r: TelemetryRecord) => number | null = hasLQ ? (r) => r.link_quality : (r) => r.rssi;
    const [lo, hi] = fieldRange(track, getV);
    return (r) => getSignalGradientColor(getV(r) ?? lo, lo, hi);
  }
  return () => '#f5a623';
}

/**
 * Segment by signal quality gradient (red=low → green=high).
 * Prefers link_quality (0-300 scale); falls back to rssi (0-1023 raw).
 */
export function segmentTrackBySignal(track: TelemetryRecord[]): GradientResult {
  // Determine which field is available
  const hasLQ = track.some((r) => r.link_quality != null && r.link_quality > 0);
  const getValue: (r: TelemetryRecord) => number | null = hasLQ
    ? (r) => r.link_quality
    : (r) => r.rssi;
  const [lo, hi] = fieldRange(track, getValue);
  return {
    segments: segmentTrackByGradient(track, getValue, lo, hi, true),
    min: lo,
    max: hi,
    fieldLabel: hasLQ ? 'LQ' : 'RSSI',
    unit: hasLQ ? '%' : '',
  };
}
