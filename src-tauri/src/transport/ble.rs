// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// BLE Transport
// Connects to a flight controller via Bluetooth Low Energy (GATT Serial Profile).
// Supports known BLE-to-serial adapters: CC2541, Nordic NRF (NUS), SpeedyBee Type 1/2.
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::sync::mpsc;
use std::time::Duration;

use super::{ByteTransport, TransportError};

/// BLE write MTU — standard BLE 4.x limit for GATT writes
const BLE_WRITE_MTU: usize = 20;

/// Timeout waiting for an MSP response
const MSP_RESPONSE_TIMEOUT_MS: u64 = 3000;

/// Timeout for BLE scan
const BLE_SCAN_TIMEOUT_MS: u64 = 8000;

/// Known BLE serial device profiles
#[derive(Debug, Clone)]
pub struct BleDeviceProfile {
    pub name: &'static str,
    pub service_uuid: uuid::Uuid,
    pub write_characteristic: uuid::Uuid,
    pub read_characteristic: uuid::Uuid,
    pub write_delay_ms: u64,
}

/// All known BLE serial profiles (from INAV Configurator)
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
            write_delay_ms: 30,
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

/// Information about a discovered BLE device
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDeviceInfo {
    /// Peripheral identifier (platform-specific: MAC on Linux, UUID on macOS/Windows)
    pub id: String,
    /// Device name (if advertised)
    pub name: String,
    /// Matched profile name
    pub profile: String,
    /// RSSI at discovery time
    pub rssi: Option<i16>,
}

/// An active BLE connection to a flight controller
pub struct BleTransport {
    device_name: String,
    profile_name: String,
    /// Channel to send write requests to the async BLE runtime thread
    write_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Channel to receive incoming bytes from the BLE notification handler
    read_rx: mpsc::Receiver<Vec<u8>>,
    /// Handle to stop the BLE runtime thread
    stop_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// Buffered bytes from previous reads that haven't been fully consumed
    read_buffer: Vec<u8>,
    /// Write delay between BLE chunks (device-specific)
    write_delay: Duration,
}

/// Scan for BLE devices.
/// Scans WITHOUT a service UUID filter because many BLE serial adapters
/// do not advertise their service UUIDs in advertisement packets —
/// those are only discoverable via service discovery after connecting.
/// Devices that DO advertise a known service UUID are tagged with the
/// matching profile name; all other named devices are shown as "Unknown"
/// so the user can try connecting (profile is matched during connect).
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    use btleplug::api::{Central, Manager as _, ScanFilter};
    use btleplug::platform::Manager;

    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager init failed: {}", e))?;

    let adapters = manager
        .adapters()
        .await
        .map_err(|e| format!("No BLE adapters found: {}", e))?;

    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| "No BLE adapter available".to_string())?;

    let profiles = known_profiles();

    // Scan WITHOUT service UUID filter — most BLE serial adapters don't
    // advertise service UUIDs, so filtering would hide them entirely.
    let scan_filter = ScanFilter { services: vec![] };

    adapter
        .start_scan(scan_filter)
        .await
        .map_err(|e| format!("BLE scan failed: {}", e))?;

    // Scan for a fixed duration
    tokio::time::sleep(Duration::from_millis(BLE_SCAN_TIMEOUT_MS)).await;

    adapter
        .stop_scan()
        .await
        .map_err(|e| format!("BLE stop scan failed: {}", e))?;

    let peripherals = adapter
        .peripherals()
        .await
        .map_err(|e| format!("Failed to list peripherals: {}", e))?;

    let mut devices = Vec::new();

    for peripheral in peripherals {
        use btleplug::api::Peripheral as _;

        let props = match peripheral.properties().await {
            Ok(Some(p)) => p,
            _ => continue,
        };

        // Skip devices without a name — they're not useful for selection
        let name = match &props.local_name {
            Some(n) if !n.is_empty() => n.clone(),
            _ => continue,
        };

        // Try to match a known profile from advertised service UUIDs
        let matched_profile = profiles.iter().find(|profile| {
            props
                .services
                .iter()
                .any(|s| *s == profile.service_uuid)
        });

        let profile_name = matched_profile
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        devices.push(BleDeviceInfo {
            id: peripheral.id().to_string(),
            name,
            profile: profile_name,
            rssi: props.rssi,
        });
    }

    // Sort: known profiles first, then by RSSI (strongest first)
    devices.sort_by(|a, b| {
        let a_known = a.profile != "Unknown";
        let b_known = b.profile != "Unknown";
        b_known.cmp(&a_known).then_with(|| {
            let a_rssi = a.rssi.unwrap_or(i16::MIN);
            let b_rssi = b.rssi.unwrap_or(i16::MIN);
            b_rssi.cmp(&a_rssi)
        })
    });

    Ok(devices)
}

/// Connect to a BLE device by its peripheral ID.
/// Spawns a background thread that bridges async btleplug to sync channels.
pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    use btleplug::api::{Central, Manager as _, Peripheral as _, WriteType, ScanFilter};
    use btleplug::platform::Manager;
    use futures::StreamExt;

    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager init failed: {}", e))?;

    let adapters = manager
        .adapters()
        .await
        .map_err(|e| format!("No BLE adapters found: {}", e))?;

    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| "No BLE adapter available".to_string())?;

    // A fresh Manager/Adapter has no cached peripherals — we need a quick scan
    // to rediscover the device the user selected from the scan results.
    adapter
        .start_scan(ScanFilter { services: vec![] })
        .await
        .map_err(|e| format!("BLE scan failed: {}", e))?;

    // Short scan — the device should be advertising and appear quickly
    tokio::time::sleep(Duration::from_millis(3000)).await;

    adapter
        .stop_scan()
        .await
        .map_err(|e| format!("BLE stop scan failed: {}", e))?;

    let peripherals = adapter
        .peripherals()
        .await
        .map_err(|e| format!("Failed to list peripherals: {}", e))?;

    let peripheral = peripherals
        .into_iter()
        .find(|p| p.id().to_string() == device_id)
        .ok_or_else(|| format!("BLE device '{}' not found. Rescan required.", device_id))?;

    // Connect
    peripheral
        .connect()
        .await
        .map_err(|e| format!("BLE connect failed: {}", e))?;

    peripheral
        .discover_services()
        .await
        .map_err(|e| format!("BLE service discovery failed: {}", e))?;

    let props = peripheral
        .properties()
        .await
        .map_err(|e| format!("Failed to get device properties: {}", e))?
        .unwrap_or_default();

    let device_name = props
        .local_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());

    // Match profile
    let profiles = known_profiles();
    let services = peripheral.services();

    let matched_profile = profiles
        .iter()
        .find(|profile| {
            services.iter().any(|s| s.uuid == profile.service_uuid)
        })
        .ok_or_else(|| "No matching BLE serial profile found on device".to_string())?
        .clone();

    // Find characteristics
    let service = services
        .iter()
        .find(|s| s.uuid == matched_profile.service_uuid)
        .ok_or_else(|| "Service not found after discovery".to_string())?;

    let write_char = service
        .characteristics
        .iter()
        .find(|c| c.uuid == matched_profile.write_characteristic)
        .ok_or_else(|| {
            format!(
                "Write characteristic {} not found",
                matched_profile.write_characteristic
            )
        })?
        .clone();

    let read_char = service
        .characteristics
        .iter()
        .find(|c| c.uuid == matched_profile.read_characteristic)
        .ok_or_else(|| {
            format!(
                "Read characteristic {} not found",
                matched_profile.read_characteristic
            )
        })?
        .clone();

    // Subscribe to notifications on read characteristic
    peripheral
        .subscribe(&read_char)
        .await
        .map_err(|e| format!("BLE subscribe failed: {}", e))?;

    let notification_stream = peripheral
        .notifications()
        .await
        .map_err(|e| format!("BLE notification stream failed: {}", e))?;

    // Set up channels for bridging async BLE to sync Transport trait
    let (write_tx, mut write_async_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>();
    let (stop_tx, mut stop_async_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let profile_name = matched_profile.name.to_string();
    let write_delay = Duration::from_millis(matched_profile.write_delay_ms);

    // Spawn async task that handles BLE I/O
    let peripheral_clone = peripheral.clone();
    tauri::async_runtime::spawn(async move {
        let mut notifications = notification_stream;

        loop {
            tokio::select! {
                // Forward BLE notifications → read channel
                Some(notification) = notifications.next() => {
                    if notification.uuid == read_char.uuid {
                        let _ = read_tx.send(notification.value);
                    }
                }
                // Handle write requests from sync side
                Some(data) = write_async_rx.recv() => {
                    for chunk in data.chunks(BLE_WRITE_MTU) {
                        if let Err(e) = peripheral_clone
                            .write(&write_char, chunk, WriteType::WithoutResponse)
                            .await
                        {
                            log::error!("BLE write failed: {}", e);
                            break;
                        }
                        if !write_delay.is_zero() {
                            tokio::time::sleep(write_delay).await;
                        }
                    }
                }
                // Stop signal
                _ = stop_async_rx.recv() => {
                    log::info!("BLE runtime stopping");
                    let _ = peripheral_clone.disconnect().await;
                    break;
                }
            }
        }
    });

    Ok(BleTransport {
        device_name,
        profile_name,
        write_tx,
        read_rx,
        stop_tx,
        read_buffer: Vec::new(),
        write_delay,
    })
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        // First, drain any buffered bytes from previous reads
        if !self.read_buffer.is_empty() {
            let n = std::cmp::min(buf.len(), self.read_buffer.len());
            buf[..n].copy_from_slice(&self.read_buffer[..n]);
            self.read_buffer.drain(..n);
            return Ok(n);
        }

        // Wait for new data from BLE notifications (100ms timeout to avoid blocking forever)
        match self.read_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(data) => {
                let n = std::cmp::min(buf.len(), data.len());
                buf[..n].copy_from_slice(&data[..n]);
                if data.len() > n {
                    self.read_buffer.extend_from_slice(&data[n..]);
                }
                Ok(n)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(0),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.write_tx
            .send(data.to_vec())
            .map_err(|_| TransportError::Disconnected)?;
        Ok(())
    }

    fn description(&self) -> String {
        format!("BLE({}, {})", self.device_name, self.profile_name)
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}
