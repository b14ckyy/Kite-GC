// Serial Transport
// Handles USB/UART serial port communication with flight controllers.
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::io::{Read, Write};
use std::time::Duration;

use super::{ByteTransport, PortInfo, TransportError};

/// Default read timeout for serial operations
const READ_TIMEOUT_MS: u64 = 1000;

/// List available serial ports on the system
pub fn list_ports() -> Vec<PortInfo> {
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .map(|p| {
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
                PortInfo {
                    path: p.port_name,
                    label,
                    port_type,
                }
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

    fn description(&self) -> String {
        format!("Serial({})", self.port_name)
    }
}
