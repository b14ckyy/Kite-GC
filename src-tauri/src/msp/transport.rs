// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Transport
// Wraps a ByteTransport with MSP v2 framing and response parsing.
// This is the bridge between protocol-agnostic byte I/O and MSP request/response.

use std::time::{Duration, Instant};

use crate::flightlog::msp_raw_logger::{log_to_sink, MspRawSink, DIR_IN, DIR_OUT};
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
    /// Set once a fatal transport error (device gone) is seen — see `Transport::is_connection_lost`.
    connection_lost: bool,
    /// Shared raw-serial log sink (ADR-049). Every outgoing frame ('o') and incoming read-chunk ('i')
    /// is captured here in mwptools' v2 format while the recorder has a logger open; otherwise a no-op.
    raw_sink: MspRawSink,
}

impl MspTransport {
    /// Wrap a ByteTransport with MSP framing. `raw_sink` is the shared MSP raw-log slot (the recorder
    /// owns its lifecycle); pass an empty `Arc::new(Mutex::new(None))` to disable raw capture.
    pub fn new(transport: Box<dyn ByteTransport>, raw_sink: MspRawSink) -> Self {
        Self {
            inner: transport,
            parser: MspParser::new(),
            connection_lost: false,
            raw_sink,
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
        if let Err(e) = self.inner.write_bytes(&frame) {
            // A failed write means the device is gone (the local port handle is invalid) — fatal,
            // distinct from a no-reply timeout on the air link.
            self.connection_lost = true;
            return Err(format!("MSP write failed: {}", e));
        }
        // Raw-log the outgoing frame (ADR-049).
        log_to_sink(&self.raw_sink, DIR_OUT, &frame);

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
                    // Raw-log the incoming chunk (ADR-049) — mirrors mwp-serial-cap (per read-chunk).
                    log_to_sink(&self.raw_sink, DIR_IN, &buf[..n]);
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
                    self.connection_lost = true; // device gone
                    return Err("Transport disconnected".to_string());
                }
                Err(e) => {
                    self.connection_lost = true; // IO error on a removed device
                    return Err(format!("MSP read error: {}", e));
                }
            }
        }
    }

    fn description(&self) -> String {
        self.inner.description()
    }

    fn is_connection_lost(&self) -> bool {
        self.connection_lost
    }
}
