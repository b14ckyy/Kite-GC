// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The FC's own mode list per vehicle (MAVLink AVAILABLE_MODES, ArduPilot 4.7+). The backend reads it per
// vehicle and emits `available-modes` once a list is complete (again after the FC's mode set changed);
// a vehicle that never answers simply has no entry here and the panel keeps Kite's static table.
//
// Kept per `vehicleId` for every vehicle, not filtered through `isActive()`: switching the active vehicle
// switches `activeFcModes` at once, with no re-read. An entry is dropped when its vehicle is lost, its
// link closes, or everything disconnects.

import { writable, derived, type Readable } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { activeVehicleId, vehicles } from './vehicles';
import type { FcMode } from '$lib/helpers/mavModes';
import { parseVehicleId } from '$lib/helpers/vehicleId';

/** Complete FC mode lists, keyed by vehicle id (`"L1:S1"`). */
export const fcModeLists = writable<ReadonlyMap<string, readonly FcMode[]>>(new Map());

/** The active vehicle's FC mode list, or null when it has none (yet). */
export const activeFcModes: Readable<readonly FcMode[] | null> = derived(
  [fcModeLists, activeVehicleId],
  ([lists, id]) => (id ? lists.get(id) ?? null : null),
);

function drop(match: (vehicleId: string) => boolean): void {
  fcModeLists.update((m) => {
    if (![...m.keys()].some(match)) return m;
    const next = new Map(m);
    for (const id of [...next.keys()]) if (match(id)) next.delete(id);
    return next;
  });
}

/** Forget every list (disconnect-all). */
export function clearFcModeLists(): void {
  fcModeLists.set(new Map());
}

// Disconnect-all empties the vehicle registry — no list may outlive it.
vehicles.subscribe((known) => {
  if (known.size === 0) clearFcModeLists();
});

let unlisteners: UnlistenFn[] = [];
let starting: Promise<void> | null = null;

/** Subscribe to the backend's `available-modes` (and the events that end a vehicle). Idempotent, kept
 *  for the app's lifetime like the vehicle registry listeners; call before `announce_vehicles` so a
 *  reload's replay is not missed. */
export function startFcModeListeners(): Promise<void> {
  if (unlisteners.length) return Promise.resolve();
  if (!starting) {
    starting = (async () => {
      unlisteners.push(
        await listen<{ vehicleId: string; modes: FcMode[] }>('available-modes', (e) => {
          const { vehicleId, modes } = e.payload;
          if (typeof vehicleId !== 'string' || !Array.isArray(modes) || modes.length === 0) return;
          fcModeLists.update((m) => new Map(m).set(vehicleId, modes));
        }),
        await listen<{ vehicleId: string }>('vehicle-lost', (e) => {
          drop((id) => id === e.payload.vehicleId);
        }),
        // A closed link takes every vehicle on it along (the backend announces only its primary as lost).
        await listen<{ linkId: number }>('link-closed', (e) => {
          drop((id) => parseVehicleId(id)?.link === e.payload.linkId);
        }),
      );
    })().finally(() => { starting = null; });
  }
  return starting;
}
