// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import type { PanelConfig } from '$lib/stores/settings';
import { WIDGET_MAP, effectiveWidgetSize, nextWidgetSize } from '$lib/config/widgetRegistry';

/** Reorder widgets within a single panel. Returns the new config. */
export function reorderPanel(
  panels: PanelConfig,
  panelId: string,
  newIds: string[],
): PanelConfig {
  return { ...panels, [panelId]: newIds };
}

/** Move a widget from one panel to another at a specific index. Returns the new config. */
export function receiveWidget(
  panels: PanelConfig,
  targetPanel: string,
  widgetId: string,
  index: number,
): PanelConfig {
  const newPanels = { ...panels };
  for (const key of ['bottom', 'right'] as const) {
    newPanels[key] = newPanels[key].filter((id) => id !== widgetId);
  }
  const targetList = [...newPanels[targetPanel as 'bottom' | 'right']];
  targetList.splice(index, 0, widgetId);
  newPanels[targetPanel as 'bottom' | 'right'] = targetList;
  newPanels.positions = {
    ...newPanels.positions,
    [widgetId]: targetPanel as 'bottom' | 'right',
  };
  return newPanels;
}

/** Toggle a widget on/off. Returns the new config. */
export function toggleWidgetVisibility(
  panels: PanelConfig,
  widgetId: string,
): PanelConfig {
  const allAssigned = [...panels.bottom, ...panels.right];
  if (allAssigned.includes(widgetId)) {
    const currentPanel = panels.bottom.includes(widgetId) ? 'bottom' : 'right';
    return {
      ...panels,
      bottom: panels.bottom.filter((id) => id !== widgetId),
      right: panels.right.filter((id) => id !== widgetId),
      positions: {
        ...panels.positions,
        [widgetId]: currentPanel as 'bottom' | 'right',
      },
    };
  }
  const target = panels.positions?.[widgetId] ?? 'bottom';
  return { ...panels, [target]: [...panels[target], widgetId] };
}

/** Step a widget to its next size state (edit-mode resize button). Returns the new config. */
export function cycleWidgetSize(
  panels: PanelConfig,
  widgetId: string,
): PanelConfig {
  const def = WIDGET_MAP.get(widgetId);
  if (!def) return panels;
  const next = nextWidgetSize(def.shape, effectiveWidgetSize(widgetId, panels.sizes));
  return { ...panels, sizes: { ...panels.sizes, [widgetId]: next } };
}

/** Drop every id the registry no longer knows (a removed widget, a platform-filtered one) from a
 *  stored layout — an unknown id would otherwise render as an empty slot. Returns the same object
 *  when nothing had to change. */
export function sanitizePanels(panels: PanelConfig): PanelConfig {
  const known = (id: string) => WIDGET_MAP.has(id);
  const bottom = panels.bottom.filter(known);
  const right = panels.right.filter(known);
  const pickKnown = <T>(rec: Record<string, T> | undefined): Record<string, T> | undefined =>
    rec && Object.fromEntries(Object.entries(rec).filter(([id]) => known(id)));
  const positions = pickKnown(panels.positions);
  const sizes = pickKnown(panels.sizes);
  const unchanged =
    bottom.length === panels.bottom.length &&
    right.length === panels.right.length &&
    Object.keys(positions ?? {}).length === Object.keys(panels.positions ?? {}).length &&
    Object.keys(sizes ?? {}).length === Object.keys(panels.sizes ?? {}).length;
  if (unchanged) return panels;
  console.log('[widgets] layout sanitised — dropped unknown widget ids');
  return { ...panels, bottom, right, positions, sizes };
}

/** Check whether a widget is currently assigned to any panel. */
export function isWidgetActive(
  panels: PanelConfig,
  widgetId: string,
): boolean {
  return panels.bottom.includes(widgetId) || panels.right.includes(widgetId);
}

/** Get the panel name a widget is on, or null if hidden. */
export function getWidgetPanel(
  panels: PanelConfig,
  widgetId: string,
): 'bottom' | 'right' | null {
  if (panels.bottom.includes(widgetId)) return 'bottom';
  if (panels.right.includes(widgetId)) return 'right';
  return null;
}
