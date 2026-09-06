// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// iOS BLE transport via CoreBluetooth (btleplug has no iOS backend). Implemented in Rust with the
// objc2 CoreBluetooth bindings: a Rust CBCentralManager/CBPeripheral delegate feeds a byte ring that
// the ByteTransport read path drains, matching the desktop btleplug module's public surface
// (scan_ble_devices / connect_ble / BleTransport) so the connection layer stays platform-agnostic.
//
// NOTE: this backend compiles for aarch64-apple-ios but is not yet validated against real BLE
// hardware - the delegate wiring and characteristic handshake need an on-device pass with a BLE FC.
//
// Threading: CoreBluetooth delivers delegate callbacks on the main dispatch queue (created with
// queue = nil), while the transport read/write happens on the scheduler thread. Shared state is
// guarded by a Mutex + Condvar. The CoreBluetooth objects are not Send, but the operations we invoke
// on them (writeValue, retrievePeripherals) are internally thread-safe in CoreBluetooth, so they are
// wrapped in `SendPtr` with that invariant documented.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType,
    CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString, NSUUID};

use super::{ByteTransport, TransportError};

// ── Known serial profiles (mirrors transport::ble::known_profiles) ──────────────────────────────

struct BleProfile {
    name: &'static str,
    service: &'static str,
    write: &'static str,
    read: &'static str,
}

const PROFILES: &[BleProfile] = &[
    BleProfile { name: "HM-10/HC-08 (CC254x)", service: "FFE0", write: "FFE1", read: "FFE1" },
    BleProfile {
        name: "Nordic UART (NUS)",
        service: "6E400001-B5A3-F393-E0A9-E50E24DCCA9E",
        write: "6E400002-B5A3-F393-E0A9-E50E24DCCA9E",
        read: "6E400003-B5A3-F393-E0A9-E50E24DCCA9E",
    },
    BleProfile { name: "SpeedyBee", service: "ABF0", write: "ABF1", read: "ABF2" },
    BleProfile { name: "Generic 1000", service: "1000", write: "1001", read: "1002" },
];

/// Device summary returned to the frontend (same shape as the desktop `BleDeviceInfo`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDeviceInfo {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub rssi: Option<i32>,
}

// ── Shared, thread-safe pointer wrapper for the CoreBluetooth objects ────────────────────────────

/// CoreBluetooth objects are `!Send`, but the calls we make on them (writeValue, cancelConnection,
/// retrievePeripherals) are safe to invoke from any thread - CoreBluetooth serialises internally.
struct SendPtr<T>(Retained<T>);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

// ── Delegate shared state ───────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Discovered {
    id: String,
    name: String,
    rssi: Option<i32>,
    service_uuids: Vec<String>,
}

#[derive(Default)]
struct SharedInner {
    powered_on: bool,
    discovered: Vec<Discovered>,
    connected: bool,
    connect_failed: Option<String>,
    services_done: bool,
    chars_done: usize,
    rx: VecDeque<u8>,
}

struct Shared {
    inner: Mutex<SharedInner>,
    cv: Condvar,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(SharedInner::default()), cv: Condvar::new() })
    }
}

struct Ivars {
    shared: Arc<Shared>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "KiteBleDelegate"]
    #[ivars = Ivars]
    struct BleDelegate;

    unsafe impl NSObjectProtocol for BleDelegate {}

    unsafe impl CBCentralManagerDelegate for BleDelegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let on = unsafe { central.state() } == CBManagerState::PoweredOn;
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.powered_on = on;
            sh.cv.notify_all();
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            _adv: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            let id = unsafe { peripheral.identifier().UUIDString() }.to_string();
            let name = unsafe { peripheral.name() }.map(|n| n.to_string()).unwrap_or_default();
            let rssi_val = rssi.as_i32();
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            if !g.discovered.iter().any(|d| d.id == id) {
                g.discovered.push(Discovered { id, name, rssi: Some(rssi_val), service_uuids: vec![] });
                sh.cv.notify_all();
            }
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, _peripheral: &CBPeripheral) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connected = true;
            sh.cv.notify_all();
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_connect(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            _error: Option<&AnyObject>,
        ) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connect_failed = Some("connection failed".into());
            sh.cv.notify_all();
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            _error: Option<&AnyObject>,
        ) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connected = false;
            sh.cv.notify_all();
        }
    }

    unsafe impl CBPeripheralDelegate for BleDelegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, _peripheral: &CBPeripheral, _error: Option<&AnyObject>) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.services_done = true;
            sh.cv.notify_all();
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_chars(
            &self,
            _peripheral: &CBPeripheral,
            _service: &CBService,
            _error: Option<&AnyObject>,
        ) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.chars_done += 1;
            sh.cv.notify_all();
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(
            &self,
            _peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            _error: Option<&AnyObject>,
        ) {
            if let Some(data) = unsafe { characteristic.value() } {
                let bytes = data.to_vec();
                let sh = &self.ivars().shared;
                let mut g = sh.inner.lock().unwrap();
                g.rx.extend(bytes);
                sh.cv.notify_all();
            }
        }
    }
);

impl BleDelegate {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { shared });
        unsafe { msg_send![super(this), init] }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

fn uuid_eq(a: &CBUUID, s: &str) -> bool {
    unsafe { a.UUIDString() }.to_string().eq_ignore_ascii_case(s)
}

/// Wait until `pred` is true or `timeout` elapses. Returns false on timeout.
fn wait_until<F: Fn(&SharedInner) -> bool>(shared: &Shared, timeout: Duration, pred: F) -> bool {
    let deadline = Instant::now() + timeout;
    let mut g = shared.inner.lock().unwrap();
    while !pred(&g) {
        let now = Instant::now();
        if now >= deadline {
            return pred(&g);
        }
        let (ng, _) = shared.cv.wait_timeout(g, deadline - now).unwrap();
        g = ng;
    }
    true
}

// ── Public API (mirrors transport::ble) ─────────────────────────────────────────────────────────

/// Scan for BLE peripherals for a few seconds and return those seen, tagging any that advertise a
/// known serial service. Runs a short-lived central; CoreBluetooth callbacks land on the main queue.
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    let shared = Shared::new();
    let delegate = BleDelegate::new(shared.clone());
    let central: Retained<CBCentralManager> = unsafe {
        let proto = ProtocolObject::from_ref(&*delegate);
        CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(proto), None)
    };

    if !wait_until(&shared, Duration::from_secs(5), |s| s.powered_on) {
        return Err("Bluetooth not powered on".into());
    }
    unsafe { central.scanForPeripheralsWithServices_options(None, None) };
    std::thread::sleep(Duration::from_secs(3));
    unsafe { central.stopScan() };

    let g = shared.inner.lock().unwrap();
    let mut out: Vec<BleDeviceInfo> = g
        .discovered
        .iter()
        .filter(|d| !d.name.is_empty())
        .map(|d| {
            let profile = PROFILES
                .iter()
                .find(|p| d.service_uuids.iter().any(|u| u.eq_ignore_ascii_case(p.service)))
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| "Unknown".into());
            BleDeviceInfo { id: d.id.clone(), name: d.name.clone(), profile, rssi: d.rssi }
        })
        .collect();
    out.sort_by(|a, b| {
        let ak = a.profile != "Unknown";
        let bk = b.profile != "Unknown";
        bk.cmp(&ak).then(b.rssi.cmp(&a.rssi))
    });
    let _ = central; // keep alive until here
    Ok(out)
}

/// Live scan session: emits a `ble-device` event per newly-seen peripheral until `stop_rx` resolves
/// (mirrors the desktop module so the frontend picker is unchanged on iOS).
pub async fn run_scan_session(
    app: tauri::AppHandle,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    use tauri::Emitter;
    let shared = Shared::new();
    let delegate = SendPtr(BleDelegate::new(shared.clone()));
    let central = {
        let proto = ProtocolObject::from_ref(&*delegate.0);
        SendPtr(unsafe {
            CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(proto), None)
        })
    };
    if !wait_until(&shared, Duration::from_secs(5), |s| s.powered_on) {
        return Err("Bluetooth not powered on".into());
    }
    unsafe { central.0.scanForPeripheralsWithServices_options(None, None) };

    let mut emitted: HashSet<String> = HashSet::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(400));
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            _ = ticker.tick() => {
                let list: Vec<BleDeviceInfo> = {
                    let g = shared.inner.lock().unwrap();
                    g.discovered.iter().filter(|d| !d.name.is_empty()).map(|d| {
                        let profile = PROFILES
                            .iter()
                            .find(|p| d.service_uuids.iter().any(|u| u.eq_ignore_ascii_case(p.service)))
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|| "Unknown".into());
                        BleDeviceInfo { id: d.id.clone(), name: d.name.clone(), profile, rssi: d.rssi }
                    }).collect()
                };
                for dev in list {
                    if emitted.insert(dev.id.clone()) {
                        let _ = app.emit("ble-device", dev);
                    }
                }
            }
        }
    }
    unsafe { central.0.stopScan() };
    Ok(())
}

/// Connect to a peripheral by its identifier UUID, discover a known serial profile, subscribe to the
/// read characteristic and return a byte transport.
pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    BleTransport::open(device_id).await
}

/// Passive-listen connect. iOS: same path as connect_ble (the profile handshake already subscribes to
/// the read characteristic). Kept for interface parity with the desktop module.
pub async fn connect_ble_listen(device_id: &str, _app: tauri::AppHandle) -> Result<BleTransport, String> {
    BleTransport::open(device_id).await
}

// ── Transport ───────────────────────────────────────────────────────────────────────────────────

/// A connected BLE serial link (GATT). Reads drain the notify buffer; writes go to the write char.
pub struct BleTransport {
    shared: Arc<Shared>,
    _delegate: SendPtr<BleDelegate>,
    central: SendPtr<CBCentralManager>,
    peripheral: SendPtr<CBPeripheral>,
    write_char: SendPtr<CBCharacteristic>,
    profile_name: String,
    read_timeout: Duration,
}

impl BleTransport {
    async fn open(device_id: &str) -> Result<Self, String> {
        let shared = Shared::new();
        let delegate = BleDelegate::new(shared.clone());
        let central: Retained<CBCentralManager> = unsafe {
            let proto = ProtocolObject::from_ref(&*delegate);
            CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(proto), None)
        };
        if !wait_until(&shared, Duration::from_secs(5), |s| s.powered_on) {
            return Err("Bluetooth not powered on".into());
        }

        // Resolve the peripheral from its identifier UUID.
        let nsuuid = NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(device_id))
            .ok_or_else(|| "invalid device id".to_string())?;
        let ids = NSArray::from_retained_slice(&[nsuuid]);
        let peripherals = unsafe { central.retrievePeripheralsWithIdentifiers(&ids) };
        let peripheral = peripherals.firstObject().ok_or_else(|| "peripheral not found".to_string())?;

        // Connect.
        unsafe { central.connectPeripheral_options(&peripheral, None) };
        if !wait_until(&shared, Duration::from_secs(15), |s| s.connected || s.connect_failed.is_some()) {
            return Err("connect timed out".into());
        }
        {
            let g = shared.inner.lock().unwrap();
            if let Some(e) = &g.connect_failed {
                return Err(e.clone());
            }
        }

        // Discover services + characteristics.
        let proto = ProtocolObject::from_ref(&*delegate);
        unsafe { peripheral.setDelegate(Some(proto)) };
        unsafe { peripheral.discoverServices(None) };
        if !wait_until(&shared, Duration::from_secs(10), |s| s.services_done) {
            return Err("service discovery timed out".into());
        }

        let services = unsafe { peripheral.services() }.ok_or_else(|| "no services".to_string())?;
        // Find the first known profile whose service is present.
        let mut chosen: Option<(&BleProfile, Retained<CBService>)> = None;
        for i in 0..services.count() {
            let svc = services.objectAtIndex(i);
            let su = unsafe { svc.UUID() };
            if let Some(p) = PROFILES.iter().find(|p| uuid_eq(&su, p.service)) {
                chosen = Some((p, svc));
                break;
            }
        }
        let (profile, service) = chosen.ok_or_else(|| "no known BLE serial profile on device".to_string())?;

        unsafe { peripheral.discoverCharacteristics_forService(None, &service) };
        if !wait_until(&shared, Duration::from_secs(10), |s| s.chars_done > 0) {
            return Err("characteristic discovery timed out".into());
        }

        let chars = unsafe { service.characteristics() }.ok_or_else(|| "no characteristics".to_string())?;
        let mut write_char: Option<Retained<CBCharacteristic>> = None;
        let mut read_char: Option<Retained<CBCharacteristic>> = None;
        for i in 0..chars.count() {
            let c = chars.objectAtIndex(i);
            let cu = unsafe { c.UUID() };
            if uuid_eq(&cu, profile.write) {
                write_char = Some(c.clone());
            }
            if uuid_eq(&cu, profile.read) {
                read_char = Some(c);
            }
        }
        let write_char = write_char.ok_or_else(|| "write characteristic not found".to_string())?;
        let read_char = read_char.ok_or_else(|| "read characteristic not found".to_string())?;

        // Subscribe to notifications on the read characteristic.
        unsafe { peripheral.setNotifyValue_forCharacteristic(true, &read_char) };

        Ok(Self {
            shared,
            _delegate: SendPtr(delegate),
            central: SendPtr(central),
            peripheral: SendPtr(peripheral),
            write_char: SendPtr(write_char),
            profile_name: profile.name.to_string(),
            read_timeout: Duration::from_millis(200),
        })
    }
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let got = wait_until(&self.shared, self.read_timeout, |s| !s.rx.is_empty() || !s.connected);
        let mut g = self.shared.inner.lock().unwrap();
        if !g.connected && g.rx.is_empty() {
            return Err(TransportError::Disconnected);
        }
        if !got || g.rx.is_empty() {
            return Ok(0); // timeout, non-fatal
        }
        let n = buf.len().min(g.rx.len());
        for slot in buf.iter_mut().take(n) {
            *slot = g.rx.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let nsdata = NSData::with_bytes(data);
        unsafe {
            self.peripheral.0.writeValue_forCharacteristic_type(
                &nsdata,
                &self.write_char.0,
                CBCharacteristicWriteType::WithoutResponse,
            );
        }
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }

    fn description(&self) -> String {
        format!("BLE ({})", self.profile_name)
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        unsafe { self.central.0.cancelPeripheralConnection(&self.peripheral.0) };
    }
}
