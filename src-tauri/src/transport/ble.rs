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

/// Timeout for BLE scan
const BLE_SCAN_TIMEOUT_MS: u64 = 8000;

// The profile table lives in `ble_profiles.rs`, shared with the Android backend.
use super::ble_profiles::known_profiles;

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
    /// Idle timeout for `read_bytes`; the pipelined MSP scheduler tightens it so its loop ticks at
    /// ~100 Hz instead of being paced by a coarse blocking read.
    read_timeout: std::time::Duration,
    restarted: bool,
}

/// Sent on `read_rx` after a transparent reconnect, so empty notifications are never forwarded.
const STREAM_RESTART: Vec<u8> = Vec::new();

type Notifications =
    std::pin::Pin<Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>>;

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
                .services.contains(&profile.service_uuid)
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

/// Run a continuous BLE scan session, emitting a `ble-device` Tauri event for each discovered or
/// updated device (named devices only) so the frontend can populate its list in real time. Runs
/// until `stop_rx` resolves (the sender is dropped or signalled), then stops the scan.
pub async fn run_scan_session(
    app: tauri::AppHandle,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
    use btleplug::platform::Manager;
    use futures::StreamExt;
    use tauri::Emitter;

    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager init failed: {}", e))?;
    let adapter = manager
        .adapters()
        .await
        .map_err(|e| format!("No BLE adapters found: {}", e))?
        .into_iter()
        .next()
        .ok_or_else(|| "No BLE adapter available".to_string())?;

    let profiles = known_profiles();
    let mut events = adapter
        .events()
        .await
        .map_err(|e| format!("BLE events stream failed: {}", e))?;
    adapter
        .start_scan(ScanFilter { services: vec![] })
        .await
        .map_err(|e| format!("BLE scan failed: {}", e))?;

    loop {
        tokio::select! {
            _ = &mut stop_rx => break, // stop signalled or sender dropped
            ev = events.next() => {
                match ev {
                    Some(CentralEvent::DeviceDiscovered(id)) | Some(CentralEvent::DeviceUpdated(id)) => {
                        let Ok(peripheral) = adapter.peripheral(&id).await else { continue };
                        let Ok(Some(props)) = peripheral.properties().await else { continue };
                        let Some(name) = props.local_name.clone().filter(|n| !n.is_empty()) else { continue };
                        let matched = profiles
                            .iter()
                            .find(|pr| props.services.contains(&pr.service_uuid));
                        let _ = app.emit("ble-device", BleDeviceInfo {
                            id: peripheral.id().to_string(),
                            name,
                            profile: matched.map(|p| p.name.to_string()).unwrap_or_else(|| "Unknown".to_string()),
                            rssi: props.rssi,
                        });
                    }
                    None => break, // event stream ended
                    _ => {}
                }
            }
        }
    }

    let _ = adapter.stop_scan().await;
    Ok(())
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

    // A lost link is re-established underneath with the channels kept open, so the layers above see a
    // stall. The task ends (dropping `read_tx` = `Disconnected`) only on stop or with Bluetooth off.
    let peripheral_clone = peripheral.clone();
    let adapter_clone = adapter.clone();
    let task_name = device_name.clone();
    tauri::async_runtime::spawn(async move {
        use btleplug::api::Peripheral as _;
        let mut notifications = notification_stream;

        loop {
            // ── Serve the live link until it drops (break → reconnect) or stop is requested (return) ──
            loop {
                tokio::select! {
                    maybe = notifications.next() => match maybe {
                        Some(notification) => {
                            if notification.uuid == read_char.uuid && !notification.value.is_empty() {
                                let _ = read_tx.send(notification.value);
                            }
                        }
                        None => {
                            log::warn!("BLE notification stream ended — link lost, reconnecting");
                            break;
                        }
                    },
                    // A write error is the *primary* disconnect signal on some adapters (CC2541/HC-04:
                    // BlueZ returns "Not connected" on write before the notification stream ends).
                    Some(data) = write_async_rx.recv() => {
                        let mut write_err = None;
                        for (i, chunk) in data.chunks(BLE_WRITE_MTU).enumerate() {
                            // Pace only *between* chunks, never after the last: a single-chunk write
                            // (every MSP telemetry request is 9 bytes = 1 chunk) then incurs no delay, so
                            // it can't park the reply notification while this branch sleeps (tokio::select!
                            // runs one branch to completion before re-polling `notifications.next()`). The
                            // inter-chunk delay still protects a small-buffered peripheral (CC2541) from an
                            // RX overrun on a multi-chunk upload.
                            if i > 0 && !write_delay.is_zero() {
                                tokio::time::sleep(write_delay).await;
                            }
                            if let Err(e) = peripheral_clone
                                .write(&write_char, chunk, WriteType::WithoutResponse)
                                .await
                            {
                                write_err = Some(e);
                                break;
                            }
                        }
                        if let Some(e) = write_err {
                            log::warn!("BLE write failed ({e}) — link lost, reconnecting");
                            break;
                        }
                    }
                    _ = stop_async_rx.recv() => {
                        log::info!("BLE runtime stopping");
                        let _ = peripheral_clone.disconnect().await;
                        return;
                    }
                }
            }

            let subscribe = std::slice::from_ref(&read_char);
            match reconnect_until_up(&adapter_clone, &peripheral_clone, subscribe, &mut stop_async_rx, &task_name).await {
                Some(stream) => {
                    notifications = stream;
                    while write_async_rx.try_recv().is_ok() {} // drop writes queued during the gap
                    let _ = read_tx.send(STREAM_RESTART);
                }
                None => return,
            }
        }
    });

    log::info!("BLE connected: {} [{}]", device_name, profile_name);
    Ok(BleTransport {
        device_name,
        profile_name,
        write_tx,
        read_rx,
        stop_tx,
        read_buffer: Vec::new(),
        read_timeout: Duration::from_millis(100),
        restarted: false,
    })
}

/// One reconnect: a brief scan so a re-advertising device is found, then connect, rediscover, resubscribe.
/// `Characteristic` handles are keyed by UUID, so the first connect's stay valid. Like the first listen
/// connect, it fails only when none of `subscribe` takes.
async fn ble_reconnect(
    adapter: &btleplug::platform::Adapter,
    peripheral: &btleplug::platform::Peripheral,
    subscribe: &[btleplug::api::Characteristic],
) -> Result<Notifications, String> {
    use btleplug::api::{Central, Peripheral as _, ScanFilter};
    // Best-effort rediscovery; ignore scan errors (the device may already be cached/advertising).
    let _ = adapter.start_scan(ScanFilter { services: vec![] }).await;
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let _ = adapter.stop_scan().await;

    if !peripheral.is_connected().await.unwrap_or(false) {
        peripheral.connect().await.map_err(|e| format!("reconnect: {e}"))?;
    }
    peripheral.discover_services().await.map_err(|e| format!("rediscover: {e}"))?;
    let mut refused = Vec::new();
    for ch in subscribe {
        if let Err(e) = peripheral.subscribe(ch).await {
            refused.push(format!("{}: {e}", ch.uuid));
        }
    }
    if !refused.is_empty() {
        if refused.len() == subscribe.len() {
            return Err(format!("resubscribe: {}", refused.join(", ")));
        }
        log::debug!("BLE resubscribe skipped {}", refused.join(", "));
    }
    peripheral.notifications().await.map_err(|e| format!("renotify: {e}"))
}

/// Retries `ble_reconnect` with capped backoff until the link is back. `None` = stop requested, or the
/// adapter reads powered off twice in a row (BlueZ also reports an unreadable adapter as off).
async fn reconnect_until_up(
    adapter: &btleplug::platform::Adapter,
    peripheral: &btleplug::platform::Peripheral,
    subscribe: &[btleplug::api::Characteristic],
    stop_rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
    name: &str,
) -> Option<Notifications> {
    use btleplug::api::{Central, CentralState, Peripheral as _};
    let _ = peripheral.disconnect().await; // clean slate before reconnecting
    let mut backoff_ms = 1000u64;
    let mut off_readings = 0;
    loop {
        match stop_rx.try_recv() {
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            _ => return None, // stop requested, or the transport was dropped
        }
        let state = tokio::time::timeout(Duration::from_secs(2), adapter.adapter_state()).await;
        if matches!(state, Ok(Ok(CentralState::PoweredOff))) {
            off_readings += 1;
            if off_readings >= 2 {
                log::warn!("BLE link to {name}: Bluetooth is off or the adapter is gone — closing the link");
                return None;
            }
        } else {
            off_readings = 0;
        }
        let started = std::time::Instant::now();
        match ble_reconnect(adapter, peripheral, subscribe).await {
            Ok(stream) => {
                // At the level of the loss: a default-level log must show both.
                log::warn!("BLE link to {name} re-established (attempt took {:.1?})", started.elapsed());
                return Some(stream);
            }
            Err(e) => {
                log::debug!(
                    "BLE reconnect to {name} failed after {:.1?}: {e} — retry in {backoff_ms}ms",
                    started.elapsed()
                );
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                    _ = stop_rx.recv() => return None,
                }
                backoff_ms = (backoff_ms * 2).min(5000);
            }
        }
    }
}

// ── BLE GATT Explorer (dev) — listen-only auto-discovery ──────────────────────
// For the passive Telemetry mode we don't know the device's profile (e.g. FrSky ETHOS radios expose
// no standard serial service). Instead of matching a known profile we connect to ANY device, dump its
// full GATT table to the Debug Monitor and subscribe to every Notify/Indicate characteristic, routing
// their bytes into the read stream. This both reveals what the radio exposes and captures the stream
// once the radio's BLE telemetry mode is active. See docs/active/RADIO_TELEMETRY.md.

/// One characteristic in the discovered GATT table (Debug Monitor `ble-gatt-services` event).
#[derive(Debug, Clone, serde::Serialize)]
pub struct GattCharInfo {
    pub uuid: String,
    pub properties: Vec<String>,
    /// True if we subscribed to it (Notify/Indicate).
    pub subscribed: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GattServiceInfo {
    pub uuid: String,
    pub characteristics: Vec<GattCharInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GattTable {
    pub device: String,
    pub services: Vec<GattServiceInfo>,
}

/// Per-notification activity, attributed to its characteristic (Debug Monitor `ble-gatt-char-data`).
#[derive(Debug, Clone, serde::Serialize)]
struct GattCharData {
    uuid: String,
    len: usize,
}

/// SIG services never carry telemetry and must not be subscribed: Service Changed (0x2A05, Indicate)
/// makes WinRT demand an authenticated link and pops a PIN prompt.
pub(crate) fn is_standard_gatt_service(uuid: uuid::Uuid) -> bool {
    // 16-bit SIG UUIDs use the base 0000xxxx-0000-1000-8000-00805f9b34fb.
    let b = uuid.as_bytes();
    const BASE: [u8; 12] = [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfb];
    if b[0] == 0 && b[1] == 0 && b[4..16] == BASE {
        let short = ((b[2] as u16) << 8) | b[3] as u16;
        // Generic Access (0x1800), Generic Attribute (0x1801, holds Service Changed), Device Info (0x180A).
        matches!(short, 0x1800 | 0x1801 | 0x180A)
    } else {
        false
    }
}

fn char_property_names(p: btleplug::api::CharPropFlags) -> Vec<String> {
    use btleplug::api::CharPropFlags as F;
    let mut v = Vec::new();
    if p.contains(F::READ) { v.push("Read".into()); }
    if p.contains(F::WRITE) { v.push("Write".into()); }
    if p.contains(F::WRITE_WITHOUT_RESPONSE) { v.push("WriteNR".into()); }
    if p.contains(F::NOTIFY) { v.push("Notify".into()); }
    if p.contains(F::INDICATE) { v.push("Indicate".into()); }
    v
}

/// Devices a listen link holds; the trainer task must not connect to (and disconnect) one of them.
static LISTEN_DEVICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Registered from the first step of a listen connect, so the trainer task never races its setup.
struct ListenGuard(String);

impl ListenGuard {
    fn new(device_id: &str) -> Self {
        if let Ok(mut held) = LISTEN_DEVICES.lock() {
            held.push(device_id.to_string());
        }
        Self(device_id.to_string())
    }
}

impl Drop for ListenGuard {
    fn drop(&mut self) {
        if let Ok(mut held) = LISTEN_DEVICES.lock() {
            if let Some(i) = held.iter().position(|d| *d == self.0) {
                held.remove(i);
            }
        }
    }
}

pub fn is_listen_device(device_id: &str) -> bool {
    LISTEN_DEVICES.lock().map(|l| l.iter().any(|d| d == device_id)).unwrap_or(false)
}

/// Valid trainer frames on a listen link before it is taken for a trainer radio.
const TRAINER_FRAMES_TO_HAND_OVER: usize = 5;

/// Payload of `ble-trainer-detected`.
#[derive(Clone, serde::Serialize)]
struct TrainerDetected {
    id: String,
    name: String,
}

/// Listen mode, no profile: subscribes to every vendor Notify/Indicate characteristic and writes to every
/// writable one. Closes itself on FrSky trainer frames (`ble-trainer-detected`).
pub async fn connect_ble_listen(
    device_id: &str,
    app: tauri::AppHandle,
) -> Result<BleTransport, String> {
    let listen_guard = ListenGuard::new(device_id);
    use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType};
    use btleplug::platform::Manager;
    use futures::StreamExt;
    use tauri::Emitter;

    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager init failed: {}", e))?;
    let adapter = manager
        .adapters()
        .await
        .map_err(|e| format!("No BLE adapters found: {}", e))?
        .into_iter()
        .next()
        .ok_or_else(|| "No BLE adapter available".to_string())?;

    // Rediscover the selected device.
    adapter
        .start_scan(ScanFilter { services: vec![] })
        .await
        .map_err(|e| format!("BLE scan failed: {}", e))?;
    tokio::time::sleep(Duration::from_millis(3000)).await;
    adapter
        .stop_scan()
        .await
        .map_err(|e| format!("BLE stop scan failed: {}", e))?;

    let peripheral = adapter
        .peripherals()
        .await
        .map_err(|e| format!("Failed to list peripherals: {}", e))?
        .into_iter()
        .find(|p| p.id().to_string() == device_id)
        .ok_or_else(|| format!("BLE device '{}' not found. Rescan required.", device_id))?;

    peripheral
        .connect()
        .await
        .map_err(|e| format!("BLE connect failed: {}", e))?;
    peripheral
        .discover_services()
        .await
        .map_err(|e| format!("BLE service discovery failed: {}", e))?;

    let device_name = peripheral
        .properties()
        .await
        .ok()
        .flatten()
        .and_then(|p| p.local_name)
        .unwrap_or_else(|| "Unknown".to_string());

    // Build the GATT table + subscribe to every Notify/Indicate characteristic.
    let services = peripheral.services();
    let mut table_services = Vec::new();
    let mut subscribed_count = 0usize;

    // Collect ALL writable vendor-service characteristics so the listen path can transmit (e.g. the
    // experimental MSP-over-SmartPort probe). We don't know which characteristic the radio's bridge
    // routes to the FC uplink, so during this trial phase we write to every candidate (e.g. FrSky
    // X20RS exposes both 0xFFF3 and 0xFFF6) and let the probe's reply detection reveal what works.
    let mut write_chars: Vec<(btleplug::api::Characteristic, WriteType)> = Vec::new();
    let mut subscribed_chars: Vec<btleplug::api::Characteristic> = Vec::new();

    for service in &services {
        // Skip standard SIG services — subscribing to their characteristics (esp. Service Changed,
        // 0x2A05) triggers a spurious WinRT pairing/PIN prompt. Telemetry is always on a vendor service.
        let skip_service = is_standard_gatt_service(service.uuid);
        let mut chars = Vec::new();
        for ch in &service.characteristics {
            let notifiable = ch
                .properties
                .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE);
            // Collect every writable vendor characteristic as a transmit candidate.
            if !skip_service {
                let can_wnr = ch.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE);
                let can_w = ch.properties.contains(CharPropFlags::WRITE);
                if can_wnr || can_w {
                    let wt = if can_wnr { WriteType::WithoutResponse } else { WriteType::WithResponse };
                    write_chars.push((ch.clone(), wt));
                }
            }
            let mut subscribed = false;
            if notifiable && skip_service {
                log::info!(
                    "BLE listen: skipping standard-service char {} (service {}) — avoids pairing prompt",
                    ch.uuid, service.uuid
                );
            } else if notifiable {
                match peripheral.subscribe(ch).await {
                    Ok(()) => {
                        subscribed = true;
                        subscribed_count += 1;
                        subscribed_chars.push(ch.clone());
                        log::info!("BLE listen: subscribed to {} (service {})", ch.uuid, service.uuid);
                    }
                    Err(e) => log::warn!("BLE listen: subscribe to {} failed: {}", ch.uuid, e),
                }
            }
            chars.push(GattCharInfo {
                uuid: ch.uuid.to_string(),
                properties: char_property_names(ch.properties),
                subscribed,
            });
        }
        table_services.push(GattServiceInfo {
            uuid: service.uuid.to_string(),
            characteristics: chars,
        });
    }

    log::info!(
        "BLE listen: '{}' — {} services, subscribed to {} notify/indicate characteristics",
        device_name,
        table_services.len(),
        subscribed_count
    );
    let _ = app.emit(
        "ble-gatt-services",
        GattTable { device: device_name.clone(), services: table_services },
    );

    let notification_stream = peripheral
        .notifications()
        .await
        .map_err(|e| format!("BLE notification stream failed: {}", e))?;

    if write_chars.is_empty() {
        log::warn!("BLE listen: no writable characteristic — transmit disabled");
    } else {
        let names: Vec<String> = write_chars
            .iter()
            .map(|(c, t)| format!("{} ({:?})", c.uuid, t))
            .collect();
        log::info!("BLE listen: transmit enabled on {} characteristic(s): {}", write_chars.len(), names.join(", "));
    }

    // Channels bridging async BLE → sync ByteTransport. The write channel is now driven if any writable
    // characteristic was found, so the listen path can transmit (e.g. the MSP-over-SmartPort probe).
    let (write_tx, mut write_async_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>();
    let (stop_tx, mut stop_async_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let peripheral_clone = peripheral.clone();
    let app_clone = app.clone();
    let trainer_id = device_id.to_string();
    let trainer_name = device_name.clone();
    tauri::async_runtime::spawn(async move {
        let _listen_guard = listen_guard;
        let mut notifications = notification_stream;
        // A radio in Bluetooth trainer mode sends RC frames, not telemetry: end the link and say so.
        let mut trainer = crate::hid::frsky_trainer::TrainerDecoder::default();
        let mut trainer_frames = Vec::new();
        let mut trainer_seen = 0usize;
        loop {
            loop {
                tokio::select! {
                    maybe = notifications.next() => {
                        match maybe {
                            Some(n) => {
                                trainer.feed(&n.value, &mut trainer_frames);
                                trainer_seen += trainer_frames.len();
                                trainer_frames.clear();
                                if trainer_seen >= TRAINER_FRAMES_TO_HAND_OVER {
                                    log::warn!("BLE listen: '{}' sends trainer frames, not telemetry — closing the link", trainer_name);
                                    let _ = app_clone.emit("ble-trainer-detected", TrainerDetected {
                                        id: trainer_id.clone(),
                                        name: trainer_name.clone(),
                                    });
                                    let _ = peripheral_clone.disconnect().await;
                                    return;
                                }
                                let _ = app_clone.emit("ble-gatt-char-data", GattCharData {
                                    uuid: n.uuid.to_string(),
                                    len: n.value.len(),
                                });
                                if !n.value.is_empty() {
                                    let _ = read_tx.send(n.value);
                                }
                            }
                            None => {
                                log::warn!("BLE listen: notification stream ended — link lost, reconnecting");
                                break;
                            }
                        }
                    }
                    Some(data) = write_async_rx.recv() => {
                        if write_chars.is_empty() {
                            log::warn!("BLE listen: write requested but no writable characteristic — dropping {} bytes", data.len());
                        } else {
                            // Trial phase: write to every writable candidate (we don't yet know which one
                            // the radio routes to the FC uplink).
                            for (wc, wt) in &write_chars {
                                for chunk in data.chunks(BLE_WRITE_MTU) {
                                    if let Err(e) = peripheral_clone.write(wc, chunk, *wt).await {
                                        log::error!("BLE listen write to {} failed: {}", wc.uuid, e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    _ = stop_async_rx.recv() => {
                        log::info!("BLE listen runtime stopping");
                        let _ = peripheral_clone.disconnect().await;
                        return;
                    }
                }
            }

            match reconnect_until_up(&adapter, &peripheral_clone, &subscribed_chars, &mut stop_async_rx, &trainer_name).await {
                Some(stream) => {
                    notifications = stream;
                    while write_async_rx.try_recv().is_ok() {} // drop writes queued during the gap
                    let _ = read_tx.send(STREAM_RESTART);
                }
                None => return,
            }
        }
    });

    Ok(BleTransport {
        device_name,
        profile_name: "Listen (auto)".to_string(),
        write_tx,
        read_rx,
        stop_tx,
        read_buffer: Vec::new(),
        read_timeout: Duration::from_millis(100),
        restarted: false,
    })
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if !self.read_buffer.is_empty() {
            let n = std::cmp::min(buf.len(), self.read_buffer.len());
            buf[..n].copy_from_slice(&self.read_buffer[..n]);
            self.read_buffer.drain(..n);
            return Ok(n);
        }

        match self.read_rx.recv_timeout(self.read_timeout) {
            // In order with the bytes: everything before the marker came from the old stream.
            Ok(data) if data.is_empty() => {
                self.restarted = true;
                Ok(0)
            }
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

    fn set_read_timeout(&mut self, timeout: std::time::Duration) {
        self.read_timeout = timeout;
    }

    fn take_stream_restart(&mut self) -> bool {
        std::mem::take(&mut self.restarted)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restart_is_reported_between_the_old_and_the_new_bytes() {
        let (write_tx, _write_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, _stop_rx) = tokio::sync::mpsc::unbounded_channel();
        let (read_tx, read_rx) = mpsc::channel();
        let mut link = BleTransport {
            device_name: "test".into(),
            profile_name: "test".into(),
            write_tx,
            read_rx,
            stop_tx,
            read_buffer: Vec::new(),
            read_timeout: Duration::from_millis(10),
            restarted: false,
        };
        read_tx.send(b"old".to_vec()).unwrap();
        read_tx.send(STREAM_RESTART).unwrap();
        read_tx.send(b"new".to_vec()).unwrap();

        let mut buf = [0u8; 2];
        let mut read = |link: &mut BleTransport| {
            let n = link.read_bytes(&mut buf).expect("the link is open");
            (buf[..n].to_vec(), link.take_stream_restart())
        };
        assert_eq!(read(&mut link), (b"ol".to_vec(), false));
        assert_eq!(read(&mut link), (b"d".to_vec(), false));
        assert_eq!(read(&mut link), (Vec::new(), true));
        assert_eq!(read(&mut link), (b"ne".to_vec(), false));
        assert_eq!(read(&mut link), (b"w".to_vec(), false));

        drop(read_tx);
        assert!(matches!(link.read_bytes(&mut buf), Err(TransportError::Disconnected)));
    }
}
