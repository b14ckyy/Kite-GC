// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Transport Module
// Abstracts communication transports: Serial, TCP/UDP, BLE, etc.
//
// Two-layer architecture:
// 1. ByteTransport — protocol-agnostic byte-level I/O (read/write/close)
// 2. Protocol layers (MspTransport, MavlinkHandler) — built on top of ByteTransport

pub mod serial;
pub mod tcp;
pub mod udp;
pub mod ble;

use std::fmt;

use crate::msp::MspMessage;

// ── Transport Error ──────────────────────────────────────────────

/// Error type for byte-level transport operations
#[derive(Debug)]
pub enum TransportError {
    /// Read/recv timed out (no data available within timeout)
    Timeout,
    /// Connection was closed by remote or is no longer valid
    Disconnected,
    /// Generic I/O error with description
    Io(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Timeout => write!(f, "Transport timeout"),
            TransportError::Disconnected => write!(f, "Transport disconnected"),
            TransportError::Io(msg) => write!(f, "Transport I/O error: {}", msg),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                TransportError::Timeout
            }
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => TransportError::Disconnected,
            _ => TransportError::Io(e.to_string()),
        }
    }
}

// ── ByteTransport Trait ──────────────────────────────────────────

/// Protocol-agnostic byte-level transport.
///
/// All communication hardware (Serial, TCP, UDP, BLE) implements this trait.
/// Protocol layers (MSP, MAVLink) are built on top.
pub trait ByteTransport: Send {
    /// Read bytes into buffer. Returns number of bytes read.
    /// May return 0 on timeout (non-fatal) or TransportError on failure.
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Write all bytes to the transport.
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError>;

    /// Set the read/recv timeout for subsequent `read_bytes` calls. Lets a read-driven protocol loop
    /// tighten its cadence on demand: the MAVLink handler shortens it while streaming RC overrides so the
    /// send rate isn't quantized by the (deliberately coarse) idle read timeout. Default: no-op — the
    /// transport keeps its built-in timeout.
    fn set_read_timeout(&mut self, timeout: std::time::Duration) {
        let _ = timeout;
    }

    /// Human-readable description (for logging)
    fn description(&self) -> String;
}

// ── Legacy Transport Trait ───────────────────────────────────────

/// MSP-level transport trait — used by scheduler and handshake code.
/// Implemented by MspTransport (wraps ByteTransport + MspParser).
pub trait Transport: Send {
    /// Send an MSP v2 request and wait for the matching response
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String>;

    /// Like `msp_request` but with a caller-chosen response timeout (ms). Used by the RC link-speed
    /// probe to bound how long it waits for each SET_RAW_RC ACK. Default: ignore the timeout and
    /// delegate (transports that don't support it keep their built-in timeout).
    fn msp_request_timeout(&mut self, code: u16, payload: &[u8], timeout_ms: u64) -> Result<MspMessage, String> {
        let _ = timeout_ms;
        self.msp_request(code, payload)
    }

    /// Send an MSP v2 frame **without** waiting for the reply (fire-and-forget). Used for the RC
    /// injection stream (SET_RAW_RC / SET_AUX_RC), where blocking on each ACK would jitter the RC rate.
    /// The FC's ACK arrives asynchronously and is surfaced via `take_unsolicited_codes`. Default:
    /// delegate to `msp_request` and drop the reply.
    fn msp_send(&mut self, code: u16, payload: &[u8]) -> Result<(), String> {
        self.msp_request(code, payload).map(|_| ())
    }

    /// Like `msp_send`, but with the MSPv2 header flag set to 1 — INAV suppresses the reply for
    /// `MSP_SET_RAW_RC` (zero downlink for the RC stream). Used for the non-probe RAW_RC stream; AUX_RC
    /// keeps `msp_send` (flag 0) so its ACK can be observed. Default: delegate to `msp_send`.
    fn msp_send_no_reply(&mut self, code: u16, payload: &[u8]) -> Result<(), String> {
        self.msp_send(code, payload)
    }

    /// Drain the MSP codes of unsolicited / out-of-order responses seen since the last call — e.g. the
    /// echoed ACK of a fire-and-forget `msp_send` (a SET_AUX_RC confirmation). Default: none.
    fn take_unsolicited_codes(&mut self) -> Vec<u16> {
        Vec::new()
    }

    /// Human-readable description of this transport (for logging)
    fn description(&self) -> String;

    /// True once a **fatal** transport failure has occurred — the device is gone (write failed, or a
    /// `Disconnected`/IO read error), as opposed to a mere response timeout (an OTA stall, where the
    /// device is still attached). The scheduler tears the connection down on this; it never reacts to
    /// timeouts. Default `false` for transports that don't track it.
    fn is_connection_lost(&self) -> bool {
        false
    }
}

/// Information about an available port/device
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PortInfo {
    pub path: String,
    pub label: String,
    pub port_type: String,
}

/// Transport type identifier (serializable for frontend communication)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    Serial,
    Tcp,
    Udp,
    Ble,
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportType::Serial => write!(f, "Serial"),
            TransportType::Tcp => write!(f, "TCP"),
            TransportType::Udp => write!(f, "UDP"),
            TransportType::Ble => write!(f, "BLE"),
        }
    }
}
