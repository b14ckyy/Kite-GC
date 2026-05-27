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
  trackOrientationEnabled: boolean; // use different orientation for tracks vs shape
  trackOrientation: number;     // degrees, only used when trackOrientationEnabled
  altMode: AltMode;             // 'relative' | 'amsl' | 'ground'
  userActionLineStartFlags: number; // bitmask: bits 0-3 = UA trigger 1-4 on line start
  userActionLineEndFlags: number;   // bitmask: bits 0-3 = UA trigger 1-4 on line end
}

export interface RectanglePatternParams extends BasePatternParams {
  center: LngLat;
  length: number;             // meters (along orientation)
  width: number;              // meters (perpendicular to orientation)
}

export interface CirclePatternParams extends BasePatternParams {
  center: LngLat;
  radius: number;             // meters
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
      trackOrientationEnabled: false,
      trackOrientation: 0,
      altMode: 'relative' as AltMode,
      userActionLineStartFlags: 0,
      userActionLineEndFlags: 0,
    } as RectanglePatternParams,
  } as any;

  activeSurveyPattern.isActive = true;
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
