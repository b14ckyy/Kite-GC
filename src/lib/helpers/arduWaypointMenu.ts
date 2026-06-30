// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared ArduPilot/PX4 waypoint context-menu items, used by both the mission list (panel) and the map
// markers (layer). The cross-autopilot counterpart of waypointMenu.ts (INAV). Operates on the current
// selection, so callers must ensure the right-clicked WP is selected before opening. ArduPilot/PX4 is
// single-mission (no "move to mission"), so the only entry is Batch Edit for a multi-selection.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { arduSelectedWpIndices } from '$lib/stores/missionArdupilot';
import { contextMenu, type ContextMenuItem } from '$lib/stores/contextMenu';
import { openArduBatchEdit } from '$lib/stores/arduBatchEdit';

export function buildArduWaypointMenu(): ContextMenuItem[] {
  const tr = get(t) as (key: string) => string;
  const selCount = get(arduSelectedWpIndices).size;

  const items: ContextMenuItem[] = [];
  // Batch edit only makes sense for a multi-selection (a single WP uses the normal editor popup).
  if (selCount > 1) {
    items.push({
      label: `${tr('mission.batchEdit')} (${selCount})`,
      icon: '✎',
      action: () => {
        const cm = get(contextMenu); // open the popup where the menu was
        openArduBatchEdit(cm.x, cm.y);
      },
    });
  }

  return items;
}
