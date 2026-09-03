// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Widget registry — defines all available widgets, their shape family, default size and metadata

import { isPhone } from '$lib/platform';

/** Shape family: `square` tiles keep 1:1, `wide` ones are 2:1 in their wide state. */
export type WidgetShape = 'square' | 'wide';

/** Per-instance size state (Dev-Docs active/WIDGET_OVERHAUL.md). The aspect ratio never changes —
 *  a widget is only rescaled by height. `W` (2:1) exists for the wide family only. */
export type WidgetSize = 'S' | 'L' | 'W';

export interface WidgetDef {
  id: string;
  label: string;
  /** i18n key for the label — use with $t() in .svelte files */
  labelKey: string;
  shape: WidgetShape;
  /** Size a fresh layout starts with (the widget's designed size). */
  defaultSize: WidgetSize;
}

const ALL_WIDGET_DEFS: WidgetDef[] = [
  { id: 'ahi',          label: 'AHI',            labelKey: 'widgets.ahi',          shape: 'square', defaultSize: 'L' },
  { id: 'speed',        label: 'Speed',          labelKey: 'widgets.speed',        shape: 'square', defaultSize: 'S' },
  { id: 'altitude',     label: 'Altitude',       labelKey: 'widgets.altitude',     shape: 'square', defaultSize: 'S' },
  { id: 'battery',      label: 'Battery',        labelKey: 'widgets.battery',      shape: 'square', defaultSize: 'S' },
  { id: 'battery2',     label: 'Battery 2',      labelKey: 'widgets.battery2',     shape: 'square', defaultSize: 'S' },
  { id: 'gps',          label: 'GPS',            labelKey: 'widgets.gps',          shape: 'square', defaultSize: 'S' },
  { id: 'rcLink',       label: 'RC Link',        labelKey: 'widgets.rcLink',       shape: 'square', defaultSize: 'S' },
  { id: 'compass',      label: 'Compass',        labelKey: 'widgets.compass',      shape: 'square', defaultSize: 'L' },
  { id: 'home',         label: 'Home',           labelKey: 'widgets.home',         shape: 'square', defaultSize: 'S' },
  { id: 'flightMode',   label: 'Flight Mode',    labelKey: 'widgets.flightMode',   shape: 'square', defaultSize: 'S' },
  { id: 'liveAgl',      label: 'Live AGL',       labelKey: 'widgets.liveAgl',      shape: 'wide',   defaultSize: 'W' },
  { id: 'terrainRadar', label: 'Terrain Radar',  labelKey: 'widgets.terrainRadar', shape: 'square', defaultSize: 'L' },
  { id: 'videoFeed',    label: 'Video',          labelKey: 'widgets.video',        shape: 'wide',   defaultSize: 'W' },
];

// Video stays on tablets (camera / OTG capture works natively there; RTSP is the Phase E item) and is
// dropped from the phone catalog until the phone UI decides how to fit it.
export const WIDGET_DEFS: WidgetDef[] = ALL_WIDGET_DEFS.filter(w => w.id !== 'videoFeed' || !isPhone);

export const WIDGET_MAP = new Map(WIDGET_DEFS.map(w => [w.id, w]));

/** The size states a shape cycles through, in tap order (the resize button steps to the next one and
 *  wraps): squares toggle S↔L, wide tiles go W → L → S → W. */
export function sizeStates(shape: WidgetShape): WidgetSize[] {
  return shape === 'wide' ? ['W', 'L', 'S'] : ['S', 'L'];
}

/** The state after `current` for `shape`; an unknown/foreign state restarts the cycle. */
export function nextWidgetSize(shape: WidgetShape, current: WidgetSize): WidgetSize {
  const states = sizeStates(shape);
  const idx = states.indexOf(current);
  return states[(idx + 1) % states.length];
}

/** Effective size of a widget: the stored state when it is valid for the widget's shape, else its
 *  default (a stale `W` on a square, an unknown id → `S`). */
export function effectiveWidgetSize(id: string, sizes: Record<string, WidgetSize> | undefined): WidgetSize {
  const def = WIDGET_MAP.get(id);
  if (!def) return 'S';
  const stored = sizes?.[id];
  return stored && sizeStates(def.shape).includes(stored) ? stored : def.defaultSize;
}

/** Large widget base size in vmin */
export const LARGE_BASE_VMIN = 22.5;
/** Small widget = 60% of large, always square */
export const SMALL_BASE_VMIN = LARGE_BASE_VMIN * 0.6; // 13.5
/** Minimum scale factor before panel is considered full */
export const MIN_SCALE = 0.5;
