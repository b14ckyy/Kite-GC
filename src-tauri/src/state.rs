// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::Mutex;

use crate::msp::FcInfo;
use crate::transport::serial::SerialConnection;

/// Global application state managed by Tauri
pub struct AppState {
    /// Active serial connection (None when disconnected)
    pub connection: Mutex<Option<SerialConnection>>,
    /// Flight controller info from last successful handshake
    pub fc_info: Mutex<Option<FcInfo>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            connection: Mutex::new(None),
            fc_info: Mutex::new(None),
        }
    }
}
