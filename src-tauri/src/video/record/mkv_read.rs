// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Reader for the Matroska files `mkv.rs` writes: one H.264 / H.265 track, SimpleBlocks, 1 ms time base.
//! The cluster index is built by walking the file, not from the cues — a recording cut off by a crash
//! has none, and its last cluster has no size either.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::video::rtsp::VideoCodec;

const ID_SEGMENT: u32 = 0x1853_8067;
const ID_INFO: u32 = 0x1549_A966;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_VIDEO: u32 = 0xE0;
const ID_CODEC_ID: u32 = 0x86;
const ID_CODEC_PRIVATE: u32 = 0x63A2;
const ID_PIXEL_WIDTH: u32 = 0xB0;
const ID_PIXEL_HEIGHT: u32 = 0xBA;
const ID_DURATION: u32 = 0x4489;
const ID_TIMESTAMP: u32 = 0xE7;
const ID_SIMPLE_BLOCK: u32 = 0xA3;

const START_CODE: [u8; 4] = [0, 0, 0, 1];
/// Headers and parameter sets are small; anything larger is not one of our files.
const SMALL_ELEMENT_MAX: u64 = 1 << 20;
const FRAME_MAX: u64 = 64 << 20;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// One picture: Annex-B access unit and its time in the file.
pub struct Frame {
    pub ms: i64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub keyframe: bool,
    pub annexb: Vec<u8>,
}

struct ClusterRef {
    /// File offset of the cluster element.
    pos: u64,
    ms: i64,
    keyframe: bool,
}

pub struct MkvFile {
    r: BufReader<File>,
    len: u64,
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    /// Last picture's time; from the index, so it is right for an unfinished file too.
    pub duration_ms: i64,
    /// The parameter sets of the track header, Annex-B framed.
    params: Vec<u8>,
    clusters: Vec<ClusterRef>,
    cluster_ms: i64,
}

impl MkvFile {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut f = Self {
            r: BufReader::with_capacity(1 << 20, file),
            len,
            codec: VideoCodec::H264,
            width: 0,
            height: 0,
            duration_ms: 0,
            params: Vec::new(),
            clusters: Vec::new(),
            cluster_ms: 0,
        };
        f.scan()?;
        if f.width == 0 || f.clusters.is_empty() {
            return Err(bad("no video track or no pictures"));
        }
        f.r.seek(SeekFrom::Start(f.clusters[0].pos))?;
        Ok(f)
    }

    fn pos(&mut self) -> io::Result<u64> {
        self.r.stream_position()
    }

    /// Element id (with its length marker) and payload size; `None` size = unknown. `None` at the end.
    fn head(&mut self) -> io::Result<Option<(u32, Option<u64>)>> {
        let mut b = [0u8; 1];
        if self.r.read(&mut b)? == 0 {
            return Ok(None);
        }
        let id_len = b[0].leading_zeros() as usize + 1;
        if id_len > 4 {
            return Err(bad("element id"));
        }
        let mut id = b[0] as u32;
        for _ in 1..id_len {
            self.r.read_exact(&mut b)?;
            id = (id << 8) | b[0] as u32;
        }
        self.r.read_exact(&mut b)?;
        let size_len = b[0].leading_zeros() as usize + 1;
        if size_len > 8 {
            return Err(bad("element size"));
        }
        let mut size = (b[0] as u64) & (0xFF >> size_len);
        let mut all_ones = size == (0xFF >> size_len) as u64;
        for _ in 1..size_len {
            self.r.read_exact(&mut b)?;
            size = (size << 8) | b[0] as u64;
            all_ones &= b[0] == 0xFF;
        }
        Ok(Some((id, if all_ones { None } else { Some(size) })))
    }

    fn payload(&mut self, size: Option<u64>, max: u64) -> io::Result<Vec<u8>> {
        let size = size.filter(|s| *s <= max).ok_or_else(|| bad("element too large"))?;
        let mut v = vec![0u8; size as usize];
        self.r.read_exact(&mut v)?;
        Ok(v)
    }

    fn skip(&mut self, size: Option<u64>) -> io::Result<()> {
        let size = size.ok_or_else(|| bad("unknown size outside a cluster"))?;
        self.r.seek_relative(size as i64)
    }

    /// Track header and the cluster index. Master elements we need are entered, the rest skipped.
    fn scan(&mut self) -> io::Result<()> {
        loop {
            let at = self.pos()?;
            if at >= self.len {
                break;
            }
            let Some((id, size)) = self.head()? else { break };
            match id {
                ID_SEGMENT | ID_TRACKS | ID_TRACK_ENTRY | ID_VIDEO | ID_INFO => {} // enter
                ID_CODEC_ID => {
                    let v = self.payload(size, SMALL_ELEMENT_MAX)?;
                    self.codec = match v.as_slice() {
                        b"V_MPEG4/ISO/AVC" => VideoCodec::H264,
                        b"V_MPEGH/ISO/HEVC" => VideoCodec::H265,
                        _ => return Err(bad("not an H.264 / H.265 track")),
                    };
                }
                ID_CODEC_PRIVATE => {
                    let v = self.payload(size, SMALL_ELEMENT_MAX)?;
                    self.params = params_annexb(self.codec, &v).ok_or_else(|| bad("decoder configuration"))?;
                }
                ID_PIXEL_WIDTH => self.width = uint(&self.payload(size, 8)?) as u32,
                ID_PIXEL_HEIGHT => self.height = uint(&self.payload(size, 8)?) as u32,
                ID_DURATION => {
                    self.payload(size, 8)?;
                }
                ID_CLUSTER => {
                    // A closed cluster is skipped as a whole once its head is read; the open one of
                    // an unfinished file is walked block by block.
                    let body = self.pos()?;
                    let mut cluster = ClusterRef { pos: at, ms: 0, keyframe: false };
                    let end = size.map(|s| body + s);
                    let mut seen_block = false;
                    loop {
                        let here = self.pos()?;
                        if end.is_some_and(|e| here >= e) || here >= self.len {
                            break;
                        }
                        let Ok(Some((cid, csize))) = self.head() else { break };
                        match cid {
                            ID_TIMESTAMP => cluster.ms = uint(&self.payload(csize, 8)?) as i64,
                            ID_SIMPLE_BLOCK => {
                                let Some(csize) = csize.filter(|s| *s >= 4 && here + *s <= self.len) else { break };
                                let mut h = [0u8; 4];
                                self.r.read_exact(&mut h)?;
                                let ms = cluster.ms + i16::from_be_bytes([h[1], h[2]]) as i64;
                                self.duration_ms = self.duration_ms.max(ms);
                                if !seen_block {
                                    cluster.keyframe = h[3] & 0x80 != 0;
                                    seen_block = true;
                                    if let Some(e) = end {
                                        self.r.seek(SeekFrom::Start(e))?;
                                        break;
                                    }
                                }
                                self.r.seek_relative(csize as i64 - 4)?;
                            }
                            // A top-level element: the cluster of unknown size ended.
                            ID_CLUSTER | 0x1C53_BB6B => {
                                self.r.seek(SeekFrom::Start(here))?;
                                break;
                            }
                            _ => {
                                if csize.is_none() {
                                    break;
                                }
                                self.skip(csize)?;
                            }
                        }
                    }
                    if seen_block {
                        self.clusters.push(cluster);
                    }
                }
                _ => self.skip(size)?,
            }
        }
        // Closed clusters were skipped after their first block: the last one's end is not known yet.
        if let Some(last) = self.clusters.last() {
            let (pos, from) = (last.pos, last.ms);
            self.r.seek(SeekFrom::Start(pos))?;
            self.cluster_ms = from;
            while let Ok(Some(f)) = self.next_frame() {
                self.duration_ms = self.duration_ms.max(f.ms);
            }
        }
        Ok(())
    }

    /// Continue at the last keyframe cluster that starts at or before `ms` (the first one otherwise).
    pub fn seek_before(&mut self, ms: i64) -> io::Result<()> {
        let mut i = self.clusters.partition_point(|c| c.ms <= ms).saturating_sub(1);
        while i > 0 && !self.clusters[i].keyframe {
            i -= 1;
        }
        self.r.seek(SeekFrom::Start(self.clusters[i].pos))?;
        Ok(())
    }

    /// Time of the last keyframe cluster at or before `ms`.
    pub fn keyframe_before(&self, ms: i64) -> i64 {
        let mut i = self.clusters.partition_point(|c| c.ms <= ms).saturating_sub(1);
        while i > 0 && !self.clusters[i].keyframe {
            i -= 1;
        }
        self.clusters[i].ms
    }

    /// The next picture in file order; `None` at the end (or where an unfinished file breaks off).
    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        loop {
            let here = self.pos()?;
            if here >= self.len {
                return Ok(None);
            }
            let Some((id, size)) = self.head()? else { return Ok(None) };
            match id {
                ID_CLUSTER => {} // enter
                ID_TIMESTAMP => self.cluster_ms = uint(&self.payload(size, 8)?) as i64,
                ID_SIMPLE_BLOCK => {
                    let Some(size) = size.filter(|s| *s >= 4 && *s <= FRAME_MAX && here + *s <= self.len) else { return Ok(None) };
                    let block = self.payload(Some(size), FRAME_MAX)?;
                    let ms = self.cluster_ms + i16::from_be_bytes([block[1], block[2]]) as i64;
                    let keyframe = block[3] & 0x80 != 0;
                    let mut annexb = Vec::with_capacity(block.len() + self.params.len() + 16);
                    let mut nals = &block[4..];
                    let mut body = Vec::with_capacity(block.len());
                    let mut has_sets = false;
                    while nals.len() >= 4 {
                        let n = u32::from_be_bytes([nals[0], nals[1], nals[2], nals[3]]) as usize;
                        let Some(nal) = nals.get(4..4 + n) else { break };
                        if let Some(b0) = nal.first() {
                            has_sets |= match self.codec {
                                VideoCodec::H265 => (b0 >> 1) & 0x3F == 33,
                                _ => b0 & 0x1F == 7,
                            };
                        }
                        body.extend(START_CODE);
                        body.extend(nal);
                        nals = &nals[4 + n..];
                    }
                    // A decoder that starts here needs the sets; the stream may carry them only once.
                    if keyframe && !has_sets {
                        annexb.extend(&self.params);
                    }
                    annexb.extend(body);
                    return Ok(Some(Frame { ms, keyframe, annexb }));
                }
                0x1C53_BB6B => return Ok(None), // cues: the pictures end here
                _ => {
                    if size.is_none() {
                        return Ok(None);
                    }
                    self.skip(size)?;
                }
            }
        }
    }
}

fn uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |v, b| (v << 8) | *b as u64)
}

/// The parameter sets of an `avcC` / `hvcC` record as Annex-B NAL units.
fn params_annexb(codec: VideoCodec, c: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut take = |c: &[u8], at: &mut usize| -> Option<()> {
        let n = u16::from_be_bytes([*c.get(*at)?, *c.get(*at + 1)?]) as usize;
        out.extend(START_CODE);
        out.extend(c.get(*at + 2..*at + 2 + n)?);
        *at += 2 + n;
        Some(())
    };
    match codec {
        VideoCodec::H265 => {
            let mut at = 23;
            for _ in 0..*c.get(22)? {
                let count = u16::from_be_bytes([*c.get(at + 1)?, *c.get(at + 2)?]);
                at += 3;
                for _ in 0..count {
                    take(c, &mut at)?;
                }
            }
        }
        _ => {
            let mut at = 6;
            for _ in 0..(c.get(5)? & 0x1F) {
                take(c, &mut at)?;
            }
            let pps = *c.get(at)?;
            at += 1;
            for _ in 0..pps {
                take(c, &mut at)?;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::mkv::MkvWriter;
    use super::super::nal::{split_nals, stream_config};
    use super::*;
    use std::io::Write;

    const SPS: [u8; 25] = [0x67, 0x42, 0xc0, 0x28, 0xda, 0x01, 0xe0, 0x08, 0x9f, 0x97, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x83, 0x2a];
    const PPS: [u8; 4] = [0x68, 0xce, 0x0f, 0xc8];

    /// 5 groups of 4 pictures, 100 ms apart; `finish` = closed properly.
    fn write(path: &Path, finish: bool) {
        let config = stream_config(VideoCodec::H264, &[&SPS, &PPS]).unwrap();
        let file = File::create(path).unwrap();
        let mut w = MkvWriter::create(std::io::BufWriter::new(file), VideoCodec::H264, &config, 0, [3; 16]).unwrap();
        for i in 0..20i64 {
            let key = i % 4 == 0;
            let au: Vec<u8> = if key { vec![0, 0, 0, 1, 0x65, i as u8] } else { vec![0, 0, 0, 1, 0x41, i as u8, 7] };
            w.write_frame(i * 100, key, &split_nals(&au)).unwrap();
        }
        if finish {
            w.finish().unwrap().flush().unwrap();
        } else {
            // What a crash leaves: everything written, nothing patched.
            let mut inner = w.into_unfinished();
            inner.flush().unwrap();
        }
    }

    fn check(path: &Path) {
        let mut f = MkvFile::open(path).unwrap();
        assert_eq!((f.codec, f.width, f.height), (VideoCodec::H264, 1920, 1080));
        assert_eq!(f.duration_ms, 1900);
        assert_eq!(f.clusters.len(), 5);
        let first = f.next_frame().unwrap().unwrap();
        assert!(first.keyframe && first.ms == 0);
        // The keyframe gets the parameter sets of the header in front.
        assert_eq!(&first.annexb[..5], &[0, 0, 0, 1, 0x67]);
        assert!(first.annexb.ends_with(&[0, 0, 0, 1, 0x65, 0]));
        let mut n = 1;
        while let Some(fr) = f.next_frame().unwrap() {
            assert_eq!(fr.ms, n * 100);
            n += 1;
        }
        assert_eq!(n, 20);
        f.seek_before(1050).unwrap();
        assert_eq!(f.keyframe_before(1050), 800);
        let fr = f.next_frame().unwrap().unwrap();
        assert!(fr.keyframe && fr.ms == 800);
    }

    #[test]
    fn reads_back_what_the_writer_wrote_finished_or_not() {
        let dir = std::env::temp_dir().join(format!("kite-mkvread-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for finish in [true, false] {
            let path = dir.join(format!("t{finish}.mkv"));
            write(&path, finish);
            check(&path);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
