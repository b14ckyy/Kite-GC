// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle id helpers — the backend addresses every vehicle by a string key `L{link}:S{sysid}`
// (see src-tauri/src/vehicle_registry). Pure functions; the frontend mostly passes keys through
// untouched and only needs to split one when it wants to group by link.

export interface ParsedVehicleId {
  link: number;
  /** MAVLink system id; 0 for MSP / passive-telemetry links (one vehicle per link). */
  sysid: number;
}

const KEY_RE = /^L(\d{1,5}):S(\d{1,3})$/;

/** Parse `"L1:S1"` → `{ link: 1, sysid: 1 }`, or null for anything malformed. */
export function parseVehicleId(key: string): ParsedVehicleId | null {
  const m = KEY_RE.exec(key);
  if (!m) return null;
  const link = Number(m[1]);
  const sysid = Number(m[2]);
  if (link < 1 || link > 65535 || sysid > 255) return null;
  return { link, sysid };
}

export function formatVehicleId(link: number, sysid: number): string {
  return `L${link}:S${sysid}`;
}
