// MAVLink Mission Microprotocol
// Implements the MAVLink mission upload/download/clear handshake.
// Ref: https://mavlink.io/en/services/mission.html
//
// This module works entirely through the MavlinkCommand channel — it does NOT
// hold the AppState mutex during the operation, so disconnect can proceed safely.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{
    MavFrame, MavCmd, MavMissionType, MavMissionResult, MavMessage,
    MISSION_REQUEST_LIST_DATA, MISSION_COUNT_DATA, MISSION_REQUEST_INT_DATA,
    MISSION_ITEM_INT_DATA, MISSION_ACK_DATA, MISSION_CLEAR_ALL_DATA,
};
use serde::{Deserialize, Serialize};

use super::handler::MavlinkCommand;

/// Timeout per individual item request/response exchange
const ITEM_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for the initial MISSION_COUNT after sending MISSION_REQUEST_LIST
const COUNT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall deadline for the upload handshake
const UPLOAD_DEADLINE: Duration = Duration::from_secs(60);

// ── Public data type ──────────────────────────────────────────────────────────

/// ArduPilot waypoint exchanged over Tauri IPC.
/// Matches the TypeScript `ArduWaypoint` interface exactly.
/// lat/lon are stored as degrees × 1e7 (i32), same internal convention as INAV.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArduWaypoint {
    pub command: u16,
    pub frame: u8,
    pub param1: f32,
    pub param2: f32,
    pub param3: f32,
    pub param4: f32,
    pub lat: i32,
    pub lon: i32,
    pub alt: f32,
    pub autocontinue: bool,
}

// ── Public protocol functions ─────────────────────────────────────────────────

/// Download the mission stored on the FC.
pub fn download(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
) -> Result<Vec<ArduWaypoint>, String> {
    let rx = register(cmd_tx)?;

    // 1. Request mission list
    send(cmd_tx, MavMessage::MISSION_REQUEST_LIST(MISSION_REQUEST_LIST_DATA {
        target_system: fc_sysid,
        target_component: 0,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
    }))?;

    // 2. Wait for item count
    let count = match wait_for_count(&rx, COUNT_TIMEOUT) {
        Ok(c) => c,
        Err(e) => { unregister(cmd_tx); return Err(e); }
    };
    log::info!("MAVLink mission download: FC reports {} items", count);

    if count == 0 {
        let _ = send(cmd_tx, make_ack(fc_sysid, MavMissionResult::MAV_MISSION_ACCEPTED));
        unregister(cmd_tx);
        return Ok(vec![]);
    }

    // 3. Request and receive each item
    let mut items = Vec::with_capacity(count as usize);
    for seq in 0..count {
        if let Err(e) = send(cmd_tx, MavMessage::MISSION_REQUEST_INT(MISSION_REQUEST_INT_DATA {
            target_system: fc_sysid,
            target_component: 0,
            seq,
            mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
        })) {
            unregister(cmd_tx);
            return Err(e);
        }

        match wait_for_item(&rx, seq, ITEM_TIMEOUT) {
            Ok(wp) => items.push(wp),
            Err(e) => { unregister(cmd_tx); return Err(e); }
        }
    }

    // 4. Acknowledge
    let _ = send(cmd_tx, make_ack(fc_sysid, MavMissionResult::MAV_MISSION_ACCEPTED));
    unregister(cmd_tx);
    log::info!("MAVLink mission download complete: {} items", items.len());
    Ok(items)
}

/// Upload waypoints to the FC, replacing its current mission.
pub fn upload(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    waypoints: &[ArduWaypoint],
) -> Result<(), String> {
    let rx = register(cmd_tx)?;
    let count = waypoints.len() as u16;

    // 1. Announce item count
    if let Err(e) = send(cmd_tx, MavMessage::MISSION_COUNT(MISSION_COUNT_DATA {
        target_system: fc_sysid,
        target_component: 0,
        count,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
        opaque_id: 0,
    })) {
        unregister(cmd_tx);
        return Err(e);
    }

    // 2. Respond to MISSION_REQUEST_INT messages until we receive MISSION_ACK
    let deadline = Instant::now() + UPLOAD_DEADLINE;
    loop {
        if Instant::now() > deadline {
            unregister(cmd_tx);
            return Err("Upload timed out waiting for FC".into());
        }
        match rx.recv_timeout(ITEM_TIMEOUT) {
            Ok(MavMessage::MISSION_REQUEST_INT(req)) => {
                let seq = req.seq as usize;
                if seq >= waypoints.len() {
                    unregister(cmd_tx);
                    return Err(format!("FC requested WP {} but mission has only {}", seq, waypoints.len()));
                }
                let item = wp_to_item(&waypoints[seq], req.seq, fc_sysid);
                if let Err(e) = send(cmd_tx, MavMessage::MISSION_ITEM_INT(item)) {
                    unregister(cmd_tx);
                    return Err(e);
                }
            }
            Ok(MavMessage::MISSION_ACK(ack)) => {
                unregister(cmd_tx);
                return if ack.mavtype == MavMissionResult::MAV_MISSION_ACCEPTED {
                    log::info!("MAVLink mission upload complete: {} items accepted", count);
                    Ok(())
                } else {
                    Err(format!("FC rejected upload: {:?}", ack.mavtype))
                };
            }
            Ok(_) => {} // other mission messages during upload — ignore
            Err(mpsc::RecvTimeoutError::Timeout) => {
                unregister(cmd_tx);
                return Err("FC stopped responding during upload".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("MAVLink handler stopped".into());
            }
        }
    }
}

/// Clear all missions stored on the FC.
pub fn clear(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
) -> Result<(), String> {
    let rx = register(cmd_tx)?;

    if let Err(e) = send(cmd_tx, MavMessage::MISSION_CLEAR_ALL(MISSION_CLEAR_ALL_DATA {
        target_system: fc_sysid,
        target_component: 0,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
    })) {
        unregister(cmd_tx);
        return Err(e);
    }

    match rx.recv_timeout(ITEM_TIMEOUT) {
        Ok(MavMessage::MISSION_ACK(ack)) => {
            unregister(cmd_tx);
            if ack.mavtype == MavMissionResult::MAV_MISSION_ACCEPTED {
                Ok(())
            } else {
                Err(format!("FC rejected clear: {:?}", ack.mavtype))
            }
        }
        Ok(_) => { unregister(cmd_tx); Err("Unexpected response during clear".into()) }
        Err(e) => { unregister(cmd_tx); Err(format!("Clear failed: {}", e)) }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn register(cmd_tx: &mpsc::Sender<MavlinkCommand>) -> Result<mpsc::Receiver<MavMessage>, String> {
    let (tx, rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::RegisterMissionReceiver(tx))
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    // Small sleep to ensure handler loop picks up the registration before we send the first message.
    // Without this there is a tiny race where the first FC response arrives before registration.
    std::thread::sleep(Duration::from_millis(10));
    Ok(rx)
}

fn unregister(cmd_tx: &mpsc::Sender<MavlinkCommand>) {
    let _ = cmd_tx.send(MavlinkCommand::UnregisterMissionReceiver);
}

fn send(cmd_tx: &mpsc::Sender<MavlinkCommand>, msg: MavMessage) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::SendMessage { msg, reply: reply_tx })
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    reply_rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "MAVLink send timed out".to_string())?
}

fn make_ack(target: u8, result: MavMissionResult) -> MavMessage {
    MavMessage::MISSION_ACK(MISSION_ACK_DATA {
        target_system: target,
        target_component: 0,
        mavtype: result,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
        opaque_id: 0,
    })
}

fn wait_for_count(rx: &mpsc::Receiver<MavMessage>, timeout: Duration) -> Result<u16, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Timeout waiting for MISSION_COUNT from FC".into());
        }
        match rx.recv_timeout(remaining) {
            Ok(MavMessage::MISSION_COUNT(mc)) => return Ok(mc.count),
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("Timeout waiting for MISSION_COUNT from FC".into());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("MAVLink handler stopped".into());
            }
        }
    }
}

fn wait_for_item(
    rx: &mpsc::Receiver<MavMessage>,
    expected_seq: u16,
    timeout: Duration,
) -> Result<ArduWaypoint, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("Timeout waiting for MISSION_ITEM_INT seq={}", expected_seq));
        }
        match rx.recv_timeout(remaining) {
            Ok(MavMessage::MISSION_ITEM_INT(item)) if item.seq == expected_seq => {
                return Ok(item_to_wp(&item));
            }
            Ok(_) => continue, // wrong seq or other mission msg — skip
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!("Timeout for MISSION_ITEM_INT seq={}", expected_seq));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("MAVLink handler stopped".into());
            }
        }
    }
}

// ── MAVLink ↔ ArduWaypoint conversion ────────────────────────────────────────

fn item_to_wp(item: &MISSION_ITEM_INT_DATA) -> ArduWaypoint {
    ArduWaypoint {
        command: item.command as u16,
        frame:   item.frame as u8,
        param1:  item.param1,
        param2:  item.param2,
        param3:  item.param3,
        param4:  item.param4,
        lat:     item.x,
        lon:     item.y,
        alt:     item.z,
        autocontinue: item.autocontinue != 0,
    }
}

fn wp_to_item(wp: &ArduWaypoint, seq: u16, target: u8) -> MISSION_ITEM_INT_DATA {
    MISSION_ITEM_INT_DATA {
        target_system:    target,
        target_component: 0,
        seq,
        frame:        u8_to_frame(wp.frame),
        command:      u16_to_cmd(wp.command),
        current:      if seq == 0 { 1 } else { 0 },
        autocontinue: wp.autocontinue as u8,
        param1:       wp.param1,
        param2:       wp.param2,
        param3:       wp.param3,
        param4:       wp.param4,
        x:            wp.lat,
        y:            wp.lon,
        z:            wp.alt,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
    }
}

fn u8_to_frame(v: u8) -> MavFrame {
    match v {
        0  => MavFrame::MAV_FRAME_GLOBAL,
        3  => MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT,
        10 => MavFrame::MAV_FRAME_GLOBAL_TERRAIN_ALT,
        _  => MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT,
    }
}

// Wire command 201 maps to `MAV_CMD_DO_SET_ROI` — that enum variant *is* value 201; the
// newer `*_ROI_LOCATION`/`*_ROI_NONE` commands have different IDs, so for parsing a mission
// item carrying cmd 201 this deprecated variant is the correct (only) match.
#[allow(deprecated)]
fn u16_to_cmd(v: u16) -> MavCmd {
    match v {
        16  => MavCmd::MAV_CMD_NAV_WAYPOINT,
        17  => MavCmd::MAV_CMD_NAV_LOITER_UNLIM,
        18  => MavCmd::MAV_CMD_NAV_LOITER_TURNS,
        19  => MavCmd::MAV_CMD_NAV_LOITER_TIME,
        20  => MavCmd::MAV_CMD_NAV_RETURN_TO_LAUNCH,
        21  => MavCmd::MAV_CMD_NAV_LAND,
        22  => MavCmd::MAV_CMD_NAV_TAKEOFF,
        112 => MavCmd::MAV_CMD_CONDITION_DELAY,
        177 => MavCmd::MAV_CMD_DO_JUMP,
        178 => MavCmd::MAV_CMD_DO_CHANGE_SPEED,
        201 => MavCmd::MAV_CMD_DO_SET_ROI,
        _   => MavCmd::MAV_CMD_NAV_WAYPOINT,
    }
}
