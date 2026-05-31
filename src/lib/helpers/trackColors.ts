// Track color helpers — flight mode classification and gradient coloring

import type { TelemetryRecord } from '$lib/stores/flightlog';

// ── INAV Flight Mode Flags (from runtime_config.h) ─────────────────

export const FLIGHT_MODE = {
  ANGLE:          1 << 0,
  HORIZON:        1 << 1,
  HEADING:        1 << 2,
  NAV_ALTHOLD:    1 << 3,
  NAV_RTH:        1 << 4,
  NAV_POSHOLD:    1 << 5,
  HEADFREE:       1 << 6,
  NAV_LAUNCH:     1 << 7,
  MANUAL:         1 << 8,
  FAILSAFE:       1 << 9,
  AUTO_TUNE:      1 << 10,
  NAV_WP:         1 << 11,
  NAV_COURSE_HOLD: 1 << 12,
  FLAPERON:       1 << 13,
  TURN_ASSISTANT: 1 << 14,
  TURTLE:         1 << 15,
  SOARING:        1 << 16,
  ANGLEHOLD:      1 << 17,
  NAV_FW_AUTOLAND: 1 << 18,
} as const;

// ── Types ───────────────────────────────────────────────────────────

export type TrackColorMode = 'flightmode' | 'altitude' | 'speed' | 'signal' | 'none';

export interface FlightModeInfo {
  id: string;
  label: string;
  color: string;
  i18nKey: string;
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

// ── Flight Mode Classification ──────────────────────────────────────
// Priority order: highest matching wins. Tests from most specific to least.

const MODES: { test: (f: number) => boolean; info: FlightModeInfo }[] = [
  {
    test: (f) => !!(f & FLIGHT_MODE.FAILSAFE) && !!(f & FLIGHT_MODE.NAV_RTH),
    info: { id: 'failsafe_rth', label: 'Failsafe RTH', color: '#e60000', i18nKey: 'flightmode.failsafe_rth' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.FAILSAFE),
    info: { id: 'failsafe', label: 'Failsafe', color: '#e60000', i18nKey: 'flightmode.failsafe' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.NAV_WP),
    info: { id: 'mission', label: 'Mission', color: '#37a8db', i18nKey: 'flightmode.mission' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.NAV_RTH),
    info: { id: 'rth', label: 'RTH', color: '#9b59b6', i18nKey: 'flightmode.rth' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.NAV_LAUNCH),
    info: { id: 'launch', label: 'Launch', color: '#e91e9c', i18nKey: 'flightmode.launch' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.NAV_POSHOLD),
    info: { id: 'poshold', label: 'PosHold', color: '#00bcd4', i18nKey: 'flightmode.poshold' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.NAV_COURSE_HOLD),
    info: { id: 'cruise', label: 'Cruise', color: '#ff8c00', i18nKey: 'flightmode.cruise' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.ANGLE),
    info: { id: 'angle', label: 'Angle', color: '#59aa29', i18nKey: 'flightmode.angle' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.HORIZON),
    info: { id: 'horizon', label: 'Horizon', color: '#59aa29', i18nKey: 'flightmode.horizon' },
  },
  {
    test: (f) => !!(f & FLIGHT_MODE.MANUAL),
    info: { id: 'manual', label: 'Manual', color: '#808080', i18nKey: 'flightmode.manual' },
  },
];

const ACRO_MODE: FlightModeInfo = {
  id: 'acro', label: 'Acro', color: '#c0c0c0', i18nKey: 'flightmode.acro',
};

/** Classify a flight mode bitmask into a named mode with color. */
export function classifyFlightMode(flags: number): FlightModeInfo {
  for (const m of MODES) {
    if (m.test(flags)) return m.info;
  }
  return ACRO_MODE;
}

// ── ArduPilot Flight Mode Tables ────────────────────────────────────
// Mode numbers from ArduPilot mode.h — separate tables for Copter and Plane.

const ARDU_COPTER_MODES: Record<number, FlightModeInfo> = {
  0:  { id: 'stabilize',    label: 'Stabilize',    color: '#59aa29', i18nKey: 'flightmode.ardu.stabilize' },
  1:  { id: 'acro',         label: 'Acro',         color: '#c0c0c0', i18nKey: 'flightmode.ardu.acro' },
  2:  { id: 'althold',      label: 'AltHold',      color: '#e8c820', i18nKey: 'flightmode.ardu.althold' },
  3:  { id: 'auto',         label: 'Auto',         color: '#37a8db', i18nKey: 'flightmode.ardu.auto' },
  4:  { id: 'guided',       label: 'Guided',       color: '#2980b9', i18nKey: 'flightmode.ardu.guided' },
  5:  { id: 'loiter',       label: 'Loiter',       color: '#00bcd4', i18nKey: 'flightmode.ardu.loiter' },
  6:  { id: 'rtl',          label: 'RTL',          color: '#9b59b6', i18nKey: 'flightmode.ardu.rtl' },
  7:  { id: 'circle',       label: 'Circle',       color: '#ff8c00', i18nKey: 'flightmode.ardu.circle' },
  9:  { id: 'land',         label: 'Land',         color: '#e67e22', i18nKey: 'flightmode.ardu.land' },
  11: { id: 'drift',        label: 'Drift',        color: '#808080', i18nKey: 'flightmode.ardu.drift' },
  13: { id: 'sport',        label: 'Sport',        color: '#27ae60', i18nKey: 'flightmode.ardu.sport' },
  14: { id: 'flip',         label: 'Flip',         color: '#e91e9c', i18nKey: 'flightmode.ardu.flip' },
  15: { id: 'autotune',     label: 'AutoTune',     color: '#e8c820', i18nKey: 'flightmode.ardu.autotune' },
  16: { id: 'poshold',      label: 'PosHold',      color: '#00bcd4', i18nKey: 'flightmode.ardu.poshold' },
  17: { id: 'brake',        label: 'Brake',        color: '#e74c3c', i18nKey: 'flightmode.ardu.brake' },
  18: { id: 'throw',        label: 'Throw',        color: '#e91e9c', i18nKey: 'flightmode.ardu.throw' },
  19: { id: 'avoid_adsb',   label: 'Avoid ADSB',   color: '#e60000', i18nKey: 'flightmode.ardu.avoid_adsb' },
  20: { id: 'guided_nogps', label: 'Guided NoGPS', color: '#2980b9', i18nKey: 'flightmode.ardu.guided_nogps' },
  21: { id: 'smartrtl',     label: 'SmartRTL',     color: '#9b59b6', i18nKey: 'flightmode.ardu.smartrtl' },
  22: { id: 'flowhold',     label: 'FlowHold',     color: '#00bcd4', i18nKey: 'flightmode.ardu.flowhold' },
  23: { id: 'follow',       label: 'Follow',       color: '#3498db', i18nKey: 'flightmode.ardu.follow' },
  24: { id: 'zigzag',       label: 'ZigZag',       color: '#f39c12', i18nKey: 'flightmode.ardu.zigzag' },
  25: { id: 'systemid',     label: 'SystemID',     color: '#808080', i18nKey: 'flightmode.ardu.systemid' },
  26: { id: 'autorotate',   label: 'Autorotate',   color: '#e60000', i18nKey: 'flightmode.ardu.autorotate' },
  27: { id: 'autortl',      label: 'AutoRTL',      color: '#9b59b6', i18nKey: 'flightmode.ardu.autortl' },
};

const ARDU_PLANE_MODES: Record<number, FlightModeInfo> = {
  0:  { id: 'manual',       label: 'Manual',       color: '#808080', i18nKey: 'flightmode.ardu.manual' },
  1:  { id: 'circle',       label: 'Circle',       color: '#ff8c00', i18nKey: 'flightmode.ardu.circle' },
  2:  { id: 'stabilize',    label: 'Stabilize',    color: '#59aa29', i18nKey: 'flightmode.ardu.stabilize' },
  3:  { id: 'training',     label: 'Training',     color: '#808080', i18nKey: 'flightmode.ardu.training' },
  4:  { id: 'acro',         label: 'Acro',         color: '#c0c0c0', i18nKey: 'flightmode.ardu.acro' },
  5:  { id: 'fbwa',         label: 'FBW-A',        color: '#59aa29', i18nKey: 'flightmode.ardu.fbwa' },
  6:  { id: 'fbwb',         label: 'FBW-B',        color: '#27ae60', i18nKey: 'flightmode.ardu.fbwb' },
  7:  { id: 'cruise',       label: 'Cruise',       color: '#ff8c00', i18nKey: 'flightmode.ardu.cruise' },
  8:  { id: 'autotune',     label: 'AutoTune',     color: '#e8c820', i18nKey: 'flightmode.ardu.autotune' },
  10: { id: 'auto',         label: 'Auto',         color: '#37a8db', i18nKey: 'flightmode.ardu.auto' },
  11: { id: 'rtl',          label: 'RTL',          color: '#9b59b6', i18nKey: 'flightmode.ardu.rtl' },
  12: { id: 'loiter',       label: 'Loiter',       color: '#00bcd4', i18nKey: 'flightmode.ardu.loiter' },
  13: { id: 'takeoff',      label: 'Takeoff',      color: '#e91e9c', i18nKey: 'flightmode.ardu.takeoff' },
  14: { id: 'avoid_adsb',   label: 'Avoid ADSB',   color: '#e60000', i18nKey: 'flightmode.ardu.avoid_adsb' },
  15: { id: 'guided',       label: 'Guided',       color: '#2980b9', i18nKey: 'flightmode.ardu.guided' },
  17: { id: 'qstabilize',   label: 'QStabilize',   color: '#59aa29', i18nKey: 'flightmode.ardu.qstabilize' },
  18: { id: 'qhover',       label: 'QHover',       color: '#00bcd4', i18nKey: 'flightmode.ardu.qhover' },
  19: { id: 'qloiter',      label: 'QLoiter',      color: '#00bcd4', i18nKey: 'flightmode.ardu.qloiter' },
  20: { id: 'qland',        label: 'QLand',        color: '#e67e22', i18nKey: 'flightmode.ardu.qland' },
  21: { id: 'qrtl',         label: 'QRTL',         color: '#9b59b6', i18nKey: 'flightmode.ardu.qrtl' },
  22: { id: 'qautotune',    label: 'QAutoTune',    color: '#e8c820', i18nKey: 'flightmode.ardu.qautotune' },
  23: { id: 'qacro',        label: 'QAcro',        color: '#c0c0c0', i18nKey: 'flightmode.ardu.qacro' },
  24: { id: 'thermal',      label: 'Thermal',      color: '#f39c12', i18nKey: 'flightmode.ardu.thermal' },
  25: { id: 'loiter_qland', label: 'Loiter QLand', color: '#e67e22', i18nKey: 'flightmode.ardu.loiter_qland' },
};

const ARDU_UNKNOWN_MODE: FlightModeInfo = {
  id: 'unknown', label: 'Unknown', color: '#808080', i18nKey: 'flightmode.ardu.unknown',
};

/** Check if an fc_variant string is ArduPilot. */
export function isArduPilot(fcVariant: string): boolean {
  return /^(Ardu|Copter|Plane|Rover|Sub|Blimp)/i.test(fcVariant);
}

/** Classify an ArduPilot mode number into a named mode with color. */
export function classifyArduPilotMode(modeNumber: number, fcVariant: string): FlightModeInfo {
  const table = /Plane/i.test(fcVariant) ? ARDU_PLANE_MODES : ARDU_COPTER_MODES;
  return table[modeNumber] ?? { ...ARDU_UNKNOWN_MODE, label: `Mode ${modeNumber}` };
}

/** Unified mode classifier — dispatches to INAV or ArduPilot based on fcVariant. */
export function classifyMode(flags: number, fcVariant: string): FlightModeInfo {
  if (isArduPilot(fcVariant)) return classifyArduPilotMode(flags, fcVariant);
  return classifyFlightMode(flags);
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

/** Segment a track by flight mode. Consecutive points with same mode = one segment. */
export function segmentTrackByFlightMode(track: TelemetryRecord[], fcVariant = 'INAV'): TrackSegment[] {
  const segments: TrackSegment[] = [];
  let current: TrackSegment | null = null;

  for (const r of track) {
    const lat = r.lat;
    const lon = r.lon;
    if (lat == null || lon == null) continue;

    const mode = classifyMode(r.active_flight_mode_flags ?? 0, fcVariant);

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
export function getUsedFlightModes(track: TelemetryRecord[], fcVariant = 'INAV'): FlightModeInfo[] {
  const seen = new Set<string>();
  const result: FlightModeInfo[] = [];
  for (const r of track) {
    const mode = classifyMode(r.active_flight_mode_flags ?? 0, fcVariant);
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
  fcVariant = 'INAV',
  warnAltitudeM = 0,
): (r: TelemetryRecord) => string {
  if (colorMode === 'flightmode') {
    return (r) => classifyMode(r.active_flight_mode_flags ?? 0, fcVariant).color;
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
