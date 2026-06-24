// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Serial Transport
// Handles USB/UART serial port communication with flight controllers.
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::io::{Read, Write};
use std::time::Duration;

use super::{ByteTransport, PortInfo, TransportError};

/// Default read timeout for serial operations. Short on purpose — bounds the latency the MAVLink
/// handler loop adds to outgoing commands (mission upload items, param sets) since it only services a
/// queued write once the current blocking read returns. See the TCP transport for the full rationale.
const READ_TIMEOUT_MS: u64 = 50;

/// Bluetooth SPP COM-port classification (Windows). A paired SPP device creates *two* virtual COM
/// ports — an outgoing (client) one we connect through and an incoming (local server) one that's
/// useless to us. We only want the outgoing one (like MWPTools). The registry tells them apart
/// locale-independently; see `bt_spp_outgoing_ports`.
#[cfg(target_os = "windows")]
fn bt_spp_outgoing_ports() -> std::collections::HashSet<String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let mut out = std::collections::HashSet::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    // BTHENUM enumerates Bluetooth RFCOMM (SPP) devices. Layout:
    //   BTHENUM\{service-class-guid}_<id>\<instance>\Device Parameters\PortName = "COMx"
    // The *incoming* (local server) ports live under a class key containing "LOCALMFG"; the *outgoing*
    // (client) ports reference the remote device address instead. We keep only the latter.
    let bthenum = match hklm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM") {
        Ok(k) => k,
        Err(e) => {
            log::debug!("BTHENUM enum not available ({}) — no Bluetooth SPP classification", e);
            return out;
        }
    };

    for class_name in bthenum.enum_keys().flatten() {
        // Skip local/incoming server ports.
        if class_name.to_uppercase().contains("LOCALMFG") {
            continue;
        }
        let class_key = match bthenum.open_subkey(&class_name) {
            Ok(k) => k,
            Err(_) => continue,
        };
        for inst_name in class_key.enum_keys().flatten() {
            let port = class_key
                .open_subkey(&inst_name)
                .ok()
                .and_then(|inst| inst.open_subkey("Device Parameters").ok())
                .and_then(|dp| dp.get_value::<String, _>("PortName").ok());
            if let Some(com) = port {
                log::debug!("Bluetooth SPP outgoing port: {} ({}\\{})", com, class_name, inst_name);
                out.insert(com.to_uppercase());
            }
        }
    }
    out
}

/// All Bluetooth SPP COM ports (outgoing + incoming), so we can drop the incoming ones from the list.
#[cfg(target_os = "windows")]
fn bt_spp_all_ports() -> std::collections::HashSet<String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let mut out = std::collections::HashSet::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let bthenum = match hklm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM") {
        Ok(k) => k,
        Err(_) => return out,
    };
    for class_name in bthenum.enum_keys().flatten() {
        let class_key = match bthenum.open_subkey(&class_name) {
            Ok(k) => k,
            Err(_) => continue,
        };
        for inst_name in class_key.enum_keys().flatten() {
            let port = class_key
                .open_subkey(&inst_name)
                .ok()
                .and_then(|inst| inst.open_subkey("Device Parameters").ok())
                .and_then(|dp| dp.get_value::<String, _>("PortName").ok());
            if let Some(com) = port {
                out.insert(com.to_uppercase());
            }
        }
    }
    out
}

/// List available serial ports on the system.
///
/// On Windows, Bluetooth SPP ports are special-cased: the *incoming* (local server) port of each
/// paired device is hidden, and the *outgoing* (client) port is tagged `bluetooth-spp` so the UI can
/// offer a custom rename (the OS gives them no useful device descriptor). All other ports keep their
/// USB/PCI descriptors.
pub fn list_ports() -> Vec<PortInfo> {
    #[cfg(target_os = "windows")]
    let (spp_outgoing, spp_all) = (bt_spp_outgoing_ports(), bt_spp_all_ports());

    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .filter_map(|p| {
                let port_upper = p.port_name.to_uppercase();

                // Windows: drop incoming Bluetooth SPP ports (any SPP port that isn't an outgoing one).
                #[cfg(target_os = "windows")]
                if spp_all.contains(&port_upper) && !spp_outgoing.contains(&port_upper) {
                    return None;
                }

                // Windows: tag outgoing Bluetooth SPP ports; the UI handles their label (+ custom name).
                #[cfg(target_os = "windows")]
                if spp_outgoing.contains(&port_upper) {
                    return Some(PortInfo {
                        path: p.port_name.clone(),
                        label: p.port_name,
                        port_type: "bluetooth-spp".to_string(),
                    });
                }

                let (label, port_type) = match &p.port_type {
                    serialport::SerialPortType::UsbPort(usb) => {
                        let label = match (&usb.manufacturer, &usb.product) {
                            (Some(mfr), Some(prod)) => {
                                format!("{} — {} ({})", p.port_name, prod, mfr)
                            }
                            (_, Some(prod)) => format!("{} — {}", p.port_name, prod),
                            (Some(mfr), _) => format!("{} — {}", p.port_name, mfr),
                            _ => p.port_name.clone(),
                        };
                        (label, "USB".to_string())
                    }
                    serialport::SerialPortType::BluetoothPort => {
                        (p.port_name.clone(), "Bluetooth".to_string())
                    }
                    serialport::SerialPortType::PciPort => {
                        (p.port_name.clone(), "PCI".to_string())
                    }
                    serialport::SerialPortType::Unknown => {
                        (p.port_name.clone(), "Unknown".to_string())
                    }
                };
                Some(PortInfo {
                    path: p.port_name,
                    label,
                    port_type,
                })
            })
            .collect(),
        Err(e) => {
            log::error!("Failed to enumerate serial ports: {}", e);
            Vec::new()
        }
    }
}

/// An active serial connection to a flight controller
pub struct SerialConnection {
    port_name: String,
    port: Box<dyn serialport::SerialPort>,
}

// Safety: serialport::SerialPort requires Send, Box<dyn SerialPort> is Send
unsafe impl Send for SerialConnection {}

impl SerialConnection {
    /// Open a serial port connection
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(READ_TIMEOUT_MS))
            .open()
            .map_err(|e| format!("Failed to open {}: {}", port_name, e))?;

        Ok(Self {
            port_name: port_name.to_string(),
            port,
        })
    }

    /// Close the connection (port is closed on drop)
    pub fn close(self) {
        drop(self);
    }

    /// Assert/deassert the DTR + RTS control lines. Some USB-serial devices (e.g. ADS-B receivers)
    /// only stream data once the host raises DTR/RTS — terminals do this by default, a bare
    /// `open()` may not. Best-effort: a failure is logged, not fatal.
    pub fn set_control_signals(&mut self, dtr: bool, rts: bool) -> Result<(), String> {
        self.port
            .write_data_terminal_ready(dtr)
            .map_err(|e| format!("DTR: {e}"))?;
        self.port
            .write_request_to_send(rts)
            .map_err(|e| format!("RTS: {e}"))?;
        Ok(())
    }
}

impl ByteTransport for SerialConnection {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(TransportError::from(e)),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.port
            .write_all(data)
            .map_err(|e| TransportError::Io(format!("Serial write failed: {}", e)))?;
        self.port
            .flush()
            .map_err(|e| TransportError::Io(format!("Serial flush failed: {}", e)))?;
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        let _ = self.port.set_timeout(timeout);
    }

    fn description(&self) -> String {
        format!("Serial({})", self.port_name)
    }
}
