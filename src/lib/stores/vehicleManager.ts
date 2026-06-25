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

/** One-shot signal: open the Manager straight into the create form with this craft name pre-filled.
 *  Set from a flight that has a craft name matching no existing vehicle; the Manager consumes it
 *  (starts a create, fills name + craft name) and resets it to null. */
export const vehicleManagerCreateCraft = writable<string | null>(null);

/** Search query for the vehicle list (name / craft name / model / type / notes). */
export const vehicleSearchQuery = writable<string>('');
