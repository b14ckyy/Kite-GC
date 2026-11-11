// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The flight-end dialogs (End-Flight summary, recovery prompt of a lost link) one at a time, in the
// order their recordings ended (Dev-Docs active/GROUP_FLIGHTS.md §5e M3). With several vehicles two
// flights can end within seconds; a second dialog must neither replace the first (its Save/Discard would
// never resolve, the session would stay pending) nor be lost. Each entry names its vehicle: when that
// vehicle's `flight-recording-resumed` / `-committed` arrives before the entry opened, the backend has
// already resolved its session and the entry is dropped (`dropQueuedFlightDialogs`).
//
// Single vehicle: the one entry opens synchronously on enqueue — the same timing as a direct call.

interface QueuedDialog {
  vehicleId: string | null;
  /** Opens the dialog and resolves once it has settled (and its follow-up work is done). */
  open: () => Promise<void>;
}

const waiting: QueuedDialog[] = [];
let busy = false;

/** Queue a flight-end dialog of `vehicleId` (null = a payload without a vehicle). */
export function enqueueFlightDialog(vehicleId: string | null, open: () => Promise<void>): void {
  waiting.push({ vehicleId, open });
  if (!busy) void pump();
}

/** The backend resolved `vehicleId`'s session (re-arm within grace, grace-lapsed commit): drop its
 *  dialogs that have not opened yet. Returns how many were dropped — when any, the event was about a
 *  waiting session, so the dialog on screen (another vehicle's) must stay open. */
export function dropQueuedFlightDialogs(vehicleId: string | null | undefined): number {
  if (vehicleId == null) return 0;
  let dropped = 0;
  for (let i = waiting.length - 1; i >= 0; i--) {
    if (waiting[i].vehicleId === vehicleId) {
      waiting.splice(i, 1);
      dropped++;
    }
  }
  return dropped;
}

async function pump(): Promise<void> {
  busy = true;
  try {
    for (let next = waiting.shift(); next; next = waiting.shift()) {
      try {
        await next.open();
      } catch (e) {
        console.warn('[flight-dialogs] dialog failed', e);
      }
    }
  } finally {
    busy = false;
  }
}
