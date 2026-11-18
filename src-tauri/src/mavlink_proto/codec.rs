// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Frame Serializer
// Builds complete MAVLink v2 wire frames from typed messages.

use ::mavlink::ardupilotmega::MavMessage;
use ::mavlink::{MavHeader, MavlinkVersion, Message};

/// GCS system ID (industry standard: 255 for GCS)
pub const GCS_SYSTEM_ID: u8 = 255;
/// GCS component ID (MAV_COMP_ID_MISSIONPLANNER = 190, widely used for GCS)
pub const GCS_COMPONENT_ID: u8 = 190;

/// Sequence counter for outgoing messages
pub struct MavSequence(u8);

impl MavSequence {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn next(&mut self) -> u8 {
        let seq = self.0;
        self.0 = self.0.wrapping_add(1);
        seq
    }
}

/// Serialize a MAVLink message into a complete v2 wire frame (ready to send).
///
/// Frame format:
/// ```text
/// 0xFD [len] [incompat_flags] [compat_flags] [seq] [sysid] [compid] [msgid_lo] [msgid_mid] [msgid_hi] [payload...] [crc_lo] [crc_hi]
/// ```
pub fn serialize_v2(header: &MavHeader, msg: &MavMessage, seq: &mut MavSequence) -> Vec<u8> {
    let mut payload_buf = [0u8; 255];
    // `ser` already applies the MAVLink2 trailing-zero trim (mavlink-core `remove_trailing_zeroes`).
    let payload_len = msg.ser(MavlinkVersion::V2, &mut payload_buf);
    let msg_id = msg.message_id();
    build_v2_frame(header, msg_id, MavMessage::extra_crc(msg_id), &payload_buf[..payload_len], seq)
}

/// Serialize a raw (hand-encoded) message payload into a complete v2 wire frame — for messages the
/// typed crate cannot represent (TUNNEL with INAV's payload type 0x8001, see `tunnel.rs`). Applies the
/// same MAVLink2 trailing-zero trim as the typed path (at least one payload byte is kept) and shares
/// its header + X.25/CRC_EXTRA path.
pub fn serialize_raw_v2(
    header: &MavHeader,
    msg_id: u32,
    crc_extra: u8,
    payload: &[u8],
    seq: &mut MavSequence,
) -> Vec<u8> {
    let mut len = payload.len().min(255);
    while len > 1 && payload[len - 1] == 0 {
        len -= 1;
    }
    build_v2_frame(header, msg_id, crc_extra, &payload[..len], seq)
}

/// Assemble STX + header + payload + CRC for an already-final (trimmed) payload.
fn build_v2_frame(
    header: &MavHeader,
    msg_id: u32,
    extra_crc: u8,
    payload: &[u8],
    seq: &mut MavSequence,
) -> Vec<u8> {
    let payload_len = payload.len();
    let sequence = seq.next();

    // Header bytes (after STX) — used for CRC calculation
    let header_bytes: [u8; 9] = [
        payload_len as u8,
        0, // incompat_flags (no signing, no IFLAG)
        0, // compat_flags
        sequence,
        header.system_id,
        header.component_id,
        (msg_id & 0xFF) as u8,
        ((msg_id >> 8) & 0xFF) as u8,
        ((msg_id >> 16) & 0xFF) as u8,
    ];

    // CRC over header + payload + extra CRC byte
    let crc = compute_crc(&header_bytes, payload, extra_crc);

    // Assemble complete frame
    let mut frame = Vec::with_capacity(1 + 9 + payload_len + 2);
    frame.push(0xFD); // STX v2
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(payload);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// Build a GCS HEARTBEAT message
pub fn gcs_heartbeat() -> MavMessage {
    MavMessage::HEARTBEAT(::mavlink::ardupilotmega::HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: ::mavlink::ardupilotmega::MavType::MAV_TYPE_GCS,
        autopilot: ::mavlink::ardupilotmega::MavAutopilot::MAV_AUTOPILOT_INVALID,
        base_mode: ::mavlink::ardupilotmega::MavModeFlag::default(),
        system_status: ::mavlink::ardupilotmega::MavState::MAV_STATE_ACTIVE,
        mavlink_version: 3,
    })
}

/// Build the default GCS MavHeader
pub fn gcs_header() -> MavHeader {
    MavHeader {
        system_id: GCS_SYSTEM_ID,
        component_id: GCS_COMPONENT_ID,
        sequence: 0, // Overridden by serialize_v2
    }
}

/// X.25 CRC computation (same as parser.rs — shared logic)
fn compute_crc(header: &[u8], payload: &[u8], extra_crc: u8) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in header {
        crc = crc_accumulate(crc, b);
    }
    for &b in payload {
        crc = crc_accumulate(crc, b);
    }
    crc = crc_accumulate(crc, extra_crc);
    crc
}

/// X.25 CRC accumulate — tmp must stay u8 through the shift step
#[inline]
fn crc_accumulate(crc: u16, byte: u8) -> u16 {
    let tmp: u8 = byte ^ (crc as u8);
    let tmp: u8 = tmp ^ (tmp << 4);
    let tmp16 = tmp as u16;
    (crc >> 8) ^ (tmp16 << 8) ^ (tmp16 << 3) ^ (tmp16 >> 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_typed_serializers_agree() {
        // The raw path must frame + trim exactly like the typed path: feed it the untrimmed typed
        // payload of a HEARTBEAT (trailing zero bytes included) and compare the wire frames.
        let msg = gcs_heartbeat();
        let mut untrimmed = [0u8; 255];
        let _ = msg.ser(MavlinkVersion::V1, &mut untrimmed); // V1 = no trim → full 9-byte payload
        let full = &untrimmed[..9];
        let typed = serialize_v2(&gcs_header(), &msg, &mut MavSequence::new());
        let raw = serialize_raw_v2(
            &gcs_header(),
            msg.message_id(),
            MavMessage::extra_crc(msg.message_id()),
            full,
            &mut MavSequence::new(),
        );
        assert_eq!(typed, raw);
    }

    #[test]
    fn raw_trim_keeps_one_byte() {
        let f = serialize_raw_v2(&gcs_header(), 385, 147, &[0u8; 133], &mut MavSequence::new());
        assert_eq!(f[1], 1, "an all-zero payload keeps one byte on the wire");
        assert_eq!(f.len(), 1 + 9 + 1 + 2);
    }
}
