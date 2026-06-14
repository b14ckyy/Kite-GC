// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// UDP Transport
// Connects to a flight controller via UDP socket (e.g. MAVLink radios, Wi-Fi telemetry).
// Note: UDP is connectionless — we bind locally and send/receive to/from a remote address.
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::net::UdpSocket;
use std::time::Duration;

use super::{ByteTransport, TransportError};

/// Read timeout for individual recv calls. Short on purpose — bounds the latency the MAVLink handler
/// loop adds to outgoing commands (it services a queued write only once the current blocking recv
/// returns). See the TCP transport for the full rationale.
const READ_TIMEOUT_MS: u64 = 50;

/// An active UDP transport to a flight controller
pub struct UdpTransport {
    address: String,
    socket: UdpSocket,
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
        })
    }
}

impl ByteTransport for UdpTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.socket.recv(buf) {
            Ok(n) => Ok(n),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Ok(0)
            }
            Err(e) => Err(TransportError::from(e)),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.socket
            .send(data)
            .map_err(|e| TransportError::Io(format!("UDP send failed: {}", e)))?;
        Ok(())
    }

    fn description(&self) -> String {
        format!("UDP({})", self.address)
    }
}
