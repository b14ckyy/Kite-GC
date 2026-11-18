// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/encodings.rs (2026-11-11)

//! Field encodings INAV writes (`blackbox_fielddefs.h` `FlightLogFieldEncoding`, INAV 9.1.0 lines
//! 149-155). Decoding logic after blackbox-tools v9.0.0 (`1918a75`) `src/decoders.c` and
//! `parser.c` `parseFrame`; the firmware writers are `blackbox_encoding.c` (tests below rebuild the
//! bytes with that logic).

use super::stream::Stream;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    /// 0 — ZigZag + variable byte.
    SignedVb,
    /// 1 — variable byte.
    UnsignedVb,
    /// 3 — unsigned VB holding the negated value, 14 bits significant.
    Neg14Bit,
    /// 6 — up to 8 consecutive fields: header byte with one bit per non-zero field, then SVBs;
    /// a group of one field has no header byte.
    Tag8_8Svb,
    /// 7 — 3 fields packed by a 2-bit selector (2/4/6 bits each, or 8/16/24/32 per field).
    Tag2_3S32,
    /// 8 — 4 fields, selector byte with 2 bits per field (0 / 4 / 8 / 16 bits).
    Tag8_4S16,
    /// 9 — nothing written, value 0.
    Null,
}

impl Encoding {
    /// The header's encoding id; `None` for ids INAV never writes (the Elias codes 4/5/10/11 of the
    /// Cleanflight era, or garbage).
    pub fn from_id(id: i32) -> Option<Self> {
        Some(match id {
            0 => Encoding::SignedVb,
            1 => Encoding::UnsignedVb,
            3 => Encoding::Neg14Bit,
            6 => Encoding::Tag8_8Svb,
            7 => Encoding::Tag2_3S32,
            8 => Encoding::Tag8_4S16,
            9 => Encoding::Null,
            _ => return None,
        })
    }
}

/// `tools.c` `signExtend14Bit` (argument truncated to 16 bits as the C `uint16_t` parameter).
fn sign_extend_14(word: u16) -> i32 {
    if word & 0x2000 != 0 {
        (word | 0xC000) as i16 as i32
    } else {
        word as i32
    }
}

fn sign_extend_2(v: u8) -> i64 {
    if v & 0x02 != 0 {
        (v | 0xFC) as i8 as i64
    } else {
        v as i64
    }
}

fn sign_extend_4(v: u8) -> i64 {
    if v & 0x08 != 0 {
        (v | 0xF0) as i8 as i64
    } else {
        v as i64
    }
}

fn sign_extend_6(v: u8) -> i64 {
    if v & 0x20 != 0 {
        (v | 0xC0) as i8 as i64
    } else {
        v as i64
    }
}

fn sign_extend_24(u: u32) -> i64 {
    if u & 0x80_0000 != 0 {
        (u | 0xFF00_0000) as i32 as i64
    } else {
        u as i64
    }
}

/// Encoding 3 (`parseFrame`: `-signExtend14Bit(streamReadUnsignedVB())`).
pub fn read_neg_14bit(s: &mut Stream) -> i64 {
    -(sign_extend_14(s.read_uvb() as u16) as i64)
}

/// Encoding 7 (`decoders.c` `streamReadTag2_3S32`).
pub fn read_tag2_3s32(s: &mut Stream, out: &mut [i64; 8]) {
    let lead = s.read_u8();
    match lead >> 6 {
        0 => {
            out[0] = sign_extend_2((lead >> 4) & 0x03);
            out[1] = sign_extend_2((lead >> 2) & 0x03);
            out[2] = sign_extend_2(lead & 0x03);
        }
        1 => {
            out[0] = sign_extend_4(lead & 0x0F);
            let b = s.read_u8();
            out[1] = sign_extend_4(b >> 4);
            out[2] = sign_extend_4(b & 0x0F);
        }
        2 => {
            out[0] = sign_extend_6(lead & 0x3F);
            out[1] = sign_extend_6(s.read_u8() & 0x3F);
            out[2] = sign_extend_6(s.read_u8() & 0x3F);
        }
        _ => {
            let mut selector = lead;
            for slot in out.iter_mut().take(3) {
                *slot = match selector & 0x03 {
                    0 => s.read_u8() as i8 as i64,
                    1 => {
                        let b = [s.read_u8(), s.read_u8()];
                        i16::from_le_bytes(b) as i64
                    }
                    2 => {
                        let b = [s.read_u8(), s.read_u8(), s.read_u8()];
                        sign_extend_24(
                            u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16,
                        )
                    }
                    _ => {
                        let b = [s.read_u8(), s.read_u8(), s.read_u8(), s.read_u8()];
                        i32::from_le_bytes(b) as i64
                    }
                };
                selector >>= 2;
            }
        }
    }
}

/// Encoding 8, `Data version` < 2 layout (`decoders.c` `streamReadTag8_4S16_v1`): a 4-bit
/// selector consumes two fields from one byte (first field in the low nibble).
pub fn read_tag8_4s16_v1(s: &mut Stream, out: &mut [i64; 8]) {
    let mut selector = s.read_u8();
    let mut i = 0;
    while i < 4 {
        match selector & 0x03 {
            0 => out[i] = 0,
            1 => {
                let combined = s.read_u8();
                out[i] = sign_extend_4(combined & 0x0F);
                i += 1;
                selector >>= 2;
                out[i] = sign_extend_4(combined >> 4);
            }
            2 => out[i] = s.read_u8() as i8 as i64,
            _ => {
                let b = [s.read_u8(), s.read_u8()];
                out[i] = i16::from_le_bytes(b) as i64;
            }
        }
        selector >>= 2;
        i += 1;
    }
}

/// Encoding 8, `Data version` ≥ 2 layout (`decoders.c` `streamReadTag8_4S16_v2`): nibble-packed,
/// high nibble first, 16-bit values big-endian.
pub fn read_tag8_4s16_v2(s: &mut Stream, out: &mut [i64; 8]) {
    let mut selector = s.read_u8();
    let mut buffer: u8 = 0;
    let mut nibble_index = 0;
    for slot in out.iter_mut().take(4) {
        *slot = match selector & 0x03 {
            0 => 0,
            1 => {
                if nibble_index == 0 {
                    buffer = s.read_u8();
                    nibble_index = 1;
                    sign_extend_4(buffer >> 4)
                } else {
                    nibble_index = 0;
                    sign_extend_4(buffer & 0x0F)
                }
            }
            2 => {
                if nibble_index == 0 {
                    s.read_u8() as i8 as i64
                } else {
                    let high = buffer << 4;
                    buffer = s.read_u8();
                    (high | buffer >> 4) as i8 as i64
                }
            }
            _ => {
                if nibble_index == 0 {
                    let b = [s.read_u8(), s.read_u8()];
                    i16::from_be_bytes(b) as i64
                } else {
                    let c1 = s.read_u8();
                    let c2 = s.read_u8();
                    let v = (u16::from(buffer) << 12) | (u16::from(c1) << 4) | (u16::from(c2) >> 4);
                    buffer = c2;
                    v as i16 as i64
                }
            }
        };
        selector >>= 2;
    }
}

/// Encoding 6 (`decoders.c` `streamReadTag8_8SVB`) for a group of `count` fields (1..=8).
pub fn read_tag8_8svb(s: &mut Stream, out: &mut [i64; 8], count: usize) {
    if count == 1 {
        out[0] = s.read_svb() as i64;
    } else {
        let mut header = s.read_u8();
        for slot in out.iter_mut() {
            *slot = if header & 0x01 != 0 {
                s.read_svb() as i64
            } else {
                0
            };
            header >>= 1;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::stream::tests::{write_svb, write_uvb};
    use super::*;

    /// Firmware `blackboxWriteTag2_3S32` (blackbox_encoding.c, INAV 9.1.0).
    pub(crate) fn write_tag2_3s32(out: &mut Vec<u8>, values: [i32; 3]) {
        const BITS_2: u8 = 0;
        const BITS_4: u8 = 1;
        const BITS_6: u8 = 2;
        const BITS_32: u8 = 3;
        let mut selector = BITS_2;
        for &v in &values {
            if !(-32..32).contains(&v) {
                selector = BITS_32;
                break;
            }
            if !(-8..8).contains(&v) {
                selector = selector.max(BITS_6);
            } else if !(-2..2).contains(&v) {
                selector = selector.max(BITS_4);
            }
        }
        match selector {
            BITS_2 => out.push(
                (selector << 6)
                    | ((values[0] & 0x03) << 4) as u8
                    | ((values[1] & 0x03) << 2) as u8
                    | (values[2] & 0x03) as u8,
            ),
            BITS_4 => {
                out.push((selector << 6) | (values[0] & 0x0F) as u8);
                out.push(((values[1] << 4) | (values[2] & 0x0F)) as u8);
            }
            BITS_6 => {
                out.push((selector << 6) | (values[0] & 0x3F) as u8);
                out.push(values[1] as u8);
                out.push(values[2] as u8);
            }
            _ => {
                let mut selector2: u8 = 0;
                for x in (0..3).rev() {
                    selector2 <<= 2;
                    let v = values[x];
                    selector2 |= if (-128..128).contains(&v) {
                        0
                    } else if (-32768..32768).contains(&v) {
                        1
                    } else if (-8_388_608..8_388_608).contains(&v) {
                        2
                    } else {
                        3
                    };
                }
                out.push((selector << 6) | selector2);
                for &v in &values {
                    let n = (selector2 & 0x03) as usize + 1;
                    out.extend_from_slice(&v.to_le_bytes()[..n]);
                    selector2 >>= 2;
                }
            }
        }
    }

    /// Firmware `blackboxWriteTag8_4S16` (the v2 layout, INAV 9.1.0).
    pub(crate) fn write_tag8_4s16(out: &mut Vec<u8>, values: [i32; 4]) {
        let mut selector: u8 = 0;
        for x in (0..4).rev() {
            selector <<= 2;
            let v = values[x];
            selector |= if v == 0 {
                0
            } else if (-8..8).contains(&v) {
                1
            } else if (-128..128).contains(&v) {
                2
            } else {
                3
            };
        }
        out.push(selector);
        let mut nibble_index = 0;
        let mut buffer: u8 = 0;
        for &v in &values {
            match selector & 0x03 {
                0 => {}
                1 => {
                    if nibble_index == 0 {
                        buffer = (v << 4) as u8;
                        nibble_index = 1;
                    } else {
                        out.push(buffer | (v & 0x0F) as u8);
                        nibble_index = 0;
                    }
                }
                2 => {
                    if nibble_index == 0 {
                        out.push(v as u8);
                    } else {
                        out.push(buffer | ((v >> 4) & 0x0F) as u8);
                        buffer = (v << 4) as u8;
                    }
                }
                _ => {
                    if nibble_index == 0 {
                        out.push((v >> 8) as u8);
                        out.push(v as u8);
                    } else {
                        out.push(buffer | ((v >> 12) & 0x0F) as u8);
                        out.push((v >> 4) as u8);
                        buffer = (v << 4) as u8;
                    }
                }
            }
            selector >>= 2;
        }
        if nibble_index == 1 {
            out.push(buffer);
        }
    }

    /// Firmware `blackboxWriteTag8_8SVB`.
    pub(crate) fn write_tag8_8svb(out: &mut Vec<u8>, values: &[i32]) {
        if values.len() == 1 {
            write_svb(out, values[0]);
            return;
        }
        let mut header: u8 = 0;
        for v in values.iter().rev() {
            header <<= 1;
            if *v != 0 {
                header |= 1;
            }
        }
        out.push(header);
        for &v in values {
            if v != 0 {
                write_svb(out, v);
            }
        }
    }

    fn assert_consumed(s: &Stream, bytes: &[u8]) {
        assert_eq!(s.pos(), bytes.len(), "bytes left over / missing");
        assert!(!s.eof());
    }

    #[test]
    fn neg_14bit() {
        // The firmware writes vbat as `blackboxWriteUnsignedVB((vbatReference - vbat) & 0x3FFF)`:
        // the decoder negates the sign-extended 14-bit value.
        for diff in [0i32, 1, -1, 100, -100, 8191, -8192] {
            let mut bytes = Vec::new();
            write_uvb(&mut bytes, (diff & 0x3FFF) as u32);
            let mut s = Stream::new(&bytes);
            assert_eq!(read_neg_14bit(&mut s), -(diff as i64), "diff {diff}");
            assert_consumed(&s, &bytes);
        }
    }

    #[test]
    fn tag2_3s32_every_width() {
        let cases: [[i32; 3]; 9] = [
            [0, 1, -2],
            [1, -1, 0],
            [7, -8, 3],
            [2, 0, -3],
            [31, -32, 9],
            [-9, 20, 0],
            [127, -32768, 8_388_607],
            [-8_388_608, 40_000, i32::MIN],
            [i32::MAX, -129, 128],
        ];
        for values in cases {
            let mut bytes = Vec::new();
            write_tag2_3s32(&mut bytes, values);
            let mut s = Stream::new(&bytes);
            let mut out = [0i64; 8];
            read_tag2_3s32(&mut s, &mut out);
            assert_eq!(&out[..3], &values.map(i64::from), "{values:?}");
            assert_consumed(&s, &bytes);
        }
    }

    #[test]
    fn tag8_4s16_v2_every_width_and_nibble_phase() {
        let cases: [[i32; 4]; 8] = [
            [0, 0, 0, 0],
            [1, -1, 7, -8],
            [3, 100, -100, 5],
            [1, 1000, -2, -32768],
            [-5, 127, -128, 32767],
            [0, 4, 0, -300],
            [200, 0, 1, 0],
            [6, -6, 120, 1],
        ];
        for values in cases {
            let mut bytes = Vec::new();
            write_tag8_4s16(&mut bytes, values);
            let mut s = Stream::new(&bytes);
            let mut out = [0i64; 8];
            read_tag8_4s16_v2(&mut s, &mut out);
            assert_eq!(&out[..4], &values.map(i64::from), "{values:?}");
            assert_consumed(&s, &bytes);
        }
    }

    #[test]
    fn tag8_4s16_v1_layout() {
        // Data version 1 (decoder logic only; no INAV ≥ 7 writes it): selector 0b11_10_00_01 →
        // fields 0+1 as two nibbles of one byte (low nibble first), field 2 8-bit, field 3 16-bit
        // little-endian.
        let bytes = [0b11_10_01_01u8, 0x93, 0x85, 0x34, 0x12];
        let mut s = Stream::new(&bytes);
        let mut out = [0i64; 8];
        read_tag8_4s16_v1(&mut s, &mut out);
        assert_eq!(&out[..4], &[3, -7, -123, 0x1234]);
        assert_consumed(&s, &bytes);
    }

    #[test]
    fn tag8_8svb_groups() {
        for values in [
            vec![5],
            vec![-7],
            vec![0, 0],
            vec![1, 0, -1000, 0, 70_000, 0, 0, 3],
            vec![0, 2, 0, 0, 0],
        ] {
            let mut bytes = Vec::new();
            write_tag8_8svb(&mut bytes, &values);
            let mut s = Stream::new(&bytes);
            let mut out = [0i64; 8];
            read_tag8_8svb(&mut s, &mut out, values.len());
            let expected: Vec<i64> = values.iter().map(|&v| v as i64).collect();
            assert_eq!(&out[..values.len()], &expected[..]);
            assert_consumed(&s, &bytes);
        }
    }

    #[test]
    fn encoding_ids() {
        assert_eq!(Encoding::from_id(0), Some(Encoding::SignedVb));
        assert_eq!(Encoding::from_id(9), Some(Encoding::Null));
        for elias in [4, 5, 10, 11, 2, -1] {
            assert_eq!(Encoding::from_id(elias), None);
        }
    }
}
