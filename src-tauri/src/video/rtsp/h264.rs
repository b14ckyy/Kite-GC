// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTP/H.264 (RFC 6184) depacketizer.
//!
//! Reassembles the RTP payloads into complete **access units** in Annex-B byte-stream
//! form (4-byte start codes), which is what the platform decoders take: single NAL units,
//! STAP-A aggregates and FU-A fragments — the three packetization modes real encoders
//! emit (`packetization-mode=1`). STAP-B/MTAP/FU-B (interleaved mode) are not produced by
//! any encoder Kite meets and mark the AU damaged.
//!
//! AU boundaries: the RTP marker bit ends an AU; a timestamp change without one (lost
//! marker packet) flushes the previous AU first. Loss policy matches the pipeline's
//! philosophy: a sequence gap tears the fragment and taints the AU, and a **damaged AU is
//! dropped whole** — the decoder is never handed a torn frame; it holds the last good
//! picture until the next complete one (`damaged_aus` counts them for the stats).
//!
//! `sprop-parameter-sets` from the SDP (SPS/PPS) is decoded once and prepended to IDR
//! access units that don't carry their own — most RTSP cameras send parameter sets only
//! out-of-band, and a decoder can't start mid-stream without them.

use super::rtp::RtpPacket;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Value of one `key=value` item in an `a=fmtp` parameter list.
pub(super) fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(key)?.strip_prefix('='))
}

/// Minimal base64 decoder (standard alphabet, padding tolerated) for the sprop-* values.
pub(super) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let s = s.trim().trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// True when the Annex-B buffer contains a NAL of `nal_type` (4-byte start codes only —
/// exactly what this module writes).
fn contains_nal_type(annexb: &[u8], nal_type: u8) -> bool {
    annexb
        .windows(5)
        .any(|w| w[..4] == START_CODE && w[4] & 0x1F == nal_type)
}

pub struct H264Depacketizer {
    au: Vec<u8>,
    au_timestamp: u32,
    au_open: bool,
    au_damaged: bool,
    /// The AU holds at least one VCL NAL (a slice, types 1..=5) — the marker bit only
    /// closes an AU that actually carries picture data. Some servers set the marker on
    /// prefix packets too (AUD/SEI — measured on obs-rtspserver's H265 twin), which
    /// would split every frame in two; the timestamp change stays as the backstop.
    au_has_vcl: bool,
    frag: Vec<u8>,
    frag_open: bool,
    last_seq: Option<u16>,
    /// SPS+PPS from `sprop-parameter-sets`, Annex-B framed. Empty when absent.
    param_sets: Vec<u8>,
    pub aus: u64,
    pub damaged_aus: u64,
}

impl H264Depacketizer {
    pub fn new(fmtp: Option<&str>) -> Self {
        let mut param_sets = Vec::new();
        if let Some(sprop) = fmtp.and_then(|f| fmtp_param(f, "sprop-parameter-sets")) {
            for part in sprop.split(',') {
                if let Some(nal) = base64_decode(part).filter(|n| !n.is_empty()) {
                    param_sets.extend_from_slice(&START_CODE);
                    param_sets.extend_from_slice(&nal);
                }
            }
        }
        Self {
            au: Vec::new(),
            au_timestamp: 0,
            au_open: false,
            au_damaged: false,
            au_has_vcl: false,
            frag: Vec::new(),
            frag_open: false,
            last_seq: None,
            param_sets,
            aus: 0,
            damaged_aus: 0,
        }
    }

    /// Feed one (in-order) RTP packet; returns zero, one, or two complete access units
    /// (two when a timestamp change flushes the previous AU and the marker closes this one).
    pub fn push(&mut self, pkt: &RtpPacket) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let p = &pkt.payload;
        if p.is_empty() {
            return out;
        }

        // A sequence gap (the reorder window gave up on a packet) tears any fragment in
        // flight and taints the AU under assembly.
        if let Some(last) = self.last_seq {
            if pkt.sequence != last.wrapping_add(1) {
                if self.frag_open {
                    self.frag.clear();
                    self.frag_open = false;
                }
                if self.au_open {
                    self.au_damaged = true;
                }
            }
        }
        self.last_seq = Some(pkt.sequence);

        // Timestamp moved on without a marker → the previous AU is complete as-is.
        if self.au_open && pkt.timestamp != self.au_timestamp {
            self.flush_au(&mut out);
        }
        if !self.au_open {
            self.au_timestamp = pkt.timestamp;
            self.au_open = true;
        }

        let nal_type = p[0] & 0x1F;
        match nal_type {
            1..=23 => self.append_nal(p),
            24 => {
                // STAP-A: [u16 len][NAL] repeated.
                let mut i = 1usize;
                while i + 2 <= p.len() {
                    let len = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                    i += 2;
                    if len == 0 || i + len > p.len() {
                        self.au_damaged = true;
                        break;
                    }
                    let nal = p[i..i + len].to_vec();
                    self.append_nal(&nal);
                    i += len;
                }
            }
            28 => {
                // FU-A: FU indicator + FU header (S|E|type) + fragment.
                if p.len() < 2 {
                    self.au_damaged = true;
                    return out;
                }
                let start = p[1] & 0x80 != 0;
                let end = p[1] & 0x40 != 0;
                if start {
                    self.frag.clear();
                    self.frag.push((p[0] & 0xE0) | (p[1] & 0x1F));
                    self.frag_open = true;
                }
                if self.frag_open {
                    self.frag.extend_from_slice(&p[2..]);
                    if end {
                        let nal = std::mem::take(&mut self.frag);
                        self.frag_open = false;
                        self.append_nal(&nal);
                    }
                } else {
                    // Continuation of a fragment whose start we never saw.
                    self.au_damaged = true;
                }
            }
            _ => {
                // STAP-B / MTAP16 / MTAP24 / FU-B (interleaved mode) or reserved.
                self.au_damaged = true;
            }
        }

        if pkt.marker && self.au_has_vcl {
            self.flush_au(&mut out);
        }
        out
    }

    fn append_nal(&mut self, nal: &[u8]) {
        if nal.is_empty() {
            return;
        }
        if (1..=5).contains(&(nal[0] & 0x1F)) {
            self.au_has_vcl = true;
        }
        self.au.extend_from_slice(&START_CODE);
        self.au.extend_from_slice(nal);
    }

    fn flush_au(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.frag_open {
            // The AU closed with a fragment still open — its NAL is torn.
            self.frag.clear();
            self.frag_open = false;
            self.au_damaged = true;
        }
        self.au_open = false;
        self.au_has_vcl = false;
        if self.au.is_empty() {
            self.au_damaged = false;
            return;
        }
        let au = std::mem::take(&mut self.au);
        if self.au_damaged {
            self.au_damaged = false;
            self.damaged_aus += 1;
            return; // never hand the decoder a torn frame
        }
        // Decoders can't start without SPS/PPS: prepend the out-of-band sets before an
        // IDR unless this AU carries its own (repeats are harmless, a missing set is not).
        let is_idr = contains_nal_type(&au, 5);
        let has_sps = contains_nal_type(&au, 7);
        let mut emit = Vec::with_capacity(self.param_sets.len() + au.len());
        if is_idr && !has_sps && !self.param_sets.is_empty() {
            emit.extend_from_slice(&self.param_sets);
        }
        emit.extend_from_slice(&au);
        self.aus += 1;
        out.push(emit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(seq: u16, ts: u32, marker: bool, payload: Vec<u8>) -> RtpPacket {
        RtpPacket { payload_type: 96, marker, sequence: seq, timestamp: ts, ssrc: 1, payload }
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in nals {
            out.extend_from_slice(&START_CODE);
            out.extend_from_slice(n);
        }
        out
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64_decode("Z2QA").unwrap(), vec![0x67, 0x64, 0x00]);
        assert_eq!(base64_decode("aO4=").unwrap(), vec![0x68, 0xEE]);
        assert!(base64_decode("!!").is_none());
    }

    #[test]
    fn marker_on_a_prefix_packet_does_not_split_the_frame() {
        // Some servers set the marker on prefix packets (SEI/AUD) too — the AU must stay
        // open until picture data (a VCL NAL) arrived, else every frame splits in two.
        let mut d = H264Depacketizer::new(None);
        let sei = vec![0x06, 5, 1, 2, 3]; // SEI (type 6)
        assert!(d.push(&pkt(1, 1000, true, sei.clone())).is_empty());
        let slice = vec![0x41, 9, 9, 9]; // non-IDR slice (type 1)
        assert_eq!(
            d.push(&pkt(2, 1000, true, slice.clone())),
            vec![annexb(&[&sei, &slice])]
        );
        assert_eq!(d.aus, 1);
    }

    #[test]
    fn single_nal_becomes_an_access_unit() {
        let mut d = H264Depacketizer::new(None);
        let nal = vec![0x41, 1, 2, 3];
        let aus = d.push(&pkt(1, 1000, true, nal.clone()));
        assert_eq!(aus, vec![annexb(&[&nal])]);
        assert_eq!(d.aus, 1);
    }

    #[test]
    fn fu_a_reassembles_the_original_nal() {
        let mut d = H264Depacketizer::new(None);
        let mut nal = vec![0x41u8];
        nal.extend(0..60u8);
        let body = &nal[1..];
        let ind = (nal[0] & 0xE0) | 28;
        let fu = |s: bool, e: bool, chunk: &[u8]| {
            let mut p = vec![ind, (u8::from(s) << 7) | (u8::from(e) << 6) | (nal[0] & 0x1F)];
            p.extend_from_slice(chunk);
            p
        };
        assert!(d.push(&pkt(1, 5, false, fu(true, false, &body[..20]))).is_empty());
        assert!(d.push(&pkt(2, 5, false, fu(false, false, &body[20..40]))).is_empty());
        let aus = d.push(&pkt(3, 5, true, fu(false, true, &body[40..])));
        assert_eq!(aus, vec![annexb(&[&nal])]);
    }

    #[test]
    fn stap_a_unpacks_all_nals() {
        let sps = [0x67u8, 0x64, 0x00, 0x1F];
        let pps = [0x68u8, 0xEE, 0x3C];
        let idr = [0x65u8, 0x88, 0x80];
        let mut stap = vec![24u8];
        for n in [&sps[..], &pps[..], &idr[..]] {
            stap.extend_from_slice(&(n.len() as u16).to_be_bytes());
            stap.extend_from_slice(n);
        }
        let mut d = H264Depacketizer::new(Some("sprop-parameter-sets=Z2QA,aO4="));
        let aus = d.push(&pkt(1, 7, true, stap));
        // In-band SPS present → the sprop sets are NOT prepended a second time.
        assert_eq!(aus, vec![annexb(&[&sps, &pps, &idr])]);
    }

    #[test]
    fn sprop_sets_prepend_before_idr_only() {
        let mut d = H264Depacketizer::new(Some("packetization-mode=1;sprop-parameter-sets=Z2QA,aO4="));
        let aus = d.push(&pkt(1, 9, true, vec![0x65, 0xAA]));
        assert_eq!(
            aus,
            vec![annexb(&[&[0x67, 0x64, 0x00], &[0x68, 0xEE], &[0x65, 0xAA]])]
        );
        // Non-IDR AU: nothing prepended.
        let aus = d.push(&pkt(2, 10, true, vec![0x41, 0xBB]));
        assert_eq!(aus, vec![annexb(&[&[0x41, 0xBB]])]);
    }

    #[test]
    fn a_gap_drops_the_torn_access_unit_but_not_the_next() {
        let mut d = H264Depacketizer::new(None);
        assert!(d.push(&pkt(1, 5, false, vec![0x41, 1])).is_empty());
        // Sequence 2 was lost — this AU is torn and must be swallowed whole.
        let aus = d.push(&pkt(3, 5, true, vec![0x41, 2]));
        assert!(aus.is_empty());
        assert_eq!(d.damaged_aus, 1);
        // The next AU is clean.
        let aus = d.push(&pkt(4, 6, true, vec![0x41, 3]));
        assert_eq!(aus.len(), 1);
        assert_eq!(d.aus, 1);
    }

    #[test]
    fn timestamp_change_flushes_a_marker_less_au() {
        let mut d = H264Depacketizer::new(None);
        assert!(d.push(&pkt(1, 100, false, vec![0x41, 1])).is_empty());
        let aus = d.push(&pkt(2, 200, true, vec![0x41, 2]));
        assert_eq!(aus.len(), 2, "previous AU flushed by the timestamp change, plus the marked one");
        assert_eq!(aus[0], annexb(&[&[0x41, 1]]));
        assert_eq!(aus[1], annexb(&[&[0x41, 2]]));
    }
}
