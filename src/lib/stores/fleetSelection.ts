// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Fleet selection — the set of vehicles a GROUP command targets. Deliberately separate from
// `activeVehicleId` (the one vehicle the widgets / control panel / RC follow): an operator watches one
// craft in detail while commanding several. Ids are the registry keys (`"L{link}:S{sysid}"`); ids that
// leave the registry (link closed, HEARTBEAT timeout) drop out of the selection automatically.
// See docs/02-design/features/fleet-control.design.md §2.1.

import { writable, get } from 'svelte/store';
import { vehicles } from '$lib/stores/vehicles';
import { settings } from '$lib/stores/settings';

export const selectedVehicleIds = writable<ReadonlySet<string>>(new Set());

/** Module-level mirror so hot paths (map marker redraws) don't pay for `get()`. */
let current: ReadonlySet<string> = new Set();
selectedVehicleIds.subscribe((s) => { current = s; });

export function isSelected(id: string): boolean {
  return current.has(id);
}

export function toggleSelected(id: string): void {
  // Behind the fleet feature gate: without it a Ctrl/Cmd-click on the one vehicle selects nothing.
  if (!get(settings).fleetEnabled) return;
  selectedVehicleIds.update((s) => {
    const next = new Set(s);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
}

export function setSelected(ids: Iterable<string>): void {
  selectedVehicleIds.set(new Set(ids));
}

export function clearSelection(): void {
  if (current.size) selectedVehicleIds.set(new Set());
}

// Prune ids the backend no longer knows.
vehicles.subscribe((known) => {
  const s = get(selectedVehicleIds);
  let changed = false;
  const next = new Set<string>();
  for (const id of s) {
    if (known.has(id)) next.add(id); else changed = true;
  }
  if (changed) selectedVehicleIds.set(next);
});
