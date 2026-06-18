// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! BLE output sink — Kite acts as the BLE central and writes the encoded telemetry to a device's write
//! characteristic (e.g. an HM-10 / Nordic-UART serial bridge on a tracker). Reuses the profile-based
//! `connect_ble` transport, so the target must match a known BLE serial profile.

use super::OutputSink;
use crate::transport::ble::{connect_ble, BleTransport};
use crate::transport::ByteTransport;

pub struct BleSink {
    transport: BleTransport,
}

impl BleSink {
    pub async fn open(device_id: &str) -> Result<Self, String> {
        let transport = connect_ble(device_id).await?;
        Ok(Self { transport })
    }
}

impl OutputSink for BleSink {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.transport.write_bytes(data).map_err(|e| format!("BLE relay write failed: {e}"))
    }

    fn description(&self) -> String {
        self.transport.description()
    }
}
