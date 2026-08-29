// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! BLE transport — **Android stand-in** for `transport/ble.rs`.
//!
//! The desktop implementation is built on `btleplug`. Its Android backend needs a companion Java
//! library (`droidplug`) loaded into the host app, which the Gradle project under `gen/android` does
//! not ship — so `btleplug` is left out of the mobile build entirely (see the
//! `cfg(not(target_os = "android"))` dependency block in `Cargo.toml`) rather than compiled into a
//! backend that panics at runtime.
//!
//! Android's own `BluetoothLeScanner` / `BluetoothGatt` are the native route, reached through a Tauri
//! mobile plugin (and gated behind the runtime `BLUETOOTH_SCAN` / `BLUETOOTH_CONNECT` permissions
//! declared in `AndroidManifest.xml`). Until that exists this module keeps the same public surface as
//! the desktop one so `commands/connection.rs` and the relay's BLE sink compile unchanged, and a
//! scan simply finds nothing instead of failing to link.

use std::time::Duration;

use super::{ByteTransport, TransportError};

const UNSUPPORTED: &str =
    "Bluetooth LE is not available on Android yet — connect over UDP or TCP instead";

/// Information about a discovered BLE device. Same shape as the desktop struct so the frontend's
/// `ble-device` event payload is identical on every platform.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDeviceInfo {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub rssi: Option<i16>,
}

/// No adapter to scan with — an empty result, which the device picker already renders as "none found".
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    Ok(Vec::new())
}

/// Live-scan session: nothing to emit, so it completes immediately. The caller drops its stop-sender
/// as usual and the UI's scan spinner ends the same way it does when a desktop scan finds nothing.
pub async fn run_scan_session(
    _app: tauri::AppHandle,
    _stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    log::info!("BLE scan requested on Android — {UNSUPPORTED}");
    Ok(())
}

/// Placeholder mirroring the desktop `BleTransport`. Uninhabited: neither `connect_ble` nor
/// `connect_ble_listen` can produce one, so the `ByteTransport` methods are unreachable.
pub struct BleTransport {
    _never: std::convert::Infallible,
}

pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    log::warn!("BLE connect refused on Android ({device_id}) — {UNSUPPORTED}");
    Err(UNSUPPORTED.to_string())
}

pub async fn connect_ble_listen(
    device_id: &str,
    _app: tauri::AppHandle,
) -> Result<BleTransport, String> {
    log::warn!("BLE listen-only connect refused on Android ({device_id}) — {UNSUPPORTED}");
    Err(UNSUPPORTED.to_string())
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
        Err(TransportError::Disconnected)
    }

    fn write_bytes(&mut self, _data: &[u8]) -> Result<(), TransportError> {
        Err(TransportError::Disconnected)
    }

    fn set_read_timeout(&mut self, _timeout: Duration) {}

    fn description(&self) -> String {
        "BLE(unavailable on Android)".to_string()
    }
}
