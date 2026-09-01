// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTP/H.265 (RFC 7798) depacketizer.
//!
//! The HEVC sibling of `h264.rs`, with the same AU-assembly and loss policy (damaged
//! access units are dropped whole; see there). Differences are mechanical: the NAL header
//! is two bytes (`type = (b0 >> 1) & 0x3F`), aggregation packets are type 48 (AP),
//! fragmentation units type 49 (FU, three header bytes), and the out-of-band parameter
//! sets arrive as `sprop-vps` / `sprop-sps` / `sprop-pps`. PACI (type 50) is not produced
//! by the encoders Kite meets and marks the AU damaged.

use super::h264::{base64_decode, fmtp_param};
use super::rtp::RtpPacket;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

fn nal_type(b0: u8) -> u8 {
    (b0 >> 1) & 0x3F
}

/// True when the Annex-B buffer contains a NAL of `ty` (4-byte start codes only).
fn contains_nal_type(annexb: &[u8], ty: u8) -> bool {
    annexb
        .windows(5)
        .any(|w| w[..4] == START_CODE && nal_type(w[4]) == ty)
}

/// IRAP pictures (BLA/IDR/CRA, types 16..=21) — where a decoder can (re)start.
fn is_irap(ty: u8) -> bool {
    (16..=21).contains(&ty)
}

pub struct H265Depacketizer {
    au: Vec<u8>,
    au_timestamp: u32,
    au_open: bool,
    au_damaged: bool,
    /// The AU holds at least one VCL NAL (a slice, type < 32) — the marker bit only
    /// closes an AU that actually carries picture data. Some servers (obs-rtspserver
    /// measured) set the marker on prefix packets too (AUD/SEI), which used to split
    /// every frame into two "AUs"; the timestamp change stays as the boundary backstop.
    au_has_vcl: bool,
    frag: Vec<u8>,
    frag_open: bool,
    last_seq: Option<u16>,
    /// VPS+SPS+PPS from the sprop-* attributes, Annex-B framed. Empty when absent.
    param_sets: Vec<u8>,
    pub aus: u64,
    pub damaged_aus: u64,
}

impl H265Depacketizer {
    pub fn new(fmtp: Option<&str>) -> Self {
        let mut param_sets = Vec::new();
        if let Some(f) = fmtp {
            for key in ["sprop-vps", "sprop-sps", "sprop-pps"] {
                if let Some(value) = fmtp_param(f, key) {
                    for part in value.split(',') {
                        if let Some(nal) = base64_decode(part).filter(|n| !n.is_empty()) {
                            param_sets.extend_from_slice(&START_CODE);
                            param_sets.extend_from_slice(&nal);
                        }
                    }
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

    /// Feed one (in-order) RTP packet; returns zero, one, or two complete access units.
    pub fn push(&mut self, pkt: &RtpPacket) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let p = &pkt.payload;
        if p.len() < 2 {
            return out;
        }

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

        if self.au_open && pkt.timestamp != self.au_timestamp {
            self.flush_au(&mut out);
        }
        if !self.au_open {
            self.au_timestamp = pkt.timestamp;
            self.au_open = true;
        }

        match nal_type(p[0]) {
            0..=47 => self.append_nal(p),
            48 => {
                // AP: payload header, then [u16 len][NAL] repeated.
                let mut i = 2usize;
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
            49 => {
                // FU: payload header (2) + FU header (S|E|FuType) + fragment.
                if p.len() < 3 {
                    self.au_damaged = true;
                    return out;
                }
                let start = p[2] & 0x80 != 0;
                let end = p[2] & 0x40 != 0;
                let fu_type = p[2] & 0x3F;
                if start {
                    self.frag.clear();
                    self.frag.push((p[0] & 0x81) | (fu_type << 1));
                    self.frag.push(p[1]);
                    self.frag_open = true;
                }
                if self.frag_open {
                    self.frag.extend_from_slice(&p[3..]);
                    if end {
                        let nal = std::mem::take(&mut self.frag);
                        self.frag_open = false;
                        self.append_nal(&nal);
                    }
                } else {
                    self.au_damaged = true;
                }
            }
            _ => {
                self.au_damaged = true; // PACI / reserved
            }
        }

        if pkt.marker && self.au_has_vcl {
            self.flush_au(&mut out);
        }
        out
    }

    fn append_nal(&mut self, nal: &[u8]) {
        if nal.len() < 2 {
            return;
        }
        if nal_type(nal[0]) < 32 {
            self.au_has_vcl = true;
        }
        self.au.extend_from_slice(&START_CODE);
        self.au.extend_from_slice(nal);
    }

    fn flush_au(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.frag_open {
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
            return;
        }
        let has_irap = au
            .windows(5)
            .any(|w| w[..4] == START_CODE && is_irap(nal_type(w[4])));
        let has_sps = contains_nal_type(&au, 33);
        let mut emit = Vec::with_capacity(self.param_sets.len() + au.len());
        if has_irap && !has_sps && !self.param_sets.is_empty() {
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

    /// TRAIL_R slice (type 1): header [0x02, 0x01].
    fn trail(data: &[u8]) -> Vec<u8> {
        let mut n = vec![0x02, 0x01];
        n.extend_from_slice(data);
        n
    }

    #[test]
    fn marker_on_a_prefix_packet_does_not_split_the_frame() {
        // obs-rtspserver sets the marker on prefix packets (AUD/SEI) too — the AU must
        // stay open until picture data (a VCL NAL) arrived, else every frame splits in two.
        let mut d = H265Depacketizer::new(None);
        let aud = vec![35u8 << 1, 0x01, 0x50]; // AUD (type 35)
        assert!(d.push(&pkt(1, 90_000, true, aud.clone())).is_empty());
        let slice = trail(&[9, 9, 9]);
        assert_eq!(
            d.push(&pkt(2, 90_000, true, slice.clone())),
            vec![annexb(&[&aud, &slice])]
        );
        assert_eq!(d.aus, 1);
    }

    #[test]
    fn single_nal_and_fu_round_trip() {
        let mut d = H265Depacketizer::new(None);
        let nal = trail(&[1, 2, 3]);
        assert_eq!(d.push(&pkt(1, 10, true, nal.clone())), vec![annexb(&[&nal])]);

        // Fragment a big NAL (IDR_W_RADL, type 19 → header [0x26, 0x01]).
        let mut idr = vec![0x26u8, 0x01];
        idr.extend(0..60u8);
        let body = &idr[2..];
        let fu_ind = [(idr[0] & 0x81) | (49 << 1), idr[1]];
        let fu = |s: bool, e: bool, chunk: &[u8]| {
            let mut p = fu_ind.to_vec();
            p.push((u8::from(s) << 7) | (u8::from(e) << 6) | 19);
            p.extend_from_slice(chunk);
            p
        };
        assert!(d.push(&pkt(2, 20, false, fu(true, false, &body[..30]))).is_empty());
        let aus = d.push(&pkt(3, 20, true, fu(false, true, &body[30..])));
        assert_eq!(aus, vec![annexb(&[&idr])]);
        assert_eq!(d.aus, 2);
    }

    #[test]
    fn ap_unpacks_and_sprop_prepends_before_irap() {
        // VPS(32)=[0x40,1,..], SPS(33)=[0x42,1,..], PPS(34)=[0x44,1,..] as sprop:
        // [0x40,0x01] = "QAE=", [0x42,0x01] = "QgE=", [0x44,0x01] = "RAE="
        let fmtp = "sprop-vps=QAE=;sprop-sps=QgE=;sprop-pps=RAE=";
        let mut d = H265Depacketizer::new(Some(fmtp));

        // IDR (type 19) alone → parameter sets prepended.
        let idr = vec![0x26u8, 0x01, 0xAA];
        let aus = d.push(&pkt(1, 5, true, idr.clone()));
        assert_eq!(
            aus,
            vec![annexb(&[&[0x40, 0x01], &[0x42, 0x01], &[0x44, 0x01], &idr])]
        );

        // AP carrying SPS + IDR in-band → nothing prepended twice.
        let sps = [0x42u8, 0x01, 0x0F];
        let idr2 = [0x26u8, 0x01, 0xBB];
        let mut ap = vec![48u8 << 1, 0x01];
        for n in [&sps[..], &idr2[..]] {
            ap.extend_from_slice(&(n.len() as u16).to_be_bytes());
            ap.extend_from_slice(n);
        }
        let aus = d.push(&pkt(2, 6, true, ap));
        assert_eq!(aus, vec![annexb(&[&sps, &idr2])]);
    }

    #[test]
    fn a_gap_drops_the_torn_access_unit() {
        let mut d = H265Depacketizer::new(None);
        assert!(d.push(&pkt(1, 5, false, trail(&[1]))).is_empty());
        let aus = d.push(&pkt(3, 5, true, trail(&[2]))); // seq 2 lost
        assert!(aus.is_empty());
        assert_eq!(d.damaged_aus, 1);
        let aus = d.push(&pkt(4, 6, true, trail(&[3])));
        assert_eq!(aus.len(), 1);
    }
}
