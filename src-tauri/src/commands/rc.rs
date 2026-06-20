// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-control FC-side commands — read the INAV config that decides whether/which GCS-injected RC
// channels actually take effect (docs/active/RC_CONTROL.md §3/§4). One-shot MSP reads through the
// scheduler. The local HID side lives in commands/hid.rs; the byte encoders in msp/rc_encode.rs.

use serde::Serialize;
use tauri::State;

use crate::msp::MSP2_COMMON_SETTING;
use crate::scheduler::SchedulerHandle;
use crate::state::{ActiveProtocol, AppState};

/// INAV settings relevant to GCS RC injection.
#[derive(Serialize)]
pub struct RcFcConfig {
    /// `receiver_type`: 0 = NONE, 1 = SERIAL, 2 = MSP.
    pub receiver_type: u8,
    /// `msp_override_channels` bitmask (CH1 = bit 0). `None` if the FC firmware lacks the setting
    /// (compiled without `USE_MSP_RC_OVERRIDE`).
    pub msp_override_channels: Option<u32>,
}

/// Read a setting by name via MSP2_COMMON_SETTING (null-terminated name → raw value bytes).
fn read_setting(handle: &SchedulerHandle, name: &str) -> Result<Vec<u8>, String> {
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    handle.msp_request(MSP2_COMMON_SETTING, &payload)
}

#[tauri::command(async)]
pub fn rc_read_fc_config(state: State<'_, AppState>) -> Result<RcFcConfig, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(_) => return Err("FC is not running MSP (INAV)".into()),
        None => return Err("Not connected".into()),
    };

    let rx = read_setting(handle, "receiver_type")?;
    let receiver_type = *rx.first().ok_or("empty receiver_type response")?;

    // Present only when the firmware compiles USE_MSP_RC_OVERRIDE; otherwise the read errors → None.
    let msp_override_channels = match read_setting(handle, "msp_override_channels") {
        Ok(b) if b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    };

    eprintln!(
        "[RC] FC config: receiver_type={receiver_type} msp_override_channels={msp_override_channels:?}"
    );
    Ok(RcFcConfig { receiver_type, msp_override_channels })
}
