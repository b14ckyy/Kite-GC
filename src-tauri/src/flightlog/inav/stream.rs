// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/stream.rs (2026-11-11)

//! Byte cursor over one log's bytes with the Blackbox variable-byte readers. Logic after
//! blackbox-tools v9.0.0 (`1918a75`) `src/stream.c`; the bit-level readers (Elias codes) are not
//! needed — INAV never writes those encodings.

/// Cursor over `data[..end]`. Reading past `end` returns 0 and sets the sticky `eof` flag, like
/// `streamReadByte` returning `EOF`.
pub struct Stream<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
    eof: bool,
}

impl<'a> Stream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Stream {
            data,
            pos: 0,
            end: data.len(),
            eof: false,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos.min(self.end);
    }

    pub fn end(&self) -> usize {
        self.end
    }

    /// Stop reading at the current position (the clean end-of-log marker was read).
    pub fn truncate_here(&mut self) {
        self.end = self.pos;
    }

    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn clear_eof(&mut self) {
        self.eof = false;
    }

    /// Next byte, `None` at the end (sets `eof`).
    pub fn read_byte(&mut self) -> Option<u8> {
        if self.pos < self.end {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            self.eof = true;
            None
        }
    }

    /// Next byte, 0 at the end (`eof` set).
    pub fn read_u8(&mut self) -> u8 {
        self.read_byte().unwrap_or(0)
    }

    /// Unsigned variable-byte integer: 7 bits per byte, low group first, high bit = more follow.
    /// At most 5 bytes; a longer run or the end of the data yields 0 (`streamReadUnsignedVB`).
    pub fn read_uvb(&mut self) -> u32 {
        let mut result: u32 = 0;
        let mut shift = 0u32;
        for _ in 0..5 {
            let Some(c) = self.read_byte() else {
                return 0;
            };
            result |= u32::from(c & 0x7F).wrapping_shl(shift);
            if c < 0x80 {
                return result;
            }
            shift += 7;
        }
        0
    }

    /// Signed variable-byte integer: ZigZag-decoded [`read_uvb`](Self::read_uvb).
    pub fn read_svb(&mut self) -> i32 {
        zigzag_decode(self.read_uvb())
    }

    /// Little-endian IEEE-754 `f32` (`streamReadRawFloat`; the firmware's `blackboxWriteFloat`).
    pub fn read_f32_le(&mut self) -> f32 {
        let b = [
            self.read_u8(),
            self.read_u8(),
            self.read_u8(),
            self.read_u8(),
        ];
        f32::from_le_bytes(b)
    }
}

/// `tools.c` `zigzagDecode`.
pub fn zigzag_decode(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Firmware `blackboxWriteUnsignedVB` (blackbox_encoding.c).
    pub fn write_uvb(out: &mut Vec<u8>, mut value: u32) {
        while value > 127 {
            out.push((value | 0x80) as u8);
            value >>= 7;
        }
        out.push(value as u8);
    }

    /// Firmware `blackboxWriteSignedVB` (`zigzagEncode` + unsigned VB).
    pub fn write_svb(out: &mut Vec<u8>, value: i32) {
        write_uvb(out, ((value << 1) ^ (value >> 31)) as u32);
    }

    #[test]
    fn uvb_round_trip() {
        for v in [
            0u32,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            2_097_151,
            0x0FFF_FFFF,
            u32::MAX,
        ] {
            let mut bytes = Vec::new();
            write_uvb(&mut bytes, v);
            let mut s = Stream::new(&bytes);
            assert_eq!(s.read_uvb(), v);
            assert!(!s.eof());
            assert_eq!(s.pos(), bytes.len());
        }
    }

    #[test]
    fn svb_round_trip() {
        for v in [
            0i32,
            1,
            -1,
            63,
            -64,
            64,
            -65,
            1_000_000,
            -1_000_000,
            i32::MAX,
            i32::MIN,
        ] {
            let mut bytes = Vec::new();
            write_svb(&mut bytes, v);
            assert_eq!(Stream::new(&bytes).read_svb(), v);
        }
    }

    #[test]
    fn uvb_too_long_or_truncated_is_zero() {
        let mut s = Stream::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]);
        assert_eq!(s.read_uvb(), 0);
        let mut s = Stream::new(&[0x80, 0x80]);
        assert_eq!(s.read_uvb(), 0);
        assert!(s.eof());
    }

    #[test]
    fn little_endian_readers() {
        let bytes = 1.5f32.to_le_bytes();
        let mut s = Stream::new(&bytes);
        assert_eq!(s.read_f32_le(), 1.5);
        assert!(!s.eof());
        assert_eq!(s.read_byte(), None);
        assert!(s.eof());
    }
}
