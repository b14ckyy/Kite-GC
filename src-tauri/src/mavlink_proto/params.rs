// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// One-shot MAVLink parameter reads issued on connect.
//
// Most state Kite needs streams continuously (telemetry), but a few facts live only in FC
// parameters. The header EKF indicator needs the active estimator core, which is the AHRS_EKF_TYPE
// parameter (2 = EKF2, 3 = EKF3) — there is no telemetry message that carries it. We fire a single
// PARAM_REQUEST_READ before the handler thread starts; the FC's PARAM_VALUE reply is decoded in the
// handler (emits `telemetry-ekf-type`). Fire-and-forget: if the reply is lost the UI just shows a
// generic "EKF" label until the next connect.

use ::mavlink::ardupilotmega::{MavCmd, MavMessage, COMMAND_LONG_DATA, PARAM_REQUEST_READ_DATA};

use crate::transport::ByteTransport;

use super::codec::{self, MavSequence};

/// Pack a parameter name into MAVLink's fixed 16-byte field (NUL-padded, truncated at 16).
fn pack_param_id(name: &str) -> [u8; 16] {
    let mut id = [0u8; 16];
    let bytes = name.as_bytes();
    let n = bytes.len().min(16);
    id[..n].copy_from_slice(&bytes[..n]);
    id
}

/// Request a single parameter by name (`param_index = -1` → look up by name). Fire-and-forget over the
/// (pre-handler) transport; the FC's PARAM_VALUE reply is decoded by the handler thread.
pub fn request_param(transport: &mut dyn ByteTransport, fc_sysid: u8, name: &str) {
    let header = codec::gcs_header();
    let mut seq = MavSequence::new();

    let msg = MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA {
        param_index: -1,
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        param_id: pack_param_id(name).into(),
    });
    let frame = codec::serialize_v2(&header, &msg, &mut seq);
    match transport.write_bytes(&frame) {
        Ok(()) => eprintln!("[MAVLINK-PARAM] requested {}", name),
        Err(e) => log::warn!("Failed to request {}: {}", name, e),
    }
}

/// Request the active EKF core (AHRS_EKF_TYPE) once (2 = EKF2, 3 = EKF3).
pub fn request_ekf_type(transport: &mut dyn ByteTransport, fc_sysid: u8) {
    request_param(transport, fc_sysid, "AHRS_EKF_TYPE");
}

/// Request Q_ENABLE once. An ArduPlane QuadPlane reports MAV_TYPE_FIXED_WING in its HEARTBEAT (so the
/// vehicle class can't be told apart from a plain plane by MAV_TYPE — ArduPilot issue #7137); Q_ENABLE=1
/// is the reliable QuadPlane signal, the same one Mission Planner uses.
pub fn request_quadplane_flag(transport: &mut dyn ByteTransport, fc_sysid: u8) {
    request_param(transport, fc_sysid, "Q_ENABLE");
}

/// HOME_POSITION MAVLink message id (common #242).
const MSG_ID_HOME_POSITION: f32 = 242.0;

/// Request HOME_POSITION once (MAV_CMD_REQUEST_MESSAGE 512) — the MAVLink counterpart of the MSP
/// path's one-shot MSP_WP(0) read, so a mid-flight connect / app restart recovers the real FC home
/// immediately. This is what Mission Planner does (via the older GET_HOME_POSITION); it cannot be
/// left to the 0.2 Hz SET_MESSAGE_INTERVAL push alone, because Full-telemetry mode resets that to
/// the FC default and ArduPilot does not stream HOME_POSITION in any SRn group — home then only
/// arrives on change. Fire-and-forget: the reply is decoded by the handler thread (emits
/// `home-position`); the COMMAND_ACK is dispatched as a stray and dropped.
pub fn request_home_position(transport: &mut dyn ByteTransport, fc_sysid: u8) {
    let header = codec::gcs_header();
    let mut seq = MavSequence::new();

    let msg = MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        command: MavCmd::MAV_CMD_REQUEST_MESSAGE,
        confirmation: 0,
        param1: MSG_ID_HOME_POSITION,
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    });
    let frame = codec::serialize_v2(&header, &msg, &mut seq);
    match transport.write_bytes(&frame) {
        Ok(()) => eprintln!("[MAVLINK-PARAM] requested HOME_POSITION"),
        Err(e) => log::warn!("Failed to request HOME_POSITION: {}", e),
    }
}
