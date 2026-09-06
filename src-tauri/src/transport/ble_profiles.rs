// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The known BLE-serial profiles — one table for every platform backend.
//!
//! A BLE "serial" adapter is a GATT service with one write and one notify characteristic; which
//! UUIDs those are depends on the chip family. This list comes from INAV Configurator and is what
//! the desktop (btleplug) and Android (native GATT) backends match against, both at scan time
//! (advertised service UUIDs, where the adapter advertises them at all) and after connecting
//! (the discovered service list — the authoritative match, since most adapters advertise nothing).
//! `ble_ios.rs` carries its own short-UUID mirror of the same entries for CoreBluetooth.

/// Known BLE serial device profile.
#[derive(Debug, Clone)]
pub struct BleDeviceProfile {
    pub name: &'static str,
    pub service_uuid: uuid::Uuid,
    pub write_characteristic: uuid::Uuid,
    pub read_characteristic: uuid::Uuid,
    /// Pause between write chunks. Only the tiny-buffered CC2541 needs pacing; ESP32/nRF-class
    /// adapters have KB-scale ring buffers and run at 0.
    pub write_delay_ms: u64,
}

/// All known BLE serial profiles (from INAV Configurator), in match-preference order.
pub fn known_profiles() -> Vec<BleDeviceProfile> {
    vec![
        BleDeviceProfile {
            name: "CC2541 based",
            service_uuid: uuid::Uuid::parse_str("0000ffe0-0000-1000-8000-00805f9b34fb").unwrap(),
            write_characteristic: uuid::Uuid::parse_str("0000ffe1-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            read_characteristic: uuid::Uuid::parse_str("0000ffe1-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            write_delay_ms: 30,
        },
        BleDeviceProfile {
            name: "Nordic NRF (NUS)",
            service_uuid: uuid::Uuid::parse_str("6e400001-b5a3-f393-e0a9-e50e24dcca9e").unwrap(),
            write_characteristic: uuid::Uuid::parse_str("6e400002-b5a3-f393-e0a9-e50e24dcca9e")
                .unwrap(),
            read_characteristic: uuid::Uuid::parse_str("6e400003-b5a3-f393-e0a9-e50e24dcca9e")
                .unwrap(),
            // No pacing: ESP32/nRF have ample UART FIFO + KB-scale ring buffers (the SpeedyBee ESP32-C3
            // profiles already run at 0). Only the tiny-buffered CC2541 keeps its inter-chunk delay.
            write_delay_ms: 0,
        },
        BleDeviceProfile {
            name: "SpeedyBee Type 2",
            service_uuid: uuid::Uuid::parse_str("0000abf0-0000-1000-8000-00805f9b34fb").unwrap(),
            write_characteristic: uuid::Uuid::parse_str("0000abf1-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            read_characteristic: uuid::Uuid::parse_str("0000abf2-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            write_delay_ms: 0,
        },
        BleDeviceProfile {
            name: "SpeedyBee Type 1",
            service_uuid: uuid::Uuid::parse_str("00001000-0000-1000-8000-00805f9b34fb").unwrap(),
            write_characteristic: uuid::Uuid::parse_str("00001001-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            read_characteristic: uuid::Uuid::parse_str("00001002-0000-1000-8000-00805f9b34fb")
                .unwrap(),
            write_delay_ms: 0,
        },
    ]
}
