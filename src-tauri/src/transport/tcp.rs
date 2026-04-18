// TCP Transport
// Connects to a flight controller via TCP socket (e.g. Wi-Fi bridges, SITL).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::msp::{MspCodec, MspMessage, MspParser};

use super::Transport;

/// TCP connection timeout
const CONNECT_TIMEOUT_MS: u64 = 5000;
/// Read timeout for individual read calls
const READ_TIMEOUT_MS: u64 = 1000;
/// Timeout waiting for an MSP response
const MSP_RESPONSE_TIMEOUT_MS: u64 = 2000;

/// An active TCP connection to a flight controller
pub struct TcpTransport {
    address: String,
    stream: TcpStream,
    parser: MspParser,
}

impl TcpTransport {
    /// Connect to a flight controller via TCP
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect_timeout(
            &addr.parse().map_err(|e| format!("Invalid address '{}': {}", addr, e))?,
            Duration::from_millis(CONNECT_TIMEOUT_MS),
        )
        .map_err(|e| format!("TCP connect to {} failed: {}", addr, e))?;

        stream
            .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("Failed to set TCP_NODELAY: {}", e))?;

        Ok(Self {
            address: addr,
            stream,
            parser: MspParser::new(),
        })
    }
}

impl Transport for TcpTransport {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String> {
        let frame = MspCodec::encode_v2(code, payload);
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("TCP write failed: {}", e))?;
        self.stream
            .flush()
            .map_err(|e| format!("TCP flush failed: {}", e))?;

        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_millis(MSP_RESPONSE_TIMEOUT_MS);

        loop {
            if Instant::now() > deadline {
                return Err(format!("MSP response timeout for command 0x{:04X}", code));
            }

            match self.stream.read(&mut buf) {
                Ok(0) => return Err("TCP connection closed by remote".to_string()),
                Ok(n) => {
                    for &byte in &buf[..n] {
                        if let Some(msg) = self.parser.push(byte) {
                            if msg.code == code {
                                return Ok(msg);
                            }
                        }
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    // Read timeout — retry until deadline
                }
                Err(e) => return Err(format!("TCP read error: {}", e)),
            }
        }
    }

    fn description(&self) -> String {
        format!("TCP({})", self.address)
    }
}
