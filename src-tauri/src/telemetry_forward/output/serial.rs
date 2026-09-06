// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Serial output sink — opens a second COM port (write-only) for relayed telemetry. Covers HC-05 /
//! BT-SPP virtual COM ports (e.g. the U360GTS antenna tracker).

use std::time::Duration;

use super::OutputSink;
use crate::transport::serial::SerialConnection;
use crate::transport::ByteTransport;

pub struct SerialSink {
    conn: SerialConnection,
}

impl SerialSink {
    /// Opens through `transport::serial` rather than the serialport crate directly. The write path is
    /// identical (write_all + flush per frame), the port timeout is set to the same 100 ms this sink
    /// always used, and the relay port gains the open-retry + DTR/RTS raising the inbound serial
    /// connection already had — which is exactly what the BT-SPP modules this sink exists for (HC-05
    /// trackers) want on first open. It also keeps this file on the platform seam, so it compiles on
    /// mobile, where an open simply returns the platform's "no serial" error.
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        let mut conn = SerialConnection::open(port_name, baud_rate)
            .map_err(|e| format!("Failed to open relay port {}: {}", port_name, e))?;
        conn.set_read_timeout(Duration::from_millis(100));
        Ok(Self { conn })
    }
}

impl OutputSink for SerialSink {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.conn
            .write_bytes(data)
            .map_err(|e| format!("Relay serial write failed: {}", e))
    }

    fn description(&self) -> String {
        self.conn.description()
    }
}
