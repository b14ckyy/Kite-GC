// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle Manager view state — a view toggle inside the Flight Logbook panel (like the Battery
// Manager). Lifted into a store so the view + selection survive the logbook close/reopen.
// See docs/active/VEHICLE_DB.md.

import { writable } from 'svelte/store';

/** Canonicalize a craft name: trim only (preserve case + inner content — it is a user-facing display
 *  string, unlike a battery serial). The flight↔vehicle match is case-insensitive on the trimmed
 *  value. MUST match the Rust `normalize_craft_name` (db.rs). */
export function normalizeCraftName(s: string): string {
  return s.trim();
}

/** Whether the logbook list is showing the Vehicle Manager instead of flights. */
export const vehicleManagerOpen = writable<boolean>(false);

/** The vehicle selected in the Manager (persists across close/reopen). */
export const vehicleManagerSelectedId = writable<number | null>(null);

/** What a "create vehicle from this flight" pre-fills: the flight's craft name and FC id, and the flight
 *  itself, which is linked to the new vehicle once it is saved. */
export interface VehicleCreatePrefill {
  craftName: string;
  fcUid: string | null;
  flightId: number | null;
}

/** One-shot signal: open the Manager straight into the create form pre-filled from a flight that
 *  belongs to no vehicle (End-Flight, flight detail). The Manager consumes it (starts a create, fills
 *  name + craft name + FC id) and resets it to null. */
export const vehicleManagerCreatePrefill = writable<VehicleCreatePrefill | null>(null);

/** Search query for the vehicle list (name / craft name / model / type / notes). */
export const vehicleSearchQuery = writable<string>('');

/** Bumped when something outside the Manager writes the library (the UAV Info panel's "Save to
 *  vehicle library"), so an open Manager reloads its list. */
export const vehicleLibraryChanged = writable<number>(0);
