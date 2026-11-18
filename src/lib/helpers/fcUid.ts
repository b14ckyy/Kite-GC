// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/** Characters of an FC hardware id (24–36 hex chars) shown before the ellipsis. */
const SHOWN = 10;

/** An FC hardware id shortened for display — the first characters and an ellipsis (the full value
 *  belongs in a tooltip). Shared by `FcUidChip` and the places that show the id as plain text. */
export function shortFcUid(uid: string): string {
  return uid.length > SHOWN ? `${uid.slice(0, SHOWN)}…` : uid;
}

/** INAV SITL's constant MSP_UID — the same on every instance and host (see the Rust `msp::hardware_uid`). */
const INAV_SITL_UID = '000000000100000002000000';

/** The FC hardware id worth comparing: null for an empty or all-zero id and for INAV SITL's constant
 *  (no aircraft identity), else the trimmed id. MUST match the Rust `msp::hardware_uid`. */
export function hardwareUid(raw: string | null | undefined): string | null {
  const uid = (raw ?? '').trim();
  if (!uid || /^0+$/.test(uid) || uid.toUpperCase() === INAV_SITL_UID) return null;
  return uid;
}
