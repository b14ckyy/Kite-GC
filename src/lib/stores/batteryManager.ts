// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Battery Manager view state — a view toggle inside the Flight Logbook panel (like the
// Mission Manager is to the Mission Planner). Lifted into a store so the view + selection +
// grouping survive the logbook close/reopen. See docs/active/BATTERY_MANAGEMENT.md.

import { writable } from 'svelte/store';

/** Grouping mode for the battery list (reuses the logbook's top-right select). */
export type BatteryGroupMode = 'cell-capacity' | 'capacity-cell' | 'flat';

/** Whether the logbook list is showing the Battery Manager (batteries) instead of flights. */
export const batteryManagerOpen = writable<boolean>(false);

/** The pack selected in the Manager (persists across close/reopen). */
export const batteryManagerSelectedId = writable<number | null>(null);

/** One-shot signal: open the Manager straight into the create form with this serial pre-filled.
 *  Set from the End-Flight flow when a flight's battery serial matches no existing pack; the Manager
 *  consumes it (starts a create, fills the serial) and resets it to null. */
export const batteryManagerCreateSerial = writable<string | null>(null);

/** List grouping mode (groups are always ordered large → small). */
export const batteryGroupMode = writable<BatteryGroupMode>('cell-capacity');

/** Leaf-pack sort direction (false = descending). Groups themselves stay large→small. */
export const batteryLeafAsc = writable<boolean>(false);

/** Sort field for the FLAT view (in grouped views the leaves always sort by serial). */
export type BatterySortField = 'cell' | 'capacity' | 'serial';
export const batterySortField = writable<BatterySortField>('serial');

/** Search query for the battery list (serial / label / maker / model / notes / connector). */
export const batterySearchQuery = writable<string>('');
