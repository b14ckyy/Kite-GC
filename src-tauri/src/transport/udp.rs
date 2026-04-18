// UDP Transport
// Connects to a flight controller via UDP socket (e.g. MAVLink radios, Wi-Fi telemetry).
// Note: UDP is connectionless — we bind locally and send/receive to/from a remote address.
// For MSP, the remote must echo responses back. Suitable for SITL and some Wi-Fi bridges.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use crate::msp::{MspCodec, MspMessage, MspParser};

use super::Transport;

/// Read timeout for individual recv calls
const READ_TIMEOUT_MS: u64 = 1000;
/// Timeout waiting for an MSP response
const MSP_RESPONSE_TIMEOUT_MS: u64 = 2000;

/// An active UDP transport to a flight controller
pub struct UdpTransport {
    address: String,
    socket: UdpSocket,
    parser: MspParser,
}

impl UdpTransport {
    /// Create a UDP transport. Binds to `0.0.0.0:0` and targets `host:port`.
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let addr = format!("{}:{}", host, port);
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("UDP bind failed: {}", e))?;

        socket
            .connect(&addr)
            .map_err(|e| format!("UDP connect to {} failed: {}", addr, e))?;

        socket
            .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;

        Ok(Self {
            address: addr,
            socket,
            parser: MspParser::new(),
        })
    }
}

impl Transport for UdpTransport {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String> {
        let frame = MspCodec::encode_v2(code, payload);
        self.socket
            .send(&frame)
            .map_err(|e| format!("UDP send failed: {}", e))?;

        let mut buf = [0u8; 1024];
        let deadline = Instant::now() + Duration::from_millis(MSP_RESPONSE_TIMEOUT_MS);

        loop {
            if Instant::now() > deadline {
                return Err(format!("MSP response timeout for command 0x{:04X}", code));
            }

            match self.socket.recv(&mut buf) {
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
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    // Recv timeout — retry until deadline
                }
                Err(e) => return Err(format!("UDP recv error: {}", e)),
            }
        }
    }

    fn description(&self) -> String {
        format!("UDP({})", self.address)
    }
}
