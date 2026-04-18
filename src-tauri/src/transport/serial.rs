// Serial Transport
// Handles USB/UART serial port communication with flight controllers.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::msp::{MspCodec, MspMessage, MspParser};

use super::{PortInfo, Transport};

/// Default read timeout for serial operations
const READ_TIMEOUT_MS: u64 = 1000;
/// Timeout waiting for an MSP response
const MSP_RESPONSE_TIMEOUT_MS: u64 = 2000;

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
    parser: MspParser,
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
            parser: MspParser::new(),
        })
    }

    /// Close the connection (port is closed on drop)
    pub fn close(self) {
        // Drop self, which drops the port
        drop(self);
    }
}

impl Transport for SerialConnection {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String> {
        // Encode and send
        let frame = MspCodec::encode_v2(code, payload);
        self.port
            .write_all(&frame)
            .map_err(|e| format!("Write failed: {}", e))?;
        self.port
            .flush()
            .map_err(|e| format!("Flush failed: {}", e))?;

        // Read until we get the response or timeout
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_millis(MSP_RESPONSE_TIMEOUT_MS);

        loop {
            if Instant::now() > deadline {
                return Err(format!("MSP response timeout for command 0x{:04X}", code));
            }

            match self.port.read(&mut buf) {
                Ok(n) if n > 0 => {
                    for &byte in &buf[..n] {
                        if let Some(msg) = self.parser.push(byte) {
                            if msg.code == code {
                                return Ok(msg);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(format!("Read error: {}", e)),
            }
        }
    }

    fn description(&self) -> String {
        format!("Serial({})", self.port_name)
    }
}
