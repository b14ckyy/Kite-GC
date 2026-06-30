// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Batch-edit popup state for ArduPilot/PX4 missions — the cross-autopilot counterpart of the INAV
// batchEdit store. Opened from the waypoint context menu (>1 selected), rendered by ArduBatchEditPopup
// at (x, y). The popup reads the selection (arduSelectedWpIndices) + arduMission; this just carries
// open + position. Kept separate from the INAV batchEdit store so only one popup reacts at a time
// (a single mission layer / panel is mounted per stack).

import { writable } from 'svelte/store';

export const arduBatchEdit = writable<{ x: number; y: number } | null>(null);

export function openArduBatchEdit(x: number, y: number): void {
  arduBatchEdit.set({ x, y });
}

export function closeArduBatchEdit(): void {
  arduBatchEdit.set(null);
}
