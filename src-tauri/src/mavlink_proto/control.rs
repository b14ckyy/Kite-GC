// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Vehicle-Control Commands
// Fire a COMMAND_LONG / COMMAND_INT to the FC and wait for its COMMAND_ACK.
// Ref: https://mavlink.io/en/services/command.html
//
// Like the mission microprotocol (mission.rs), this works entirely through the MavlinkCommand
// channel — it never holds the AppState mutex during the exchange, so disconnect stays safe.
//
// MAVLink commands are NOT request→response like MSP: we send the command and the FC replies
// asynchronously with COMMAND_ACK (ACCEPTED / DENIED / TEMPORARILY_REJECTED / UNSUPPORTED / …).
// We register a receiver, send, then match the ACK by command id and resolve the blocking call.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{
    MavMessage, MavCmd, MavFrame, MavResult, MavParamType,
    COMMAND_LONG_DATA, COMMAND_INT_DATA, PARAM_SET_DATA,
};

use super::handler::MavlinkCommand;

/// The autopilot is conventionally component 1 (MAV_COMP_ID_AUTOPILOT1). ArduPilot and PX4 both
/// answer commands addressed to it.
const AUTOPILOT_COMPONENT: u8 = 1;

/// How long we wait for the COMMAND_ACK before giving up. Commands are cheap; the FC acks fast.
const ACK_TIMEOUT: Duration = Duration::from_secs(3);

/// Send a `COMMAND_LONG` and wait for its `COMMAND_ACK`. `params` are param1..param7.
pub fn send_command_long(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    command: MavCmd,
    params: [f32; 7],
) -> Result<(), String> {
    let rx = register(cmd_tx)?;
    let msg = MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: fc_sysid,
        target_component: AUTOPILOT_COMPONENT,
        command,
        confirmation: 0,
        param1: params[0],
        param2: params[1],
        param3: params[2],
        param4: params[3],
        param5: params[4],
        param6: params[5],
        param7: params[6],
    });
    if let Err(e) = send(cmd_tx, msg) {
        unregister(cmd_tx);
        return Err(e);
    }
    let result = wait_for_ack(&rx, command, ACK_TIMEOUT);
    unregister(cmd_tx);
    result
}

/// Send a `COMMAND_INT` and wait for its `COMMAND_ACK`. Used for commands carrying a global
/// position (lat/lon as int32 × 1e7) — `DO_REPOSITION` — where COMMAND_LONG's f32 params would
/// lose coordinate precision. `params` are param1..param4; `x`/`y` are lat/lon × 1e7; `z` is alt (m).
pub fn send_command_int(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    frame: MavFrame,
    command: MavCmd,
    params: [f32; 4],
    x: i32,
    y: i32,
    z: f32,
) -> Result<(), String> {
    let rx = register(cmd_tx)?;
    let msg = MavMessage::COMMAND_INT(COMMAND_INT_DATA {
        target_system: fc_sysid,
        target_component: AUTOPILOT_COMPONENT,
        frame,
        command,
        current: 0,
        autocontinue: 0,
        param1: params[0],
        param2: params[1],
        param3: params[2],
        param4: params[3],
        x,
        y,
        z,
    });
    if let Err(e) = send(cmd_tx, msg) {
        unregister(cmd_tx);
        return Err(e);
    }
    let result = wait_for_ack(&rx, command, ACK_TIMEOUT);
    unregister(cmd_tx);
    result
}

/// Set a single FC parameter (fire-and-forget `PARAM_SET`). Used for tunables that have no dedicated
/// command — e.g. the fixed-wing loiter radius (`WP_LOITER_RAD`). ArduPilot ignores `param_type` for
/// its REAL32 params; we don't wait for the PARAM_VALUE echo (non-critical, keeps it simple).
pub fn set_param(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    name: &str,
    value: f32,
) -> Result<(), String> {
    let mut param_id = [0u8; 16];
    let bytes = name.as_bytes();
    let n = bytes.len().min(16);
    param_id[..n].copy_from_slice(&bytes[..n]);

    send(cmd_tx, MavMessage::PARAM_SET(PARAM_SET_DATA {
        target_system: fc_sysid,
        target_component: AUTOPILOT_COMPONENT,
        param_id: param_id.into(),
        param_value: value,
        param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
    }))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn register(cmd_tx: &mpsc::Sender<MavlinkCommand>) -> Result<mpsc::Receiver<MavMessage>, String> {
    let (tx, rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::RegisterCommandReceiver(tx))
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    // Same tiny race guard as mission.rs: let the handler pick up the registration before we send.
    std::thread::sleep(Duration::from_millis(10));
    Ok(rx)
}

fn unregister(cmd_tx: &mpsc::Sender<MavlinkCommand>) {
    let _ = cmd_tx.send(MavlinkCommand::UnregisterCommandReceiver);
}

fn send(cmd_tx: &mpsc::Sender<MavlinkCommand>, msg: MavMessage) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::SendMessage { msg, reply: reply_tx })
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    reply_rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "MAVLink send timed out".to_string())?
}

/// Wait for the COMMAND_ACK that matches `command`. `IN_PROGRESS` acks are intermediate (some
/// commands report progress) — keep waiting for the final result.
fn wait_for_ack(
    rx: &mpsc::Receiver<MavMessage>,
    command: MavCmd,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("No COMMAND_ACK from FC (timed out)".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(MavMessage::COMMAND_ACK(ack)) if ack.command == command => {
                return match ack.result {
                    MavResult::MAV_RESULT_ACCEPTED => Ok(()),
                    MavResult::MAV_RESULT_IN_PROGRESS => continue,
                    MavResult::MAV_RESULT_TEMPORARILY_REJECTED =>
                        Err("Command temporarily rejected — try again".into()),
                    MavResult::MAV_RESULT_DENIED =>
                        Err("Command denied by the flight controller".into()),
                    MavResult::MAV_RESULT_UNSUPPORTED =>
                        Err("Command not supported by this firmware".into()),
                    MavResult::MAV_RESULT_FAILED =>
                        Err("Command failed on the flight controller".into()),
                    other => Err(format!("Command rejected: {:?}", other)),
                };
            }
            Ok(_) => continue, // ack for a different command, or stray msg — keep waiting
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("No COMMAND_ACK from FC (timed out)".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("MAVLink handler stopped".into());
            }
        }
    }
}
