// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

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
use num_traits::FromPrimitive;
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
///
/// `reserve_home` reflects the firmware's home-slot convention: ArduPilot stores the home location as
/// mission item 0 (dropped here so the planner only edits real waypoints), while PX4 has no home slot
/// (item 0 is the first real waypoint). See `ardu_mission_download`.
/// `progress(current, total)` is invoked once with `(0, count)` as soon as the FC reports its item
/// count, then after each item is received — so callers can surface an "x of n" download indicator.
/// Pass `|_, _| {}` when no progress reporting is needed (fence/rally).
pub fn download(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    reserve_home: bool,
    mission_type: MavMissionType,
    mut progress: impl FnMut(u16, u16),
) -> Result<Vec<ArduWaypoint>, String> {
    let rx = register(cmd_tx)?;

    // 1. Request mission list
    send(cmd_tx, MavMessage::MISSION_REQUEST_LIST(MISSION_REQUEST_LIST_DATA {
        target_system: fc_sysid,
        target_component: 0,
        mission_type,
    }))?;

    // 2. Wait for item count
    let count = match wait_for_count(&rx, COUNT_TIMEOUT) {
        Ok(c) => c,
        Err(e) => { unregister(cmd_tx); return Err(e); }
    };
    log::info!("MAVLink mission download: FC reports {} items", count);
    // Report progress in terms of *real* waypoints: ArduPilot's seq-0 home slot (reserve_home) is
    // dropped from the result, so it's excluded from the count too — keeping "x of n" consistent with
    // the final waypoint total across MSP/PX4/ArduPilot.
    let total = count.saturating_sub(reserve_home as u16);
    progress(0, total);

    if count == 0 {
        let _ = send(cmd_tx, make_ack(fc_sysid, MavMissionResult::MAV_MISSION_ACCEPTED, mission_type));
        unregister(cmd_tx);
        return Ok(vec![]);
    }

    // 3. Request and receive each item.
    // ArduPilot mission slot 0 is ALWAYS the home location, not a real waypoint — every item the
    // operator authored is at seq 1..count. We still request seq 0 (the FC streams items in order),
    // but drop it from the returned list so the planner shows/edits only real waypoints. `upload`
    // mirrors this by re-injecting a home placeholder at seq 0. PX4 has no home slot: every item
    // (including seq 0) is a real waypoint, so when `reserve_home` is false nothing is dropped.
    let mut items = Vec::with_capacity(count.saturating_sub(reserve_home as u16) as usize);
    for seq in 0..count {
        if let Err(e) = send(cmd_tx, MavMessage::MISSION_REQUEST_INT(MISSION_REQUEST_INT_DATA {
            target_system: fc_sysid,
            target_component: 0,
            seq,
            mission_type,
        })) {
            unregister(cmd_tx);
            return Err(e);
        }

        match wait_for_item(&rx, seq, ITEM_TIMEOUT) {
            Ok(wp) => {
                if reserve_home && seq == 0 {
                    log::debug!("MAVLink download seq 0 (home slot, dropped): {wp:?}");
                } else {
                    log::debug!("MAVLink download item seq {seq}: {wp:?}");
                    items.push(wp);
                }
            }
            Err(e) => { unregister(cmd_tx); return Err(e); }
        }
        // seq 0 with reserve_home is the home slot → stays "0 of n"; real WPs count from 1.
        progress((seq + 1).saturating_sub(reserve_home as u16), total);
    }

    // 4. Acknowledge
    let _ = send(cmd_tx, make_ack(fc_sysid, MavMissionResult::MAV_MISSION_ACCEPTED, mission_type));
    unregister(cmd_tx);
    log::info!("MAVLink mission download complete: {} items", items.len());
    Ok(items)
}

/// Upload waypoints to the FC, replacing its current mission.
// `#[allow(deprecated)]`: ArduPilot can request items with the deprecated `MISSION_REQUEST` (float)
// variant instead of `MISSION_REQUEST_INT` — we must answer both or the FC stalls and cancels.
#[allow(deprecated)]
pub fn upload(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    waypoints: &[ArduWaypoint],
    reserve_home: bool,
    mission_type: MavMissionType,
) -> Result<(), String> {
    let rx = register(cmd_tx)?;
    // ArduPilot reserves mission slot 0 for home, so the wire count is waypoints + 1: seq 0 is a
    // home placeholder and the operator's waypoints follow at seq 1.. (download dropped slot 0). PX4
    // has no home slot: the waypoints map straight to seq 0..len.
    let count = waypoints.len() as u16 + reserve_home as u16;

    // 1. Announce item count
    log::info!(
        "MAVLink mission upload start: announcing {} items ({} waypoints{})",
        count, waypoints.len(), if reserve_home { " + home slot" } else { "" }
    );
    if let Err(e) = send(cmd_tx, MavMessage::MISSION_COUNT(MISSION_COUNT_DATA {
        target_system: fc_sysid,
        target_component: 0,
        count,
        mission_type,
        opaque_id: 0,
    })) {
        unregister(cmd_tx);
        return Err(e);
    }

    // 2. Respond to the FC's item requests (MISSION_REQUEST_INT *or* the deprecated MISSION_REQUEST)
    //    until we receive MISSION_ACK.
    let deadline = Instant::now() + UPLOAD_DEADLINE;
    loop {
        if Instant::now() > deadline {
            unregister(cmd_tx);
            return Err("Upload timed out waiting for FC".into());
        }
        match rx.recv_timeout(ITEM_TIMEOUT) {
            Ok(MavMessage::MISSION_REQUEST_INT(req)) => {
                if let Err(e) = respond_item(cmd_tx, req.seq, waypoints, fc_sysid, reserve_home, mission_type) {
                    unregister(cmd_tx);
                    return Err(e);
                }
            }
            Ok(MavMessage::MISSION_REQUEST(req)) => {
                // Deprecated float-coordinate request — still answer with MISSION_ITEM_INT (the FC
                // accepts it). Without this the upload stalls → MAV_MISSION_OPERATION_CANCELLED.
                // (ArduPilot SITL uses this variant.)
                if let Err(e) = respond_item(cmd_tx, req.seq, waypoints, fc_sysid, reserve_home, mission_type) {
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
            Ok(other) => {
                // Anything else on the mission channel during an upload is unexpected (e.g. another GCS
                // starting its own mission op → the FC cancels ours). Log it so we can tell.
                log::warn!("MAVLink upload: unexpected mission msg during upload: {other:?}");
            }
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

/// Send the mission item the FC requested for `seq`. With `reserve_home` (ArduPilot): seq 0 = home
/// placeholder, seq 1..=len = the operator's waypoints. Without it (PX4): seq 0..len map straight to
/// the waypoints. Always replies with MISSION_ITEM_INT (the FC accepts it for either request variant).
fn respond_item(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    seq: u16,
    waypoints: &[ArduWaypoint],
    fc_sysid: u8,
    reserve_home: bool,
    mission_type: MavMissionType,
) -> Result<(), String> {
    let item = if reserve_home && seq == 0 {
        log::debug!("MAVLink upload seq 0 (home placeholder)");
        home_item(waypoints, fc_sysid)
    } else {
        let idx = seq as usize - reserve_home as usize;
        if idx >= waypoints.len() {
            return Err(format!("FC requested WP {} but mission has only {}", seq, waypoints.len()));
        }
        log::debug!("MAVLink upload item seq {seq}: {:?}", waypoints[idx]);
        wp_to_item(&waypoints[idx], seq, fc_sysid, mission_type)
    };
    send(cmd_tx, MavMessage::MISSION_ITEM_INT(item))
}

/// Clear all missions stored on the FC.
pub fn clear(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    mission_type: MavMissionType,
) -> Result<(), String> {
    let rx = register(cmd_tx)?;

    if let Err(e) = send(cmd_tx, MavMessage::MISSION_CLEAR_ALL(MISSION_CLEAR_ALL_DATA {
        target_system: fc_sysid,
        target_component: 0,
        mission_type,
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

fn make_ack(target: u8, result: MavMissionResult, mission_type: MavMissionType) -> MavMessage {
    MavMessage::MISSION_ACK(MISSION_ACK_DATA {
        target_system: target,
        target_component: 0,
        mavtype: result,
        mission_type,
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

/// Home placeholder for mission slot 0. ArduPilot replaces slot 0 with the vehicle's actual home on
/// upload, so the coordinates are nominal — we seed them from the first authored waypoint (a sane,
/// in-area fallback) so a firmware that *does* keep them never lands on 0/0 in the ocean.
fn home_item(waypoints: &[ArduWaypoint], target: u8) -> MISSION_ITEM_INT_DATA {
    let (lat, lon) = waypoints.first().map(|w| (w.lat, w.lon)).unwrap_or((0, 0));
    MISSION_ITEM_INT_DATA {
        target_system:    target,
        target_component: 0,
        seq:              0,
        frame:            MavFrame::MAV_FRAME_GLOBAL,
        command:          MavCmd::MAV_CMD_NAV_WAYPOINT,
        current:          1, // slot 0 is the "current"/home item
        autocontinue:     1,
        param1: 0.0, param2: 0.0, param3: 0.0, param4: 0.0,
        x: lat, y: lon, z: 0.0,
        mission_type: MavMissionType::MAV_MISSION_TYPE_MISSION,
    }
}

fn wp_to_item(wp: &ArduWaypoint, seq: u16, target: u8, mission_type: MavMissionType) -> MISSION_ITEM_INT_DATA {
    MISSION_ITEM_INT_DATA {
        target_system:    target,
        target_component: 0,
        seq,
        frame:        u8_to_frame(wp.frame),
        command:      u16_to_cmd(wp.command),
        // `current` flags the active waypoint of a real mission (slot 0); meaningless for fence/rally.
        current:      if seq == 0 && mission_type == MavMissionType::MAV_MISSION_TYPE_MISSION { 1 } else { 0 },
        autocontinue: wp.autocontinue as u8,
        param1:       wp.param1,
        param2:       wp.param2,
        param3:       wp.param3,
        param4:       wp.param4,
        x:            wp.lat,
        y:            wp.lon,
        z:            wp.alt,
        mission_type,
    }
}

// Round-trip-faithful int → enum: the MavCmd/MavFrame enums (ardupilotmega dialect) cover every
// command/frame ArduPilot emits, so we map by their numeric discriminant via FromPrimitive instead
// of a hand-maintained whitelist. This preserves any command the FC sends — including ones Kite has
// no dedicated editor for yet — on download→upload, instead of silently rewriting them to a plain
// waypoint. Truly-unknown values (not in the dialect) fall back to a safe default.
fn u8_to_frame(v: u8) -> MavFrame {
    MavFrame::from_u8(v).unwrap_or(MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT)
}

fn u16_to_cmd(v: u16) -> MavCmd {
    MavCmd::from_u16(v).unwrap_or(MavCmd::MAV_CMD_NAV_WAYPOINT)
}
