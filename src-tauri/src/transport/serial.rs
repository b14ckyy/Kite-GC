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

/// Open-retry budget. Bluetooth SPP ports often fail the *first* open with Windows error 121
/// ("The semaphore timeout period has expired") because opening the COM port is what triggers the
/// RFCOMM link to be (re)established in the BT stack — if the remote is asleep, out of range, or a
/// previous owner (e.g. Mission Planner) hasn't fully released the channel yet, the driver's internal
/// semaphore times out. A short retry usually succeeds once the link is up. Harmless for USB serial
/// (a genuinely busy/missing port just fails all attempts and we still report it).
const OPEN_RETRY_ATTEMPTS: usize = 3;
const OPEN_RETRY_DELAY_MS: u64 = 500;

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
    // Outgoing (client) ports reference a real REMOTE device address in their instance name; the
    // incoming (local RFCOMM server) ports use the null address (00:00:…:00). We classify on the
    // instance address — NOT the class-key name: a real outgoing port can sit under a `…_LOCALMFG&…`
    // class (observed in the field), so the old "class contains LOCALMFG ⇒ incoming" test wrongly
    // dropped working outgoing ports.
    let bthenum = match hklm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM") {
        Ok(k) => k,
        Err(e) => {
            log::debug!("BTHENUM enum not available ({}) — no Bluetooth SPP classification", e);
            return out;
        }
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
                if bt_spp_is_outgoing(&inst_name) {
                    log::debug!("Bluetooth SPP outgoing port: {} ({}\\{})", com, class_name, inst_name);
                    out.insert(com.to_uppercase());
                }
            }
        }
    }
    out
}

/// Classify a BTHENUM instance as an *outgoing* (client) SPP port vs an *incoming* (local server) one.
/// The instance name ends in `…&<REMOTE_ADDR>_<suffix>` — outgoing ports carry a real remote address
/// (e.g. `042404160E7B_C00000000`), incoming/server ports carry the null address (`000000000000_…`).
/// Unparseable instances default to **outgoing** (fail-safe: never hide a possibly-real port).
#[cfg(target_os = "windows")]
fn bt_spp_is_outgoing(inst_name: &str) -> bool {
    let tail = inst_name.rsplit('&').next().unwrap_or("");
    let addr = tail.split('_').next().unwrap_or(tail);
    !(!addr.is_empty() && addr.chars().all(|c| c == '0'))
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

/// One-time dump (per process) of the raw serial enumeration + Bluetooth-SPP registry classification.
/// Reveals, in a tester's log, exactly which COM ports the OS reports, the type `serialport` assigns
/// each, and how every BTHENUM (Bluetooth RFCOMM/SPP) entry is classified — so a "missing port" can be
/// traced to either the OS enumeration or our classification.
#[cfg(target_os = "windows")]
fn log_port_enumeration_once(
    ports: &[serialport::SerialPortInfo],
    spp_all: &std::collections::HashSet<String>,
) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let spp_outgoing = bt_spp_outgoing_ports();
        let dump = || {
            eprintln!("[PORTS] serialport::available_ports() reported {} port(s):", ports.len());
            for p in ports {
                let kind = match &p.port_type {
                    serialport::SerialPortType::UsbPort(_) => "USB",
                    serialport::SerialPortType::BluetoothPort => "Bluetooth",
                    serialport::SerialPortType::PciPort => "PCI",
                    serialport::SerialPortType::Unknown => "Unknown",
                };
                let up = p.port_name.to_uppercase();
                let cls = if spp_outgoing.contains(&up) {
                    "SPP-outgoing"
                } else if spp_all.contains(&up) {
                    "SPP-incoming/unclassified"
                } else {
                    "non-SPP"
                };
                eprintln!("[PORTS]   {} type={} class={}", p.port_name, kind, cls);
            }
            eprintln!("[PORTS] BTHENUM outgoing set: {:?}", spp_outgoing);
            eprintln!("[PORTS] BTHENUM all set:      {:?}", spp_all);
            log_bthenum_tree();
        };
        dump();
    });
}

/// Walk the entire BTHENUM registry subtree and log each class/instance + its PortName and whether the
/// class key looks like a local (incoming) server (`LOCALMFG`). Diagnostic only.
#[cfg(target_os = "windows")]
fn log_bthenum_tree() {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let bthenum = match hklm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM") {
        Ok(k) => k,
        Err(e) => {
            eprintln!("[PORTS] BTHENUM not readable: {}", e);
            return;
        }
    };
    eprintln!("[PORTS] --- BTHENUM tree ---");
    for class_name in bthenum.enum_keys().flatten() {
        let localmfg = class_name.to_uppercase().contains("LOCALMFG");
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
            eprintln!(
                "[PORTS]   class={} localmfg={} inst={} port={:?}",
                class_name, localmfg, inst_name, port
            );
        }
    }
    eprintln!("[PORTS] --- end BTHENUM tree ---");
}

/// List available serial ports on the system.
///
/// Linux registers the legacy 8250 UARTs as `/dev/ttyS0..31` even when no real hardware sits behind
/// them, so the raw enumeration is full of non-connectable phantom stubs (one machine showed ~20).
/// Such stubs report `PORT_UNKNOWN` via the `TIOCGSERIAL` ioctl — exactly what `setserial` prints as
/// "UART: unknown" — whereas a real port reports a concrete UART type. We probe ONLY `ttyS*` nodes:
/// USB-serial (`ttyUSB*`/`ttyACM*`) doesn't implement `TIOCGSERIAL` meaningfully (it also reads as
/// `PORT_UNKNOWN`) and is already trustworthy via its USB descriptor. If a node can't be opened or
/// probed we keep it — better a stray entry than a hidden real port.
#[cfg(target_os = "linux")]
fn is_phantom_serial_stub(port_name: &str) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    // TIOCGSERIAL get-config ioctl; `struct serial_struct`'s first field is `int type`, PORT_UNKNOWN == 0.
    const TIOCGSERIAL: libc::c_ulong = 0x541E;

    let leaf = port_name.rsplit('/').next().unwrap_or(port_name);
    if !leaf.starts_with("ttyS") {
        return false; // not an 8250 node → never a phantom 8250 stub
    }

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(port_name)
    {
        Ok(f) => f,
        Err(_) => return false, // can't open (e.g. no permission) → don't hide it
    };

    // `serial_struct` is ~72 bytes; over-allocate so the kernel never writes past our buffer.
    let mut serinfo = [0u8; 128];
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), TIOCGSERIAL, serinfo.as_mut_ptr()) };
    if rc != 0 {
        return false; // ioctl unsupported/failed → keep the port
    }
    let port_type = i32::from_ne_bytes([serinfo[0], serinfo[1], serinfo[2], serinfo[3]]);
    port_type == 0 // PORT_UNKNOWN → phantom stub
}

/// On Windows, Bluetooth SPP ports are special-cased: the *incoming* (local server) port of each
/// paired device is hidden, and the *outgoing* (client) port is tagged `bluetooth-spp` so the UI can
/// offer a custom rename (the OS gives them no useful device descriptor). All other ports keep their
/// USB/PCI descriptors.
pub fn list_ports() -> Vec<PortInfo> {
    #[cfg(target_os = "windows")]
    let (spp_outgoing, spp_all) = (bt_spp_outgoing_ports(), bt_spp_all_ports());

    match serialport::available_ports() {
        Ok(ports) => {
            // One-time diagnostics: the raw enumeration + Bluetooth-SPP registry classification, so a
            // tester's log reveals exactly which ports the OS reports and how each is classified.
            #[cfg(target_os = "windows")]
            log_port_enumeration_once(&ports, &spp_all);

            ports
            .into_iter()
            .filter_map(|p| {
                #[cfg(target_os = "windows")]
                let port_upper = p.port_name.to_uppercase();

                // Windows: hide the *incoming* (local server) SPP port of each paired device — it can't
                // be used to connect out. Classification is by the instance's remote BT address (see
                // bt_spp_is_outgoing), so any SPP port that is known and NOT outgoing is incoming.
                #[cfg(target_os = "windows")]
                if spp_all.contains(&port_upper) && !spp_outgoing.contains(&port_upper) {
                    return None;
                }

                // Windows: tag outgoing Bluetooth-SPP ports so the UI offers the custom rename (the OS
                // gives them no useful descriptor).
                #[cfg(target_os = "windows")]
                if spp_outgoing.contains(&port_upper) {
                    return Some(PortInfo {
                        path: p.port_name.clone(),
                        label: p.port_name,
                        port_type: "bluetooth-spp".to_string(),
                    });
                }

                // Linux: drop phantom 8250 stubs (/dev/ttyS* with no real UART behind them) — they
                // clutter the list with non-connectable entries. See is_phantom_serial_stub.
                #[cfg(target_os = "linux")]
                if is_phantom_serial_stub(&p.port_name) {
                    return None;
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
            .collect()
        }
        Err(e) => {
            log::error!("Failed to enumerate serial ports: {}", e);
            Vec::new()
        }
    }
}

/// Log what `serialport` reports for `port_name` (USB / Bluetooth / PCI / Unknown), or that it isn't
/// listed at all — tells a real USB-serial problem apart from a Bluetooth-SPP one in a tester's log.
fn log_serialport_type(port_name: &str) {
    match serialport::available_ports() {
        Ok(ports) => match ports.iter().find(|p| p.port_name.eq_ignore_ascii_case(port_name)) {
            Some(p) => log::info!("serialport reports {}: type={:?}", port_name, p.port_type),
            None => log::warn!(
                "serialport does NOT list {} among available ports ({:?})",
                port_name,
                ports.iter().map(|p| p.port_name.clone()).collect::<Vec<_>>()
            ),
        },
        Err(e) => log::warn!("serialport enumeration failed while opening {}: {}", port_name, e),
    }
}

/// On open failure, dump how every Bluetooth-SPP port is classified (outgoing client vs incoming
/// server) and where `target` falls. Logged at `warn` so it lands in a tester's log at the default
/// level. Reveals the misclassification case: if `target` is an *incoming/server* port it can never
/// be opened outbound and will always fail with Windows error 121.
#[cfg(target_os = "windows")]
fn log_bt_spp_diagnostics(target: &str) {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let target_u = target.to_uppercase();
    let outgoing = bt_spp_outgoing_ports();
    let all = bt_spp_all_ports();
    let verdict = if outgoing.contains(&target_u) {
        "OUTGOING (client) — should be openable"
    } else if all.contains(&target_u) {
        "INCOMING (server) — NOT openable outbound (likely the cause!)"
    } else {
        "not classified as a Bluetooth-SPP port"
    };
    log::warn!(
        "BT-SPP diagnostics for {}: {} | outgoing-client ports: {:?}",
        target, verdict, outgoing
    );
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    match hklm.open_subkey(r"SYSTEM\CurrentControlSet\Enum\BTHENUM") {
        Ok(bthenum) => {
            for class_name in bthenum.enum_keys().flatten() {
                let is_local = class_name.to_uppercase().contains("LOCALMFG");
                let Ok(class_key) = bthenum.open_subkey(&class_name) else { continue };
                for inst_name in class_key.enum_keys().flatten() {
                    let port = class_key
                        .open_subkey(&inst_name)
                        .ok()
                        .and_then(|inst| inst.open_subkey("Device Parameters").ok())
                        .and_then(|dp| dp.get_value::<String, _>("PortName").ok());
                    if let Some(com) = port {
                        log::warn!(
                            "  BTHENUM {}: {} (class={}, instance={})",
                            com,
                            if is_local { "INCOMING/server (LOCALMFG)" } else { "OUTGOING/client" },
                            class_name,
                            inst_name
                        );
                    }
                }
            }
        }
        Err(e) => log::warn!("BT-SPP diagnostics: BTHENUM registry not readable: {}", e),
    }
}

#[cfg(not(target_os = "windows"))]
fn log_bt_spp_diagnostics(_target: &str) {}

/// An active serial connection to a flight controller
pub struct SerialConnection {
    port_name: String,
    port: Box<dyn serialport::SerialPort>,
}

// Safety: serialport::SerialPort requires Send, Box<dyn SerialPort> is Send
unsafe impl Send for SerialConnection {}

impl SerialConnection {
    /// Open a serial port connection. Retries a few times on failure (BT-SPP first-open flakiness —
    /// Windows error 121) and, on final failure, dumps Bluetooth-SPP port diagnostics to the log so a
    /// tester's log file shows the exact OS error + how every BT port was classified.
    pub fn open(port_name: &str, baud_rate: u32) -> Result<Self, String> {
        log::info!("Serial open requested: {} @ {} baud", port_name, baud_rate);
        log_serialport_type(port_name);

        let mut last_err = String::new();
        for attempt in 1..=OPEN_RETRY_ATTEMPTS {
            match serialport::new(port_name, baud_rate)
                .timeout(Duration::from_millis(READ_TIMEOUT_MS))
                .open()
            {
                Ok(port) => {
                    if attempt > 1 {
                        eprintln!("[serial] {} opened on attempt {}/{}", port_name, attempt, OPEN_RETRY_ATTEMPTS);
                        log::info!("Serial port {} opened on attempt {}/{}", port_name, attempt, OPEN_RETRY_ATTEMPTS);
                    }
                    return Ok(Self {
                        port_name: port_name.to_string(),
                        port,
                    });
                }
                Err(e) => {
                    last_err = e.to_string();
                    eprintln!(
                        "[serial] open {} attempt {}/{} failed: kind={:?} err={}",
                        port_name, attempt, OPEN_RETRY_ATTEMPTS, e.kind(), e
                    );
                    log::warn!(
                        "Serial open {} attempt {}/{} failed (kind={:?}): {}",
                        port_name, attempt, OPEN_RETRY_ATTEMPTS, e.kind(), e
                    );
                    if attempt < OPEN_RETRY_ATTEMPTS {
                        std::thread::sleep(Duration::from_millis(OPEN_RETRY_DELAY_MS));
                    }
                }
            }
        }

        // All attempts failed — emit the BT-SPP classification table so the tester's log reveals
        // whether the chosen port is actually an *incoming* (server) port that can never open outbound.
        log_bt_spp_diagnostics(port_name);
        Err(format!(
            "Failed to open {} after {} attempts: {}",
            port_name, OPEN_RETRY_ATTEMPTS, last_err
        ))
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
