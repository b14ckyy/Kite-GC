// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink TUNNEL (#385) raw codec for INAV's MSP-over-MAVLink tunnel (INAV 10.0+).
//
// The `mavlink` crate (0.17.1) types `TUNNEL.payload_type` as the enum MAV_TUNNEL_PAYLOAD_TYPE (max
// 212), so INAV's private payload type 0x8001 fails the typed parse with `InvalidEnum` in both
// directions. TUNNEL is therefore encoded/decoded by hand here, on the raw payload bytes, and framed by
// `codec::serialize_raw_v2` / recognised in `parser.rs` before the typed parse.
//
// Wire layout (MAVLink2 field order, 133 bytes before trailing-zero trimming):
//   u16 payload_type LE | u8 target_system | u8 target_component | u8 payload_length | u8[128] payload
// Firmware facts: Dev-Docs reference/MSP_OVER_MAVLINK.md (maintenance-10.x @ 3d2c8fd).

/// MAVLink message id of TUNNEL.
pub const TUNNEL_MSG_ID: u32 = 385;
/// CRC_EXTRA of TUNNEL (common.xml).
pub const TUNNEL_CRC_EXTRA: u8 = 147;
/// INAV's private payload type for the MSP tunnel (`MAVLINK_TUNNEL_PAYLOAD_TYPE_INAV_MSP`).
pub const PAYLOAD_TYPE_INAV_MSP: u16 = 0x8001;
/// Max MSP bytes per TUNNEL message (the size of the `payload` array).
pub const TUNNEL_CHUNK_MAX: usize = 128;
/// Full (untrimmed) TUNNEL payload length: 2 + 1 + 1 + 1 + 128.
pub const TUNNEL_PAYLOAD_LEN: usize = 5 + TUNNEL_CHUNK_MAX;

/// A decoded TUNNEL message (raw — no enum typing of `payload_type`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRaw {
    pub payload_type: u16,
    pub target_system: u8,
    pub target_component: u8,
    /// Exactly `payload_length` bytes (zero-extended if the wire frame was trimmed).
    pub payload: Vec<u8>,
}

/// Build the 133-byte TUNNEL payload carrying one slice of the framed MSP stream, addressed to
/// `(target_system, target_component)`, payload type 0x8001. The chunk is zero-padded to 128 bytes;
/// the MAVLink2 serializer trims the trailing zeros on the wire (INAV zero-extends on receive).
pub fn encode_chunk(target_system: u8, target_component: u8, chunk: &[u8]) -> [u8; TUNNEL_PAYLOAD_LEN] {
    debug_assert!(chunk.len() <= TUNNEL_CHUNK_MAX, "TUNNEL chunk larger than 128 bytes");
    let len = chunk.len().min(TUNNEL_CHUNK_MAX);
    let mut out = [0u8; TUNNEL_PAYLOAD_LEN];
    out[0..2].copy_from_slice(&PAYLOAD_TYPE_INAV_MSP.to_le_bytes());
    out[2] = target_system;
    out[3] = target_component;
    out[4] = len as u8;
    out[5..5 + len].copy_from_slice(&chunk[..len]);
    out
}

/// Decode a TUNNEL payload as it came off the wire (possibly trimmed of trailing zeros). Returns `None`
/// when `payload_length` exceeds 128 (INAV resets its tunnel state on that too) or the payload is
/// longer than a TUNNEL can be.
pub fn decode(wire_payload: &[u8]) -> Option<TunnelRaw> {
    if wire_payload.len() > TUNNEL_PAYLOAD_LEN {
        return None;
    }
    let mut full = [0u8; TUNNEL_PAYLOAD_LEN];
    full[..wire_payload.len()].copy_from_slice(wire_payload);
    let payload_length = full[4] as usize;
    if payload_length > TUNNEL_CHUNK_MAX {
        return None;
    }
    Some(TunnelRaw {
        payload_type: u16::from_le_bytes([full[0], full[1]]),
        target_system: full[2],
        target_component: full[3],
        payload: full[5..5 + payload_length].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_crc_extra_matches() {
        // The parser validates the CRC with the crate's table before the raw decode — it must agree.
        use ::mavlink::ardupilotmega::MavMessage;
        use ::mavlink::Message;
        assert_eq!(MavMessage::extra_crc(TUNNEL_MSG_ID), TUNNEL_CRC_EXTRA);
    }

    #[test]
    fn encode_layout() {
        let p = encode_chunk(7, 1, &[0x24, 0x58, 0x3C]);
        assert_eq!(&p[0..5], &[0x01, 0x80, 7, 1, 3]);
        assert_eq!(&p[5..8], &[0x24, 0x58, 0x3C]);
        assert!(p[8..].iter().all(|&b| b == 0));
    }

    #[test]
    fn decode_zero_extends_trimmed_payload() {
        // Chunk ends in zero bytes that MAVLink2 trimming would have removed from the wire.
        let full = encode_chunk(255, 190, &[1, 2, 0, 0]);
        let trimmed = &full[..7]; // payload_type, targets, len=4, [1, 2]
        let t = decode(trimmed).expect("decodes");
        assert_eq!(t.payload_type, PAYLOAD_TYPE_INAV_MSP);
        assert_eq!((t.target_system, t.target_component), (255, 190));
        assert_eq!(t.payload, vec![1, 2, 0, 0]);
    }

    #[test]
    fn decode_rejects_oversized_length() {
        let mut full = encode_chunk(1, 1, &[0xAA]);
        full[4] = 129;
        assert!(decode(&full).is_none());
    }
}
