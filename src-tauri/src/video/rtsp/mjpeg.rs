// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTP/JPEG (RFC 2435) depacketizer.
//!
//! The RFC strips every JPEG header from the wire: a packet carries an 8-byte JPEG header
//! (fragment offset, type, Q, dimensions), optionally restart-marker and quantization-table
//! headers, and then raw entropy-coded scan data. The depacketizer reassembles the
//! fragments of one frame (offset-contiguous, frame ends on the RTP marker bit) and
//! re-synthesizes a complete JFIF byte stream: quantization tables scaled from Q via the
//! RFC's reference tables (or taken inline for Q ≥ 128), the standard Annex-K Huffman
//! tables (mandated by the RFC — senders never transmit them), SOF0 with the packet
//! dimensions, DRI when restart markers are in use.
//!
//! Loss policy matches the low-latency philosophy: a missing fragment abandons the frame
//! (counted in `dropped_frames`), collection resumes at the next fragment offset 0 — no
//! waiting, no retransmit.

use super::rtp::RtpPacket;

// ── RFC 2435 Appendix A reference quantization tables (zigzag order, as emitted) ──────

const LUMA_QUANT_ZIGZAG: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49,
    35, 37, 29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56,
    80, 109, 81, 87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103,
    99,
];

const CHROMA_QUANT_ZIGZAG: [u8; 64] = [
    17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

// ── JPEG Annex K standard Huffman tables (the RFC mandates these on the decode side) ──

const HUFF_DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const HUFF_DC_LUMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const HUFF_DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const HUFF_DC_CHROMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const HUFF_AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];
const HUFF_AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51,
    0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1,
    0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18,
    0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57,
    0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92,
    0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8,
    0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2,
    0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

const HUFF_AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const HUFF_AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07,
    0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09,
    0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25,
    0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38,
    0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56,
    0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74,
    0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba,
    0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
    0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2,
    0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

/// Scale the RFC reference tables by Q (1..99): factor = Q<50 ? 5000/Q : 200-2Q, each
/// value (base*factor+50)/100 clamped to 1..255. Q ≥ 100 is reserved / inline-table space.
fn scale_quant_tables(q: u8) -> ([u8; 64], [u8; 64]) {
    let q = i32::from(q.clamp(1, 99));
    let factor = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let scale = |base: &[u8; 64]| {
        let mut out = [0u8; 64];
        for (o, &b) in out.iter_mut().zip(base.iter()) {
            *o = ((i32::from(b) * factor + 50) / 100).clamp(1, 255) as u8;
        }
        out
    };
    (scale(&LUMA_QUANT_ZIGZAG), scale(&CHROMA_QUANT_ZIGZAG))
}

fn append_dht(out: &mut Vec<u8>, class_and_id: u8, bits: &[u8; 16], vals: &[u8]) {
    out.extend_from_slice(&[0xFF, 0xC4]);
    out.extend_from_slice(&((3 + 16 + vals.len()) as u16).to_be_bytes());
    out.push(class_and_id);
    out.extend_from_slice(bits);
    out.extend_from_slice(vals);
}

/// Everything from SOI up to and including the SOS header — scan data appends directly.
fn build_jpeg_headers(
    jpeg_type: u8,
    width: u16,
    height: u16,
    luma_q: &[u8; 64],
    chroma_q: &[u8; 64],
    dri: u16,
) -> Vec<u8> {
    let mut o = Vec::with_capacity(660);
    o.extend_from_slice(&[0xFF, 0xD8]); // SOI

    // DQT — both tables in one segment.
    o.extend_from_slice(&[0xFF, 0xDB]);
    o.extend_from_slice(&(2u16 + 65 + 65).to_be_bytes());
    o.push(0x00);
    o.extend_from_slice(luma_q);
    o.push(0x01);
    o.extend_from_slice(chroma_q);

    if dri > 0 {
        o.extend_from_slice(&[0xFF, 0xDD, 0x00, 0x04]);
        o.extend_from_slice(&dri.to_be_bytes());
    }

    // SOF0 — type even = 4:2:2 (2x1), type odd = 4:2:0 (2x2).
    o.extend_from_slice(&[0xFF, 0xC0]);
    o.extend_from_slice(&17u16.to_be_bytes());
    o.push(8); // precision
    o.extend_from_slice(&height.to_be_bytes());
    o.extend_from_slice(&width.to_be_bytes());
    o.push(3);
    let luma_sampling = if jpeg_type & 1 == 0 { 0x21 } else { 0x22 };
    o.extend_from_slice(&[1, luma_sampling, 0]);
    o.extend_from_slice(&[2, 0x11, 1]);
    o.extend_from_slice(&[3, 0x11, 1]);

    append_dht(&mut o, 0x00, &HUFF_DC_LUMA_BITS, &HUFF_DC_LUMA_VALS);
    append_dht(&mut o, 0x10, &HUFF_AC_LUMA_BITS, &HUFF_AC_LUMA_VALS);
    append_dht(&mut o, 0x01, &HUFF_DC_CHROMA_BITS, &HUFF_DC_CHROMA_VALS);
    append_dht(&mut o, 0x11, &HUFF_AC_CHROMA_BITS, &HUFF_AC_CHROMA_VALS);

    // SOS
    o.extend_from_slice(&[0xFF, 0xDA]);
    o.extend_from_slice(&12u16.to_be_bytes());
    o.push(3);
    o.extend_from_slice(&[1, 0x00, 2, 0x11, 3, 0x11]);
    o.extend_from_slice(&[0, 63, 0]);
    o
}

#[derive(Default)]
pub struct MjpegDepacketizer {
    headers: Vec<u8>,
    scan: Vec<u8>,
    collecting: bool,
    pub frames: u64,
    pub dropped_frames: u64,
}

impl MjpegDepacketizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one (in-order) RTP packet; returns a complete JPEG when the frame closes.
    pub fn push(&mut self, pkt: &RtpPacket) -> Option<Vec<u8>> {
        let p = &pkt.payload;
        if p.len() < 8 {
            return None;
        }
        let frag_offset = u32::from_be_bytes([0, p[1], p[2], p[3]]) as usize;
        let jpeg_type = p[4];
        let q = p[5];
        let width = u16::from(p[6]) * 8;
        let height = u16::from(p[7]) * 8;
        if width == 0 || height == 0 {
            return None;
        }
        let mut idx = 8usize;
        let mut dri = 0u16;
        if (64..=127).contains(&jpeg_type) {
            if p.len() < idx + 4 {
                return None;
            }
            dri = u16::from_be_bytes([p[idx], p[idx + 1]]);
            idx += 4;
        }

        if frag_offset == 0 {
            if self.collecting {
                self.dropped_frames += 1; // previous frame never saw its marker
            }
            self.scan.clear();
            self.collecting = true;

            let (luma_q, chroma_q): ([u8; 64], [u8; 64]);
            if q >= 128 {
                // Inline quantization tables (Quantization Table header, 8-bit precision).
                if p.len() < idx + 4 {
                    self.collecting = false;
                    return None;
                }
                let precision = p[idx + 1];
                let tlen = u16::from_be_bytes([p[idx + 2], p[idx + 3]]) as usize;
                idx += 4;
                if precision != 0 || tlen < 64 || p.len() < idx + tlen {
                    self.collecting = false;
                    return None; // 16-bit tables unsupported / malformed header
                }
                let mut l = [0u8; 64];
                l.copy_from_slice(&p[idx..idx + 64]);
                let mut c = l;
                if tlen >= 128 {
                    c.copy_from_slice(&p[idx + 64..idx + 128]);
                }
                idx += tlen;
                luma_q = l;
                chroma_q = c;
            } else {
                let t = scale_quant_tables(q);
                luma_q = t.0;
                chroma_q = t.1;
            }
            self.headers = build_jpeg_headers(jpeg_type, width, height, &luma_q, &chroma_q, dri);
        } else {
            if !self.collecting {
                return None; // lost the frame start — wait for the next offset 0
            }
            if self.scan.len() != frag_offset {
                // A fragment went missing mid-frame — abandon, resume at the next frame.
                self.collecting = false;
                self.dropped_frames += 1;
                return None;
            }
        }

        self.scan.extend_from_slice(&p[idx..]);
        if pkt.marker {
            self.collecting = false;
            let mut out = self.headers.clone();
            out.extend_from_slice(&self.scan);
            if !out.ends_with(&[0xFF, 0xD9]) {
                out.extend_from_slice(&[0xFF, 0xD9]); // EOI
            }
            self.frames += 1;
            return Some(out);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(
        seq: u16,
        marker: bool,
        offset: u32,
        jpeg_type: u8,
        q: u8,
        extra_header: &[u8],
        scan: &[u8],
    ) -> RtpPacket {
        let mut payload = vec![0u8];
        payload.extend_from_slice(&offset.to_be_bytes()[1..]); // 24-bit offset
        payload.extend_from_slice(&[jpeg_type, q, 64 / 8, 48 / 8]); // 64x48
        payload.extend_from_slice(extra_header);
        payload.extend_from_slice(scan);
        RtpPacket { marker, sequence: seq, timestamp: 0, payload }
    }

    /// Walk the produced JPEG's marker segments up to (and including) SOS.
    fn markers(jpeg: &[u8]) -> Vec<(u8, usize, usize)> {
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI");
        let mut out = Vec::new();
        let mut i = 2usize;
        while i + 4 <= jpeg.len() {
            assert_eq!(jpeg[i], 0xFF, "marker prefix at {i}");
            let m = jpeg[i + 1];
            let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
            out.push((m, i, len));
            i += 2 + len;
            if m == 0xDA {
                break;
            }
        }
        out
    }

    #[test]
    fn huffman_tables_are_consistent() {
        for (bits, vals) in [
            (&HUFF_DC_LUMA_BITS, &HUFF_DC_LUMA_VALS[..]),
            (&HUFF_DC_CHROMA_BITS, &HUFF_DC_CHROMA_VALS[..]),
            (&HUFF_AC_LUMA_BITS, &HUFF_AC_LUMA_VALS[..]),
            (&HUFF_AC_CHROMA_BITS, &HUFF_AC_CHROMA_VALS[..]),
        ] {
            let total: usize = bits.iter().map(|&b| b as usize).sum();
            assert_eq!(total, vals.len());
        }
    }

    #[test]
    fn reassembles_a_two_fragment_frame() {
        let scan: Vec<u8> = (0..=99u8).collect();
        let mut d = MjpegDepacketizer::new();
        assert!(d.push(&packet(1, false, 0, 1, 50, &[], &scan[..60])).is_none());
        let jpeg = d
            .push(&packet(2, true, 60, 1, 50, &[], &scan[60..]))
            .expect("complete frame");

        assert!(jpeg.ends_with(&[0xFF, 0xD9]), "EOI appended");
        let ms = markers(&jpeg);
        let kinds: Vec<u8> = ms.iter().map(|(m, _, _)| *m).collect();
        assert_eq!(kinds, vec![0xDB, 0xC0, 0xC4, 0xC4, 0xC4, 0xC4, 0xDA]);

        // SOF0 carries the dimensions and 4:2:0 sampling (type 1).
        let (_, sof, _) = ms.iter().find(|(m, _, _)| *m == 0xC0).copied().unwrap();
        assert_eq!(u16::from_be_bytes([jpeg[sof + 5], jpeg[sof + 6]]), 48);
        assert_eq!(u16::from_be_bytes([jpeg[sof + 7], jpeg[sof + 8]]), 64);
        assert_eq!(jpeg[sof + 11], 0x22);

        // Scan data lands verbatim between the SOS header and the EOI.
        let (_, sos, len) = ms.iter().find(|(m, _, _)| *m == 0xDA).copied().unwrap();
        let scan_start = sos + 2 + len;
        assert_eq!(&jpeg[scan_start..jpeg.len() - 2], &scan[..]);
        assert_eq!(d.frames, 1);
        assert_eq!(d.dropped_frames, 0);
    }

    #[test]
    fn inline_quant_tables_are_used_verbatim() {
        let mut tables = Vec::new();
        let mut qhdr = vec![0u8, 0u8]; // MBZ, precision 0
        qhdr.extend_from_slice(&128u16.to_be_bytes());
        for i in 0..128u8 {
            tables.push(i.wrapping_add(1));
        }
        qhdr.extend_from_slice(&tables);

        let mut d = MjpegDepacketizer::new();
        let jpeg = d
            .push(&packet(1, true, 0, 0, 255, &qhdr, b"scan"))
            .expect("frame");
        let ms = markers(&jpeg);
        let (_, dqt, _) = ms.iter().find(|(m, _, _)| *m == 0xDB).copied().unwrap();
        // Layout: FF DB len id0 [64 bytes] id1 [64 bytes]
        assert_eq!(&jpeg[dqt + 5..dqt + 5 + 64], &tables[..64]);
        assert_eq!(&jpeg[dqt + 5 + 64 + 1..dqt + 5 + 64 + 1 + 64], &tables[64..]);
        // Type 0 → 4:2:2 sampling.
        let (_, sof, _) = ms.iter().find(|(m, _, _)| *m == 0xC0).copied().unwrap();
        assert_eq!(jpeg[sof + 11], 0x21);
    }

    #[test]
    fn lost_fragment_drops_the_frame_and_recovers() {
        let scan: Vec<u8> = (0..=99u8).collect();
        let mut d = MjpegDepacketizer::new();
        assert!(d.push(&packet(1, false, 0, 1, 50, &[], &scan[..40])).is_none());
        // Fragment at offset 40 lost; the marker fragment arrives with offset 80.
        assert!(d.push(&packet(3, true, 80, 1, 50, &[], &scan[80..])).is_none());
        assert_eq!(d.dropped_frames, 1);
        // Next complete frame goes through untouched.
        let jpeg = d.push(&packet(4, true, 0, 1, 50, &[], &scan)).expect("frame");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
        assert_eq!(d.frames, 1);
    }
}
