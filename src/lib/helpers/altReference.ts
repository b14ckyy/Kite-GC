// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Fallback for a flight without a stored `flights.alt_relative` (imported logs, opened files): the same
// rule on the `protocol` label as `ALT_RELATIVE_PROTOCOL_SQL` (db.rs) — change both together.

/** Flight-row `protocol` labels written by the passive backend (see `passive_telemetry/handler.rs`). */
const RELATIVE_ALTITUDE_PROTOCOLS = ['CRSF', 'LTM'];

/**
 * True when a recorded flight's GPS altitude (`alt_m`) is relative to the arming point rather than
 * true MSL — currently the passive CRSF and LTM links, which carry no MSL at all.
 *
 * Callers must anchor such a track to a ground reference (terrain elevation at the start point)
 * instead of trusting `alt_m` as an absolute height.
 */
export function isRelativeAltitudeProtocol(protocol: string | null | undefined): boolean {
  if (!protocol) return false;
  // Only the passive "Telemetry (…)" labels are relative; an MSP or MAVLink flight reports true MSL.
  if (!protocol.startsWith('Telemetry (')) return false;
  return RELATIVE_ALTITUDE_PROTOCOLS.some((p) => protocol.includes(p));
}
