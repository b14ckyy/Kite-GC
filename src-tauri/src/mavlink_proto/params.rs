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

use ::mavlink::ardupilotmega::{MavMessage, PARAM_REQUEST_READ_DATA};

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

/// Request the active EKF core (AHRS_EKF_TYPE) once. Fire-and-forget over the (pre-handler) transport.
pub fn request_ekf_type(transport: &mut dyn ByteTransport, fc_sysid: u8) {
    let header = codec::gcs_header();
    let mut seq = MavSequence::new();

    let msg = MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA {
        param_index: -1, // -1 → look up by name
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        param_id: pack_param_id("AHRS_EKF_TYPE").into(),
    });
    let frame = codec::serialize_v2(&header, &msg, &mut seq);
    match transport.write_bytes(&frame) {
        Ok(()) => eprintln!("[MAVLINK-PARAM] requested AHRS_EKF_TYPE"),
        Err(e) => log::warn!("Failed to request AHRS_EKF_TYPE: {}", e),
    }
}
