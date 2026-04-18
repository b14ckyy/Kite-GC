// Transport Module
// Abstracts communication transports: Serial, TCP/UDP, BLE, etc.

pub mod serial;
pub mod tcp;
pub mod udp;
pub mod ble;

use std::fmt;

use crate::msp::MspMessage;

/// Unified transport trait — all transports implement this
/// so the scheduler and handshake code are transport-agnostic.
pub trait Transport: Send {
    /// Send an MSP v2 request and wait for the matching response
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> Result<MspMessage, String>;

    /// Human-readable description of this transport (for logging)
    fn description(&self) -> String;
}

/// Connection state shared across all transports
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionState::Disconnected => write!(f, "Disconnected"),
            ConnectionState::Connecting => write!(f, "Connecting..."),
            ConnectionState::Connected => write!(f, "Connected"),
            ConnectionState::Error(e) => write!(f, "Error: {}", e),
        }
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
