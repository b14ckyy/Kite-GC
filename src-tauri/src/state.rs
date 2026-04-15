// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::Mutex;

use crate::msp::FcInfo;
use crate::scheduler::SchedulerHandle;

/// Global application state managed by Tauri
pub struct AppState {
    /// Active scheduler handle (None when disconnected)
    pub scheduler: Mutex<Option<SchedulerHandle>>,
    /// Flight controller info from last successful handshake
    pub fc_info: Mutex<Option<FcInfo>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            scheduler: Mutex::new(None),
            fc_info: Mutex::new(None),
        }
    }
}
