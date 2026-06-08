// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Codec — Encode and decode MSP v1/v2 frames

use super::types::*;

pub struct MspCodec;

impl MspCodec {
    /// Encode an MSP v1 request frame
    pub fn encode_v1(code: u16, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(6 + payload.len());
        frame.push(b'$');
        frame.push(b'M');
        frame.push(b'<');
        frame.push(payload.len() as u8);
        frame.push(code as u8);

        let mut checksum: u8 = payload.len() as u8 ^ code as u8;
        for &byte in payload {
            frame.push(byte);
            checksum ^= byte;
        }
        frame.push(checksum);
        frame
    }

    /// Encode an MSP v1 **response** frame (direction `>`) — for emulating an FC (e.g. FormationFlight).
    pub fn encode_v1_response(code: u16, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(6 + payload.len());
        frame.push(b'$');
        frame.push(b'M');
        frame.push(b'>');
        frame.push(payload.len() as u8);
        frame.push(code as u8);
        let mut checksum: u8 = payload.len() as u8 ^ code as u8;
        for &byte in payload {
            frame.push(byte);
            checksum ^= byte;
        }
        frame.push(checksum);
        frame
    }

    /// Encode an MSP v2 **response** frame (direction `>`).
    pub fn encode_v2_response(code: u16, payload: &[u8]) -> Vec<u8> {
        let payload_len = payload.len() as u16;
        let mut frame = Vec::with_capacity(9 + payload.len());
        frame.push(b'$');
        frame.push(b'X');
        frame.push(b'>');
        frame.push(0); // flag
        frame.push((code & 0xFF) as u8);
        frame.push((code >> 8) as u8);
        frame.push((payload_len & 0xFF) as u8);
        frame.push((payload_len >> 8) as u8);
        frame.extend_from_slice(payload);
        let crc = Self::crc8_dvb_s2(&frame[3..]);
        frame.push(crc);
        frame
    }

    /// Encode an MSP v2 request frame
    pub fn encode_v2(code: u16, payload: &[u8]) -> Vec<u8> {
        let payload_len = payload.len() as u16;
        let mut frame = Vec::with_capacity(9 + payload.len());
        frame.push(b'$');
        frame.push(b'X');
        frame.push(b'<');
        frame.push(0); // flag
        frame.push((code & 0xFF) as u8);
        frame.push((code >> 8) as u8);
        frame.push((payload_len & 0xFF) as u8);
        frame.push((payload_len >> 8) as u8);

        for &byte in payload {
            frame.push(byte);
        }

        let crc = Self::crc8_dvb_s2(&frame[3..]);
        frame.push(crc);
        frame
    }

    /// Attempt to decode an MSP frame from a buffer.
    /// Returns (message, bytes_consumed) on success.
    pub fn decode(buf: &[u8]) -> Option<(MspMessage, usize)> {
        if buf.len() < 6 {
            return None;
        }

        if buf[0] != b'$' {
            return None;
        }

        match buf[1] {
            b'M' => Self::decode_v1(buf),
            b'X' => Self::decode_v2(buf),
            _ => None,
        }
    }

    fn decode_v1(buf: &[u8]) -> Option<(MspMessage, usize)> {
        if buf.len() < 6 {
            return None;
        }

        let direction = match buf[2] {
            b'<' => MspDirection::Request,
            b'>' => MspDirection::Response,
            b'!' => MspDirection::Error,
            _ => return None,
        };

        let payload_len = buf[3] as usize;
        let total_len = 6 + payload_len;

        if buf.len() < total_len {
            return None;
        }

        let code = buf[4] as u16;
        let payload = buf[5..5 + payload_len].to_vec();

        // Verify checksum
        let mut checksum: u8 = buf[3] ^ buf[4];
        for &byte in &payload {
            checksum ^= byte;
        }
        if checksum != buf[5 + payload_len] {
            return None;
        }

        Some((
            MspMessage {
                version: MspVersion::V1,
                direction,
                code,
                payload,
            },
            total_len,
        ))
    }

    fn decode_v2(buf: &[u8]) -> Option<(MspMessage, usize)> {
        if buf.len() < 9 {
            return None;
        }

        let direction = match buf[2] {
            b'<' => MspDirection::Request,
            b'>' => MspDirection::Response,
            b'!' => MspDirection::Error,
            _ => return None,
        };

        let code = (buf[4] as u16) | ((buf[5] as u16) << 8);
        let payload_len = (buf[6] as u16) | ((buf[7] as u16) << 8);
        let total_len = 9 + payload_len as usize;

        if buf.len() < total_len {
            return None;
        }

        let payload = buf[8..8 + payload_len as usize].to_vec();

        // Verify CRC
        let expected_crc = Self::crc8_dvb_s2(&buf[3..total_len - 1]);
        if expected_crc != buf[total_len - 1] {
            return None;
        }

        Some((
            MspMessage {
                version: MspVersion::V2,
                direction,
                code,
                payload,
            },
            total_len,
        ))
    }

    /// CRC8 DVB-S2 used by MSP v2
    pub fn crc8_dvb_s2(data: &[u8]) -> u8 {
        let mut crc: u8 = 0;
        for &byte in data {
            crc ^= byte;
            for _ in 0..8 {
                if crc & 0x80 != 0 {
                    crc = (crc << 1) ^ 0xD5;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v1_encode_decode_roundtrip() {
        let frame = MspCodec::encode_v1(MSP_API_VERSION, &[]);
        let (msg, consumed) = MspCodec::decode(&frame).expect("decode failed");
        assert_eq!(consumed, frame.len());
        assert_eq!(msg.code, MSP_API_VERSION);
        assert_eq!(msg.version, MspVersion::V1);
        assert_eq!(msg.direction, MspDirection::Request);
        assert!(msg.payload.is_empty());
    }

    #[test]
    fn test_v2_encode_decode_roundtrip() {
        let payload = vec![1, 2, 3, 4];
        let frame = MspCodec::encode_v2(MSP_STATUS, &payload);
        let (msg, consumed) = MspCodec::decode(&frame).expect("decode failed");
        assert_eq!(consumed, frame.len());
        assert_eq!(msg.code, MSP_STATUS);
        assert_eq!(msg.version, MspVersion::V2);
        assert_eq!(msg.payload, payload);
    }
}
