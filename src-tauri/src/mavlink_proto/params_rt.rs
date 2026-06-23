// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Runtime MAVLink parameter reads (geofence params: ArduPilot FENCE_* / PX4 GF_*). Mirrors the mission
// microprotocol's request/receiver pattern via `RegisterParamReceiver`. Writes reuse
// `control::set_param` (fire-and-forget PARAM_SET). See docs/active/GEOFENCE.md.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{MavMessage, PARAM_REQUEST_READ_DATA};

use super::handler::MavlinkCommand;

const PARAM_TIMEOUT: Duration = Duration::from_secs(3);

fn pack_param_id(name: &str) -> [u8; 16] {
    let mut id = [0u8; 16];
    let b = name.as_bytes();
    let n = b.len().min(16);
    id[..n].copy_from_slice(&b[..n]);
    id
}

fn send(cmd_tx: &mpsc::Sender<MavlinkCommand>, msg: MavMessage) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::SendMessage { msg, reply: reply_tx })
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    reply_rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "MAVLink send timed out".to_string())?
}

/// Read the given parameters by name; returns the ones the FC actually reports (missing names are
/// simply absent — e.g. FENCE_* on PX4 or GF_* on ArduPilot). Best-effort, used to populate the
/// geofence panel's core-param controls.
pub fn read_params(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    names: &[&str],
) -> HashMap<String, f32> {
    let (tx, rx) = mpsc::channel();
    if cmd_tx.send(MavlinkCommand::RegisterParamReceiver(tx)).is_err() {
        return HashMap::new();
    }
    std::thread::sleep(Duration::from_millis(10)); // let the handler pick up the registration

    let mut out: HashMap<String, f32> = HashMap::new();
    for &name in names {
        if send(cmd_tx, MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA {
            param_index: -1,
            target_system: fc_sysid,
            target_component: 1, // MAV_COMP_ID_AUTOPILOT1
            param_id: pack_param_id(name).into(),
        })).is_err() {
            continue;
        }
        let deadline = Instant::now() + PARAM_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { break; }
            match rx.recv_timeout(remaining) {
                Ok(MavMessage::PARAM_VALUE(pv)) => {
                    let end = pv.param_id.iter().position(|&c| c == 0).unwrap_or(pv.param_id.len());
                    let pname: String = pv.param_id[..end].iter().map(|&c| c as char).collect();
                    out.insert(pname.clone(), pv.param_value);
                    if pname == name { break; } // got the one we asked for
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    }
    let _ = cmd_tx.send(MavlinkCommand::UnregisterParamReceiver);
    out
}
