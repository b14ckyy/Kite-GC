// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Battery Manager view state — a view toggle inside the Flight Logbook panel (like the
// Mission Manager is to the Mission Planner). Lifted into a store so the view + selection +
// grouping survive the logbook close/reopen. See docs/active/BATTERY_MANAGEMENT.md.

import { writable } from 'svelte/store';

/** Canonicalize a battery serial: ASCII letters + digits only, upper-cased (drops spaces, punctuation
 *  and case). The serial is the soft-link key, so any inconsistency silently breaks the flight↔pack
 *  link; hardware barcodes are upper alnum anyway. MUST match the Rust `normalize_serial` (db.rs) —
 *  applied live in the inputs AND enforced again on the backend. */
export function normalizeSerial(s: string): string {
  return s.toUpperCase().replace(/[^A-Z0-9]/g, '');
}

/** Live input normalization for a serial *list* (one flight may link several packs): upper-case and
 *  keep alnum plus the `,` / space separators the user is typing. Per-token cleanup + de-dupe happen
 *  on save via {@link normalizeSerialList} — doing it live would fight the cursor. */
export function normalizeSerialInput(s: string): string {
  return s.toUpperCase().replace(/[^A-Z0-9, ]/g, '');
}

/** Split a serial-list string into normalized, de-duplicated, order-preserving tokens (drops empties). */
export function serialTokens(s: string): string[] {
  const out: string[] = [];
  for (const tok of s.split(',')) {
    const n = normalizeSerial(tok);
    if (n && !out.includes(n)) out.push(n);
  }
  return out;
}

/** Canonical serial-list for storage/display: normalized tokens rejoined with ", ". The backend
 *  re-normalizes to its own canonical form on store; MUST stay token-compatible with Rust
 *  `normalize_serial_list` (db.rs). */
export function normalizeSerialList(s: string): string {
  return serialTokens(s).join(', ');
}

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
