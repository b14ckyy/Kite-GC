// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Transport
// Wraps a ByteTransport with MSP v2 framing and response parsing.
// This is the bridge between protocol-agnostic byte I/O and MSP request/response.

use std::time::{Duration, Instant};

use crate::transport::{ByteTransport, Transport};

use super::{MspCodec, MspMessage, MspParser};

/// Timeout waiting for an MSP response (2 seconds)
const MSP_RESPONSE_TIMEOUT_MS: u64 = 2000;

/// MSP protocol layer on top of a ByteTransport.
///
/// Owns a ByteTransport and an MspParser, provides MSP request/response semantics.
/// Implements the `Transport` trait so the scheduler and handshake code work unchanged.
pub struct MspTransport {
    inner: Box<dyn ByteTransport>,
    parser: MspParser,
}

impl MspTransport {
    /// Wrap a ByteTransport with MSP framing
    pub fn new(transport: Box<dyn ByteTransport>) -> Self {
        Self {
            inner: transport,
            parser: MspParser::new(),
        }
    }

    /// Unwrap and return the inner ByteTransport (e.g. for protocol switching)
    #[allow(dead_code)]
    pub fn into_inner(self) -> Box<dyn ByteTransport> {
        self.inner
    }
}

impl Transport for MspTransport {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String> {
        // Encode and send MSP v2 frame
        let frame = MspCodec::encode_v2(code, payload);
        self.inner
            .write_bytes(&frame)
            .map_err(|e| format!("MSP write failed: {}", e))?;

        // Read until we get the matching response or timeout
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_millis(MSP_RESPONSE_TIMEOUT_MS);

        loop {
            if Instant::now() > deadline {
                return Err(format!("MSP response timeout for command 0x{:04X}", code));
            }

            match self.inner.read_bytes(&mut buf) {
                Ok(0) => {
                    // No data available (timeout from underlying transport) — retry
                }
                Ok(n) => {
                    for &byte in &buf[..n] {
                        if let Some(msg) = self.parser.push(byte) {
                            if msg.code == code {
                                return Ok(msg);
                            }
                            // Non-matching message — discard (unsolicited or out-of-order)
                        }
                    }
                }
                Err(crate::transport::TransportError::Timeout) => {
                    // Retry until deadline
                }
                Err(crate::transport::TransportError::Disconnected) => {
                    return Err("Transport disconnected".to_string());
                }
                Err(e) => {
                    return Err(format!("MSP read error: {}", e));
                }
            }
        }
    }

    fn description(&self) -> String {
        self.inner.description()
    }
}
