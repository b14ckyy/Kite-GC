// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Serial transport for Android, over the USB Host API.
//!
//! Android has no `/dev/tty*` an unprivileged app may open, so the `serialport` crate the desktop
//! implementation is built on has no backend there and is not compiled into the mobile build (see the
//! `cfg(not(target_os = "android"))` dependency block in `Cargo.toml`). The platform route is the USB
//! Host API, which only exists in Java — so the driver lives in `UsbSerial.kt` in the Android app
//! module and this file is the JNI shim in front of it.
//!
//! The split is deliberate: everything crossing the boundary is a primitive, a `String` or a byte
//! array, and the Kotlin side owns all device state behind an integer handle. That keeps the JNI
//! surface small enough to read in one sitting, and means a driver fix for a new USB-serial chip is a
//! Kotlin change with no Rust involvement.
//!
//! Which devices work is decided in `UsbSerial.kt`: CDC-ACM (every mainstream flight controller
//! connected directly by USB) and Silicon Labs CP210x (most SiK telemetry radios). FTDI and CH340 are
//! not driven yet.
//!
//! Permissions are per device and per session on Android. `SerialConnection::open` blocks on the
//! system dialog the first time a device is used; plugging the cable in and choosing Kite from the
//! system's app picker grants it up front and skips the prompt.

use std::time::Duration;

use jni::objects::{JByteArray, JValue};

use super::{ByteTransport, PortInfo, TransportError};
use crate::android::jvm;

/// Binary name of the Kotlin bridge — see `gen/android/app/src/main/java/com/kitegc/app/UsbSerial.kt`.
const BRIDGE: &str = "com.kitegc.app.UsbSerial";

/// Matches the desktop transport's read timeout: short on purpose, because it bounds the latency the
/// protocol handler adds to an outgoing command (it only services a queued write once the current
/// blocking read returns).
const READ_TIMEOUT_MS: u64 = 50;

/// Write timeout. Generous compared to the read side — a write that cannot make progress means the
/// link is gone, and there is no polling loop depending on this returning quickly.
const WRITE_TIMEOUT_MS: i32 = 1000;

/// Handle passed to [`last_error`] for calls that have no handle of their own — `open` and the
/// device listing. Kotlin keys its no-handle slot the same way; real handles start at 1.
const NO_HANDLE: i32 = 0;

/// Fetch `UsbSerial.lastError(handle)`. Called only after a bridge call reports failure, so the cost
/// of a second JNI round-trip does not matter; a failure to read it is itself reported rather than
/// hidden.
///
/// The handle is passed because the Kotlin side keeps the reason per link, not one global string:
/// several connections can be open at once, each driven by its own thread, and a shared slot would
/// let one link's failure be reported against another's.
fn last_error(handle: i32) -> String {
    let mut env = match jvm::env() {
        Ok(env) => env,
        Err(e) => return e,
    };
    let class = match jvm::app_class(&mut env, BRIDGE) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let r = env
        .call_static_method(
            &class,
            "lastError",
            "(I)Ljava/lang/String;",
            &[JValue::Int(handle)],
        )
        .and_then(|v| v.l());
    let obj = match jvm::check(&mut env, r, "UsbSerial.lastError") {
        Ok(obj) => obj,
        Err(e) => return e,
    };
    let r = env.get_string(&obj.into()).map(String::from);
    jvm::check(&mut env, r, "reading the error text").unwrap_or_else(|e| e)
}

/// List connectable USB-serial devices.
///
/// The Kotlin side answers with a JSON array of `{path, label, type}` — the same shape [`PortInfo`]
/// serialises to, so the port picker needs no Android-specific handling. Any failure yields an empty
/// list, exactly like a desktop machine with nothing plugged in: enumeration runs on a timer and must
/// never surface an error dialog.
pub fn list_ports() -> Vec<PortInfo> {
    let json = match list_ports_json() {
        Ok(json) => json,
        Err(e) => {
            log::warn!("[usb-serial] device enumeration failed: {e}");
            return Vec::new();
        }
    };

    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        label: String,
        #[serde(rename = "type")]
        port_type: String,
    }

    match serde_json::from_str::<Vec<Entry>>(&json) {
        Ok(entries) => entries
            .into_iter()
            .map(|e| PortInfo {
                path: e.path,
                label: e.label,
                port_type: e.port_type,
            })
            .collect(),
        Err(e) => {
            log::warn!("[usb-serial] could not parse the device list ({e}): {json}");
            Vec::new()
        }
    }
}

fn list_ports_json() -> Result<String, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env
        .call_static_method(&class, "listDevices", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l());
    let value = jvm::check(&mut env, r, "UsbSerial.listDevices")?;
    let r = env.get_string(&value.into()).map(String::from);
    jvm::check(&mut env, r, "reading the device list")
}

/// An open USB-serial link. The Kotlin side owns the `UsbDeviceConnection`; this is the handle to it.
pub struct SerialConnection {
    port_name: String,
    handle: i32,
    read_timeout_ms: i32,
}

impl SerialConnection {
    /// Open `port_name` (an Android device node such as `/dev/bus/usb/001/002`, as reported by
    /// [`list_ports`]) at `baud_rate`.
    ///
    /// Blocks while the USB permission dialog is up — up to a minute — so it must not be called from
    /// a thread that has to stay responsive. That matches the desktop implementation, which retries a
    /// flaky open for up to 1.5 s; both are driven from the connection command, off any UI path.
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        log::info!("[usb-serial] open requested: {port_name} @ {baud_rate} baud");

        let mut env = jvm::env()?;
        let class = jvm::app_class(&mut env, BRIDGE)?;
        let r = env.new_string(port_name);
        let name = jvm::check(&mut env, r, "passing the device name to Android")?;

        let r = env
            .call_static_method(
                &class,
                "open",
                "(Ljava/lang/String;I)I",
                &[JValue::Object(&name), JValue::Int(baud_rate as i32)],
            )
            .and_then(|v| v.i());
        let handle = jvm::check(&mut env, r, "UsbSerial.open")?;

        if handle < 0 {
            let reason = last_error(NO_HANDLE);
            log::warn!("[usb-serial] open {port_name} failed: {reason}");
            return Err(format!("Failed to open {port_name}: {reason}"));
        }

        let mut conn = Self {
            port_name: port_name.to_string(),
            handle,
            read_timeout_ms: READ_TIMEOUT_MS as i32,
        };

        // Raise DTR/RTS on open, like every standard tool does. USB-CDC devices gate their
        // device→host stream on DTR (`tud_cdc_n_connected()`), so without this the link looks dead in
        // exactly one direction — the same failure the desktop backend was fixed for. Best-effort:
        // some bridges do not implement the lines.
        if let Err(e) = conn.set_control_signals(true, true) {
            log::warn!("[usb-serial] {port_name}: raising DTR/RTS failed (continuing): {e}");
        }

        log::info!("[usb-serial] {port_name} opened (handle {handle})");
        Ok(conn)
    }

    /// Assert/deassert DTR + RTS. See the note in [`SerialConnection::open`] for why this matters.
    pub fn set_control_signals(&mut self, dtr: bool, rts: bool) -> Result<(), String> {
        let mut env = jvm::env()?;
        let class = jvm::app_class(&mut env, BRIDGE)?;
        let r = env
            .call_static_method(
                &class,
                "setControlLines",
                "(IZZ)Z",
                &[
                    JValue::Int(self.handle),
                    JValue::Bool(dtr as u8),
                    JValue::Bool(rts as u8),
                ],
            )
            .and_then(|v| v.z());
        let ok = jvm::check(&mut env, r, "UsbSerial.setControlLines")?;
        if ok {
            Ok(())
        } else {
            Err(last_error(self.handle))
        }
    }
}

impl Drop for SerialConnection {
    fn drop(&mut self) {
        let closed = (|| -> Result<(), String> {
            let mut env = jvm::env()?;
            let class = jvm::app_class(&mut env, BRIDGE)?;
            let r = env
                .call_static_method(&class, "close", "(I)V", &[JValue::Int(self.handle)])
                .map(|_| ());
            jvm::check(&mut env, r, "UsbSerial.close")
        })();
        if let Err(e) = closed {
            // Nothing to escalate to from a Drop, but leaking a claimed USB interface silently would
            // make the *next* open fail with "already claimed" and no explanation.
            log::warn!("[usb-serial] closing {} failed: {e}", self.port_name);
        }
    }
}

impl ByteTransport for SerialConnection {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let mut env = jvm::env().map_err(TransportError::Io)?;
        let class = jvm::app_class(&mut env, BRIDGE).map_err(TransportError::Io)?;

        // A fresh Java array per read rather than a cached one: `GetByteArrayElements` would let the
        // VM hand back a copy anyway, and a shared buffer would have to be pinned against the GC for
        // the whole call. At 50 Hz with MSP-sized frames this allocation is not what costs anything.
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
        let n = jvm::check(&mut env, r, "UsbSerial.read").map_err(TransportError::Io)?;

        // Negative means the device is gone; zero is an ordinary idle timeout, which the protocol
        // loops treat as "nothing this tick" rather than an error.
        if n < 0 {
            return Err(TransportError::Disconnected);
        }
        if n == 0 {
            return Ok(0);
        }

        let n = n as usize;
        // SAFETY-adjacent: `i8` and `u8` have the same layout, and we only read back the `n` bytes the
        // Java side reported writing.
        let slice = unsafe {
            std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut i8, n.min(buf.len()))
        };
        let r = env.get_byte_array_region(&jbuf, 0, slice);
        jvm::check(&mut env, r, "copying the read buffer").map_err(TransportError::Io)?;
        Ok(n.min(buf.len()))
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
                "(I[BI)Z",
                &[
                    JValue::Int(self.handle),
                    JValue::Object(&jdata),
                    JValue::Int(WRITE_TIMEOUT_MS),
                ],
            )
            .and_then(|v| v.z());
        let ok = jvm::check(&mut env, r, "UsbSerial.write").map_err(TransportError::Io)?;

        if ok {
            Ok(())
        } else {
            Err(TransportError::Io(format!(
                "Serial write failed: {}",
                last_error(self.handle)
            )))
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        // Clamped to at least 1 ms: a zero timeout makes Android's `bulkTransfer` block forever, which
        // is the opposite of what a caller asking for a tight poll wants.
        self.read_timeout_ms = (timeout.as_millis() as i32).max(1);
    }

    fn description(&self) -> String {
        format!("Serial({})", self.port_name)
    }
}
