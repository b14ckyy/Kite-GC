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
    /// The OS error text behind `connection_lost`, kept so the scheduler's teardown warning can name the
    /// actual cause. Recorded here rather than at the call sites because the scheduler sends
    /// fire-and-forget (`msp_send(..).is_ok()`) and would otherwise drop the write error on the floor.
    lost_reason: Option<String>,
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
            lost_reason: None,
            raw_sink,
        }
    }

    /// Flag the transport as fatally lost and keep the first cause seen (later errors are follow-ups of
    /// the same failure, so the first one is the informative one).
    fn mark_lost(&mut self, reason: String) {
        self.connection_lost = true;
        if self.lost_reason.is_none() {
            self.lost_reason = Some(reason);
        }
    }

    /// Unwrap and return the inner ByteTransport (e.g. for protocol switching)
    #[allow(dead_code)]
    pub fn into_inner(self) -> Box<dyn ByteTransport> {
        self.inner
    }

    /// Write a pre-encoded MSP frame, raw-logging it on success. A write TIMEOUT is a retryable
    /// stall, not a lost device: on Windows the short scheduler read timeout applies to writes too
    /// (`WriteTotalTimeoutConstant`), and a USB-CDC FC that NAKs its OUT endpoint while producing
    /// telemetry trips it (ERROR_SEM_TIMEOUT, os error 121 — field case TBS Lucid H7 Wing). The
    /// frame is dropped (a telemetry poll simply retries next cycle; the FC's MSP parser resyncs on
    /// the next '$' if part of the frame made it out) and the session lives on. Any OTHER write
    /// error still means the device is gone and flags the connection lost.
    fn write_frame(&mut self, frame: Vec<u8>) -> Result<(), String> {
        match self.inner.write_bytes(&frame) {
            Ok(()) => {
                log_to_sink(&self.raw_sink, DIR_OUT, &frame);
                Ok(())
            }
            Err(crate::transport::TransportError::Timeout) => {
                log::debug!("MSP write timed out — frame dropped (device busy, not gone)");
                Err("MSP write timeout (frame dropped)".to_string())
            }
            Err(e) => {
                self.mark_lost(format!("write failed: {}", e));
                Err(format!("MSP send failed: {}", e))
            }
        }
    }
}

impl Transport for MspTransport {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String> {
        self.msp_request_timeout(code, payload, MSP_RESPONSE_TIMEOUT_MS)
    }

    fn msp_request_timeout(&mut self, code: u16, payload: &[u8], timeout_ms: u64) -> Result<MspMessage, String> {
        // Encode and send MSP v2 frame. `write_frame` raw-logs it (ADR-049) and classifies write
        // errors: a write timeout is a dropped frame (device busy), anything else flags the
        // connection lost.
        self.write_frame(MspCodec::encode_v2(code, payload))?;

        // Read until we get the matching response or timeout
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

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
                            // Non-matching frame (out-of-order/unsolicited) — drop it. This blocking path is
                            // only used for the pre-scheduler handshake; the running scheduler drains and
                            // matches all frames via `poll_incoming`.
                        }
                    }
                }
                Err(crate::transport::TransportError::Timeout) => {
                    // Retry until deadline
                }
                Err(crate::transport::TransportError::Disconnected) => {
                    self.mark_lost("read: transport disconnected".to_string()); // device gone
                    return Err("Transport disconnected".to_string());
                }
                Err(e) => {
                    // IO error on a removed device.
                    self.mark_lost(format!("read failed: {}", e));
                    return Err(format!("MSP read error: {}", e));
                }
            }
        }
    }

    fn msp_send(&mut self, code: u16, payload: &[u8]) -> Result<(), String> {
        self.write_frame(MspCodec::encode_v2(code, payload))
    }

    fn msp_send_no_reply(&mut self, code: u16, payload: &[u8]) -> Result<(), String> {
        // flag = 1 → INAV sends no reply for SET_RAW_RC (zero downlink for the RC stream).
        self.write_frame(MspCodec::encode_v2_flagged(code, payload, 1))
    }

    fn poll_incoming(&mut self) -> Result<Vec<MspMessage>, String> {
        // One read, bounded by the inner transport's read timeout (the scheduler sets this short so its
        // loop stays responsive). Parse every byte; return all complete frames for the scheduler to match
        // by code. A partially-received frame stays buffered in the parser across calls.
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        match self.inner.read_bytes(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                log_to_sink(&self.raw_sink, DIR_IN, &buf[..n]);
                for &byte in &buf[..n] {
                    if let Some(msg) = self.parser.push(byte) {
                        out.push(msg);
                    }
                }
            }
            Err(crate::transport::TransportError::Timeout) => {}
            Err(crate::transport::TransportError::Disconnected) => {
                self.mark_lost("poll: transport disconnected".to_string());
                return Err("Transport disconnected".to_string());
            }
            Err(e) => {
                self.mark_lost(format!("poll read failed: {}", e));
                return Err(format!("MSP read error: {}", e));
            }
        }
        Ok(out)
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.inner.set_read_timeout(timeout);
    }

    fn description(&self) -> String {
        self.inner.description()
    }

    fn is_connection_lost(&self) -> bool {
        self.connection_lost
    }

    fn connection_lost_reason(&self) -> Option<String> {
        self.lost_reason.clone()
    }
}
