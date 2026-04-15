// Transport Module
// Abstracts communication transports: Serial, TCP, BLE, etc.

pub mod serial;

use std::fmt;

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
