// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Handshake
// Waits for the first HEARTBEAT from the FC, extracts system info,
// sends a GCS HEARTBEAT back, and returns FcInfo.

use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{
    MavAutopilot, MavCmd, MavMessage,
    AUTOPILOT_VERSION_DATA, COMMAND_LONG_DATA,
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
/// 4. Return (FcInfo, fc_system_id, fc_component_id, uid2_valid)
///
/// `uid2_valid` is true only when AUTOPILOT_VERSION arrived with a non-zero `uid2` (ArduPilot / PX4
/// fill it from the MCU serial). INAV sends it all-zero — that, or no AUTOPILOT_VERSION at all, makes
/// the connect path probe for INAV's MSP-over-MAVLink tunnel.
///
/// The transport is borrowed mutably — caller retains ownership.
pub fn perform_handshake(transport: &mut dyn ByteTransport) -> Result<(FcInfo, u8, u8, bool), String> {
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
    let fc_sysid: u8;
    let fc_compid: u8;
    let mut fc_info = FcInfo::default();
    let mut total_bytes_read: usize = 0;
    let mut uid2_valid = false;
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
        if read_calls > 0 && read_calls.is_multiple_of(2) {
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

                        // Lock onto an *autopilot* heartbeat, not the first non-GCS one. PX4 (and many
                        // vehicles) stream HEARTBEATs from several components — a gimbal / companion /
                        // ADS-B receiver typically reports autopilot=INVALID from a component_id ≠ 1.
                        // Accepting one of those would set fc_variant to "MAV_AUTOPILOT_INVALID" and the
                        // PX4 path (is_px4) would never engage. Accept when the autopilot field is set,
                        // or the heartbeat comes from the autopilot component (compid 1 = AUTOPILOT1).
                        let is_autopilot = hb.autopilot != MavAutopilot::MAV_AUTOPILOT_INVALID
                            || frame.header.component_id == 1;
                        if !is_autopilot {
                            log::info!(
                                "MAVLink handshake: skipping non-autopilot heartbeat (sysid={} compid={} autopilot={:?}) — waiting for the autopilot",
                                frame.header.system_id, frame.header.component_id, hb.autopilot,
                            );
                            continue;
                        }

                        fc_sysid = frame.header.system_id;
                        // Lock onto this component too — only its HEARTBEATs drive mode/armed in the
                        // handler loop, so peripherals sharing the FC's system_id can't flip them.
                        fc_compid = frame.header.component_id;
                        log::info!(
                            "MAVLink handshake: locked onto autopilot heartbeat (sysid={} compid={} autopilot={:?})",
                            frame.header.system_id, frame.header.component_id, hb.autopilot,
                        );

                        // Shared with the tlog importer — see `vehicle::identify`.
                        let (variant, platform_type) =
                            crate::mavlink_proto::vehicle::identify(hb.autopilot, hb.mavtype);
                        fc_info.fc_variant = variant;
                        fc_info.platform_type = platform_type;

                        // Raw MAV_TYPE — the frontend uses it to detect QuadPlane (VTOL_* range), which
                        // the per-vehicle fc_variant string can't express (QuadPlane reports ArduPlane).
                        fc_info.mav_type = hb.mavtype as u8;

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

    // Request AUTOPILOT_VERSION to get firmware version and hardware id
    let version_request = autopilot_version_request(fc_sysid, 1); // MAV_COMP_ID_AUTOPILOT1
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
                        let v = parse_autopilot_version(ver);
                        fc_info.fc_version = v.fc_version;
                        if let Some(rev) = v.hardware_revision {
                            fc_info.hardware_revision = rev;
                        }
                        uid2_valid = v.uid2_valid;
                        fc_info.fc_uid = v.fc_uid;
                        log::info!(
                            "AUTOPILOT_VERSION: {} v{} (sw=0x{:08X}, board={})",
                            fc_info.fc_variant,
                            fc_info.fc_version,
                            ver.flight_sw_version,
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

    Ok((fc_info, fc_sysid, fc_compid, uid2_valid))
}

/// `MAV_CMD_REQUEST_MESSAGE` for `AUTOPILOT_VERSION` (148), addressed to `sysid` / `compid` — the
/// handshake asks the primary vehicle's autopilot (component 1), the handler a vehicle discovered later on
/// a shared link at the component its HEARTBEAT came from.
pub(crate) fn autopilot_version_request(sysid: u8, compid: u8) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: sysid,
        target_component: compid,
        command: MavCmd::MAV_CMD_REQUEST_MESSAGE,
        confirmation: 0,
        param1: 148.0, // AUTOPILOT_VERSION msg_id
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    })
}

/// What Kite takes from an `AUTOPILOT_VERSION`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AutopilotVersion {
    /// `flight_sw_version` as "major.minor.patch[-type]".
    pub fc_version: String,
    /// `board_version`, when the autopilot fills it.
    pub hardware_revision: Option<u16>,
    /// The hardware id (`uid2` through `msp::hardware_uid`); `None` when the autopilot gives none.
    pub fc_uid: Option<String>,
    /// `uid2` has any non-zero byte (ArduPilot / PX4 fill it from the MCU serial; INAV sends zeros).
    pub uid2_valid: bool,
}

/// Decode an `AUTOPILOT_VERSION` — shared by the handshake (primary) and the handler (vehicles
/// discovered later). An all-zero `uid2` means no hardware id: the legacy 64-bit `uid` is not used —
/// INAV fills it with the sysid, PX4 SITL with a constant + sysid.
pub(crate) fn parse_autopilot_version(ver: &AUTOPILOT_VERSION_DATA) -> AutopilotVersion {
    // flight_sw_version: major << 24 | minor << 16 | patch << 8 | type
    let sw = ver.flight_sw_version;
    let (major, minor, patch, dev_type) = ((sw >> 24) & 0xFF, (sw >> 16) & 0xFF, (sw >> 8) & 0xFF, sw & 0xFF);
    // Type: 0=dev, 64=alpha, 128=beta, 192=rc, 255=release
    let suffix = match dev_type {
        255 => "",
        192..=254 => "-rc",
        128..=191 => "-beta",
        64..=127 => "-alpha",
        _ => "-dev",
    };
    let uid2: String = ver.uid2.iter().map(|b| format!("{:02X}", b)).collect();
    let fc_uid = crate::msp::types::hardware_uid(&uid2);
    if fc_uid.is_none() && ver.uid != 0 {
        log::info!("AUTOPILOT_VERSION: uid2 is zero — uid {:016X} is not used as a hardware id", ver.uid);
    }
    AutopilotVersion {
        fc_version: format!("{}.{}.{}{}", major, minor, patch, suffix),
        hardware_revision: (ver.board_version > 0).then_some(ver.board_version as u16),
        fc_uid,
        uid2_valid: ver.uid2.iter().any(|b| *b != 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(sw: u32, board: u32, uid: u64, uid2: [u8; 18]) -> AUTOPILOT_VERSION_DATA {
        AUTOPILOT_VERSION_DATA { flight_sw_version: sw, board_version: board, uid, uid2, ..Default::default() }
    }

    #[test]
    fn autopilot_version_takes_uid2_as_the_hardware_id() {
        let mut uid2 = [0u8; 18];
        uid2[..4].copy_from_slice(&[0x2F, 0x00, 0x34, 0x00]);
        let v = parse_autopilot_version(&version(0x0406_02FF, 0x0123, 7, uid2));
        assert_eq!(v.fc_version, "4.6.2");
        assert_eq!(v.hardware_revision, Some(0x0123));
        assert_eq!(v.fc_uid, Some(format!("2F003400{}", "00".repeat(14))));
        assert!(v.uid2_valid);
    }

    #[test]
    fn autopilot_version_without_uid2_has_no_hardware_id() {
        // INAV / PX4 SITL at sysid 1: zero uid2, a synthetic `uid` — never used.
        let v = parse_autopilot_version(&version(0x0A00_00C0, 0, 1, [0; 18]));
        assert_eq!((v.fc_version.as_str(), v.hardware_revision, v.fc_uid, v.uid2_valid), ("10.0.0-rc", None, None, false));
        // PX4 SITL at sysid 2: byte 0 = sysid - 1 — kept (distinct per instance).
        let mut uid2 = [0u8; 18];
        uid2[0] = 1;
        let v = parse_autopilot_version(&version(0x010F_0000, 0, 0, uid2));
        assert_eq!(v.fc_version, "1.15.0-dev");
        assert_eq!(v.fc_uid, Some(format!("01{}", "00".repeat(17))));
    }
}
