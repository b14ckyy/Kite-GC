// This is a Svelte 5 rune module (.svelte.ts)
// Top-level runes like $state are allowed here.

import type { LngLat, AltMode } from '$lib/helpers/surveyPatterns';
export type { LngLat, AltMode };

export type SurveyShape = 'rectangle' | 'rectangle-lawnmower' | 'polygon' | 'polygon-lawnmower' | 'circle' | 'spiral';

export interface BasePatternParams {
  shapeOrientation: number;     // degrees, 0 = north, clockwise — rotates the shape
  baseAltitude: number;         // meters
  baseSpeed: number;            // m/s
  targetLineSpacing: number;    // meters between legs (renamed from lineSpacing)
  actualLineSpacing: number;    // computed by algorithm, read-only feedback
  turnDistance: number;         // meters, extension beyond shape (default 0)
  reverse: boolean;             // reverse path direction
  clockwise: boolean;           // true = CW, false = CCW (lawnmower patterns only)
  startCorner: number;          // 1-based corner index where the path starts (lawnmower patterns only)
  trackOrientationEnabled: boolean; // use different orientation for tracks vs shape
  trackOrientation: number;     // degrees, only used when trackOrientationEnabled
  altMode: AltMode;             // 'relative' | 'amsl' | 'ground'
  userActionLineStartFlags: number; // bitmask: bits 0-3 = UA trigger 1-4 on line start (zigzag)
  userActionLineEndFlags: number;   // bitmask: bits 0-3 = UA trigger 1-4 on line end (zigzag)
  userActionStartFlags: number;     // bitmask for the very first waypoint (lawnmower)
  userActionTrackFlags: number;     // bitmask for all interior waypoints (lawnmower)
  userActionEndFlags: number;       // bitmask for the very last waypoint (lawnmower)
}

export interface RectanglePatternParams extends BasePatternParams {
  center: LngLat;
  length: number;             // meters (along orientation)
  width: number;              // meters (perpendicular to orientation)
}

export interface CirclePatternParams extends BasePatternParams {
  center: LngLat;
  radius: number;             // meters
  ringPoints: number;         // max waypoints per ring (auto-reduced for small inner rings)
}

export interface PolygonPatternParams extends BasePatternParams {
  points: LngLat[];           // in order
}

export type SurveyPatternConfig =
  | { shape: 'rectangle'; params: RectanglePatternParams }
  | { shape: 'rectangle-lawnmower'; params: RectanglePatternParams }
  | { shape: 'polygon'; params: PolygonPatternParams }
  | { shape: 'polygon-lawnmower'; params: PolygonPatternParams }
  | { shape: 'circle'; params: CirclePatternParams }
  | { shape: 'spiral'; params: CirclePatternParams };

// Full saved pattern (for future persistence / reuse)
export interface SavedSurveyPattern {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  version: number;
  config: SurveyPatternConfig;
}

// Preserved params per shape family — survives shape-family switches within a session.
// Keyed by family: 'rect' (rectangle / rectangle-lawnmower) | 'circle' (circle / spiral)
const _paramsCache: Partial<Record<'rect' | 'circle', any>> = {};

// Current active pattern being edited (null when not in pattern mode)
export const activeSurveyPattern = $state<{
  config: SurveyPatternConfig | null;
  isActive: boolean;
}>({
  config: null,
  isActive: false,
});

export function enterPatternMode(initialShape: SurveyShape = 'rectangle', initialCenter?: LngLat) {
  // Reuse existing config if available (preserves params from previous session)
  if (activeSurveyPattern.config) {
    activeSurveyPattern.isActive = true;
    return;
  }
  const center = initialCenter ?? { lat: 48.0, lng: 11.0 };

  activeSurveyPattern.config = {
    shape: initialShape,
    params: {
      center,
      length: 400,
      width: 200,
      shapeOrientation: 90,
      baseAltitude: 50,
      baseSpeed: 15,
      targetLineSpacing: 50,
      actualLineSpacing: 50,
      turnDistance: 0,
      reverse: false,
      clockwise: true,
      startCorner: 1,
      trackOrientationEnabled: false,
      trackOrientation: 0,
      altMode: 'relative' as AltMode,
      userActionLineStartFlags: 0,
      userActionLineEndFlags: 0,
      userActionStartFlags: 0,
      userActionTrackFlags: 0,
      userActionEndFlags: 0,
    } as RectanglePatternParams,
  } as any;

  activeSurveyPattern.isActive = true;
}

/**
 * Switch to a different shape at runtime.
 * - rectangle ↔ rectangle-lawnmower: preserves all params, only the shape name changes
 * - any other transition: resets to shape-appropriate defaults, preserves center + common base params
 */
export function switchShape(newShape: SurveyShape) {
  const current = activeSurveyPattern.config;
  if (!current) return;

  const isRect   = (s: SurveyShape) => s === 'rectangle' || s === 'rectangle-lawnmower';
  const isCircle = (s: SurveyShape) => s === 'circle'    || s === 'spiral';
  const familyOf = (s: SurveyShape): 'rect' | 'circle' | null =>
    isRect(s) ? 'rect' : isCircle(s) ? 'circle' : null;

  const currentFamily = familyOf(current.shape);
  const newFamily     = familyOf(newShape);

  if (currentFamily === newFamily && currentFamily !== null) {
    // Same family (rect↔rect-lawnmower or circle↔spiral) — just rename
    activeSurveyPattern.config = { ...current, shape: newShape } as any;
    return;
  }

  // Save current params to cache before switching family
  if (currentFamily) _paramsCache[currentFamily] = current.params;

  const base = current.params as any;
  const center: LngLat = 'center' in base ? base.center : { lat: 48.0, lng: 11.0 };

  if (isRect(newShape)) {
    // Restore cached rect params if available, else build defaults
    const cached = _paramsCache['rect'];
    activeSurveyPattern.config = {
      shape: newShape,
      params: cached
        ? { ...cached, center }
        : {
            center, length: 400, width: 200, shapeOrientation: 90,
            baseAltitude: 50, baseSpeed: 15, targetLineSpacing: 50, actualLineSpacing: 50,
            turnDistance: 0, reverse: false, clockwise: true, startCorner: 1,
            trackOrientationEnabled: false, trackOrientation: 0, altMode: 'relative' as AltMode,
            userActionLineStartFlags: 0, userActionLineEndFlags: 0,
            userActionStartFlags: 0, userActionTrackFlags: 0, userActionEndFlags: 0,
          } as RectanglePatternParams,
    } as any;
  } else if (isCircle(newShape)) {
    // Restore cached circle params if available, else build defaults
    const cached = _paramsCache['circle'];
    activeSurveyPattern.config = {
      shape: newShape,
      params: cached
        ? { ...cached, center }
        : {
            center, radius: 200, ringPoints: 10, shapeOrientation: 0, trackOrientation: 0,
            baseAltitude: 50, baseSpeed: 15, targetLineSpacing: 50, actualLineSpacing: 50,
            turnDistance: 0, reverse: false, clockwise: true, startCorner: 1,
            trackOrientationEnabled: false, altMode: 'relative' as AltMode,
            userActionLineStartFlags: 0, userActionLineEndFlags: 0,
            userActionStartFlags: 0, userActionTrackFlags: 0, userActionEndFlags: 0,
          } as CirclePatternParams,
    } as any;
  } else {
    // polygon, polygon-lawnmower, … — minimal params for placeholder rendering
    activeSurveyPattern.config = {
      shape: newShape,
      params: { ...base, center } as any,
    } as any;
  }
}

export function exitPatternMode() {
  activeSurveyPattern.isActive = false;
  // Keep config alive so params persist when re-entering pattern mode
}

export function updateRectangleParams(updates: Partial<RectanglePatternParams>) {
  if (!activeSurveyPattern.config || !['rectangle', 'rectangle-lawnmower'].includes(activeSurveyPattern.config.shape)) return;

  const current = activeSurveyPattern.config.params as RectanglePatternParams;
  // New object assignment to trigger reactivity chain
  activeSurveyPattern.config = {
    ...activeSurveyPattern.config,
    params: { ...current, ...updates },
  } as any;
}

export function updateCircleParams(updates: Partial<CirclePatternParams>) {
  if (!activeSurveyPattern.config || !['circle', 'spiral'].includes(activeSurveyPattern.config.shape)) return;

  const current = activeSurveyPattern.config.params as CirclePatternParams;
  activeSurveyPattern.config = {
    ...activeSurveyPattern.config,
    params: { ...current, ...updates },
  } as any;
}

/**
 * Called from the map layer when the user drags the circle center or radius handle.
 * Only affects circle / spiral shapes.
 */
export function applyCircleDragUpdate(update: Partial<CirclePatternParams>) {
  if (!activeSurveyPattern.config || !['circle', 'spiral'].includes(activeSurveyPattern.config.shape)) return;

  const current = activeSurveyPattern.config.params as CirclePatternParams;
  activeSurveyPattern.config = {
    ...activeSurveyPattern.config,
    params: { ...current, ...update },
  } as any;
}

/**
 * Called from the map layer when the user drags the center or a corner.
 * Only affects rectangle / rectangle-lawnmower shapes.
 */
export function applyRectangleDragUpdate(update: Partial<RectanglePatternParams>) {
  if (!activeSurveyPattern.config || !['rectangle', 'rectangle-lawnmower'].includes(activeSurveyPattern.config.shape)) return;

  const current = activeSurveyPattern.config.params as RectanglePatternParams;
  // New object assignment to trigger reactivity chain
  activeSurveyPattern.config = {
    ...activeSurveyPattern.config,
    params: { ...current, ...update },
  } as any;
}

