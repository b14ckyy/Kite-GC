// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Serial transport — **iOS stand-in** for `transport/serial.rs`.
//!
//! iOS grants an app no serial-port access of any kind (no /dev nodes, no USB host API), so there is
//! nothing to implement — but the module still exists, with the same public surface as the desktop
//! one, so no caller has to know. `transport/mod.rs` picks the implementation once; everything above
//! it (the connect command, the relay's serial sink, the radar's serial sources) compiles unchanged
//! and gets an honest runtime error / an empty port list instead of being compiled out. The UI never
//! offers serial on mobile (form-factor tiers), so these paths are a safety net, not UX.
//!
//! Android is different: it DOES have a serial route (the USB host API) and gets a real
//! implementation of this same surface when the Android port lands — not this stand-in.

use super::{ByteTransport, PortInfo, TransportError};

const UNAVAILABLE: &str = "Serial ports are not available on iOS — connect over Wi-Fi (TCP/UDP) or BLE";

/// No serial subsystem to enumerate — always empty. The connection dialog renders this as
/// "no ports found" (and hides serial on mobile anyway).
pub fn list_ports() -> Vec<PortInfo> {
    Vec::new()
}

/// Same name and surface as the desktop `SerialConnection`. `open` is the only constructor and it
/// always fails, so the methods below are formally required dead ends, never reached.
pub struct SerialConnection {}

impl SerialConnection {
    pub fn open(port_name: &str, _baud_rate: u32) -> Result<Self, String> {
        log::warn!("Serial open requested on iOS ({port_name}) — rejected, no serial access exists");
        Err(UNAVAILABLE.into())
    }

    /// DTR/RTS have no meaning without a port; unreachable (no instance can exist).
    pub fn set_control_signals(&mut self, _dtr: bool, _rts: bool) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }
}

impl ByteTransport for SerialConnection {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
        Err(TransportError::Disconnected)
    }

    fn write_bytes(&mut self, _data: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::Disconnected)
    }

    fn description(&self) -> String {
        "Serial(unavailable on iOS)".to_string()
    }
}
