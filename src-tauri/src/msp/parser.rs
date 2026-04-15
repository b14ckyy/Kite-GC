// MSP Streaming Parser — byte-by-byte state machine
// Mirrors the INAV Configurator's msp.js decoder states

use super::types::*;

/// Decoder states matching INAV Configurator msp.js
#[derive(Debug, Clone, Copy, PartialEq)]
enum DecoderState {
    Idle,
    ProtoIdentifier,
    DirectionV1,
    DirectionV2,
    FlagV2,
    PayloadLengthV1,
    PayloadLengthJumboLow,
    PayloadLengthJumboHigh,
    PayloadLengthV2Low,
    PayloadLengthV2High,
    CodeV1,
    CodeJumboV1,
    CodeV2Low,
    CodeV2High,
    PayloadV1,
    PayloadV2,
    ChecksumV1,
    ChecksumV2,
}

/// Streaming MSP parser that processes one byte at a time.
/// Feed serial data via `push()` and receive complete `MspMessage`s.
pub struct MspParser {
    state: DecoderState,
    version: MspVersion,
    direction: MspDirection,
    code: u16,
    flag: u8,
    payload_length_expected: u16,
    payload_length_received: u16,
    payload: Vec<u8>,
    checksum: u8,
    packet_errors: u32,
}

impl MspParser {
    pub fn new() -> Self {
        Self {
            state: DecoderState::Idle,
            version: MspVersion::V1,
            direction: MspDirection::Response,
            code: 0,
            flag: 0,
            payload_length_expected: 0,
            payload_length_received: 0,
            payload: Vec::new(),
            checksum: 0,
            packet_errors: 0,
        }
    }

    /// Feed one byte to the parser. Returns `Some(MspMessage)` when a complete
    /// valid message has been decoded.
    pub fn push(&mut self, byte: u8) -> Option<MspMessage> {
        match self.state {
            DecoderState::Idle => {
                if byte == b'$' {
                    self.state = DecoderState::ProtoIdentifier;
                }
                None
            }

            DecoderState::ProtoIdentifier => {
                match byte {
                    b'M' => {
                        self.version = MspVersion::V1;
                        self.state = DecoderState::DirectionV1;
                    }
                    b'X' => {
                        self.version = MspVersion::V2;
                        self.state = DecoderState::DirectionV2;
                    }
                    _ => {
                        self.state = DecoderState::Idle;
                    }
                }
                None
            }

            DecoderState::DirectionV1 | DecoderState::DirectionV2 => {
                self.direction = match byte {
                    b'>' => MspDirection::Response,
                    b'<' => MspDirection::Request,
                    b'!' => MspDirection::Error,
                    _ => {
                        self.state = DecoderState::Idle;
                        return None;
                    }
                };
                self.state = if self.state == DecoderState::DirectionV1 {
                    DecoderState::PayloadLengthV1
                } else {
                    DecoderState::FlagV2
                };
                None
            }

            DecoderState::FlagV2 => {
                self.flag = byte;
                self.state = DecoderState::CodeV2Low;
                None
            }

            DecoderState::PayloadLengthV1 => {
                self.payload_length_expected = byte as u16;
                if byte == JUMBO_FRAME_MIN_SIZE {
                    self.state = DecoderState::CodeJumboV1;
                } else {
                    self.init_payload_buffer();
                    self.state = DecoderState::CodeV1;
                }
                None
            }

            DecoderState::CodeV1 | DecoderState::CodeJumboV1 => {
                self.code = byte as u16;
                if self.payload_length_expected > 0 {
                    if self.state == DecoderState::CodeJumboV1 {
                        self.state = DecoderState::PayloadLengthJumboLow;
                    } else {
                        self.state = DecoderState::PayloadV1;
                    }
                } else {
                    self.state = DecoderState::ChecksumV1;
                }
                None
            }

            DecoderState::PayloadLengthJumboLow => {
                self.payload_length_expected = byte as u16;
                self.state = DecoderState::PayloadLengthJumboHigh;
                None
            }

            DecoderState::PayloadLengthJumboHigh => {
                self.payload_length_expected |= (byte as u16) << 8;
                self.init_payload_buffer();
                self.state = DecoderState::PayloadV1;
                None
            }

            DecoderState::CodeV2Low => {
                self.code = byte as u16;
                self.state = DecoderState::CodeV2High;
                None
            }

            DecoderState::CodeV2High => {
                self.code |= (byte as u16) << 8;
                self.state = DecoderState::PayloadLengthV2Low;
                None
            }

            DecoderState::PayloadLengthV2Low => {
                self.payload_length_expected = byte as u16;
                self.state = DecoderState::PayloadLengthV2High;
                None
            }

            DecoderState::PayloadLengthV2High => {
                self.payload_length_expected |= (byte as u16) << 8;
                self.init_payload_buffer();
                if self.payload_length_expected > 0 {
                    self.state = DecoderState::PayloadV2;
                } else {
                    self.state = DecoderState::ChecksumV2;
                }
                None
            }

            DecoderState::PayloadV1 | DecoderState::PayloadV2 => {
                self.payload[self.payload_length_received as usize] = byte;
                self.payload_length_received += 1;

                if self.payload_length_received >= self.payload_length_expected {
                    self.state = if self.state == DecoderState::PayloadV1 {
                        DecoderState::ChecksumV1
                    } else {
                        DecoderState::ChecksumV2
                    };
                }
                None
            }

            DecoderState::ChecksumV1 => {
                let result = self.verify_v1_checksum(byte);
                self.state = DecoderState::Idle;
                result
            }

            DecoderState::ChecksumV2 => {
                let result = self.verify_v2_checksum(byte);
                self.state = DecoderState::Idle;
                result
            }
        }
    }

    fn init_payload_buffer(&mut self) {
        self.payload = vec![0u8; self.payload_length_expected as usize];
        self.payload_length_received = 0;
    }

    fn verify_v1_checksum(&mut self, received_crc: u8) -> Option<MspMessage> {
        let mut checksum: u8;

        // Jumbo frames use 255 as the initial length byte in checksum calc
        if self.payload_length_expected >= JUMBO_FRAME_MIN_SIZE as u16 {
            checksum = JUMBO_FRAME_MIN_SIZE;
        } else {
            checksum = self.payload_length_expected as u8;
        }

        checksum ^= self.code as u8;

        // For jumbo frames, actual length bytes are included in checksum
        if self.payload_length_expected >= JUMBO_FRAME_MIN_SIZE as u16 {
            checksum ^= (self.payload_length_expected & 0xFF) as u8;
            checksum ^= ((self.payload_length_expected >> 8) & 0xFF) as u8;
        }

        for i in 0..self.payload_length_received as usize {
            checksum ^= self.payload[i];
        }

        if checksum == received_crc {
            Some(self.build_message())
        } else {
            self.packet_errors += 1;
            None
        }
    }

    fn verify_v2_checksum(&mut self, received_crc: u8) -> Option<MspMessage> {
        let mut crc: u8 = 0;
        crc = Self::crc8_dvb_s2_byte(crc, self.flag);
        crc = Self::crc8_dvb_s2_byte(crc, (self.code & 0xFF) as u8);
        crc = Self::crc8_dvb_s2_byte(crc, ((self.code >> 8) & 0xFF) as u8);
        crc = Self::crc8_dvb_s2_byte(crc, (self.payload_length_expected & 0xFF) as u8);
        crc = Self::crc8_dvb_s2_byte(crc, ((self.payload_length_expected >> 8) & 0xFF) as u8);

        for i in 0..self.payload_length_received as usize {
            crc = Self::crc8_dvb_s2_byte(crc, self.payload[i]);
        }

        if crc == received_crc {
            Some(self.build_message())
        } else {
            self.packet_errors += 1;
            None
        }
    }

    fn build_message(&self) -> MspMessage {
        MspMessage {
            version: self.version,
            direction: self.direction,
            code: self.code,
            payload: self.payload[..self.payload_length_received as usize].to_vec(),
        }
    }

    /// CRC8 DVB-S2 per-byte — identical to INAV Configurator _crc8_dvb_s2
    fn crc8_dvb_s2_byte(mut crc: u8, byte: u8) -> u8 {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0xD5;
            } else {
                crc <<= 1;
            }
        }
        crc
    }

    pub fn packet_errors(&self) -> u32 {
        self.packet_errors
    }

    /// Reset the parser to idle state
    pub fn reset(&mut self) {
        self.state = DecoderState::Idle;
        self.payload_length_received = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msp::MspCodec;

    #[test]
    fn test_parser_v2_frame() {
        let frame = MspCodec::encode_v2(MSP_API_VERSION, &[]);
        let mut parser = MspParser::new();

        let mut result = None;
        for &byte in &frame {
            if let Some(msg) = parser.push(byte) {
                result = Some(msg);
            }
        }

        let msg = result.expect("parser should produce a message");
        assert_eq!(msg.code, MSP_API_VERSION);
        assert_eq!(msg.version, MspVersion::V2);
        assert!(msg.payload.is_empty());
    }

    #[test]
    fn test_parser_v1_frame() {
        let frame = MspCodec::encode_v1(MSP_FC_VARIANT, &[]);
        let mut parser = MspParser::new();

        let mut result = None;
        for &byte in &frame {
            if let Some(msg) = parser.push(byte) {
                result = Some(msg);
            }
        }

        let msg = result.expect("parser should produce a message");
        assert_eq!(msg.code, MSP_FC_VARIANT);
        assert_eq!(msg.version, MspVersion::V1);
    }

    #[test]
    fn test_parser_v2_with_payload() {
        let payload = vec![0, 2, 5]; // mock MSP_API_VERSION response
        let frame = MspCodec::encode_v2(MSP_API_VERSION, &payload);

        let mut parser = MspParser::new();
        let mut result = None;
        for &byte in &frame {
            if let Some(msg) = parser.push(byte) {
                result = Some(msg);
            }
        }

        let msg = result.expect("parser should decode payload");
        assert_eq!(msg.code, MSP_API_VERSION);
        assert_eq!(msg.payload, payload);
    }

    #[test]
    fn test_parser_with_garbage_prefix() {
        let garbage = vec![0xFF, 0x00, 0xAA, 0x55];
        let frame = MspCodec::encode_v2(MSP_FC_VERSION, &[1, 2, 3]);

        let mut data = garbage;
        data.extend_from_slice(&frame);

        let mut parser = MspParser::new();
        let mut result = None;
        for &byte in &data {
            if let Some(msg) = parser.push(byte) {
                result = Some(msg);
            }
        }

        let msg = result.expect("parser should skip garbage and decode");
        assert_eq!(msg.code, MSP_FC_VERSION);
        assert_eq!(msg.payload, vec![1, 2, 3]);
    }

    #[test]
    fn test_parser_multiple_frames() {
        let frame1 = MspCodec::encode_v2(MSP_API_VERSION, &[]);
        let frame2 = MspCodec::encode_v2(MSP_FC_VARIANT, &[b'I', b'N', b'A', b'V']);

        let mut data = frame1;
        data.extend_from_slice(&frame2);

        let mut parser = MspParser::new();
        let mut messages = Vec::new();
        for &byte in &data {
            if let Some(msg) = parser.push(byte) {
                messages.push(msg);
            }
        }

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].code, MSP_API_VERSION);
        assert_eq!(messages[1].code, MSP_FC_VARIANT);
        assert_eq!(messages[1].payload, vec![b'I', b'N', b'A', b'V']);
    }

    #[test]
    fn test_parser_bad_crc_rejected() {
        let mut frame = MspCodec::encode_v2(MSP_API_VERSION, &[]);
        // Corrupt the CRC (last byte)
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;

        let mut parser = MspParser::new();
        let mut result = None;
        for &byte in &frame {
            if let Some(msg) = parser.push(byte) {
                result = Some(msg);
            }
        }

        assert!(result.is_none());
        assert_eq!(parser.packet_errors(), 1);
    }
}
