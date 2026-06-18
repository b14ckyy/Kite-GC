// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Handshake
// Waits for the first HEARTBEAT from the FC, extracts system info,
// sends a GCS HEARTBEAT back, and returns FcInfo.

use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{
    MavAutopilot, MavCmd, MavMessage, MavType,
    COMMAND_LONG_DATA,
};
use ::mavlink::Message;

use crate::msp::FcInfo;
use crate::transport::ByteTransport;

use super::codec;
use super::parser::MavParser;

/// Timeout waiting for the first FC HEARTBEAT
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Perform the MAVLink handshake on a ByteTransport.
///
/// 1. Read bytes until we receive a HEARTBEAT from the FC
/// 2. Extract FC info (autopilot type, vehicle type, system ID)
/// 3. Send a GCS HEARTBEAT back
/// 4. Return (FcInfo, fc_system_id)
///
/// The transport is borrowed mutably — caller retains ownership.
pub fn perform_handshake(transport: &mut dyn ByteTransport) -> Result<(FcInfo, u8), String> {
    let mut parser = MavParser::new();
    let mut buf = [0u8; 512];
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;

    log::info!("MAVLink handshake: sending initial GCS HEARTBEAT, then waiting for FC HEARTBEAT...");

    // Send a GCS HEARTBEAT first — some FC firmware (ArduPilot) won't stream
    // telemetry until they detect a GCS on the link.
    {
        let header = codec::gcs_header();
        let hb_msg = codec::gcs_heartbeat();
        let mut seq = codec::MavSequence::new();
        let frame = codec::serialize_v2(&header, &hb_msg, &mut seq);
        if let Err(e) = transport.write_bytes(&frame) {
            log::warn!("Failed to send initial GCS HEARTBEAT: {}", e);
        } else {
            log::info!("MAVLink handshake: initial GCS HEARTBEAT sent ({} bytes)", frame.len());
        }
    }

    // Wait for HEARTBEAT from the FC
    let mut fc_sysid: u8 = 0;
    let mut fc_info = FcInfo::default();
    let mut total_bytes_read: usize = 0;
    let mut read_calls: u32 = 0;

    'outer: loop {
        if Instant::now() > deadline {
            return Err(format!(
                "MAVLink handshake timeout: no HEARTBEAT received within 10s \
                 (read {} bytes in {} calls, parser errors={})",
                total_bytes_read, read_calls, parser.packet_errors()
            ));
        }

        // Send GCS heartbeat periodically during handshake (every ~2s)
        if read_calls > 0 && read_calls % 2 == 0 {
            let header = codec::gcs_header();
            let hb_msg = codec::gcs_heartbeat();
            let mut seq = codec::MavSequence::new();
            let frame = codec::serialize_v2(&header, &hb_msg, &mut seq);
            let _ = transport.write_bytes(&frame);
        }

        read_calls += 1;
        match transport.read_bytes(&mut buf) {
            Ok(0) => {
                log::trace!("MAVLink handshake: read returned 0 bytes (timeout)");
                continue;
            }
            Ok(n) => {
                total_bytes_read += n;
                // Log first bytes for debugging
                let preview_len = n.min(32);
                log::debug!(
                    "MAVLink handshake: read {} bytes, first {}: {:02X?}",
                    n, preview_len, &buf[..preview_len]
                );

                let frames = parser.parse_bytes(&buf[..n]);
                log::debug!("MAVLink handshake: parser returned {} frames", frames.len());

                for frame in frames {
                    log::debug!(
                        "MAVLink handshake: frame msg_id={} sysid={} compid={}",
                        frame.message.message_id(),
                        frame.header.system_id,
                        frame.header.component_id,
                    );
                    if let MavMessage::HEARTBEAT(ref hb) = frame.message {
                        // Ignore heartbeats from other GCS
                        if frame.header.system_id == codec::GCS_SYSTEM_ID {
                            log::debug!("MAVLink handshake: ignoring GCS heartbeat (sysid={})", frame.header.system_id);
                            continue;
                        }

                        fc_sysid = frame.header.system_id;

                        // Map autopilot (+ vehicle type) to fc_variant string. For ArduPilot the
                        // variant is per-vehicle ("ArduPlane"/"ArduCopter"/...) because the frontend
                        // picks the flight-mode name table from this string (Plane vs Copter mode
                        // numbers differ entirely) — a generic "ArduPilot" would wrongly use Copter.
                        fc_info.fc_variant = match hb.autopilot {
                            MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA => match hb.mavtype {
                                // Note: QuadPlane (VTOL_* types) also runs ArduPlane and uses the
                                // Plane mode table — it currently falls back to "ArduCopter" naming;
                                // refine the VTOL types here if QuadPlane support is needed.
                                MavType::MAV_TYPE_FIXED_WING => "ArduPlane".into(),
                                MavType::MAV_TYPE_GROUND_ROVER => "ArduRover".into(),
                                MavType::MAV_TYPE_SUBMARINE => "ArduSub".into(),
                                _ => "ArduCopter".into(),
                            },
                            MavAutopilot::MAV_AUTOPILOT_PX4 => "PX4".into(),
                            MavAutopilot::MAV_AUTOPILOT_GENERIC => "Generic".into(),
                            other => format!("{:?}", other),
                        };

                        // Raw MAV_TYPE — the frontend uses it to detect QuadPlane (VTOL_* range), which
                        // the per-vehicle fc_variant string can't express (QuadPlane reports ArduPlane).
                        fc_info.mav_type = hb.mavtype as u8;

                        // Map vehicle type to platform_type
                        fc_info.platform_type = match hb.mavtype {
                            MavType::MAV_TYPE_FIXED_WING => 1,
                            MavType::MAV_TYPE_QUADROTOR
                            | MavType::MAV_TYPE_HEXAROTOR
                            | MavType::MAV_TYPE_OCTOROTOR
                            | MavType::MAV_TYPE_TRICOPTER => 2,
                            MavType::MAV_TYPE_HELICOPTER => 3,
                            MavType::MAV_TYPE_GROUND_ROVER => 10,
                            MavType::MAV_TYPE_SUBMARINE => 12,
                            _ => 0,
                        };

                        // Store custom_mode for later flight mode decoding
                        fc_info.mixer_preset = hb.custom_mode as i16;

                        log::info!(
                            "MAVLink handshake: {} (sysid={}) vehicle={:?} mode={}",
                            fc_info.fc_variant,
                            fc_sysid,
                            hb.mavtype,
                            hb.custom_mode,
                        );

                        break 'outer;
                    }
                }
            }
            Err(crate::transport::TransportError::Timeout) => continue,
            Err(crate::transport::TransportError::Disconnected) => {
                return Err("Transport disconnected during MAVLink handshake".into());
            }
            Err(e) => {
                return Err(format!("Transport error during MAVLink handshake: {}", e));
            }
        }
    }

    // Send GCS HEARTBEAT back
    let header = codec::gcs_header();
    let hb_msg = codec::gcs_heartbeat();
    let mut seq = codec::MavSequence::new();
    let frame = codec::serialize_v2(&header, &hb_msg, &mut seq);

    transport
        .write_bytes(&frame)
        .map_err(|e| format!("Failed to send GCS HEARTBEAT: {}", e))?;

    log::info!("MAVLink handshake complete — GCS HEARTBEAT sent");

    fc_info.api_version = "MAVLink v2".into();
    fc_info.board_id = "MAVLink".into();

    // Request AUTOPILOT_VERSION to get firmware version
    // MAV_CMD_REQUEST_MESSAGE (512), param1 = message_id of AUTOPILOT_VERSION (148)
    let version_request = MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        command: MavCmd::MAV_CMD_REQUEST_MESSAGE,
        confirmation: 0,
        param1: 148.0, // AUTOPILOT_VERSION msg_id
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    });
    let frame = codec::serialize_v2(&header, &version_request, &mut seq);
    if let Err(e) = transport.write_bytes(&frame) {
        log::warn!("Failed to request AUTOPILOT_VERSION: {}", e);
    } else {
        log::info!("MAVLink handshake: requested AUTOPILOT_VERSION");
    }

    // Wait up to 3s for AUTOPILOT_VERSION response
    let version_deadline = Instant::now() + Duration::from_secs(3);
    'version: loop {
        if Instant::now() > version_deadline {
            log::warn!("AUTOPILOT_VERSION not received within 3s — continuing with unknown version");
            fc_info.fc_version = "unknown".into();
            break 'version;
        }

        match transport.read_bytes(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                for frame in parser.parse_bytes(&buf[..n]) {
                    if frame.header.system_id != fc_sysid {
                        continue;
                    }
                    if let MavMessage::AUTOPILOT_VERSION(ref ver) = frame.message {
                        // Extract version from flight_sw_version (semver encoded in u32)
                        // Format: major << 24 | minor << 16 | patch << 8 | type
                        let sw = ver.flight_sw_version;
                        let major = (sw >> 24) & 0xFF;
                        let minor = (sw >> 16) & 0xFF;
                        let patch = (sw >> 8) & 0xFF;
                        let dev_type = sw & 0xFF;

                        // Type: 0=dev, 64=alpha, 128=beta, 192=rc, 255=release
                        let suffix = match dev_type {
                            255 => "".to_string(),
                            192..=254 => "-rc".to_string(),
                            128..=191 => "-beta".to_string(),
                            64..=127 => "-alpha".to_string(),
                            _ => "-dev".to_string(),
                        };

                        fc_info.fc_version = format!("{}.{}.{}{}", major, minor, patch, suffix);

                        // Try to get board version from board_version field
                        if ver.board_version > 0 {
                            fc_info.hardware_revision = ver.board_version as u16;
                        }

                        log::info!(
                            "AUTOPILOT_VERSION: {} v{} (sw=0x{:08X}, board={})",
                            fc_info.fc_variant,
                            fc_info.fc_version,
                            sw,
                            ver.board_version,
                        );
                        break 'version;
                    }
                }
            }
            Err(crate::transport::TransportError::Timeout) => continue,
            Err(_) => break 'version,
        }
    }

    Ok((fc_info, fc_sysid))
}
