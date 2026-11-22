// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! BLE transport for Android: the JNI shim in front of BleSerial.kt, same contract as the USB-serial shim
//! (primitives across the boundary, Kotlin owns the device behind an integer handle). The profile table is
//! Rust's (ble_profiles.rs); listen-only mode subscribes without one.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jni::objects::{JByteArray, JString, JValue};

use super::ble_profiles::known_profiles;
use super::{ByteTransport, TransportError};
use crate::android::jvm;

const BRIDGE: &str = "com.kitegc.app.BleSerial";

/// Sentinel handle for errors that belong to no link (scan, permissions) — mirrors the Kotlin side.
const NO_HANDLE: i32 = -1;

/// One-shot scan length. Long enough for slow advertisers (some HM-10 clones beacon at 1 Hz).
const SCAN_MS: u64 = 3000;
/// Polling cadence of the live scan session.
const SCAN_POLL_MS: u64 = 400;
/// Idle read timeout before the scheduler tightens it.
const READ_TIMEOUT_MS: i32 = 100;
/// Per-chunk write acknowledgement timeout. A fading link holds the caller's thread this long per write.
const WRITE_TIMEOUT_MS: i32 = 1000;
/// Pause before the first reconnect attempt after a loss; doubles per failure up to the cap.
const RECONNECT_FIRST: Duration = Duration::from_millis(500);
const RECONNECT_CAP: Duration = Duration::from_secs(5);
const OFF_RECHECK: Duration = Duration::from_secs(2);

/// Information about a discovered BLE device. Same shape as the desktop struct so the frontend's
/// `ble-device` event payload is identical on every platform.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BleDeviceInfo {
    /// Bluetooth address (`AA:BB:CC:DD:EE:FF`) — what Android identifies a peripheral by.
    pub id: String,
    pub name: String,
    pub profile: String,
    pub rssi: Option<i16>,
}

// ── JNI helpers ─────────────────────────────────────────────────────────────────────────────────

fn call_bool(name: &str, sig: &str, args: &[JValue]) -> Result<bool, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.call_static_method(&class, name, sig, args).and_then(|v| v.z());
    jvm::check(&mut env, r, &format!("BleSerial.{name}"))
}

fn call_void(name: &str, sig: &str, args: &[JValue]) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.call_static_method(&class, name, sig, args).map(|_| ());
    jvm::check(&mut env, r, &format!("BleSerial.{name}"))
}

fn call_int(name: &str, sig: &str, args: &[JValue]) -> Result<i32, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.call_static_method(&class, name, sig, args).and_then(|v| v.i());
    jvm::check(&mut env, r, &format!("BleSerial.{name}"))
}

fn call_string(name: &str, sig: &str, args: &[JValue]) -> Result<String, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.call_static_method(&class, name, sig, args).and_then(|v| v.l());
    let value = jvm::check(&mut env, r, &format!("BleSerial.{name}"))?;
    if value.is_null() {
        return Ok(String::new());
    }
    env.get_string(&JString::from(value))
        .map(String::from)
        .map_err(|e| format!("reading BleSerial.{name}'s result: {e}"))
}

/// Fetch-and-clear the Kotlin side's reason for the last failure on `handle`.
fn last_error(handle: i32) -> String {
    match call_string("lastError", "(I)Ljava/lang/String;", &[JValue::Int(handle)]) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => "no reason reported".to_string(),
        Err(e) => e,
    }
}

/// Raise the runtime-permission dialog if needed and wait for the answer (blocking — callers wrap
/// this in `spawn_blocking`).
fn ensure_permissions() -> Result<(), String> {
    if call_bool("ensurePermissions", "()Z", &[])? {
        Ok(())
    } else {
        Err(format!("Bluetooth permission: {}", last_error(NO_HANDLE)))
    }
}

// ── Scanning ────────────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ScanEntry {
    id: String,
    name: String,
    rssi: i32,
    #[serde(default)]
    services: Vec<String>,
}

/// One scan poll → device summaries, tagged with a known profile where the advertisement carried a
/// service UUID (most adapters carry none; those show as "Unknown" and are matched on connect).
fn poll_scan() -> Result<Vec<BleDeviceInfo>, String> {
    let json = call_string("scanPoll", "()Ljava/lang/String;", &[])?;
    let entries: Vec<ScanEntry> =
        serde_json::from_str(&json).map_err(|e| format!("bad scan result from Android: {e}"))?;
    let profiles = known_profiles();
    Ok(entries
        .into_iter()
        .map(|e| {
            let profile = profiles
                .iter()
                .find(|p| {
                    let want = p.service_uuid.to_string();
                    e.services.iter().any(|s| s.eq_ignore_ascii_case(&want))
                })
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            BleDeviceInfo {
                id: e.id,
                name: e.name,
                profile,
                rssi: Some(e.rssi.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
            }
        })
        .collect())
}

fn start_scan() -> Result<(), String> {
    if call_bool("scanStart", "()Z", &[])? {
        Ok(())
    } else {
        Err(format!("BLE scan failed: {}", last_error(NO_HANDLE)))
    }
}

fn stop_scan() {
    if let Err(e) = call_void("scanStop", "()V", &[]) {
        log::warn!("[ble] stopping the scan failed: {e}");
    }
}

/// Scan for a fixed few seconds and return everything named that was seen — known profiles first,
/// then by signal strength, like the desktop backend.
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    tauri::async_runtime::spawn_blocking(ensure_permissions)
        .await
        .map_err(|e| format!("permission task failed: {e}"))??;
    start_scan()?;
    tokio::time::sleep(Duration::from_millis(SCAN_MS)).await;
    let result = poll_scan();
    stop_scan();
    let mut devices = result?;
    devices.sort_by(|a, b| {
        let a_known = a.profile != "Unknown";
        let b_known = b.profile != "Unknown";
        b_known.cmp(&a_known).then_with(|| {
            b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN))
        })
    });
    Ok(devices)
}

/// Live scan session: emits a `ble-device` event per newly seen peripheral until `stop_rx` resolves
/// (the frontend picker is unchanged across platforms).
pub async fn run_scan_session(
    app: tauri::AppHandle,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    use tauri::Emitter;

    tauri::async_runtime::spawn_blocking(ensure_permissions)
        .await
        .map_err(|e| format!("permission task failed: {e}"))??;
    start_scan()?;

    let mut emitted: HashSet<String> = HashSet::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(SCAN_POLL_MS));
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            _ = ticker.tick() => {
                match poll_scan() {
                    Ok(list) => {
                        for dev in list {
                            if emitted.insert(dev.id.clone()) {
                                let _ = app.emit("ble-device", dev);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("[ble] scan poll failed: {e}");
                        break;
                    }
                }
            }
        }
    }
    stop_scan();
    Ok(())
}

// ── Connections ─────────────────────────────────────────────────────────────────────────────────

/// Connect to a peripheral by address, pick the first known serial profile among its services,
/// subscribe to the read characteristic and return a byte transport.
pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    let address = device_id.to_string();
    tauri::async_runtime::spawn_blocking(move || BleTransport::open(&address))
        .await
        .map_err(|e| format!("BLE connect task failed: {e}"))?
}

/// Listen-only connect, no profile needed: a radio's telemetry sits on a vendor service (FrSky:
/// 0xFFF0), so every notifying characteristic outside the standard services is subscribed. Emits the
/// GATT table as `ble-gatt-services` for the Debug Monitor, like the desktop backend.
pub async fn connect_ble_listen(
    device_id: &str,
    app: tauri::AppHandle,
) -> Result<BleTransport, String> {
    use tauri::Emitter;
    let address = device_id.to_string();
    let (transport, services) = tauri::async_runtime::spawn_blocking(move || BleTransport::open_listen(&address))
        .await
        .map_err(|e| format!("BLE connect task failed: {e}"))??;
    let _ = app.emit("ble-gatt-services", serde_json::json!({ "device": device_id, "services": services }));
    Ok(transport)
}

/// What a (re)connect subscribes to.
#[derive(Clone)]
enum Setup {
    Profile { service: String, read: String, write: String },
    /// No write characteristic: writes fail.
    Listen,
}

/// A BLE-serial link; the Kotlin side owns the `BluetoothGatt` behind `handle`. A lost link is
/// re-established underneath (the layers above see a stall); `Disconnected` only once Bluetooth is off
/// or its permission is gone.
pub struct BleTransport {
    address: String,
    /// `None` while the link is down and being re-established.
    handle: Option<i32>,
    setup: Setup,
    profile_name: String,
    write_delay_ms: i32,
    read_timeout_ms: i32,
    backoff: Duration,
    next_attempt: Instant,
    attempt: Option<Arc<Attempt>>,
    /// Set by a reconnect: a loss within `RECONNECT_CAP` of it logs at debug and keeps the backoff
    /// growing, so a device that connects and drops cannot flood the log.
    unproven: bool,
    up_since: Instant,
    restarted: bool,
    ended: bool,
    off_readings: u8,
}

impl BleTransport {
    fn new(address: &str, handle: i32, setup: Setup, profile_name: String, write_delay_ms: i32) -> Self {
        Self {
            address: address.to_string(),
            handle: Some(handle),
            setup,
            profile_name,
            write_delay_ms,
            read_timeout_ms: READ_TIMEOUT_MS,
            backoff: RECONNECT_FIRST,
            next_attempt: Instant::now(),
            attempt: None,
            unproven: false,
            up_since: Instant::now(),
            restarted: false,
            ended: false,
            off_readings: 0,
        }
    }

    /// Blocking: connect and discovery as in `open`, then MTU (≤2 s) and up to 5 s per notifying characteristic.
    fn open_listen(address: &str) -> Result<(Self, serde_json::Value), String> {
        log::info!("[ble] listen-only connect requested: {address}");
        ensure_permissions()?;
        let handle = Self::connect_handle(address)?;
        let json = match call_string("listen", "(I)Ljava/lang/String;", &[JValue::Int(handle)]) {
            Ok(json) => json,
            Err(e) => {
                close_handle(handle, address);
                return Err(e);
            }
        };
        if json.is_empty() {
            let reason = last_error(handle);
            close_handle(handle, address);
            log::warn!("[ble] listen on {address} failed: {reason}");
            return Err(format!("BLE listen failed: {reason}"));
        }
        let services: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => {
                close_handle(handle, address);
                return Err(format!("bad GATT table from Android: {e}"));
            }
        };
        log::info!("[ble] {address} listening (handle {handle}): {json}");
        Ok((Self::new(address, handle, Setup::Listen, "listen-only".to_string(), 0), services))
    }

    fn connect_handle(address: &str) -> Result<i32, String> {
        let env = jvm::env()?;
        let j_addr = env
            .new_string(address)
            .map_err(|e| format!("passing the address to Android: {e}"))?;
        drop(env);
        let handle = call_int("connect", "(Ljava/lang/String;)I", &[JValue::Object(&j_addr)])?;
        if handle < 0 {
            let reason = last_error(NO_HANDLE);
            log::warn!("[ble] connect {address} failed: {reason}");
            return Err(format!("BLE connect failed: {reason}"));
        }
        Ok(handle)
    }

    /// Blocking: permission dialog, connect (≤15 s), service discovery (≤10 s), subscribe (≤7 s).
    fn open(address: &str) -> Result<Self, String> {
        log::info!("[ble] connect requested: {address}");
        ensure_permissions()?;
        let handle = Self::connect_handle(address)?;

        // The device's services decide the profile — most adapters advertise nothing, so this is
        // the authoritative match, not the scan-time tag.
        let services = call_string("services", "(I)Ljava/lang/String;", &[JValue::Int(handle)]).and_then(|json| {
            serde_json::from_str::<Vec<String>>(&json).map_err(|e| format!("bad service list from Android: {e}"))
        });
        let services = match services {
            Ok(services) => services,
            Err(e) => {
                close_handle(handle, address);
                return Err(e);
            }
        };
        let profile = known_profiles().into_iter().find(|p| {
            let want = p.service_uuid.to_string();
            services.iter().any(|s| s.eq_ignore_ascii_case(&want))
        });
        let Some(profile) = profile else {
            close_handle(handle, address);
            return Err(format!(
                "no known BLE serial profile on {address} (services: {})",
                services.join(", ")
            ));
        };

        let setup = Setup::Profile {
            service: profile.service_uuid.to_string(),
            read: profile.read_characteristic.to_string(),
            write: profile.write_characteristic.to_string(),
        };
        match subscribe(handle, &setup) {
            Ok(true) => {}
            Ok(false) => {
                let reason = last_error(handle);
                close_handle(handle, address);
                return Err(format!("BLE profile setup failed ({}): {reason}", profile.name));
            }
            Err(e) => {
                close_handle(handle, address);
                return Err(e);
            }
        }

        log::info!("[ble] {address} connected as {} (handle {handle})", profile.name);
        Ok(Self::new(address, handle, setup, profile.name.to_string(), profile.write_delay_ms as i32))
    }

    fn lost(&mut self, reason: &str) {
        if let Some(handle) = self.handle.take() {
            close_handle(handle, &self.address);
        }
        if self.unproven && self.up_since.elapsed() < RECONNECT_CAP {
            self.backoff = (self.backoff * 2).min(RECONNECT_CAP);
            log::debug!("[ble] BLE link to {} lost again ({reason}) — retry in {:?}", self.address, self.backoff);
        } else {
            self.unproven = false;
            self.backoff = RECONNECT_FIRST;
            log::warn!("[ble] BLE link to {} lost ({reason}) — reconnecting", self.address);
        }
        self.next_attempt = Instant::now() + self.backoff;
    }

    fn give_up(&mut self, reason: &str) -> TransportError {
        if !self.ended {
            self.ended = true;
            log::warn!("[ble] BLE link to {}: {reason} — closing the link", self.address);
        }
        TransportError::Disconnected
    }

    /// While down: take a finished attempt's result, start the next attempt when due.
    fn service_reconnect(&mut self) -> Result<(), TransportError> {
        let finished = self.attempt.as_ref().and_then(|a| a.take_result().map(|r| (r, a.started.elapsed())));
        if let Some((result, took)) = finished {
            self.attempt = None;
            match result {
                Ok(handle) => {
                    // At the level of the loss: a default-level log must show both.
                    let level = if self.unproven { log::Level::Debug } else { log::Level::Warn };
                    log::log!(level, "[ble] BLE link to {} re-established (attempt took {took:.1?})", self.address);
                    self.handle = Some(handle);
                    self.unproven = true;
                    self.up_since = Instant::now();
                    self.restarted = true;
                    self.off_readings = 0;
                    return Ok(());
                }
                // A Bluetooth stack that restarts itself reads as off for a moment: only the second
                // reading in a row ends the link.
                Err(Failure::End(reason)) => {
                    self.off_readings += 1;
                    if self.off_readings >= 2 {
                        return Err(self.give_up(&reason));
                    }
                    self.next_attempt = Instant::now() + OFF_RECHECK;
                    log::debug!("[ble] {}: {reason} — checking again in {OFF_RECHECK:?}", self.address);
                }
                Err(Failure::Retry(reason)) => {
                    self.off_readings = 0;
                    self.backoff = (self.backoff * 2).min(RECONNECT_CAP);
                    self.next_attempt = Instant::now() + self.backoff;
                    log::debug!(
                        "[ble] reconnect to {} failed after {took:.1?}: {reason} — retry in {:?}",
                        self.address,
                        self.backoff
                    );
                }
            }
        }
        if self.attempt.is_none() && Instant::now() >= self.next_attempt {
            let attempt = Arc::new(Attempt { state: Mutex::new(AttemptState::Running(None)), started: Instant::now() });
            let (address, setup, shared) = (self.address.clone(), self.setup.clone(), attempt.clone());
            let spawned = std::thread::Builder::new()
                .name("ble-reconnect".into())
                .spawn(move || run_attempt(&address, &setup, &shared));
            match spawned {
                Ok(_) => self.attempt = Some(attempt),
                Err(e) => {
                    self.next_attempt = Instant::now() + self.backoff;
                    log::debug!("[ble] reconnect thread for {} failed to start: {e}", self.address);
                }
            }
        }
        Ok(())
    }
}

/// Blocking: subscribe to the profile's read characteristic and keep its write one, or (listen mode) to
/// every notifying characteristic. False with the reason in `last_error(handle)`.
fn subscribe(handle: i32, setup: &Setup) -> Result<bool, String> {
    let Setup::Profile { service, read, write } = setup else {
        let json = call_string("listen", "(I)Ljava/lang/String;", &[JValue::Int(handle)])?;
        return Ok(!json.is_empty());
    };
    let env = jvm::env()?;
    let svc = env.new_string(service).map_err(|e| e.to_string())?;
    let rd = env.new_string(read).map_err(|e| e.to_string())?;
    let wr = env.new_string(write).map_err(|e| e.to_string())?;
    call_bool(
        "subscribe",
        "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
        &[
            JValue::Int(handle),
            JValue::Object(&svc),
            JValue::Object(&rd),
            JValue::Object(&wr),
        ],
    )
}

enum Failure {
    Retry(String),
    /// Bluetooth is off or its permission is gone.
    End(String),
}

enum AttemptState {
    /// The handle of the link being opened, once there is one.
    Running(Option<i32>),
    Done(Result<i32, Failure>),
    /// Taken or cancelled: the attempt's handle is no longer its own.
    Cancelled,
}

/// One reconnect, run on its own thread: a connect blocks up to ~25 s, the caller's thread must not.
struct Attempt {
    state: Mutex<AttemptState>,
    started: Instant,
}

impl Attempt {
    fn lock(&self) -> MutexGuard<'_, AttemptState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn take_result(&self) -> Option<Result<i32, Failure>> {
        let mut state = self.lock();
        if !matches!(*state, AttemptState::Done(_)) {
            return None;
        }
        match std::mem::replace(&mut *state, AttemptState::Cancelled) {
            AttemptState::Done(result) => Some(result),
            _ => None,
        }
    }

    /// Closing the handle releases the connect, discovery and subscribe waits, so the thread ends promptly.
    fn cancel(&self, address: &str) {
        let previous = std::mem::replace(&mut *self.lock(), AttemptState::Cancelled);
        if let AttemptState::Running(Some(handle)) | AttemptState::Done(Ok(handle)) = previous {
            close_handle(handle, address);
        }
    }
}

fn run_attempt(address: &str, setup: &Setup, attempt: &Attempt) {
    let result = reopen(address, setup, attempt);
    let mut state = attempt.lock();
    if matches!(*state, AttemptState::Cancelled) {
        drop(state);
        // `cancel` closed a published handle already; `close` is idempotent.
        if let Ok(handle) = result {
            close_handle(handle, address);
        }
        return;
    }
    *state = AttemptState::Done(result);
}

/// Blocking: the same device and subscription as the first connect, on a fresh Kotlin link.
fn reopen(address: &str, setup: &Setup, attempt: &Attempt) -> Result<i32, Failure> {
    if !call_bool("canConnect", "()Z", &[]).map_err(Failure::Retry)? {
        return Err(Failure::End(last_error(NO_HANDLE)));
    }
    let env = jvm::env().map_err(Failure::Retry)?;
    let j_addr = env.new_string(address).map_err(|e| Failure::Retry(e.to_string()))?;
    let handle = call_int("newLink", "(Ljava/lang/String;)I", &[JValue::Object(&j_addr)]).map_err(Failure::Retry)?;
    if handle < 0 {
        return Err(Failure::Retry(last_error(NO_HANDLE)));
    }
    {
        let mut state = attempt.lock();
        if matches!(*state, AttemptState::Cancelled) {
            drop(state);
            close_handle(handle, address);
            return Err(Failure::Retry("cancelled".into()));
        }
        *state = AttemptState::Running(Some(handle));
    }
    let step = match call_bool("openLink", "(I)Z", &[JValue::Int(handle)]) {
        Ok(true) => match subscribe(handle, setup) {
            Ok(true) => return Ok(handle),
            Ok(false) => "subscribe: ",
            Err(e) => {
                close_handle(handle, address);
                return Err(Failure::Retry(format!("subscribe: {e}")));
            }
        },
        // openLink's reasons name their step (connect / service discovery).
        Ok(false) => "",
        Err(e) => {
            close_handle(handle, address);
            return Err(Failure::Retry(e));
        }
    };
    let reason = last_error(handle);
    close_handle(handle, address);
    Err(Failure::Retry(format!("{step}{reason}")))
}

fn close_handle(handle: i32, address: &str) {
    if let Err(e) = call_void("close", "(I)V", &[JValue::Int(handle)]) {
        log::warn!("[ble] closing {address} failed: {e}");
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            attempt.cancel(&self.address);
        }
        if let Some(handle) = self.handle.take() {
            close_handle(handle, &self.address);
        }
    }
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.ended {
            return Err(TransportError::Disconnected);
        }
        let Some(handle) = self.handle else {
            let start = Instant::now();
            self.service_reconnect()?;
            if self.handle.is_none() {
                // Pace the caller like a read that timed out — it loops on `Ok(0)` without waiting.
                let timeout = Duration::from_millis(self.read_timeout_ms as u64);
                std::thread::sleep(timeout.saturating_sub(start.elapsed()));
            }
            return Ok(0);
        };
        let mut env = jvm::env().map_err(TransportError::Io)?;
        let class = jvm::app_class(&mut env, BRIDGE).map_err(TransportError::Io)?;
        let r = env.new_byte_array(buf.len() as i32);
        let jbuf = jvm::check(&mut env, r, "allocating a read buffer").map_err(TransportError::Io)?;
        let r = env
            .call_static_method(
                &class,
                "read",
                "(I[BI)I",
                &[
                    JValue::Int(handle),
                    JValue::Object(&jbuf),
                    JValue::Int(self.read_timeout_ms),
                ],
            )
            .and_then(|v| v.i());
        let n = jvm::check(&mut env, r, "BleSerial.read").map_err(TransportError::Io)?;
        // Negative = the link is gone and drained; zero = idle timeout, which the protocol loops treat
        // as "nothing this tick".
        if n < 0 {
            let reason = last_error(handle);
            self.lost(&reason);
            return Ok(0);
        }
        if n == 0 {
            return Ok(0);
        }
        let n = (n as usize).min(buf.len());
        // `i8` and `u8` share a layout; only the `n` bytes Java reported are read back.
        let slice = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut i8, n) };
        let r = env.get_byte_array_region(&jbuf, 0, slice);
        jvm::check(&mut env, r, "copying the read buffer").map_err(TransportError::Io)?;
        Ok(n)
    }

    /// While the link is down the data is dropped, like on any stalled link. A failed write counts as a
    /// loss: a fading link fails its writes before Android reports the disconnect.
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if self.ended {
            return Err(TransportError::Disconnected);
        }
        if matches!(self.setup, Setup::Listen) {
            return Err(TransportError::Io("listen-only BLE link has no write characteristic".into()));
        }
        let Some(handle) = self.handle else {
            // Write-only users (the relay sink) never read, so the reconnect is driven from here too.
            return self.service_reconnect();
        };
        let mut env = jvm::env().map_err(TransportError::Io)?;
        let class = jvm::app_class(&mut env, BRIDGE).map_err(TransportError::Io)?;
        let r = env.byte_array_from_slice(data);
        let jdata: JByteArray =
            jvm::check(&mut env, r, "staging the write buffer").map_err(TransportError::Io)?;
        let r = env
            .call_static_method(
                &class,
                "write",
                "(I[BII)Z",
                &[
                    JValue::Int(handle),
                    JValue::Object(&jdata),
                    JValue::Int(WRITE_TIMEOUT_MS),
                    JValue::Int(self.write_delay_ms),
                ],
            )
            .and_then(|v| v.z());
        let ok = jvm::check(&mut env, r, "BleSerial.write").map_err(TransportError::Io)?;
        if !ok {
            let reason = last_error(handle);
            self.lost(&format!("write: {reason}"));
        }
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout_ms = (timeout.as_millis() as i32).max(1);
    }

    fn take_stream_restart(&mut self) -> bool {
        std::mem::take(&mut self.restarted)
    }

    fn description(&self) -> String {
        format!("BLE({}, {})", self.address, self.profile_name)
    }
}
