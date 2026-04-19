// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::Mutex;

use crate::mavlink_proto::MavlinkHandle;
use crate::msp::FcInfo;
use crate::scheduler::SchedulerHandle;

/// Which protocol is currently active
pub enum ActiveProtocol {
    Msp(SchedulerHandle),
    Mavlink(MavlinkHandle),
}

/// Global application state managed by Tauri
pub struct AppState {
    /// Active protocol handler (None when disconnected)
    pub protocol: Mutex<Option<ActiveProtocol>>,
    /// Flight controller info from last successful handshake
    pub fc_info: Mutex<Option<FcInfo>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            protocol: Mutex::new(None),
            fc_info: Mutex::new(None),
        }
    }
}
