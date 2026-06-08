// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import type { PanelConfig } from '$lib/stores/settings';

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
