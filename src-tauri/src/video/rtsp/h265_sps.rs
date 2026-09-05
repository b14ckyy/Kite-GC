// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! H.265 SPS probe: does the sequence carry a conformance-window crop? (Coded 1280×736
//! shown as 1280×720, every 1080p stream.) The Linux sink needs to know before it builds
//! the pipeline — see `linux_sink` for why.

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const NAL_SPS: u8 = 33;

/// `Some(true)` when the SPS in this Annex-B access unit crops the coded picture,
/// `Some(false)` when it does not, `None` without a parseable SPS.
pub fn au_needs_crop(annexb: &[u8]) -> Option<bool> {
    let start = annexb
        .windows(5)
        .position(|w| w[..4] == START_CODE && (w[4] >> 1) & 0x3F == NAL_SPS)?
        + 4;
    let rest = &annexb[start..];
    let end = rest
        .windows(3)
        .position(|w| w == [0, 0, 1])
        .map(|p| p.saturating_sub(1))
        .unwrap_or(rest.len());
    sps_needs_crop(&rest[..end])
}

/// `nal` = the SPS NAL including its two header bytes.
fn sps_needs_crop(nal: &[u8]) -> Option<bool> {
    let rbsp = strip_emulation(nal.get(2..)?);
    let mut r = BitReader::new(&rbsp);
    r.bits(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.bits(3)? as usize;
    r.bits(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(&mut r, max_sub_layers_minus1)?;
    r.ue()?; // sps_seq_parameter_set_id
    if r.ue()? == 3 {
        r.bits(1)?; // separate_colour_plane_flag
    }
    r.ue()?; // pic_width_in_luma_samples
    r.ue()?; // pic_height_in_luma_samples
    if r.bits(1)? == 0 {
        return Some(false);
    }
    let offsets = [r.ue()?, r.ue()?, r.ue()?, r.ue()?];
    Some(offsets.iter().any(|&o| o != 0))
}

fn profile_tier_level(r: &mut BitReader, max_sub_layers_minus1: usize) -> Option<()> {
    r.skip(88 + 8)?; // general profile + general_level_idc
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for i in 0..max_sub_layers_minus1 {
        profile_present[i] = r.bits(1)? == 1;
        level_present[i] = r.bits(1)? == 1;
    }
    if max_sub_layers_minus1 > 0 {
        r.skip(2 * (8 - max_sub_layers_minus1))?; // reserved_zero_2bits
    }
    for i in 0..max_sub_layers_minus1 {
        if profile_present[i] {
            r.skip(88)?;
        }
        if level_present[i] {
            r.skip(8)?;
        }
    }
    Some(())
}

/// Emulation-prevention removal: `00 00 03` → `00 00`.
fn strip_emulation(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0usize;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        if self.pos + n > self.data.len() * 8 {
            return None;
        }
        self.pos += n;
        Some(())
    }

    /// Exp-Golomb `ue(v)`.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn au_with(sps: &[u8]) -> Vec<u8> {
        let mut au = vec![0, 0, 0, 1, 0x40, 0x01, 0x0c]; // a VPS first
        au.extend_from_slice(&START_CODE);
        au.extend_from_slice(sps);
        au.extend_from_slice(&[0, 0, 0, 1, 0x26, 0x01, 0xaf]); // then an IDR
        au
    }

    // OBS (NVENC) 1280×720: coded 1280×736, conformance window bottom offset.
    const OBS_SPS: &str = "420101016000000300900000030000030078a00280802e1f1396554a4211917543016a02020208000003000800000301e3002ef2880006acfc0000989682";
    // x265 1280×720 — coded size equals the picture, no window.
    const X265_720_SPS: &str = "420101016000000300900000030000030078a00280802d165ba4a4c2f0168080000003008000001e04";
    // x265 1280×730 — coded 736, window crops 6 rows.
    const X265_730_SPS: &str = "420101016000000300900000030000030078a00280802e1f265ba4a4c2f0168080000003008000001e04";

    #[test]
    fn real_encoders() {
        assert_eq!(au_needs_crop(&au_with(&hex(OBS_SPS))), Some(true));
        assert_eq!(au_needs_crop(&au_with(&hex(X265_720_SPS))), Some(false));
        assert_eq!(au_needs_crop(&au_with(&hex(X265_730_SPS))), Some(true));
    }

    #[test]
    fn no_sps_or_truncated_is_unknown() {
        assert_eq!(au_needs_crop(&[0, 0, 0, 1, 0x26, 0x01, 0xaf]), None);
        let mut short = hex(OBS_SPS);
        short.truncate(14); // ends inside profile_tier_level
        assert_eq!(au_needs_crop(&au_with(&short)), None);
    }

    #[test]
    fn a_window_with_zero_offsets_is_no_crop() {
        // Hand-built SPS, one sub-layer: window flag set, all four offsets zero.
        let mut w = BitWriter::default();
        w.bits(0x42 << 8 | 0x01, 16); // NAL header
        w.bits(0, 4);
        w.bits(0, 3);
        w.bits(1, 1);
        w.bits(0, 88 + 8);
        w.ue(0);
        w.ue(1);
        w.ue(1280);
        w.ue(720);
        w.bits(1, 1);
        for _ in 0..4 {
            w.ue(0);
        }
        w.bits(1, 1); // rbsp trailing
        assert_eq!(au_needs_crop(&au_with(&w.emulated())), Some(false));
    }

    #[derive(Default)]
    struct BitWriter {
        bits: Vec<bool>,
    }

    impl BitWriter {
        fn bits(&mut self, v: u32, n: usize) {
            for i in (0..n).rev() {
                self.bits.push(i < 32 && (v >> i) & 1 == 1);
            }
        }
        fn ue(&mut self, v: u32) {
            let x = v + 1;
            let len = 32 - x.leading_zeros() as usize;
            self.bits(0, len - 1);
            self.bits(x, len);
        }
        /// Bytes with emulation-prevention inserted, as an encoder would.
        fn emulated(&self) -> Vec<u8> {
            let mut raw = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut b = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    b |= (*bit as u8) << (7 - i);
                }
                raw.push(b);
            }
            let mut out = Vec::new();
            let mut zeros = 0;
            for b in raw {
                if zeros >= 2 && b <= 3 {
                    out.push(3);
                    zeros = 0;
                }
                out.push(b);
                zeros = if b == 0 { zeros + 1 } else { 0 };
            }
            out
        }
    }
}
