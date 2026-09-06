// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTP header parsing + a small bounded reorder window.
//!
//! The window trades completeness for latency, deliberately: it holds at most
//! [`MAX_HELD`] packets to bridge UDP reordering, and gives a gap up when the window
//! fills — counting the skipped sequence numbers as lost and moving on. It is NOT a
//! jitter buffer; nothing here waits on wall-clock time.

#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub payload_type: u8,
    pub marker: bool,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Vec<u8>,
}

/// Parse one RTP packet (RFC 3550): version 2, CSRC list and header extension skipped,
/// padding stripped. Returns `None` for anything malformed.
pub fn parse(buf: &[u8]) -> Option<RtpPacket> {
    if buf.len() < 12 {
        return None;
    }
    let b0 = buf[0];
    if b0 >> 6 != 2 {
        return None;
    }
    let has_padding = b0 & 0x20 != 0;
    let has_extension = b0 & 0x10 != 0;
    let csrc_count = (b0 & 0x0F) as usize;
    let marker = buf[1] & 0x80 != 0;
    let payload_type = buf[1] & 0x7F;
    let sequence = u16::from_be_bytes([buf[2], buf[3]]);
    let timestamp = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ssrc = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);

    let mut offset = 12 + csrc_count * 4;
    if has_extension {
        if buf.len() < offset + 4 {
            return None;
        }
        let words = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        offset += 4 + words * 4;
    }
    let mut end = buf.len();
    if has_padding {
        let pad = *buf.last()? as usize;
        if pad == 0 || pad > end.saturating_sub(offset) {
            return None;
        }
        end -= pad;
    }
    if offset > end {
        return None;
    }
    Some(RtpPacket {
        payload_type,
        marker,
        sequence,
        timestamp,
        ssrc,
        payload: buf[offset..end].to_vec(),
    })
}

#[derive(Debug, Default, Clone)]
pub struct RtpStats {
    pub received: u64,
    /// Sequence numbers skipped after the reorder window gave a gap up.
    pub lost: u64,
    /// Packets that arrived after their slot had already been given up on.
    pub late_dropped: u64,
    /// Packets that arrived ahead of a gap and were held in the window.
    pub reordered: u64,
    pub bytes: u64,
}

/// Signed distance from `b` to `a` in sequence-number space (wrap-aware).
fn seq_delta(a: u16, b: u16) -> i16 {
    a.wrapping_sub(b) as i16
}

/// Bound on held packets — the only "buffering" in the pipeline (≈ a few ms at video rates).
const MAX_HELD: usize = 16;

#[derive(Default)]
pub struct Reorderer {
    expected: Option<u16>,
    held: Vec<RtpPacket>,
    pub stats: RtpStats,
}

impl Reorderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a received packet; every packet that is now deliverable in order is appended
    /// to `out` (zero, one, or several — a filled gap releases the run behind it).
    pub fn push(&mut self, pkt: RtpPacket, out: &mut Vec<RtpPacket>) {
        self.stats.received += 1;
        self.stats.bytes += pkt.payload.len() as u64;

        let expected = match self.expected {
            None => {
                self.expected = Some(pkt.sequence.wrapping_add(1));
                out.push(pkt);
                return;
            }
            Some(e) => e,
        };

        let delta = seq_delta(pkt.sequence, expected);
        if delta < 0 {
            self.stats.late_dropped += 1;
            return;
        }
        if delta == 0 {
            self.expected = Some(pkt.sequence.wrapping_add(1));
            out.push(pkt);
            self.drain(out);
            return;
        }

        // Ahead of a gap — hold it (drop exact duplicates).
        if self.held.iter().any(|p| p.sequence == pkt.sequence) {
            self.stats.late_dropped += 1;
            return;
        }
        self.stats.reordered += 1;
        self.held.push(pkt);

        if self.held.len() > MAX_HELD {
            // Give the gap up: jump to the earliest held packet, count the skipped range.
            let min_delta = self
                .held
                .iter()
                .map(|p| seq_delta(p.sequence, expected))
                .min()
                .unwrap_or(0)
                .max(0);
            self.stats.lost += min_delta as u64;
            self.expected = Some(expected.wrapping_add(min_delta as u16));
            self.drain(out);
        }
    }

    /// Deliver any held run that now starts at `expected`.
    fn drain(&mut self, out: &mut Vec<RtpPacket>) {
        loop {
            let e = match self.expected {
                Some(e) => e,
                None => return,
            };
            match self.held.iter().position(|p| p.sequence == e) {
                Some(pos) => {
                    let p = self.held.swap_remove(pos);
                    self.expected = Some(e.wrapping_add(1));
                    out.push(p);
                }
                None => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(seq: u16) -> RtpPacket {
        RtpPacket {
            payload_type: 26,
            marker: false,
            sequence: seq,
            timestamp: 0,
            ssrc: 1,
            payload: vec![0u8; 10],
        }
    }

    fn seqs(v: &[RtpPacket]) -> Vec<u16> {
        v.iter().map(|p| p.sequence).collect()
    }

    #[test]
    fn parses_a_plain_packet() {
        let mut raw = vec![0x80, 0x80 | 26, 0x12, 0x34, 0, 0, 0, 1, 0, 0, 0, 2];
        raw.extend_from_slice(b"data");
        let p = parse(&raw).expect("parse");
        assert!(p.marker);
        assert_eq!(p.payload_type, 26);
        assert_eq!(p.sequence, 0x1234);
        assert_eq!(p.payload, b"data");
    }

    #[test]
    fn strips_extension_and_padding() {
        // V=2 + X + P, one extension word, 3 padding bytes.
        let mut raw = vec![0xB0, 26, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2];
        raw.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x01, 9, 9, 9, 9]); // extension
        raw.extend_from_slice(b"xy");
        raw.extend_from_slice(&[0, 0, 3]); // padding
        let p = parse(&raw).expect("parse");
        assert_eq!(p.payload, b"xy");
    }

    #[test]
    fn in_order_and_swap() {
        let mut r = Reorderer::new();
        let mut out = Vec::new();
        r.push(pkt(10), &mut out);
        r.push(pkt(12), &mut out); // ahead — held
        r.push(pkt(11), &mut out); // fills the gap → releases 11 + 12
        assert_eq!(seqs(&out), vec![10, 11, 12]);
        assert_eq!(r.stats.reordered, 1);
        assert_eq!(r.stats.lost, 0);
    }

    #[test]
    fn gives_up_a_gap_and_counts_loss() {
        let mut r = Reorderer::new();
        let mut out = Vec::new();
        r.push(pkt(0), &mut out);
        // 1 is lost; 2.. keep arriving until the window overflows.
        for s in 2..(2 + MAX_HELD as u16 + 1) {
            r.push(pkt(s), &mut out);
        }
        assert_eq!(out[0].sequence, 0);
        assert_eq!(out[1].sequence, 2, "resumed after the abandoned gap");
        assert_eq!(r.stats.lost, 1);
        let last = out.last().unwrap().sequence;
        assert_eq!(last, 2 + MAX_HELD as u16);
    }

    #[test]
    fn wraps_around_the_sequence_space() {
        let mut r = Reorderer::new();
        let mut out = Vec::new();
        r.push(pkt(65534), &mut out);
        r.push(pkt(65535), &mut out);
        r.push(pkt(0), &mut out);
        r.push(pkt(1), &mut out);
        assert_eq!(seqs(&out), vec![65534, 65535, 0, 1]);
        assert_eq!(r.stats.lost, 0);
    }

    #[test]
    fn late_duplicate_is_dropped() {
        let mut r = Reorderer::new();
        let mut out = Vec::new();
        r.push(pkt(5), &mut out);
        r.push(pkt(6), &mut out);
        r.push(pkt(5), &mut out);
        assert_eq!(seqs(&out), vec![5, 6]);
        assert_eq!(r.stats.late_dropped, 1);
    }
}
