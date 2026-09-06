// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! H.265 SPS conformance window: probe it, and strip it. An encoder that pads the picture
//! to its block size (NVENC 720p → coded 1280×736, every 1080p → 1088) declares the visible
//! part as a window; the Pi 5's V4L2 decoder implements that crop as a CPU copy the GL path
//! cannot take (see `linux_sink`). Without the window the decoder hands the full coded frame
//! out zero-copy and the sink hides the padding under the WebView instead.

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const NAL_SPS: u8 = 33;

/// Conformance window of a sequence, in luma pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub coded_w: u32,
    pub coded_h: u32,
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Window {
    pub fn display_w(&self) -> u32 {
        self.coded_w.saturating_sub(self.left + self.right)
    }
    pub fn display_h(&self) -> u32 {
        self.coded_h.saturating_sub(self.top + self.bottom)
    }
}

/// What an access unit's SPS says about cropping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpsProbe {
    /// No SPS in this AU.
    NoSps,
    /// An SPS is there but did not parse — treat the stream as unknown.
    Unparsed,
    /// The coded picture is the picture.
    NoWindow,
    /// The SPS crops the coded picture.
    Window(Window),
}

/// Probe the first SPS of an Annex-B access unit.
pub fn probe_au(annexb: &[u8]) -> SpsProbe {
    let Some((start, end)) = find_sps(annexb) else { return SpsProbe::NoSps };
    match parse_sps(&annexb[start..end]) {
        Some(p) if p.window_flag_bit.is_some() => match p.window {
            Some(w) if w.left | w.top | w.right | w.bottom != 0 => SpsProbe::Window(w),
            _ => SpsProbe::NoWindow,
        },
        Some(_) => SpsProbe::NoWindow,
        None => SpsProbe::Unparsed,
    }
}

/// The AU with every SPS rewritten to declare no conformance window (the coded size becomes
/// the picture size). `None` when the AU carries no SPS — push it unchanged.
pub fn strip_window(annexb: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(annexb.len());
    let mut pos = 0;
    let mut touched = false;
    while let Some((start, end)) = find_sps(&annexb[pos..]) {
        let (start, end) = (pos + start, pos + end);
        out.extend_from_slice(&annexb[pos..start]);
        match rewrite_sps(&annexb[start..end]) {
            Some(nal) => out.extend_from_slice(&nal),
            None => out.extend_from_slice(&annexb[start..end]),
        }
        touched = true;
        pos = end;
    }
    if !touched {
        return None;
    }
    out.extend_from_slice(&annexb[pos..]);
    Some(out)
}

/// Byte range of the first SPS NAL (header included, start code excluded).
fn find_sps(annexb: &[u8]) -> Option<(usize, usize)> {
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
    Some((start, start + end))
}

struct ParsedSps {
    rbsp: Vec<u8>,
    /// Bit offset of `conformance_window_flag`, if the header parsed that far.
    window_flag_bit: Option<usize>,
    /// Bit offset right after the four window offsets (or after a zero flag).
    after_window_bit: usize,
    window: Option<Window>,
}

/// `nal` = the SPS NAL including its two header bytes.
fn parse_sps(nal: &[u8]) -> Option<ParsedSps> {
    let rbsp = strip_emulation(nal.get(2..)?);
    let mut r = BitReader::new(&rbsp);
    r.bits(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.bits(3)? as usize;
    r.bits(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(&mut r, max_sub_layers_minus1)?;
    r.ue()?; // sps_seq_parameter_set_id
    let chroma_format_idc = r.ue()?;
    if chroma_format_idc == 3 {
        r.bits(1)?; // separate_colour_plane_flag
    }
    let coded_w = r.ue()?;
    let coded_h = r.ue()?;
    let window_flag_bit = r.pos;
    let mut window = None;
    if r.bits(1)? == 1 {
        let (sub_w, sub_h) = match chroma_format_idc {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        let (left, right, top, bottom) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        window = Some(Window {
            coded_w,
            coded_h,
            left: left * sub_w,
            right: right * sub_w,
            top: top * sub_h,
            bottom: bottom * sub_h,
        });
    }
    let after_window_bit = r.pos;
    Some(ParsedSps { rbsp, window_flag_bit: Some(window_flag_bit), after_window_bit, window })
}

/// The SPS NAL with `conformance_window_flag = 0` and the offsets removed; `None` when it
/// has no window or does not parse.
fn rewrite_sps(nal: &[u8]) -> Option<Vec<u8>> {
    let p = parse_sps(nal)?;
    p.window?;
    let flag = p.window_flag_bit?;
    let total = p.rbsp.len() * 8;
    // rbsp_trailing_bits: the stop bit is the last 1 in the payload.
    let stop = (0..total).rev().find(|&i| bit(&p.rbsp, i) == 1)?;
    if p.after_window_bit > stop {
        return None;
    }
    let mut bits: Vec<u8> = (0..flag).map(|i| bit(&p.rbsp, i)).collect();
    bits.push(0);
    bits.extend((p.after_window_bit..=stop).map(|i| bit(&p.rbsp, i)));
    while bits.len() % 8 != 0 {
        bits.push(0);
    }
    let body: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |acc, &b| (acc << 1) | b)).collect();
    let mut out = nal[..2].to_vec();
    out.extend(add_emulation(&body));
    Some(out)
}

fn bit(data: &[u8], i: usize) -> u8 {
    (data[i / 8] >> (7 - i % 8)) & 1
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

/// Emulation-prevention insertion: `00 00 {00..03}` → `00 00 03 {..}`.
fn add_emulation(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    let mut zeros = 0usize;
    for &b in data {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
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

    // OBS (NVENC) 1280×720: coded 1280×736, window bottom 16.
    const OBS_SPS: &str = "420101016000000300900000030000030078a00280802e1f1396554a4211917543016a02020208000003000800000301e3002ef2880006acfc0000989682";
    // The same SPS without the window (independent Python bit surgery, ffprobe-verified 1280×736).
    const OBS_SPS_STRIPPED: &str = "420101016000000300900000030000030078a00280802e16595529084645d50c05a80808082000000300200000078c00bbca20001ab3f00002625a08";
    // x265 1280×720 — coded size equals the picture, no window.
    const X265_720_SPS: &str = "420101016000000300900000030000030078a00280802d165ba4a4c2f0168080000003008000001e04";
    // x265 1280×730 — coded 736, window bottom 6.
    const X265_730_SPS: &str = "420101016000000300900000030000030078a00280802e1f265ba4a4c2f0168080000003008000001e04";
    const X265_730_SPS_STRIPPED: &str = "420101016000000300900000030000030078a00280802e165ba4a4c2f0168080000003008000001e04";
    // OBS/NVENC 1280×720 with B-frames off (2026-09-05 evening stream), Python reference.
    const OBS2_SPS: &str = "420101016000000300900000030000030078a00280802e1f13965d29084645d50c05a80808082000000300200000078c00bbca20001ab3f00002625a08";
    const OBS2_SPS_STRIPPED: &str = "420101016000000300900000030000030078a00280802e165974a4211917543016a020202080000003008000001e3002ef2880006acfc00009896820";

    #[test]
    fn real_encoders() {
        let obs = Window { coded_w: 1280, coded_h: 736, left: 0, top: 0, right: 0, bottom: 16 };
        assert_eq!(probe_au(&au_with(&hex(OBS_SPS))), SpsProbe::Window(obs));
        assert_eq!((obs.display_w(), obs.display_h()), (1280, 720));
        assert_eq!(probe_au(&au_with(&hex(X265_720_SPS))), SpsProbe::NoWindow);
        assert_eq!(
            probe_au(&au_with(&hex(X265_730_SPS))),
            SpsProbe::Window(Window { coded_w: 1280, coded_h: 736, left: 0, top: 0, right: 0, bottom: 6 })
        );
    }

    #[test]
    fn strip_matches_the_reference_bytes_and_reprobes_as_no_window() {
        for (sps, stripped) in [
            (OBS_SPS, OBS_SPS_STRIPPED),
            (OBS2_SPS, OBS2_SPS_STRIPPED),
            (X265_730_SPS, X265_730_SPS_STRIPPED),
        ] {
            let au = strip_window(&au_with(&hex(sps))).expect("has an SPS");
            assert_eq!(au, au_with(&hex(stripped)));
            assert_eq!(probe_au(&au), SpsProbe::NoWindow);
        }
        // No window → bytes untouched; no SPS → None.
        let plain = au_with(&hex(X265_720_SPS));
        assert_eq!(strip_window(&plain), Some(plain.clone()));
        assert_eq!(strip_window(&[0, 0, 0, 1, 0x26, 0x01, 0xaf]), None);
    }

    #[test]
    fn no_sps_or_truncated() {
        assert_eq!(probe_au(&[0, 0, 0, 1, 0x26, 0x01, 0xaf]), SpsProbe::NoSps);
        let mut short = hex(OBS_SPS);
        short.truncate(14); // ends inside profile_tier_level
        assert_eq!(probe_au(&au_with(&short)), SpsProbe::Unparsed);
    }

    #[test]
    fn a_window_with_zero_offsets_is_no_window() {
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
        assert_eq!(probe_au(&au_with(&w.emulated())), SpsProbe::NoWindow);
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
        fn emulated(&self) -> Vec<u8> {
            let raw: Vec<u8> = self
                .bits
                .chunks(8)
                .map(|c| c.iter().enumerate().fold(0u8, |acc, (i, b)| acc | ((*b as u8) << (7 - i))))
                .collect();
            add_emulation(&raw)
        }
    }
}
