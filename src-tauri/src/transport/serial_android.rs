// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Android serial: USB devices through the USB Host API (UsbSerial.kt owns all device state behind
//! an integer handle; this is the JNI shim), built-in /dev/tty* nodes of known hardware through the
//! serialport crate's posix backend. The path decides the backend (is_tty_node).

use std::io::{Read, Write};
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

/// USB devices first (a plugged-in FC keeps the top of the picker), then built-in UARTs. Never an
/// error: enumeration runs on a timer, a failure yields an empty list.
pub fn list_ports() -> Vec<PortInfo> {
    let mut ports = usb_ports();
    ports.extend(crate::android::device::internal_serial_ports());
    ports
}

fn usb_ports() -> Vec<PortInfo> {
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

enum Backend {
    /// A USB device driven by `UsbSerial.kt`; the Kotlin side owns the `UsbDeviceConnection` and
    /// this is the handle to it.
    Usb { handle: i32, read_timeout_ms: i32 },
    /// A tty node of the device itself (the AX12's `/dev/ttyS1`), opened through the `serialport`
    /// crate exactly as the Linux desktop build does.
    Tty(Box<dyn serialport::SerialPort>),
}

pub struct SerialConnection {
    port_name: String,
    backend: Backend,
}

/// USB device names are always /dev/bus/usb/… (UsbManager), so this prefix cannot misroute one.
fn is_tty_node(port_name: &str) -> bool {
    port_name.starts_with("/dev/tty")
}

impl SerialConnection {
    /// A USB node (/dev/bus/usb/…) or a tty node (/dev/ttyS1), as listed by [`list_ports`]. A USB
    /// open blocks while the permission dialog is up (up to a minute): never call it from a thread
    /// that must stay responsive.
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        if is_tty_node(port_name) {
            return Self::open_tty(port_name, baud_rate);
        }
        Self::open_usb(port_name, baud_rate)
    }

    /// The tty path. No retry loop: a node that is there and accessible opens on the first try, and
    /// the two ways it fails — missing, or not ours to open — do not change with waiting.
    fn open_tty(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        log::info!("[tty-serial] open requested: {port_name} @ {baud_rate} baud");
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(READ_TIMEOUT_MS))
            .open()
            .map_err(|e| {
                log::warn!("[tty-serial] open {port_name} failed (kind={:?}): {e}", e.kind());
                format!("Failed to open {port_name}: {e}")
            })?;
        let mut conn = Self {
            port_name: port_name.to_string(),
            backend: Backend::Tty(port),
        };
        // Same DTR/RTS convention as every other backend (see `open_usb`). The AX12's UART1 has both
        // modem lines wired and asserted by the firmware already; harmless either way.
        if let Err(e) = conn.set_control_signals(true, true) {
            log::warn!("[tty-serial] {port_name}: raising DTR/RTS failed (continuing): {e}");
        }
        log::info!("[tty-serial] {port_name} opened");
        Ok(conn)
    }

    fn open_usb(port_name: &str, baud_rate: u32) -> Result<Self, String> {
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
            backend: Backend::Usb {
                handle,
                read_timeout_ms: READ_TIMEOUT_MS as i32,
            },
        };

        // Raise DTR/RTS: USB-CDC devices gate their device→host stream on DTR
        // (`tud_cdc_n_connected()`). Best-effort: some bridges do not implement the lines.
        if let Err(e) = conn.set_control_signals(true, true) {
            log::warn!("[usb-serial] {port_name}: raising DTR/RTS failed (continuing): {e}");
        }

        log::info!("[usb-serial] {port_name} opened (handle {handle})");
        Ok(conn)
    }

    /// Assert/deassert DTR + RTS. See the note in [`SerialConnection::open_usb`] for why this matters.
    pub fn set_control_signals(&mut self, dtr: bool, rts: bool) -> Result<(), String> {
        let handle = match &mut self.backend {
            Backend::Tty(port) => {
                port.write_data_terminal_ready(dtr).map_err(|e| format!("DTR: {e}"))?;
                port.write_request_to_send(rts).map_err(|e| format!("RTS: {e}"))?;
                return Ok(());
            }
            Backend::Usb { handle, .. } => *handle,
        };
        let mut env = jvm::env()?;
        let class = jvm::app_class(&mut env, BRIDGE)?;
        let r = env
            .call_static_method(
                &class,
                "setControlLines",
                "(IZZ)Z",
                &[
                    JValue::Int(handle),
                    JValue::Bool(dtr as u8),
                    JValue::Bool(rts as u8),
                ],
            )
            .and_then(|v| v.z());
        let ok = jvm::check(&mut env, r, "UsbSerial.setControlLines")?;
        if ok {
            Ok(())
        } else {
            Err(last_error(handle))
        }
    }
}

impl Drop for SerialConnection {
    fn drop(&mut self) {
        // A tty closes itself with its `serialport` handle; only the USB side has Kotlin-held state.
        let Backend::Usb { handle, .. } = self.backend else {
            return;
        };
        let closed = (|| -> Result<(), String> {
            let mut env = jvm::env()?;
            let class = jvm::app_class(&mut env, BRIDGE)?;
            let r = env
                .call_static_method(&class, "close", "(I)V", &[JValue::Int(handle)])
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
        let (handle, read_timeout_ms) = match &mut self.backend {
            Backend::Tty(port) => {
                // Mirrors the desktop backend: a read timeout is an idle tick, not an error.
                return match port.read(buf) {
                    Ok(n) => Ok(n),
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
                    Err(e) => Err(TransportError::from(e)),
                };
            }
            Backend::Usb { handle, read_timeout_ms } => (*handle, *read_timeout_ms),
        };
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
                    JValue::Int(handle),
                    JValue::Object(&jbuf),
                    JValue::Int(read_timeout_ms),
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
        let handle = match &mut self.backend {
            Backend::Tty(port) => {
                port.write_all(data).map_err(TransportError::from)?;
                port.flush().map_err(TransportError::from)?;
                return Ok(());
            }
            Backend::Usb { handle, .. } => *handle,
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
                "(I[BI)Z",
                &[
                    JValue::Int(handle),
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
                last_error(handle)
            )))
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        match &mut self.backend {
            Backend::Tty(port) => {
                let _ = port.set_timeout(timeout);
            }
            Backend::Usb { read_timeout_ms, .. } => {
                // Clamped to at least 1 ms: a zero timeout makes Android's `bulkTransfer` block
                // forever, which is the opposite of what a caller asking for a tight poll wants.
                *read_timeout_ms = (timeout.as_millis() as i32).max(1);
            }
        }
    }

    fn description(&self) -> String {
        format!("Serial({})", self.port_name)
    }
}
