// Batch-edit popup state. Opened from the waypoint context menu (>1 selected),
// rendered by BatchEditPopup at (x, y). The popup itself reads the selection
// (selectedWpIndices) + mission store; this just carries open + position.

import { writable } from 'svelte/store';

export const batchEdit = writable<{ x: number; y: number } | null>(null);

export function openBatchEdit(x: number, y: number): void {
  batchEdit.set({ x, y });
}

export function closeBatchEdit(): void {
  batchEdit.set(null);
}
