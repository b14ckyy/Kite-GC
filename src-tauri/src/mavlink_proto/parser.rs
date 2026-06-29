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

    /// Feed a single byte. Returns a parsed frame if one is complete.
    pub fn push(&mut self, byte: u8) -> Option<MavFrame> {
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
    ) -> Option<MavFrame> {
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

        // Parse message payload into typed enum
        match MavMessage::parse(*version, msg_id, payload) {
            Ok(message) => {
                // Reconstruct full frame: STX + header + payload + CRC
                let stx = match version {
                    MavlinkVersion::V1 => 0xFE,
                    MavlinkVersion::V2 => 0xFD,
                };
                let mut raw = Vec::with_capacity(1 + header_buf.len() + payload_buf.len());
                raw.push(stx);
                raw.extend_from_slice(header_buf);
                raw.extend_from_slice(payload_buf);

                Some(MavFrame {
                    header,
                    message,
                    protocol_version: *version,
                    raw_bytes: raw,
                })
            }
            Err(e) => {
                log::debug!("MAVLink parse error for msg_id {}: {:?}", msg_id, e);
                None
            }
        }
    }

    /// Feed multiple bytes, collect all parsed frames
    pub fn parse_bytes(&mut self, data: &[u8]) -> Vec<MavFrame> {
        data.iter().filter_map(|&b| self.push(b)).collect()
    }
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
        let crc = compute_crc(&header_bytes, &payload, extra_crc);

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
}
