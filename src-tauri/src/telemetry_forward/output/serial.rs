// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Serial output sink — opens a second COM port (write-only) for relayed telemetry. Covers HC-05 /
//! BT-SPP virtual COM ports (e.g. the U360GTS antenna tracker).

use std::io::Write;
use std::time::Duration;

use super::OutputSink;

pub struct SerialSink {
    name: String,
    port: Box<dyn serialport::SerialPort>,
}

// serialport's Box<dyn SerialPort> is Send in practice (mirrors transport::serial::SerialConnection).
unsafe impl Send for SerialSink {}

impl SerialSink {
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Failed to open relay port {}: {}", port_name, e))?;
        Ok(Self { name: port_name.to_string(), port })
    }
}

impl OutputSink for SerialSink {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.port
            .write_all(data)
            .map_err(|e| format!("Relay serial write failed: {}", e))?;
        self.port
            .flush()
            .map_err(|e| format!("Relay serial flush failed: {}", e))?;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Serial({})", self.name)
    }
}
