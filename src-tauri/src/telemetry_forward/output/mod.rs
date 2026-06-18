// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Relay output transports. Where the inbound transport (`transport/`) reads, these write encoded
//! telemetry out a *second* link. Phase 1 = serial (covers HC-05 / BT-SPP virtual COM, e.g. U360GTS);
//! BLE / TCP-server / UDP follow in Phase 2.

pub mod ble;
pub mod serial;
pub mod tcp;
pub mod udp;

/// A telemetry output transport — write-only.
pub trait OutputSink: Send {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;
    fn description(&self) -> String;
    /// True when the sink is open but has no consumer yet (e.g. a TCP server with no client connected),
    /// so the UI can show "waiting" rather than "active". Default: always ready.
    fn pending(&self) -> bool {
        false
    }
}
