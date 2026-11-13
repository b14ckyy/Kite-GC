// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Frame Parser
// State machine that accepts raw bytes and emits parsed MAVLink messages.
// Supports both MAVLink v1 (0xFE) and v2 (0xFD) frames.

use ::mavlink::ardupilotmega::MavMessage;
use ::mavlink::{MavHeader, MavlinkVersion, Message};

/// MAVLink v1 header size (after STX): len(1) + seq(1) + sysid(1) + compid(1) + msgid(1) = 5
const V1_HEADER_SIZE: usize = 5;
/// MAVLink v2 header size (after STX): len(1) + incompat(1) + compat(1) + seq(1) + sysid(1) + compid(1) + msgid(3) = 9
const V2_HEADER_SIZE: usize = 9;
/// CRC size for both versions
const CRC_SIZE: usize = 2;

/// Parser states
enum State {
    /// Waiting for a start-of-frame marker (0xFE or 0xFD)
    WaitingForStx,
    /// Collecting header bytes (after STX)
    ReadingHeader {
        version: MavlinkVersion,
        header_buf: Vec<u8>,
        header_len: usize,
    },
    /// Collecting payload + CRC bytes
    ReadingPayload {
        version: MavlinkVersion,
        header_buf: Vec<u8>,
        payload_buf: Vec<u8>,
        payload_len: usize,
    },
}

/// A successfully parsed MAVLink frame
pub struct MavFrame {
    pub header: MavHeader,
    pub message: MavMessage,
    /// MAVLink wire version (V1/V2) this frame was decoded from — parsed but not yet consumed.
    #[allow(dead_code)]
    pub protocol_version: MavlinkVersion,
    /// Complete raw frame bytes (STX through CRC) for tlog recording
    pub raw_bytes: Vec<u8>,
}

/// Parser output: a typed message, or a TUNNEL (#385) frame decoded by hand because the typed crate
/// rejects INAV's payload type 0x8001 (see `tunnel.rs`). CRC-validated like every other frame.
// `Msg` is the per-frame hot path — boxing it would add an allocation to every telemetry message.
#[allow(clippy::large_enum_variant)]
pub enum Parsed {
    Msg(MavFrame),
    Tunnel {
        header: MavHeader,
        /// Complete raw frame bytes (STX through CRC) for tlog recording
        raw_bytes: Vec<u8>,
        tunnel: super::tunnel::TunnelRaw,
    },
}

/// MAVLink byte-level frame parser.
/// Feed bytes via `push()`, get parsed frames out.
pub struct MavParser {
    state: State,
    error_count: u32,
}

impl MavParser {
    pub fn new() -> Self {
        Self {
            state: State::WaitingForStx,
            error_count: 0,
        }
    }

    /// Number of frames that failed CRC or parse
    pub fn packet_errors(&self) -> u32 {
        self.error_count
    }

    /// Feed a single byte. Returns a parsed frame if one is complete. TUNNEL frames are not typed
    /// messages and are skipped here — use `push_any` / `parse_all` to receive them.
    pub fn push(&mut self, byte: u8) -> Option<MavFrame> {
        match self.push_any(byte) {
            Some(Parsed::Msg(frame)) => Some(frame),
            _ => None,
        }
    }

    /// Feed a single byte. Returns a typed frame or a raw TUNNEL frame once one is complete.
    pub fn push_any(&mut self, byte: u8) -> Option<Parsed> {
        match std::mem::replace(&mut self.state, State::WaitingForStx) {
            State::WaitingForStx => {
                match byte {
                    0xFE => {
                        self.state = State::ReadingHeader {
                            version: MavlinkVersion::V1,
                            header_buf: Vec::with_capacity(V1_HEADER_SIZE),
                            header_len: V1_HEADER_SIZE,
                        };
                    }
                    0xFD => {
                        self.state = State::ReadingHeader {
                            version: MavlinkVersion::V2,
                            header_buf: Vec::with_capacity(V2_HEADER_SIZE),
                            header_len: V2_HEADER_SIZE,
                        };
                    }
                    _ => {} // Not a start marker — stay in WaitingForStx
                }
                None
            }

            State::ReadingHeader {
                version,
                mut header_buf,
                header_len,
            } => {
                header_buf.push(byte);
                if header_buf.len() < header_len {
                    self.state = State::ReadingHeader {
                        version,
                        header_buf,
                        header_len,
                    };
                    None
                } else {
                    let payload_len = header_buf[0] as usize;
                    self.state = State::ReadingPayload {
                        version,
                        header_buf,
                        payload_buf: Vec::with_capacity(payload_len + CRC_SIZE),
                        payload_len,
                    };
                    None
                }
            }

            State::ReadingPayload {
                version,
                header_buf,
                mut payload_buf,
                payload_len,
            } => {
                payload_buf.push(byte);
                let total_needed = payload_len + CRC_SIZE;
                if payload_buf.len() < total_needed {
                    self.state = State::ReadingPayload {
                        version,
                        header_buf,
                        payload_buf,
                        payload_len,
                    };
                    None
                } else {
                    // Frame complete — validate CRC and parse
                    self.state = State::WaitingForStx;
                    let result = self.try_parse_frame(&version, &header_buf, &payload_buf, payload_len);
                    if result.is_none() {
                        self.error_count += 1;
                    }
                    result
                }
            }
        }
    }

    /// Parse a complete frame from header + payload + CRC bytes
    fn try_parse_frame(
        &self,
        version: &MavlinkVersion,
        header_buf: &[u8],
        payload_buf: &[u8],
        payload_len: usize,
    ) -> Option<Parsed> {
        let (header, msg_id) = match version {
            MavlinkVersion::V1 => {
                // header_buf: [len, seq, sysid, compid, msgid]
                let header = MavHeader {
                    system_id: header_buf[2],
                    component_id: header_buf[3],
                    sequence: header_buf[1],
                };
                let msg_id = header_buf[4] as u32;
                (header, msg_id)
            }
            MavlinkVersion::V2 => {
                // header_buf: [len, incompat_flags, compat_flags, seq, sysid, compid, msgid_lo, msgid_mid, msgid_hi]
                let header = MavHeader {
                    system_id: header_buf[4],
                    component_id: header_buf[5],
                    sequence: header_buf[3],
                };
                let msg_id = (header_buf[6] as u32)
                    | ((header_buf[7] as u32) << 8)
                    | ((header_buf[8] as u32) << 16);
                (header, msg_id)
            }
        };

        let payload = &payload_buf[..payload_len];
        let crc_received =
            (payload_buf[payload_len] as u16) | ((payload_buf[payload_len + 1] as u16) << 8);

        // Calculate expected CRC
        let extra_crc = MavMessage::extra_crc(msg_id);
        let crc_computed = compute_crc(header_buf, payload, extra_crc);

        if crc_computed != crc_received {
            // CRC mismatch is normal for dialect version differences —
            // MAVLink receivers silently discard invalid frames.
            log::debug!(
                "MAVLink CRC mismatch for msg_id {}: computed 0x{:04X}, received 0x{:04X} (extra_crc=0x{:02X})",
                msg_id, crc_computed, crc_received, extra_crc
            );
            return None;
        }

        // TUNNEL (#385): decoded by hand BEFORE the typed parse — the crate types `payload_type` as an
        // enum that rejects INAV's 0x8001 (`InvalidEnum`), which would drop every MSP tunnel reply.
        if msg_id == super::tunnel::TUNNEL_MSG_ID {
            return match super::tunnel::decode(payload) {
                Some(tunnel) => Some(Parsed::Tunnel {
                    header,
                    raw_bytes: raw_frame(version, header_buf, payload_buf),
                    tunnel,
                }),
                None => {
                    log::debug!("MAVLink TUNNEL rejected: payload_length > 128 (len byte {:?})", payload.get(4));
                    None
                }
            };
        }

        // Parse message payload into typed enum.
        //
        // POSITION_TARGET_GLOBAL_INT (87): ArduPilot sets the undefined upper bits of `type_mask`
        // (POSITION_TARGET_TYPEMASK_LAST_BYTE, 0xF000 — "for future use" in GCS_Common), and the
        // mavlink crate's strict bitflags reject the whole message (`InvalidFlag`, observed live:
        // 0xFDF8). Clear the undefined bits (type_mask = u16 at payload offset 48) and retry —
        // we only consume the position fields, never the mask.
        let parsed = MavMessage::parse(*version, msg_id, payload).or_else(|e| {
            if msg_id == 87 && payload_len >= 50 {
                let mut fixed = payload.to_vec();
                fixed[49] &= 0x0F;
                MavMessage::parse(*version, msg_id, &fixed).map_err(|_| e)
            } else {
                Err(e)
            }
        });
        match parsed {
            Ok(message) => {
                Some(Parsed::Msg(MavFrame {
                    header,
                    message,
                    protocol_version: *version,
                    raw_bytes: raw_frame(version, header_buf, payload_buf),
                }))
            }
            Err(e) => {
                log::debug!("MAVLink parse error for msg_id {}: {:?}", msg_id, e);
                None
            }
        }
    }

    /// Feed multiple bytes, collect all parsed frames (typed messages only — TUNNEL frames are skipped)
    pub fn parse_bytes(&mut self, data: &[u8]) -> Vec<MavFrame> {
        data.iter().filter_map(|&b| self.push(b)).collect()
    }

    /// Feed multiple bytes, collect every parsed frame including raw TUNNEL frames
    pub fn parse_all(&mut self, data: &[u8]) -> Vec<Parsed> {
        data.iter().filter_map(|&b| self.push_any(b)).collect()
    }
}

/// Reconstruct the full wire frame: STX + header + payload + CRC
fn raw_frame(version: &MavlinkVersion, header_buf: &[u8], payload_buf: &[u8]) -> Vec<u8> {
    let stx = match version {
        MavlinkVersion::V1 => 0xFE,
        MavlinkVersion::V2 => 0xFD,
    };
    let mut raw = Vec::with_capacity(1 + header_buf.len() + payload_buf.len());
    raw.push(stx);
    raw.extend_from_slice(header_buf);
    raw.extend_from_slice(payload_buf);
    raw
}

/// X.25 CRC computation for MAVLink frames.
/// Covers header bytes (after STX) + payload + CRC extra byte.
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

/// X.25 CRC accumulate one byte.
/// CRITICAL: tmp must stay u8 through the `^ (tmp << 4)` step so the
/// upper nibble is naturally discarded, matching the C reference impl.
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
    fn test_parser_ignores_garbage() {
        let mut parser = MavParser::new();
        for b in [0x00, 0x01, 0xFF, 0x42, 0x99] {
            assert!(parser.push(b).is_none());
        }
    }

    #[test]
    fn test_crc_known_value() {
        // CRC of empty data + extra_crc=0 starting from 0xFFFF should produce a known result
        let crc = compute_crc(&[], &[], 0);
        assert_ne!(crc, 0xFFFF);
    }

    #[test]
    fn test_v2_heartbeat_frame() {
        // Build a valid MAVLink v2 HEARTBEAT frame and verify parser accepts it
        use ::mavlink::ardupilotmega::MavMessage;
        use ::mavlink::Message;

        let msg = MavMessage::HEARTBEAT(::mavlink::ardupilotmega::HEARTBEAT_DATA {
            custom_mode: 0,
            mavtype: ::mavlink::ardupilotmega::MavType::MAV_TYPE_QUADROTOR,
            autopilot: ::mavlink::ardupilotmega::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
            base_mode: ::mavlink::ardupilotmega::MavModeFlag::default(),
            system_status: ::mavlink::ardupilotmega::MavState::MAV_STATE_STANDBY,
            mavlink_version: 3,
        });

        let mut payload_buf = [0u8; 255];
        let payload_len = msg.ser(MavlinkVersion::V2, &mut payload_buf);
        let payload = &payload_buf[..payload_len];
        let msg_id = msg.message_id();

        // Build header bytes (after STX)
        let header_bytes: Vec<u8> = vec![
            payload_len as u8,    // len
            0,              // incompat_flags
            0,              // compat_flags
            0,              // sequence
            1,              // system_id (FC)
            1,              // component_id
            (msg_id & 0xFF) as u8,
            ((msg_id >> 8) & 0xFF) as u8,
            ((msg_id >> 16) & 0xFF) as u8,
        ];

        // Compute CRC
        let extra_crc = MavMessage::extra_crc(msg_id);
        let crc = compute_crc(&header_bytes, payload, extra_crc);

        // Build complete frame
        let mut frame = vec![0xFD]; // STX v2
        frame.extend_from_slice(&header_bytes);
        frame.extend_from_slice(payload);
        frame.push((crc & 0xFF) as u8);
        frame.push((crc >> 8) as u8);

        // Parse
        let mut parser = MavParser::new();
        let frames = parser.parse_bytes(&frame);
        assert_eq!(frames.len(), 1, "Should parse exactly one frame");
        assert_eq!(frames[0].header.system_id, 1);
        matches!(&frames[0].message, MavMessage::HEARTBEAT(_));
    }

    /// A TUNNEL frame as INAV sends it: from (1, 1) to Kite (255, 190), MSP slice as payload.
    fn tunnel_frame(chunk: &[u8]) -> Vec<u8> {
        use crate::mavlink_proto::{codec, tunnel};
        let header = MavHeader { system_id: 1, component_id: 1, sequence: 0 };
        let payload = tunnel::encode_chunk(codec::GCS_SYSTEM_ID, codec::GCS_COMPONENT_ID, chunk);
        codec::serialize_raw_v2(
            &header,
            tunnel::TUNNEL_MSG_ID,
            tunnel::TUNNEL_CRC_EXTRA,
            &payload,
            &mut codec::MavSequence::new(),
        )
    }

    #[test]
    fn test_tunnel_roundtrip() {
        let chunk: Vec<u8> = (1..=128u8).collect(); // full chunk, no trailing zeros
        let frame = tunnel_frame(&chunk);
        let mut parser = MavParser::new();
        let out = parser.parse_all(&frame);
        assert_eq!(out.len(), 1);
        match &out[0] {
            Parsed::Tunnel { header, raw_bytes, tunnel } => {
                assert_eq!((header.system_id, header.component_id), (1, 1));
                assert_eq!(raw_bytes, &frame);
                assert_eq!(tunnel.payload_type, 0x8001);
                assert_eq!((tunnel.target_system, tunnel.target_component), (255, 190));
                assert_eq!(tunnel.payload, chunk);
            }
            Parsed::Msg(_) => panic!("TUNNEL must come out as Parsed::Tunnel"),
        }
        // The typed-only API skips it (and does not count it as an error).
        let mut parser = MavParser::new();
        assert!(parser.parse_bytes(&frame).is_empty());
        assert_eq!(parser.packet_errors(), 0);
    }

    #[test]
    fn test_tunnel_trimmed_frame_zero_extends() {
        // An MSP slice ending in zero bytes: MAVLink2 trims them off the wire, the parser restores them
        // up to payload_length.
        let chunk = [0x24, 0x58, 0x3E, 0x00, 0x01, 0x00, 0x00, 0x00];
        let frame = tunnel_frame(&chunk);
        assert_eq!(frame[1] as usize, 5 + 5, "wire payload trimmed to the last non-zero byte");
        let mut parser = MavParser::new();
        match parser.parse_all(&frame).pop() {
            Some(Parsed::Tunnel { tunnel, .. }) => assert_eq!(tunnel.payload, chunk.to_vec()),
            _ => panic!("expected a TUNNEL frame"),
        }
    }

    #[test]
    fn test_tunnel_crc_guards_frame() {
        let mut frame = tunnel_frame(&[0x24, 0x58, 0x3E, 0x10]);
        let last_payload = frame.len() - 3;
        frame[last_payload] ^= 0xFF; // corrupt one payload byte
        let mut parser = MavParser::new();
        assert!(parser.parse_all(&frame).is_empty());
        assert_eq!(parser.packet_errors(), 1);
    }

    #[test]
    fn test_tunnel_rejects_oversized_length() {
        use crate::mavlink_proto::{codec, tunnel};
        let mut payload = tunnel::encode_chunk(255, 190, &[1, 2, 3]);
        payload[4] = 200; // payload_length > 128
        let header = MavHeader { system_id: 1, component_id: 1, sequence: 0 };
        let frame = codec::serialize_raw_v2(&header, 385, 147, &payload, &mut codec::MavSequence::new());
        let mut parser = MavParser::new();
        assert!(parser.parse_all(&frame).is_empty());
    }
}
