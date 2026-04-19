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
    let payload_len = msg.ser(MavlinkVersion::V2, &mut payload_buf);
    let payload = &payload_buf[..payload_len];
    let msg_id = msg.message_id();
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
    let extra_crc = MavMessage::extra_crc(msg_id);
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
