// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Which *recorded* flights carry arming-relative altitude instead of true MSL.
//
// The live path already knows this: the passive backend emits `telemetry-alt-ref` when it locks a
// protocol (`handler.rs`, `msl = !matches!(p, Protocol::Ltm | Protocol::Crsf)`), and the frontend
// anchors the relative value to a ground MSL captured at the arm edge. A *replayed* flight has no such
// event — the only surviving hint is the `protocol` label the recorder wrote onto the flight row.
//
// So this matches on that label. It is deliberately the ONLY place that does, because the match makes
// the label part of a data contract: renaming it in `handler.rs` would silently break every stored
// flight. The clean fix is a dedicated column plus a migration that back-fills it from exactly this
// function — worth doing on the next schema change, not worth a migration on its own.

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
