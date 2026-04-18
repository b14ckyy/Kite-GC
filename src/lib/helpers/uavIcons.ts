// UAV icon factory — provides Leaflet DivIcons for different platform types.
// Used by both live telemetry and flight log playback.

import L from 'leaflet';

// ── INAV Platform Types (from mixerConfig.platformType) ─────────────

export const PLATFORM_MULTIROTOR = 0;
export const PLATFORM_AIRPLANE   = 1;
export const PLATFORM_HELICOPTER = 2;
export const PLATFORM_TRICOPTER  = 3;
export const PLATFORM_BOAT       = 4;
export const PLATFORM_OTHER      = 5;

export type PlatformType = number;

// ── SVG shape definitions ───────────────────────────────────────────
// Each shape is a viewBox-24 SVG path string.  Stroke is applied externally.

interface UavShape {
  /** SVG path d-attribute, drawn inside a 24×24 viewBox */
  path: string;
  /** Icon pixel size (width=height) */
  size: number;
}

const SHAPE_MULTIROTOR: UavShape = {
  // Arrow / quad silhouette — current default
  path: 'M12 2 L5 20 L12 16 L19 20 Z',
  size: 28,
};

const SHAPE_AIRPLANE: UavShape = {
  // Fixed-wing top-down silhouette
  path: 'M12 2 L11 8 L3 13 L3 14.5 L11 12 L11 19 L8 21 L8 22.5 L12 21 L16 22.5 L16 21 L13 19 L13 12 L21 14.5 L21 13 L13 8 Z',
  size: 32,
};

const SHAPE_HELICOPTER: UavShape = {
  // Simple helicopter top-down — body + tail boom
  path: 'M12 3 L10 7 L6 8 L4 10 L6 11 L10 10 L11 18 L9 21 L10 22 L12 20 L14 22 L15 21 L13 18 L14 10 L18 11 L20 10 L18 8 L14 7 Z',
  size: 30,
};

// Fallback for unknown types (same as multirotor)
const SHAPE_DEFAULT: UavShape = SHAPE_MULTIROTOR;

function shapeForPlatform(platformType: PlatformType): UavShape {
  switch (platformType) {
    case PLATFORM_AIRPLANE:   return SHAPE_AIRPLANE;
    case PLATFORM_HELICOPTER: return SHAPE_HELICOPTER;
    case PLATFORM_TRICOPTER:  return SHAPE_MULTIROTOR; // same arrow for now
    case PLATFORM_MULTIROTOR: return SHAPE_MULTIROTOR;
    default:                  return SHAPE_DEFAULT;
  }
}

// ── Default colors ──────────────────────────────────────────────────

export const DEFAULT_UAV_COLOR = '#37a8db';
const STROKE_COLOR = '#1a1a1a';
const STROKE_WIDTH = 1.5;

// ── Icon factory ────────────────────────────────────────────────────

export interface UavIconOptions {
  heading?: number;
  fillColor?: string;
  platformType?: PlatformType;
}

/**
 * Create a Leaflet DivIcon for a UAV marker.
 *
 * @param opts.heading     Compass heading in degrees (0 = north).
 * @param opts.fillColor   Fill color (default: INAV blue).
 * @param opts.platformType  INAV platform type enum value.
 */
export function createUavIcon(opts: UavIconOptions = {}): L.DivIcon {
  const {
    heading = 0,
    fillColor = DEFAULT_UAV_COLOR,
    platformType = PLATFORM_MULTIROTOR,
  } = opts;

  const shape = shapeForPlatform(platformType);
  const half = shape.size / 2;

  return L.divIcon({
    className: 'uav-icon',
    html: `<div style="transform:rotate(${heading}deg);width:${shape.size}px;height:${shape.size}px;">` +
      `<svg viewBox="0 0 24 24" width="${shape.size}" height="${shape.size}">` +
      `<path d="${shape.path}" fill="${fillColor}" stroke="${STROKE_COLOR}" stroke-width="${STROKE_WIDTH}" stroke-linejoin="round"/>` +
      `</svg></div>`,
    iconSize: [shape.size, shape.size],
    iconAnchor: [half, half],
  });
}
