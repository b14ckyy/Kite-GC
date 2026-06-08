// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// TCP Transport
// Connects to a flight controller via TCP socket (e.g. Wi-Fi bridges, SITL).
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::{ByteTransport, TransportError};

/// TCP connection timeout
const CONNECT_TIMEOUT_MS: u64 = 5000;
/// Read timeout for individual read calls
const READ_TIMEOUT_MS: u64 = 1000;

/// An active TCP connection to a flight controller
pub struct TcpTransport {
    address: String,
    stream: TcpStream,
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
        })
    }
}

impl ByteTransport for TcpTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.stream.read(buf) {
            Ok(0) => Err(TransportError::Disconnected),
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
        self.stream
            .write_all(data)
            .map_err(|e| TransportError::Io(format!("TCP write failed: {}", e)))?;
        self.stream
            .flush()
            .map_err(|e| TransportError::Io(format!("TCP flush failed: {}", e)))?;
        Ok(())
    }

    fn description(&self) -> String {
        format!("TCP({})", self.address)
    }
}
