// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! BLE transport for Android, on the platform's own GATT stack.
//!
//! `btleplug` has no usable Android backend without its `droidplug` Java companion, so — like the
//! iOS backend with CoreBluetooth — this one is native: `BleSerial.kt` drives `BluetoothLeScanner`
//! and `BluetoothGatt`, and this file is the JNI shim in front of it, under the same contract as
//! the USB-serial shim (primitives, Strings and byte arrays across the boundary; the Kotlin side
//! owns all device state behind an integer handle; the reason for a failure is fetched afterwards).
//!
//! The split of responsibilities is deliberate: Kotlin knows nothing about which GATT service is a
//! "serial port". After connecting it hands back the device's service list, this side picks the
//! first known profile (`transport/ble_profiles.rs` — shared with the desktop backend) and tells
//! Kotlin which characteristics to subscribe to and write to. A new adapter family is therefore a
//! one-line Rust change.
//!
//! Public surface is identical to `transport/ble.rs`, so `commands/connection.rs` and the relay's
//! BLE sink compile unchanged. Listen-only mode (the desktop's GATT dump) is not implemented yet
//! and behaves like a normal connect.

use std::collections::HashSet;
use std::time::Duration;

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
/// Per-chunk write acknowledgement timeout.
const WRITE_TIMEOUT_MS: i32 = 2000;

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

/// Listen-only connect. Android: same path as `connect_ble` for now — the profile handshake already
/// subscribes to the read characteristic; the desktop's whole-GATT dump is not implemented here yet.
pub async fn connect_ble_listen(
    device_id: &str,
    _app: tauri::AppHandle,
) -> Result<BleTransport, String> {
    log::info!("[ble] listen-only connect on Android uses the normal profile path ({device_id})");
    connect_ble(device_id).await
}

/// A connected BLE-serial link. The Kotlin side owns the `BluetoothGatt`; this is the handle to it.
pub struct BleTransport {
    address: String,
    handle: i32,
    profile_name: String,
    write_delay_ms: i32,
    read_timeout_ms: i32,
}

impl BleTransport {
    /// Blocking: permission dialog, connect (≤15 s), service discovery (≤10 s), subscribe (≤7 s).
    fn open(address: &str) -> Result<Self, String> {
        log::info!("[ble] connect requested: {address}");
        ensure_permissions()?;

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

        // The device's services decide the profile — most adapters advertise nothing, so this is
        // the authoritative match, not the scan-time tag.
        let json = call_string("services", "(I)Ljava/lang/String;", &[JValue::Int(handle)])?;
        let services: Vec<String> =
            serde_json::from_str(&json).map_err(|e| format!("bad service list from Android: {e}"))?;
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

        let env = jvm::env()?;
        let svc = env.new_string(profile.service_uuid.to_string()).map_err(|e| e.to_string())?;
        let rd = env.new_string(profile.read_characteristic.to_string()).map_err(|e| e.to_string())?;
        let wr = env.new_string(profile.write_characteristic.to_string()).map_err(|e| e.to_string())?;
        drop(env);
        let ok = call_bool(
            "subscribe",
            "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
            &[
                JValue::Int(handle),
                JValue::Object(&svc),
                JValue::Object(&rd),
                JValue::Object(&wr),
            ],
        )?;
        if !ok {
            let reason = last_error(handle);
            close_handle(handle, address);
            return Err(format!("BLE profile setup failed ({}): {reason}", profile.name));
        }

        log::info!("[ble] {address} connected as {} (handle {handle})", profile.name);
        Ok(Self {
            address: address.to_string(),
            handle,
            profile_name: profile.name.to_string(),
            write_delay_ms: profile.write_delay_ms as i32,
            read_timeout_ms: READ_TIMEOUT_MS,
        })
    }
}

fn close_handle(handle: i32, address: &str) {
    if let Err(e) = call_void("close", "(I)V", &[JValue::Int(handle)]) {
        log::warn!("[ble] closing {address} failed: {e}");
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        close_handle(self.handle, &self.address);
    }
}

impl ByteTransport for BleTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
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
                    JValue::Int(self.handle),
                    JValue::Object(&jbuf),
                    JValue::Int(self.read_timeout_ms),
                ],
            )
            .and_then(|v| v.i());
        let n = jvm::check(&mut env, r, "BleSerial.read").map_err(TransportError::Io)?;
        // Negative = the link is gone and drained; zero = idle timeout, which the protocol loops treat
        // as "nothing this tick".
        if n < 0 {
            return Err(TransportError::Disconnected);
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

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
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
                    JValue::Int(self.handle),
                    JValue::Object(&jdata),
                    JValue::Int(WRITE_TIMEOUT_MS),
                    JValue::Int(self.write_delay_ms),
                ],
            )
            .and_then(|v| v.z());
        let ok = jvm::check(&mut env, r, "BleSerial.write").map_err(TransportError::Io)?;
        if ok {
            Ok(())
        } else {
            Err(TransportError::Io(format!("BLE write failed: {}", last_error(self.handle))))
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout_ms = (timeout.as_millis() as i32).max(1);
    }

    fn description(&self) -> String {
        format!("BLE({}, {})", self.address, self.profile_name)
    }
}
