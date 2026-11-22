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
// on them (writeValue, retrievePeripherals, connect) are internally thread-safe in CoreBluetooth, so they are
// wrapped in `SendPtr` with that invariant documented.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicProperties,
    CBCharacteristicWriteType, CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService, CBUUID,
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

/// Generic Access, Generic Attribute, Device Information never carry telemetry.
const STANDARD_SERVICES: [&str; 3] = ["1800", "1801", "180A"];

/// Pause before the next connect request after a loss or a failed setup; doubles per failure up to the cap.
const RECONNECT_FIRST: Duration = Duration::from_millis(500);
const RECONNECT_CAP: Duration = Duration::from_secs(5);

/// Device summary returned to the frontend (same shape as the desktop `BleDeviceInfo`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDeviceInfo {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub rssi: Option<i32>,
}

// ── Shared, thread-safe pointer wrapper for the CoreBluetooth objects ────────────────────────────

/// CoreBluetooth objects are `!Send`, but the calls we make on them (writeValue, connect, cancelConnection,
/// retrievePeripherals, setDelegate) are safe to invoke from any thread - CoreBluetooth serialises internally.
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
    /// Set once the first connect is done; from then on the delegate re-establishes a dropped link.
    session: Option<Session>,
    /// Bluetooth will not come back by itself (off, not allowed, unsupported): the link ends.
    unavailable: Option<&'static str>,
}

/// What a reconnect subscribes to: the profile's read characteristic, or every vendor notifier.
#[derive(Clone, Copy)]
enum Setup {
    Profile(&'static BleProfile),
    Listen,
}

enum Phase {
    Up,
    /// Down; the transport sends the next connect request at `next_attempt`.
    Waiting,
    /// Has no timeout: CoreBluetooth completes it once the device is back in range.
    Connecting,
    Discovering,
    Characteristics { pending: usize, subscribed: usize },
}

struct Session {
    id: String,
    setup: Setup,
    central: SendPtr<CBCentralManager>,
    phase: Phase,
    /// `None` while down and on a listen-only link.
    write_char: Option<SendPtr<CBCharacteristic>>,
    backoff: Duration,
    next_attempt: Instant,
    attempt_started: Instant,
    /// Set by a reconnect: a loss within `RECONNECT_CAP` of it logs at debug and keeps the backoff
    /// growing, so a device that connects and drops cannot flood the log.
    unproven: bool,
    up_since: Instant,
    restarted: bool,
    ended: bool,
}

impl Session {
    fn lost(&mut self, reason: &str) {
        self.phase = Phase::Waiting;
        self.write_char = None;
        if self.unproven && self.up_since.elapsed() < RECONNECT_CAP {
            self.backoff = (self.backoff * 2).min(RECONNECT_CAP);
            log::debug!("[ble] BLE link to {} lost again ({reason}) — retry in {:?}", self.id, self.backoff);
        } else {
            self.unproven = false;
            self.backoff = RECONNECT_FIRST;
            log::warn!("[ble] BLE link to {} lost ({reason}) — reconnecting", self.id);
        }
        self.next_attempt = Instant::now() + self.backoff;
    }

    /// The caller cancels the connection if the attempt got that far.
    fn failed(&mut self, reason: &str) {
        self.phase = Phase::Waiting;
        self.write_char = None;
        self.backoff = (self.backoff * 2).min(RECONNECT_CAP);
        self.next_attempt = Instant::now() + self.backoff;
        log::debug!(
            "[ble] reconnect to {} failed after {:.1?}: {reason} — retry in {:?}",
            self.id,
            self.attempt_started.elapsed(),
            self.backoff
        );
    }
}

/// Bytes still queued from the old link end mid-frame; the reader sees `restarted` with the first new byte.
fn come_up(inner: &mut SharedInner, write_char: Option<Retained<CBCharacteristic>>) {
    inner.rx.clear();
    let Some(s) = inner.session.as_mut() else { return };
    // At the level of the loss: a default-level log must show both.
    let level = if s.unproven { log::Level::Debug } else { log::Level::Warn };
    log::log!(level, "[ble] BLE link to {} re-established (attempt took {:.1?})", s.id, s.attempt_started.elapsed());
    s.phase = Phase::Up;
    s.write_char = write_char.map(SendPtr);
    s.unproven = true;
    s.up_since = Instant::now();
    s.restarted = true;
}

fn unavailable_reason(state: CBManagerState) -> Option<&'static str> {
    if state == CBManagerState::PoweredOff {
        Some("Bluetooth is turned off")
    } else if state == CBManagerState::Unauthorized {
        Some("Bluetooth permission revoked")
    } else if state == CBManagerState::Unsupported {
        Some("this device has no Bluetooth LE")
    } else {
        None
    }
}

fn describe_error(error: Option<&AnyObject>) -> String {
    let Some(error) = error else { return "no error reported".into() };
    let code: isize = unsafe { msg_send![error, code] };
    let text: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
    format!("error {code}: {}", text.map(|t| t.to_string()).unwrap_or_default())
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
            let state = unsafe { central.state() };
            let on = state == CBManagerState::PoweredOn;
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.powered_on = on;
            g.unavailable = unavailable_reason(state);
            // Resetting / Unknown: connections and pending requests are gone; reissued once powered on.
            if !on && g.unavailable.is_none() {
                if let Some(s) = g.session.as_mut() {
                    let reason = format!("Bluetooth state {}", state.0);
                    match s.phase {
                        Phase::Up => s.lost(&reason),
                        Phase::Waiting => {}
                        _ => s.failed(&reason),
                    }
                }
            }
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
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connected = true;
            let discover = match g.session.as_mut() {
                Some(s) if matches!(s.phase, Phase::Connecting) => {
                    s.phase = Phase::Discovering;
                    true
                }
                _ => false,
            };
            sh.cv.notify_all();
            drop(g);
            if discover {
                unsafe { peripheral.discoverServices(None) };
            }
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_connect(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            error: Option<&AnyObject>,
        ) {
            let reason = describe_error(error);
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connect_failed = Some("connection failed".into());
            if let Some(s) = g.session.as_mut().filter(|s| matches!(s.phase, Phase::Connecting)) {
                s.failed(&format!("connect: {reason}"));
            }
            sh.cv.notify_all();
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            error: Option<&AnyObject>,
        ) {
            let reason = describe_error(error);
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.connected = false;
            let ending = g.unavailable.is_some();
            if let Some(s) = g.session.as_mut() {
                match s.phase {
                    // With Bluetooth unavailable the next read ends the link; no "reconnecting" line first.
                    Phase::Up if !ending => s.lost(&reason),
                    Phase::Up | Phase::Waiting => {}
                    _ => s.failed(&format!("link dropped during setup: {reason}")),
                }
            }
            sh.cv.notify_all();
        }
    }

    unsafe impl CBPeripheralDelegate for BleDelegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, peripheral: &CBPeripheral, error: Option<&AnyObject>) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.services_done = true;
            sh.cv.notify_all();
            let Some(setup) = g.session.as_ref().filter(|s| matches!(s.phase, Phase::Discovering)).map(|s| s.setup)
            else {
                return;
            };
            drop(g);
            let found = match error {
                Some(_) => Err(format!("service discovery: {}", describe_error(error))),
                None => reconnect_services(peripheral, setup),
            };
            let mut g = sh.inner.lock().unwrap();
            let Some(s) = g.session.as_mut().filter(|s| matches!(s.phase, Phase::Discovering)) else { return };
            match found {
                Ok(services) => {
                    s.phase = Phase::Characteristics { pending: services.len(), subscribed: 0 };
                    drop(g);
                    for svc in &services {
                        unsafe { peripheral.discoverCharacteristics_forService(None, svc) };
                    }
                }
                Err(reason) => {
                    s.failed(&reason);
                    let central = s.central.0.clone();
                    drop(g);
                    unsafe { central.cancelPeripheralConnection(peripheral) };
                }
            }
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_chars(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            _error: Option<&AnyObject>,
        ) {
            let sh = &self.ivars().shared;
            let mut g = sh.inner.lock().unwrap();
            g.chars_done += 1;
            sh.cv.notify_all();
            let Some(setup) = g
                .session
                .as_ref()
                .filter(|s| matches!(s.phase, Phase::Characteristics { .. }))
                .map(|s| s.setup)
            else {
                return;
            };
            drop(g);
            resubscribe(sh, peripheral, service, setup);
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

/// The services of a fresh discovery that a reconnect needs characteristics from.
fn reconnect_services(peripheral: &CBPeripheral, setup: Setup) -> Result<Vec<Retained<CBService>>, String> {
    let services = unsafe { peripheral.services() }.ok_or_else(|| "no services".to_string())?;
    let mut wanted = Vec::new();
    for i in 0..services.count() {
        let svc = services.objectAtIndex(i);
        let su = unsafe { svc.UUID() };
        match setup {
            Setup::Profile(p) if uuid_eq(&su, p.service) => return Ok(vec![svc]),
            Setup::Listen if !STANDARD_SERVICES.iter().any(|s| uuid_eq(&su, s)) => wanted.push(svc),
            _ => {}
        }
    }
    match setup {
        _ if !wanted.is_empty() => Ok(wanted),
        Setup::Profile(p) => Err(format!("service {} not on device", p.service)),
        Setup::Listen => Err("no vendor service on device".into()),
    }
}

/// Subscribes in `service` what the first connect subscribed; the last outstanding service brings the
/// link up, or fails the attempt when nothing could be subscribed.
fn resubscribe(shared: &Shared, peripheral: &CBPeripheral, service: &CBService, setup: Setup) {
    let chars: Vec<Retained<CBCharacteristic>> = match unsafe { service.characteristics() } {
        Some(cs) => (0..cs.count()).map(|i| cs.objectAtIndex(i)).collect(),
        None => Vec::new(),
    };
    let find = |uuid: &str| {
        chars
            .iter()
            .find(|c| {
                let cu = unsafe { c.UUID() };
                uuid_eq(&cu, uuid)
            })
            .cloned()
    };
    let (subscribe, write_char) = match setup {
        Setup::Profile(p) => match (find(p.read), find(p.write)) {
            (Some(read), Some(write)) => (vec![read], Some(write)),
            _ => (Vec::new(), None),
        },
        Setup::Listen => {
            let notifies = CBCharacteristicProperties::Notify | CBCharacteristicProperties::Indicate;
            let notifying = chars.iter().filter(|c| unsafe { c.properties() }.intersects(notifies)).cloned().collect();
            (notifying, None)
        }
    };
    for c in &subscribe {
        unsafe { peripheral.setNotifyValue_forCharacteristic(true, c) };
    }

    let mut g = shared.inner.lock().unwrap();
    let inner = &mut *g;
    let Some(s) = inner.session.as_mut() else { return };
    let (left, total) = match &mut s.phase {
        Phase::Characteristics { pending, subscribed } => {
            *pending = pending.saturating_sub(1);
            *subscribed += subscribe.len();
            (*pending, *subscribed)
        }
        _ => return,
    };
    if left > 0 {
        return;
    }
    if total == 0 {
        s.failed(match setup {
            Setup::Profile(_) => "read or write characteristic not found",
            Setup::Listen => "nothing on this device notifies",
        });
        let central = s.central.0.clone();
        drop(g);
        unsafe { central.cancelPeripheralConnection(peripheral) };
        return;
    }
    come_up(inner, write_char);
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
    BleTransport::open(device_id, false).await
}

/// Listen-only connect, no profile needed: a radio's telemetry sits on a vendor service (FrSky:
/// 0xFFF0), so every notifying characteristic outside the standard services is subscribed.
pub async fn connect_ble_listen(device_id: &str, _app: tauri::AppHandle) -> Result<BleTransport, String> {
    BleTransport::open(device_id, true).await
}

// ── Transport ───────────────────────────────────────────────────────────────────────────────────

/// A connected BLE serial link (GATT). Reads drain the notify buffer; writes go to the write char.
/// A lost link is re-established underneath (the layers above see a stall); `Disconnected` only once
/// Bluetooth is off, not allowed or unsupported.
pub struct BleTransport {
    shared: Arc<Shared>,
    delegate: SendPtr<BleDelegate>,
    central: SendPtr<CBCentralManager>,
    /// The object of the last connect request; replaced by each reconnect.
    peripheral: SendPtr<CBPeripheral>,
    device_id: String,
    listen: bool,
    profile_name: String,
    read_timeout: Duration,
    restarted: bool,
}

impl BleTransport {
    async fn open(device_id: &str, listen: bool) -> Result<Self, String> {
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

        if listen {
            let mut vendor: Vec<Retained<CBService>> = Vec::new();
            for i in 0..services.count() {
                let svc = services.objectAtIndex(i);
                let su = unsafe { svc.UUID() };
                if STANDARD_SERVICES.iter().any(|s| uuid_eq(&su, s)) {
                    continue;
                }
                unsafe { peripheral.discoverCharacteristics_forService(None, &svc) };
                vendor.push(svc);
            }
            let wanted = vendor.len();
            if !wait_until(&shared, Duration::from_secs(10), |s| s.chars_done >= wanted) {
                return Err("characteristic discovery timed out".into());
            }
            let mut subscribed = 0usize;
            for svc in &vendor {
                let Some(chars) = (unsafe { svc.characteristics() }) else { continue };
                for i in 0..chars.count() {
                    let c = chars.objectAtIndex(i);
                    let notifies = unsafe { c.properties() }
                        .intersects(CBCharacteristicProperties::Notify | CBCharacteristicProperties::Indicate);
                    log::info!(
                        "[ble] listen: service {} characteristic {} notifies={notifies}",
                        unsafe { svc.UUID().UUIDString() },
                        unsafe { c.UUID().UUIDString() },
                    );
                    if notifies {
                        unsafe { peripheral.setNotifyValue_forCharacteristic(true, &c) };
                        subscribed += 1;
                    }
                }
            }
            if subscribed == 0 {
                unsafe { central.cancelPeripheralConnection(&peripheral) };
                return Err("nothing on this device notifies".into());
            }
            return Ok(Self::start(device_id, shared, delegate, central, peripheral, Setup::Listen, None));
        }

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

        Ok(Self::start(device_id, shared, delegate, central, peripheral, Setup::Profile(profile), Some(write_char)))
    }

    /// Hands the first connect's link to the session that re-establishes it after a loss.
    fn start(
        device_id: &str,
        shared: Arc<Shared>,
        delegate: Retained<BleDelegate>,
        central: Retained<CBCentralManager>,
        peripheral: Retained<CBPeripheral>,
        setup: Setup,
        write_char: Option<Retained<CBCharacteristic>>,
    ) -> Self {
        let now = Instant::now();
        let mut session = Session {
            id: device_id.to_string(),
            setup,
            central: SendPtr(central.clone()),
            phase: Phase::Up,
            write_char: write_char.map(SendPtr),
            backoff: RECONNECT_FIRST,
            next_attempt: now,
            attempt_started: now,
            unproven: false,
            up_since: now,
            restarted: false,
            ended: false,
        };
        {
            let mut g = shared.inner.lock().unwrap();
            // A drop during the last setup steps came before there was a session to notice it.
            if !g.connected {
                session.lost("dropped while connecting");
            }
            g.session = Some(session);
        }
        let profile_name = match setup {
            Setup::Profile(p) => p.name.to_string(),
            Setup::Listen => "listen-only".to_string(),
        };
        Self {
            shared,
            delegate: SendPtr(delegate),
            central: SendPtr(central),
            peripheral: SendPtr(peripheral),
            device_id: device_id.to_string(),
            listen: matches!(setup, Setup::Listen),
            profile_name,
            read_timeout: Duration::from_millis(200),
            restarted: false,
        }
    }

    /// Ends the link once Bluetooth is unavailable; sends the next connect request when one is due.
    fn service_reconnect(&mut self) -> Result<(), TransportError> {
        let mut g = self.shared.inner.lock().unwrap();
        let (unavailable, powered_on) = (g.unavailable, g.powered_on);
        let Some(s) = g.session.as_mut() else { return Ok(()) };
        if s.ended {
            return Err(TransportError::Disconnected);
        }
        if let Some(reason) = unavailable {
            s.ended = true;
            log::warn!("[ble] BLE link to {}: {reason} — closing the link", s.id);
            return Err(TransportError::Disconnected);
        }
        if !matches!(s.phase, Phase::Waiting) || !powered_on || Instant::now() < s.next_attempt {
            return Ok(());
        }
        s.phase = Phase::Connecting;
        s.attempt_started = Instant::now();
        drop(g);

        // Retrieved anew each time: a Bluetooth reset invalidates the peripheral objects.
        let peripheral = NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(&self.device_id))
            .and_then(|id| {
                let ids = NSArray::from_retained_slice(&[id]);
                unsafe { self.central.0.retrievePeripheralsWithIdentifiers(&ids) }.firstObject()
            });
        let Some(peripheral) = peripheral else {
            let mut g = self.shared.inner.lock().unwrap();
            if let Some(s) = g.session.as_mut().filter(|s| matches!(s.phase, Phase::Connecting)) {
                s.failed("the system no longer knows the peripheral");
            }
            return Ok(());
        };
        unsafe {
            peripheral.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate.0)));
            self.central.0.connectPeripheral_options(&peripheral, None);
        }
        self.peripheral = SendPtr(peripheral);
        Ok(())
    }
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.service_reconnect()?;
        // While the link is down this waits out the timeout: the caller loops on `Ok(0)` without pausing.
        wait_until(&self.shared, self.read_timeout, |s| !s.rx.is_empty() || s.unavailable.is_some());
        let mut g = self.shared.inner.lock().unwrap();
        if let Some(s) = g.session.as_mut() {
            if std::mem::take(&mut s.restarted) {
                self.restarted = true;
            }
        }
        if g.rx.is_empty() {
            return Ok(0); // timeout, non-fatal
        }
        let n = buf.len().min(g.rx.len());
        for slot in buf.iter_mut().take(n) {
            *slot = g.rx.pop_front().unwrap();
        }
        Ok(n)
    }

    /// While the link is down the data is dropped, like on any stalled link.
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.service_reconnect()?;
        if self.listen {
            return Err(TransportError::Io("listen-only BLE link has no write characteristic".into()));
        }
        let write_char = {
            let g = self.shared.inner.lock().unwrap();
            g.session.as_ref().and_then(|s| s.write_char.as_ref()).map(|c| c.0.clone())
        };
        let Some(write_char) = write_char else { return Ok(()) };
        let nsdata = NSData::with_bytes(data);
        unsafe {
            self.peripheral.0.writeValue_forCharacteristic_type(
                &nsdata,
                &write_char,
                CBCharacteristicWriteType::WithoutResponse,
            );
        }
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }

    fn take_stream_restart(&mut self) -> bool {
        std::mem::take(&mut self.restarted)
    }

    fn description(&self) -> String {
        format!("BLE ({})", self.profile_name)
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        // Without a session the delegate drives nothing further; the cancel also covers a pending connect.
        self.shared.inner.lock().unwrap_or_else(|e| e.into_inner()).session = None;
        unsafe { self.central.0.cancelPeripheralConnection(&self.peripheral.0) };
    }
}
