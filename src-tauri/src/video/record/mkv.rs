// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Matroska writer for ONE H.264 / H.265 video track, written as the stream arrives.
//! - A file that is never finished (crash, power loss) plays up to the last cluster: segment and open
//!   cluster carry the "unknown size" marker until they are closed.
//! - `finish` adds what a player needs to seek fast: the cue index, the seek head and the duration.
//! - Blocks carry presentation times only, which is all RTP delivers; Matroska needs no decode times.

use std::io::{self, Seek, SeekFrom, Write};

use super::nal::StreamConfig;
use crate::video::rtsp::VideoCodec;

const ID_EBML: &[u8] = &[0x1A, 0x45, 0xDF, 0xA3];
const ID_SEGMENT: &[u8] = &[0x18, 0x53, 0x80, 0x67];
const ID_SEEK_HEAD: &[u8] = &[0x11, 0x4D, 0x9B, 0x74];
const ID_INFO: &[u8] = &[0x15, 0x49, 0xA9, 0x66];
const ID_TRACKS: &[u8] = &[0x16, 0x54, 0xAE, 0x6B];
const ID_CLUSTER: &[u8] = &[0x1F, 0x43, 0xB6, 0x75];
const ID_CUES: &[u8] = &[0x1C, 0x53, 0xBB, 0x6B];

/// "Size unknown" in 8 bytes — the reserved all-ones value.
const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
/// Seek head with three entries of fixed width; a Void of this size holds its place until `finish`.
const SEEK_HEAD_LEN: u64 = 4 + 1 + 3 * 21;
/// A block's time is a signed 16-bit offset from its cluster's, in ms.
const CLUSTER_MAX_MS: i64 = 30_000;
/// 2001-01-01T00:00:00Z, the epoch of Matroska's DateUTC, in Unix ms.
const MKV_EPOCH_UNIX_MS: i64 = 978_307_200_000;

/// An 8-byte size field: patchable in place whatever the final value.
fn size8(n: u64) -> [u8; 8] {
    let mut b = n.to_be_bytes();
    b[0] = 0x01;
    b
}

/// The shortest size field for `n`.
fn size(n: u64) -> Vec<u8> {
    for len in 1..=8u32 {
        if n < (1u64 << (7 * len)) - 1 {
            let v = n | (1u64 << (7 * len));
            return v.to_be_bytes()[8 - len as usize..].to_vec();
        }
    }
    size8(n).to_vec()
}

fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
    [id, &size(payload.len() as u64), payload].concat()
}

fn uint(id: &[u8], v: u64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().take(7).take_while(|b| **b == 0).count();
    element(id, &bytes[skip..])
}

/// One video track into `W`. Times are milliseconds from the first frame.
pub struct MkvWriter<W: Write + Seek> {
    out: W,
    /// Write position; kept here so a patch can return to it without asking the stream.
    pos: u64,
    segment_start: u64,
    seek_head_at: u64,
    info_at: u64,
    tracks_at: u64,
    duration_at: u64,
    /// Start of the open cluster's size field and its timestamp.
    cluster: Option<(u64, i64)>,
    /// (time, cluster position relative to the segment) of every cluster that starts with a keyframe.
    cues: Vec<(i64, u64)>,
    frames: u64,
    first_ms: Option<i64>,
    last_ms: i64,
}

impl<W: Write + Seek> MkvWriter<W> {
    pub fn create(mut out: W, codec: VideoCodec, cfg: &StreamConfig, start_unix_ms: i64, uid: [u8; 16]) -> io::Result<Self> {
        let codec_id: &[u8] = match codec {
            VideoCodec::H265 => b"V_MPEGH/ISO/HEVC",
            _ => b"V_MPEG4/ISO/AVC",
        };
        let header = element(
            ID_EBML,
            &[
                uint(&[0x42, 0x86], 1),
                uint(&[0x42, 0xF7], 1),
                uint(&[0x42, 0xF2], 4),
                uint(&[0x42, 0xF3], 8),
                element(&[0x42, 0x82], b"matroska"),
                uint(&[0x42, 0x87], 4),
                uint(&[0x42, 0x85], 2),
            ]
            .concat(),
        );
        let app = b"Kite GC";
        let date_ns = (start_unix_ms - MKV_EPOCH_UNIX_MS).saturating_mul(1_000_000);
        let info_head = [
            uint(&[0x2A, 0xD7, 0xB1], 1_000_000), // TimestampScale: 1 ms
            element(&[0x4D, 0x80], app),
            element(&[0x57, 0x41], app),
            element(&[0x73, 0xA4], &uid),
            element(&[0x44, 0x61], &date_ns.to_be_bytes()),
        ]
        .concat();
        let duration = element(&[0x44, 0x89], &0f64.to_be_bytes());
        let info = element(ID_INFO, &[&info_head[..], &duration].concat());
        let track_uid = u64::from_be_bytes(uid[..8].try_into().unwrap()) | 1;
        let tracks = element(
            ID_TRACKS,
            &element(
                &[0xAE],
                &[
                    uint(&[0xD7], 1),
                    uint(&[0x73, 0xC5], track_uid),
                    uint(&[0x83], 1), // video
                    uint(&[0x9C], 0), // no lacing
                    element(&[0x86], codec_id),
                    element(&[0x63, 0xA2], &cfg.codec_private),
                    element(&[0xE0], &[uint(&[0xB0], cfg.width as u64), uint(&[0xBA], cfg.height as u64)].concat()),
                ]
                .concat(),
            ),
        );
        let void = [&[0xEC][..], &size(SEEK_HEAD_LEN - 2), &vec![0u8; SEEK_HEAD_LEN as usize - 2]].concat();

        let segment_start = (header.len() + ID_SEGMENT.len() + 8) as u64;
        let seek_head_at = segment_start;
        let info_at = seek_head_at + SEEK_HEAD_LEN;
        let tracks_at = info_at + info.len() as u64;
        // The duration's 8 payload bytes are the last bytes of Info.
        let duration_at = tracks_at - 8;
        let head = [&header[..], ID_SEGMENT, &UNKNOWN_SIZE, &void, &info, &tracks].concat();
        out.write_all(&head)?;
        Ok(Self {
            out,
            pos: head.len() as u64,
            segment_start,
            seek_head_at,
            info_at,
            tracks_at,
            duration_at,
            cluster: None,
            cues: Vec::new(),
            frames: 0,
            first_ms: None,
            last_ms: 0,
        })
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn patch(&mut self, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.out.seek(SeekFrom::Start(at))?;
        self.out.write_all(bytes)?;
        self.out.seek(SeekFrom::Start(self.pos))?;
        Ok(())
    }

    fn close_cluster(&mut self) -> io::Result<()> {
        if let Some((size_at, _)) = self.cluster.take() {
            let len = self.pos - (size_at + 8);
            self.patch(size_at, &size8(len))?;
            // What a crash leaves behind ends on a whole cluster.
            self.out.flush()?;
        }
        Ok(())
    }

    /// One access unit, its NAL units without start codes. `ms` is the presentation time; the first
    /// frame must be a keyframe.
    pub fn write_frame(&mut self, ms: i64, keyframe: bool, nals: &[&[u8]]) -> io::Result<()> {
        let ms = ms.max(0);
        let open = match self.cluster {
            Some((_, at)) if !keyframe && (ms - at).abs() < CLUSTER_MAX_MS => at,
            _ => {
                self.close_cluster()?;
                if keyframe {
                    self.cues.push((ms, self.pos - self.segment_start));
                }
                let size_at = self.pos + ID_CLUSTER.len() as u64;
                let head = [ID_CLUSTER, &UNKNOWN_SIZE, &[0xE7, 0x88], &(ms as u64).to_be_bytes()].concat();
                self.put(&head)?;
                self.cluster = Some((size_at, ms));
                ms
            }
        };
        let payload: usize = nals.iter().map(|n| 4 + n.len()).sum();
        let mut block = Vec::with_capacity(payload + 16);
        block.push(0xA3);
        block.extend(size(4 + payload as u64));
        block.push(0x81); // track 1
        block.extend(((ms - open) as i16).to_be_bytes());
        block.push(if keyframe { 0x80 } else { 0 });
        for n in nals {
            block.extend((n.len() as u32).to_be_bytes());
            block.extend(*n);
        }
        self.put(&block)?;
        self.frames += 1;
        self.first_ms.get_or_insert(ms);
        self.last_ms = self.last_ms.max(ms);
        Ok(())
    }

    pub fn bytes(&self) -> u64 {
        self.pos
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Length of the recording: the last frame's time plus one average frame.
    pub fn duration_ms(&self) -> i64 {
        let span = self.last_ms - self.first_ms.unwrap_or(0);
        if self.frames > 1 { self.last_ms + span / (self.frames as i64 - 1) } else { self.last_ms }
    }

    /// The stream as a crash would leave it.
    #[cfg(test)]
    pub fn into_unfinished(self) -> W {
        self.out
    }

    /// Close the file: cue index, seek head, duration, segment size.
    pub fn finish(mut self) -> io::Result<W> {
        self.close_cluster()?;
        let cues_at = self.pos;
        let points: Vec<u8> = self
            .cues
            .iter()
            .flat_map(|&(ms, at)| {
                let position = element(&[0xB7], &[uint(&[0xF7], 1), uint(&[0xF1], at)].concat());
                element(&[0xBB], &[uint(&[0xB3], ms as u64), position].concat())
            })
            .collect();
        let cues = element(ID_CUES, &points);
        self.put(&cues)?;

        let seek = |id: &[u8], at: u64| {
            let position = [&[0x53, 0xAC, 0x88][..], &(at - self.segment_start).to_be_bytes()].concat();
            element(&[0x4D, 0xBB], &[element(&[0x53, 0xAB], id), position].concat())
        };
        let seek_head = element(
            ID_SEEK_HEAD,
            &[seek(ID_INFO, self.info_at), seek(ID_TRACKS, self.tracks_at), seek(ID_CUES, cues_at)].concat(),
        );
        debug_assert_eq!(seek_head.len() as u64, SEEK_HEAD_LEN);
        self.patch(self.seek_head_at, &seek_head)?;
        self.patch(self.duration_at, &(self.duration_ms() as f64).to_be_bytes())?;
        self.patch(self.segment_start - 8, &size8(self.pos - self.segment_start))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn config() -> StreamConfig {
        StreamConfig { width: 1280, height: 720, codec_private: vec![1, 2, 3, 4] }
    }

    #[test]
    fn size_fields() {
        assert_eq!(size(0), [0x80]);
        assert_eq!(size(126), [0xFE]);
        assert_eq!(size(127), [0x40, 0x7F]); // 0xFF would read as "unknown"
        assert_eq!(size(16_382), [0x7F, 0xFE]);
        assert_eq!(size(16_383), [0x20, 0x3F, 0xFF]);
        assert_eq!(size8(5), [1, 0, 0, 0, 0, 0, 0, 5]);
    }

    #[test]
    fn unfinished_file_keeps_the_unknown_sizes_and_finish_patches_them() {
        let mut w = MkvWriter::create(Cursor::new(Vec::new()), VideoCodec::H264, &config(), MKV_EPOCH_UNIX_MS, [7; 16]).unwrap();
        let segment_size_at = (w.segment_start - 8) as usize;
        w.write_frame(0, true, &[&[0x65, 1, 2]]).unwrap();
        w.write_frame(33, false, &[&[0x41, 3]]).unwrap();
        w.write_frame(1000, true, &[&[0x65, 4]]).unwrap();
        assert_eq!(w.frames(), 3);
        assert_eq!(w.duration_ms(), 1500);
        let cues = w.cues.clone();
        assert_eq!(cues.len(), 2);
        // Open: the segment still says "unknown", the place of the seek head is a Void.
        let raw = w.out.get_ref().clone();
        assert_eq!(&raw[segment_size_at..segment_size_at + 8], &UNKNOWN_SIZE);
        assert_eq!(raw[w.seek_head_at as usize], 0xEC);

        let (seek_head_at, duration_at, segment_start) = (w.seek_head_at as usize, w.duration_at as usize, w.segment_start as usize);
        let done = w.finish().unwrap().into_inner();
        assert_eq!(&done[seek_head_at..seek_head_at + 4], ID_SEEK_HEAD);
        assert_eq!(&done[duration_at..duration_at + 8], &1500f64.to_be_bytes());
        assert_eq!(&done[segment_size_at..segment_size_at + 8], &size8((done.len() - segment_start) as u64));
        // Every cue points at a cluster.
        for (_, at) in cues {
            let at = segment_start + at as usize;
            assert_eq!(&done[at..at + 4], ID_CLUSTER);
        }
    }

    #[test]
    fn a_long_gap_without_a_keyframe_opens_a_new_cluster() {
        let mut w = MkvWriter::create(Cursor::new(Vec::new()), VideoCodec::H265, &config(), 0, [1; 16]).unwrap();
        w.write_frame(0, true, &[&[0x26, 1]]).unwrap();
        w.write_frame(29_999, false, &[&[0x02, 1]]).unwrap();
        let before = w.cluster.unwrap().0;
        w.write_frame(30_000, false, &[&[0x02, 1]]).unwrap();
        assert_ne!(w.cluster.unwrap().0, before);
        assert_eq!(w.cues.len(), 1);
    }
}
