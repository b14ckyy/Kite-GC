// Shared waypoint context-menu items, used by both the mission list (panel) and
// the map markers (layer). Operates on the current selection (single or multi),
// so callers must ensure the right-clicked WP is selected before opening.
// Only genuinely-new actions live here — the rest of waypoint editing is already
// reachable in the panel UI.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { activeMissionIndex, missionCount, selectedWpIndices, moveSelectedWpsToMission } from '$lib/stores/mission';
import { contextMenu, type ContextMenuItem } from '$lib/stores/contextMenu';
import { openBatchEdit } from '$lib/stores/batchEdit';

export function buildWaypointMenu(): ContextMenuItem[] {
  const tr = get(t) as (key: string) => string;
  const active = get(activeMissionIndex);
  const count = get(missionCount);
  const selCount = get(selectedWpIndices).size;

  // "Move to mission" → submenu of the other mission slots (INAV multi-mission)
  const others: ContextMenuItem[] = [];
  for (let n = 1; n <= count; n++) {
    if (n === active) continue;
    others.push({ label: `${tr('mission.mission')} ${n}`, action: () => void moveSelectedWpsToMission(n) });
  }

  const moveLabel = selCount > 1 ? `${tr('mission.moveToMission')} (${selCount})` : tr('mission.moveToMission');
  const items: ContextMenuItem[] = [
    {
      label: moveLabel,
      icon: '⇄',
      disabled: others.length === 0,
      submenu: others.length ? others : undefined,
    },
  ];

  // Batch edit only makes sense for a multi-selection (single WP uses the normal editor).
  if (selCount > 1) {
    items.push({
      label: `${tr('mission.batchEdit')} (${selCount})`,
      icon: '✎',
      action: () => {
        const cm = get(contextMenu); // open the popup where the menu was
        openBatchEdit(cm.x, cm.y);
      },
    });
  }

  return items;
}
