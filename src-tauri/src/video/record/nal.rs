// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! What the Matroska track header needs from an H.264 / H.265 stream: the NAL units of an access unit,
//! the picture size, and the decoder configuration record (`avcC` / `hvcC`, ISO/IEC 14496-15).

use crate::video::rtsp::{profile_tier_level, strip_emulation, BitReader, VideoCodec};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// The NAL units of an Annex-B access unit. The depacketizers write 4-byte start codes only.
pub fn split_nals(annexb: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<usize> = annexb
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == START_CODE)
        .map(|(i, _)| i)
        .collect();
    starts.push(annexb.len());
    starts
        .windows(2)
        .map(|w| &annexb[w[0] + 4..w[1]])
        .filter(|n| !n.is_empty())
        .collect()
}

fn nal_type(codec: VideoCodec, nal: &[u8]) -> u8 {
    match codec {
        VideoCodec::H265 => (nal[0] >> 1) & 0x3F,
        _ => nal[0] & 0x1F,
    }
}

/// A picture a decoder can start at: H.264 IDR, H.265 IRAP (BLA / IDR / CRA).
pub fn is_keyframe(codec: VideoCodec, nals: &[&[u8]]) -> bool {
    nals.iter().any(|n| match codec {
        VideoCodec::H265 => (16..=21).contains(&nal_type(codec, n)),
        _ => nal_type(codec, n) == 5,
    })
}

/// Picture size and decoder configuration of a stream, taken from the parameter sets of a keyframe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    pub width: u32,
    pub height: u32,
    /// `avcC` / `hvcC` — Matroska's CodecPrivate.
    pub codec_private: Vec<u8>,
}

/// `None` while the access unit carries no complete set of parameter sets, or they do not parse.
pub fn stream_config(codec: VideoCodec, nals: &[&[u8]]) -> Option<StreamConfig> {
    let of = |ty: u8| nals.iter().copied().find(|n| nal_type(codec, n) == ty);
    match codec {
        VideoCodec::H264 => {
            let (sps, pps) = (of(7)?, of(8)?);
            let (width, height) = h264_size(sps)?;
            let mut c = vec![1, *sps.get(1)?, *sps.get(2)?, *sps.get(3)?, 0xFF, 0xE1];
            c.extend((sps.len() as u16).to_be_bytes());
            c.extend(sps);
            c.push(1);
            c.extend((pps.len() as u16).to_be_bytes());
            c.extend(pps);
            Some(StreamConfig { width, height, codec_private: c })
        }
        VideoCodec::H265 => {
            let (vps, sps, pps) = (of(32)?, of(33)?, of(34)?);
            let s = h265_sps(sps)?;
            let mut c = vec![1];
            c.extend(s.general_ptl); // profile space / tier / profile, compatibility, constraints, level
            c.extend([0xF0, 0x00, 0xFC]); // min_spatial_segmentation 0, parallelism unknown
            c.push(0xFC | s.chroma_format_idc as u8);
            c.push(0xF8 | s.bit_depth_luma_minus8 as u8);
            c.push(0xF8 | s.bit_depth_chroma_minus8 as u8);
            c.extend([0, 0]); // avgFrameRate unknown
            // constantFrameRate 0, numTemporalLayers, temporalIdNested, 4-byte NAL lengths
            c.push(((s.max_sub_layers_minus1 as u8 + 1) << 3) | ((s.temporal_id_nesting as u8) << 2) | 3);
            c.push(3);
            for (ty, nal) in [(32u8, vps), (33, sps), (34, pps)] {
                c.push(0x80 | ty); // array_completeness: these are all the sets there are
                c.extend(1u16.to_be_bytes());
                c.extend((nal.len() as u16).to_be_bytes());
                c.extend(nal);
            }
            Some(StreamConfig { width: s.width, height: s.height, codec_private: c })
        }
        VideoCodec::Mjpeg => None,
    }
}

/// Exp-Golomb `se(v)`.
fn se(r: &mut BitReader) -> Option<i32> {
    let k = r.ue()?;
    Some(if k & 1 == 1 { (k / 2 + 1) as i32 } else { -((k / 2) as i32) })
}

/// Displayed size from an H.264 SPS NAL (header byte included).
fn h264_size(sps: &[u8]) -> Option<(u32, u32)> {
    let rbsp = strip_emulation(sps.get(1..)?);
    let mut r = BitReader::new(&rbsp);
    let profile = r.bits(8)?;
    r.skip(16)?; // constraint flags, level
    r.ue()?; // seq_parameter_set_id
    let mut chroma = 1;
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        chroma = r.ue()?;
        if chroma == 3 {
            r.skip(1)?; // separate_colour_plane_flag
        }
        r.ue()?; // bit_depth_luma_minus8
        r.ue()?; // bit_depth_chroma_minus8
        r.skip(1)?; // qpprime_y_zero_transform_bypass_flag
        if r.bits(1)? == 1 {
            for i in 0..if chroma == 3 { 12 } else { 8 } {
                if r.bits(1)? == 0 {
                    continue;
                }
                let (mut last, mut next) = (8i32, 8i32);
                for _ in 0..if i < 6 { 16 } else { 64 } {
                    if next != 0 {
                        next = (last + se(&mut r)? + 256) % 256;
                    }
                    if next != 0 {
                        last = next;
                    }
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => {
            r.ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            r.skip(1)?; // delta_pic_order_always_zero_flag
            se(&mut r)?;
            se(&mut r)?;
            for _ in 0..r.ue()?.min(255) {
                se(&mut r)?;
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.skip(1)?; // gaps_in_frame_num_value_allowed_flag
    let width_mbs = r.ue()? + 1;
    let height_units = r.ue()? + 1;
    let frame_mbs_only = r.bits(1)?;
    if frame_mbs_only == 0 {
        r.skip(1)?; // mb_adaptive_frame_field_flag
    }
    r.skip(1)?; // direct_8x8_inference_flag
    let (mut crop_x, mut crop_y) = (0, 0);
    if r.bits(1)? == 1 {
        let (left, right, top, bottom) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        let (unit_x, unit_y) = match chroma {
            0 => (1, 1),
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        crop_x = (left + right).checked_mul(unit_x)?;
        crop_y = (top + bottom).checked_mul(unit_y * (2 - frame_mbs_only))?;
    }
    let width = (width_mbs.checked_mul(16)?).checked_sub(crop_x)?;
    let height = (height_units.checked_mul(16 * (2 - frame_mbs_only))?).checked_sub(crop_y)?;
    (width > 0 && height > 0).then_some((width, height))
}

struct H265Sps {
    width: u32,
    height: u32,
    general_ptl: [u8; 12],
    max_sub_layers_minus1: u32,
    temporal_id_nesting: u32,
    chroma_format_idc: u32,
    bit_depth_luma_minus8: u32,
    bit_depth_chroma_minus8: u32,
}

/// `sps` = the SPS NAL including its two header bytes.
fn h265_sps(sps: &[u8]) -> Option<H265Sps> {
    let rbsp = strip_emulation(sps.get(2..)?);
    let general_ptl: [u8; 12] = rbsp.get(1..13)?.try_into().ok()?;
    let mut r = BitReader::new(&rbsp);
    r.skip(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.bits(3)?;
    let temporal_id_nesting = r.bits(1)?;
    profile_tier_level(&mut r, max_sub_layers_minus1 as usize)?;
    r.ue()?; // sps_seq_parameter_set_id
    let chroma_format_idc = r.ue()?;
    if chroma_format_idc == 3 {
        r.skip(1)?; // separate_colour_plane_flag
    }
    let (coded_w, coded_h) = (r.ue()?, r.ue()?);
    let (mut crop_x, mut crop_y) = (0, 0);
    if r.bits(1)? == 1 {
        let (left, right, top, bottom) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        let (unit_x, unit_y) = match chroma_format_idc {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        crop_x = (left + right).checked_mul(unit_x)?;
        crop_y = (top + bottom).checked_mul(unit_y)?;
    }
    let bit_depth_luma_minus8 = r.ue()?;
    let bit_depth_chroma_minus8 = r.ue()?;
    let (width, height) = (coded_w.checked_sub(crop_x)?, coded_h.checked_sub(crop_y)?);
    if width == 0 || height == 0 || chroma_format_idc > 3 || bit_depth_luma_minus8 > 7 || bit_depth_chroma_minus8 > 7 {
        return None;
    }
    Some(H265Sps {
        width,
        height,
        general_ptl,
        max_sub_layers_minus1,
        temporal_id_nesting,
        chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|n| [&START_CODE[..], n].concat()).collect()
    }

    // x264 Baseline 1920x1080 (coded 1088, cropped), x264 Main 854x480 (coded 864, cropped), x265 Main 1280x720.
    const H264_SPS_1080: &str = "6742c028da01e0089f970110000003001000000303c0f1832a";
    const H264_SPS_480: &str = "674d401feca06c1ef3780880000003008000001e078c18cb";
    const H264_PPS: &str = "68ce0fc8";
    const H265_VPS: &str = "40010c01ffff01600000030090000003000003005d959009";
    const H265_SPS_720: &str = "42010101600000030090000003000003005da00280802d16595912930bc05a020000030002000003003c10";
    const H265_PPS: &str = "4401c073c189";

    #[test]
    fn splits_an_access_unit() {
        let au = annexb(&[&[0x67, 1, 2], &[0x68, 3], &[0x65, 4, 5, 6]]);
        let nals = split_nals(&au);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[2], &[0x65, 4, 5, 6]);
        assert!(is_keyframe(VideoCodec::H264, &nals));
        assert!(!is_keyframe(VideoCodec::H264, &nals[..2]));
    }

    #[test]
    fn h264_config_carries_size_and_sets() {
        let (sps, pps) = (hex(H264_SPS_1080), hex(H264_PPS));
        let c = stream_config(VideoCodec::H264, &[&sps, &pps, &[0x65, 0]]).unwrap();
        assert_eq!((c.width, c.height), (1920, 1080));
        assert_eq!(&c.codec_private[..6], &[1, 0x42, 0xc0, 0x28, 0xFF, 0xE1]);
        assert_eq!(c.codec_private.len(), 6 + 2 + sps.len() + 1 + 2 + pps.len());
        assert!(stream_config(VideoCodec::H264, &[&sps, &[0x65, 0]]).is_none());
        assert_eq!(h264_size(&hex(H264_SPS_480)), Some((854, 480)));
    }

    #[test]
    fn h265_config_carries_size_profile_and_sets() {
        let (vps, sps, pps) = (hex(H265_VPS), hex(H265_SPS_720), hex(H265_PPS));
        let c = stream_config(VideoCodec::H265, &[&vps, &sps, &pps]).unwrap();
        assert_eq!((c.width, c.height), (1280, 720));
        // version, Main profile, level 93 (3.1), 4:2:0, 8 bit, three arrays
        assert_eq!(c.codec_private[0], 1);
        assert_eq!(c.codec_private[1], 0x01);
        assert_eq!(c.codec_private[12], 93);
        assert_eq!(&c.codec_private[16..19], &[0xFD, 0xF8, 0xF8]);
        assert_eq!(c.codec_private[22], 3);
        assert!(is_keyframe(VideoCodec::H265, &[&[19 << 1, 1]]));
        assert!(!is_keyframe(VideoCodec::H265, &[&[1 << 1, 1]]));
    }
}
