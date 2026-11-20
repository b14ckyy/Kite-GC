// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Recording of the native RTSP stream: the H.264 / H.265 access units are written to a Matroska file as
//! they arrive, without re-encoding. The RTSP thread only copies an access unit into a queue; a writer
//! thread owns the file, so a slow disk costs recorded frames and never the live picture.

mod mkv;
mod mkv_read;
mod nal;
pub mod replay;

use std::collections::VecDeque;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::video::rtsp::VideoCodec;
use mkv::MkvWriter;
pub use mkv_read::{kite_segment_uid, segment_uid};
use nal::{is_keyframe, split_nals, stream_config, StreamConfig};

/// The pre-roll is the running group of pictures, so a recording starts at the keyframe before the
/// start. A stream with huge groups (or none) must not grow it without bound.
const PREROLL_MAX_BYTES: usize = 48 * 1024 * 1024;
/// Access units the writer may lag behind before the recording drops frames.
const QUEUE_LEN: usize = 600;
/// A new part starts at the next keyframe above this size: FAT32 ends a file at 4 GB.
const PART_MAX_BYTES: u64 = 3_900 * 1024 * 1024;

fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// 16 bytes no other recording has. `RandomState` is seeded from the OS per instance.
/// Called with every part the moment its file exists.
pub type OnOpen = Box<dyn Fn(&RecordedVideo) + Send>;

/// What an unclosed part holds, read from the file (its last cluster tells the length). `None` when the
/// file is gone, is another recording by now, or cannot be read.
pub fn read_unfinished(part: &RecordedVideo) -> Option<RecordedVideo> {
    let path = Path::new(&part.path);
    if kite_segment_uid(path).as_deref() != Some(part.uid.as_str()) {
        return None;
    }
    let file = mkv_read::MkvFile::open(path).ok()?;
    Some(RecordedVideo {
        end_unix_ms: part.start_unix_ms + file.duration_ms.max(1),
        duration_ms: file.duration_ms,
        width: file.width,
        height: file.height,
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        ..part.clone()
    })
}

fn new_segment_uid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let part = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_i64(unix_ms());
        h.finish().to_be_bytes()
    };
    let mut uid = [0u8; 16];
    uid[..8].copy_from_slice(&part(1));
    uid[8..].copy_from_slice(&part(2));
    uid
}

/// A recording file. `end_unix_ms == 0` = announced when it was opened and not closed (yet): the file
/// of a part still being written, or what a crash left behind.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedVideo {
    pub path: String,
    /// The file's Matroska segment UID (hex): identifies it whatever it is renamed to.
    pub uid: String,
    /// Wall-clock arrival of the first and the last written picture (Unix ms).
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub duration_ms: i64,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub size: u64,
}

struct Au {
    codec: VideoCodec,
    data: Vec<u8>,
    /// Unwrapped RTP time, 90 kHz.
    ts: u64,
    arrived_ms: i64,
    keyframe: bool,
}

enum Msg {
    Au(Au),
    /// The stream restarted: its clock and maybe its format are new, so the file ends here.
    Cut,
}

struct Active {
    tx: SyncSender<Msg>,
    thread: JoinHandle<()>,
    /// Frames were dropped: nothing more goes out until the next keyframe.
    wait_keyframe: bool,
    /// The stream restarted and the writer does not know yet: no picture of the new stream before it.
    cut_pending: bool,
}

#[derive(Default)]
struct Inner {
    /// Buffer the running group of pictures (the recording feature is switched on).
    enabled: bool,
    preroll: VecDeque<Au>,
    preroll_bytes: usize,
    active: Option<Active>,
    dropped: u64,
}

/// Live status for the panel.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RecordingStatus {
    pub recording: bool,
    pub file: Option<String>,
    pub bytes: u64,
    pub dropped: u64,
    pub error: Option<String>,
}

#[derive(Default)]
struct Shared {
    finished: Vec<RecordedVideo>,
    file: Option<String>,
    bytes: u64,
    error: Option<String>,
}

#[derive(Default)]
pub struct VideoRecorder {
    inner: Mutex<Inner>,
    shared: Arc<Mutex<Shared>>,
}

impl VideoRecorder {
    /// Keep (or stop keeping) the pre-roll. Off = `feed` costs nothing.
    pub fn set_enabled(&self, on: bool) {
        let mut g = self.inner.lock().unwrap();
        g.enabled = on;
        if !on {
            g.preroll.clear();
            g.preroll_bytes = 0;
        }
    }

    /// The RTSP thread starts a new stream: the pre-roll is from the old one.
    pub fn stream_reset(&self) {
        let mut g = self.inner.lock().unwrap();
        g.preroll.clear();
        g.preroll_bytes = 0;
        if let Some(a) = g.active.as_mut() {
            a.wait_keyframe = true;
            a.cut_pending = a.tx.try_send(Msg::Cut).is_err();
        }
    }

    /// One access unit from the RTSP thread (`ts` = unwrapped RTP time, 90 kHz). Never blocks.
    pub fn feed(&self, codec: VideoCodec, annexb: &[u8], ts: u64) {
        let mut g = self.inner.lock().unwrap();
        if !g.enabled && g.active.is_none() {
            return;
        }
        let keyframe = is_keyframe(codec, &split_nals(annexb));
        let au = || Au { codec, data: annexb.to_vec(), ts, arrived_ms: unix_ms(), keyframe };
        if g.enabled {
            if g.preroll_bytes + annexb.len() > PREROLL_MAX_BYTES {
                g.preroll.clear();
            } else if keyframe {
                // Earlier pieces of this same picture (they share its RTP time) stay.
                g.preroll.retain(|a| a.ts == ts);
            }
            g.preroll_bytes = g.preroll.iter().map(|a| a.data.len()).sum();
            // Without a keyframe at its head the buffer is useless: wait for the next one.
            if keyframe || !g.preroll.is_empty() {
                g.preroll_bytes += annexb.len();
                g.preroll.push_back(au());
            }
        }
        let mut dropped = false;
        if let Some(a) = g.active.as_mut() {
            if a.cut_pending {
                a.cut_pending = a.tx.try_send(Msg::Cut).is_err();
            }
            if a.cut_pending || (a.wait_keyframe && !keyframe) {
                dropped = true;
            } else {
                match a.tx.try_send(Msg::Au(au())) {
                    Ok(()) => a.wait_keyframe = false,
                    Err(TrySendError::Full(_)) => {
                        a.wait_keyframe = true;
                        dropped = true;
                    }
                    // The writer ended on an error; `stop` reports it.
                    Err(TrySendError::Disconnected(_)) => {}
                }
            }
        }
        if dropped {
            g.dropped += 1;
        }
    }

    /// Start writing into `dir`, beginning with the buffered group of pictures. `name` is the file name
    /// without extension; parts after the first get a suffix.
    pub fn start(&self, dir: &Path, name: &str, on_open: OnOpen) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut g = self.inner.lock().unwrap();
        if g.active.is_some() {
            return Ok(());
        }
        {
            let mut s = self.shared.lock().unwrap();
            s.error = None;
            s.file = None;
            s.bytes = 0;
        }
        g.dropped = 0;
        let (tx, rx) = sync_channel::<Msg>(QUEUE_LEN.max(g.preroll.len() + 1));
        for au in g.preroll.iter() {
            let copy = Au { codec: au.codec, data: au.data.clone(), ts: au.ts, arrived_ms: au.arrived_ms, keyframe: au.keyframe };
            let _ = tx.try_send(Msg::Au(copy));
        }
        let (dir, name, shared) = (dir.to_path_buf(), name.to_string(), self.shared.clone());
        let thread = std::thread::Builder::new()
            .name("video-record".into())
            .spawn(move || writer_loop(rx, dir, name, shared, on_open))
            .map_err(|e| format!("cannot start the recording thread: {e}"))?;
        // An empty pre-roll means the stream is mid-group: the file starts at the next keyframe.
        let wait_keyframe = g.preroll.is_empty();
        g.active = Some(Active { tx, thread, wait_keyframe, cut_pending: false });
        Ok(())
    }

    /// Stop and close the file. Returns every file of this recording, in order.
    pub fn stop(&self) -> Vec<RecordedVideo> {
        let active = self.inner.lock().unwrap().active.take();
        if let Some(a) = active {
            drop(a.tx);
            let _ = a.thread.join();
        }
        std::mem::take(&mut self.shared.lock().unwrap().finished)
    }

    /// Segment UIDs of the parts a running recording has closed already: they are in no list on disk
    /// until `stop` hands them out.
    pub fn pending_uids(&self) -> Vec<String> {
        self.shared.lock().unwrap().finished.iter().map(|v| v.uid.clone()).collect()
    }

    pub fn status(&self) -> RecordingStatus {
        let g = self.inner.lock().unwrap();
        let s = self.shared.lock().unwrap();
        RecordingStatus {
            recording: g.active.is_some(),
            file: s.file.clone(),
            bytes: s.bytes,
            dropped: g.dropped,
            error: s.error.clone(),
        }
    }
}

fn codec_name(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H265 => "hevc",
        _ => "h264",
    }
}

struct OpenFile {
    writer: MkvWriter<BufWriter<File>>,
    path: PathBuf,
    uid: [u8; 16],
    codec: VideoCodec,
    config: StreamConfig,
    first_ts: u64,
    start_unix_ms: i64,
    end_unix_ms: i64,
}

/// `finish` = write the index; without it (after a write error) the file stays as it is on disk, playable
/// up to its last cluster. Either way it is the flight's recording.
fn close(file: OpenFile, shared: &Mutex<Shared>, finish: bool) {
    let (frames, duration_ms) = (file.writer.frames(), file.writer.duration_ms());
    let path = file.path.to_string_lossy().to_string();
    if finish {
        let done = file.writer.finish().and_then(|w| w.into_inner().map_err(|e| e.into_error())).and_then(|f| f.sync_all());
        if let Err(e) = done {
            log::warn!("[video] recording {path} could not be closed cleanly: {e}");
            shared.lock().unwrap().error.get_or_insert(format!("{path}: {e}"));
        }
    }
    let size = std::fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0);
    log::info!("[video] recording closed: {path}, {frames} frames, {} s, {size} bytes", duration_ms / 1000);
    shared.lock().unwrap().finished.push(RecordedVideo {
        path,
        uid: file.uid.iter().map(|b| format!("{b:02x}")).collect(),
        start_unix_ms: file.start_unix_ms,
        end_unix_ms: file.end_unix_ms,
        duration_ms,
        codec: codec_name(file.codec).into(),
        width: file.config.width,
        height: file.config.height,
        frames,
        size,
    });
}

/// A new file under `stem`, never over an existing one: a second recording within the same second gets
/// a counter.
fn create_unique(dir: &Path, stem: &str) -> std::io::Result<(File, PathBuf)> {
    let mut n = 1u32;
    loop {
        let name = if n == 1 { format!("{stem}.mkv") } else { format!("{stem}_{n}.mkv") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    }
}

struct FileWriter {
    dir: PathBuf,
    name: String,
    shared: Arc<Mutex<Shared>>,
    on_open: OnOpen,
    open: Option<OpenFile>,
    part: u32,
}

impl FileWriter {
    fn cut(&mut self) {
        if let Some(f) = self.open.take() {
            close(f, &self.shared, true);
        }
    }

    /// One whole picture. `false` = the file cannot be written, the recording is over.
    fn write(&mut self, au: Au) -> bool {
        let nals = split_nals(&au.data);
        if au.keyframe {
            let config = stream_config(au.codec, &nals);
            let changed = self.open.as_ref().is_some_and(|f| {
                f.codec != au.codec || config.as_ref().is_some_and(|c| *c != f.config) || f.writer.bytes() > PART_MAX_BYTES
            });
            if changed {
                self.cut();
            }
            if self.open.is_none() {
                // A keyframe without parameter sets cannot head a file; the next one may carry them.
                let Some(config) = config else { return true };
                self.part += 1;
                let stem = if self.part == 1 { self.name.clone() } else { format!("{}_part{}", self.name, self.part) };
                let uid = new_segment_uid();
                let created = create_unique(&self.dir, &stem).and_then(|(f, path)| {
                    MkvWriter::create(BufWriter::with_capacity(1 << 20, f), au.codec, &config, au.arrived_ms, uid).map(|w| (w, path))
                });
                match created {
                    Ok((writer, path)) => {
                        log::info!("[video] recording to {}", path.display());
                        self.shared.lock().unwrap().file = Some(path.to_string_lossy().to_string());
                        (self.on_open)(&RecordedVideo {
                            path: path.to_string_lossy().to_string(),
                            uid: uid.iter().map(|b| format!("{b:02x}")).collect(),
                            start_unix_ms: au.arrived_ms,
                            end_unix_ms: 0,
                            duration_ms: 0,
                            codec: codec_name(au.codec).into(),
                            width: config.width,
                            height: config.height,
                            frames: 0,
                            size: 0,
                        });
                        self.open = Some(OpenFile {
                            writer,
                            path,
                            uid,
                            codec: au.codec,
                            config,
                            first_ts: au.ts,
                            start_unix_ms: au.arrived_ms,
                            end_unix_ms: au.arrived_ms,
                        });
                    }
                    Err(e) => {
                        log::warn!("[video] recording: cannot create {stem}.mkv in {}: {e}", self.dir.display());
                        self.shared.lock().unwrap().error = Some(format!("{}: {e}", self.dir.join(stem).display()));
                        return false;
                    }
                }
            }
        }
        let Some(f) = self.open.as_mut() else { return true };
        let ms = (au.ts.wrapping_sub(f.first_ts) as i64) / 90;
        if let Err(e) = f.writer.write_frame(ms, au.keyframe, &nals) {
            log::warn!("[video] recording: write to {} failed: {e}", f.path.display());
            self.shared.lock().unwrap().error = Some(format!("{}: {e}", f.path.display()));
            // Finishing would only fail the same way.
            if let Some(f) = self.open.take() {
                close(f, &self.shared, false);
            }
            return false;
        }
        f.end_unix_ms = au.arrived_ms;
        self.shared.lock().unwrap().bytes = f.writer.bytes();
        true
    }
}

fn writer_loop(rx: Receiver<Msg>, dir: PathBuf, name: String, shared: Arc<Mutex<Shared>>, on_open: OnOpen) {
    let mut w = FileWriter { dir, name, shared, on_open, open: None, part: 0 };
    // The depacketizer may hand out one picture in pieces (parameter sets or slices of their own): pieces
    // share the RTP time, and a block must hold the whole picture.
    let mut picture: Option<Au> = None;
    for msg in rx {
        match msg {
            Msg::Cut => {
                if picture.take().is_some_and(|p| !w.write(p)) {
                    return;
                }
                w.cut();
            }
            Msg::Au(au) => match picture.as_mut() {
                Some(p) if p.ts == au.ts && p.codec == au.codec => {
                    p.data.extend_from_slice(&au.data);
                    p.keyframe |= au.keyframe;
                }
                _ => {
                    if picture.replace(au).is_some_and(|p| !w.write(p)) {
                        return;
                    }
                }
            },
        }
    }
    if picture.take().is_some_and(|p| !w.write(p)) {
        return;
    }
    w.cut();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Annex-B elementary stream → access units, one per picture NAL (enough for encoder output).
    fn access_units(codec: VideoCodec, es: &[u8]) -> Vec<Vec<u8>> {
        let mut starts: Vec<usize> = Vec::new();
        let mut i = 0;
        while i + 3 <= es.len() {
            if es[i..].starts_with(&[0, 0, 1]) {
                starts.push(i + 3);
                i += 3;
            } else {
                i += 1;
            }
        }
        let mut aus = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        for (k, &s) in starts.iter().enumerate() {
            let end = starts.get(k + 1).map(|n| n - 3).unwrap_or(es.len());
            let nal = &es[s..end];
            let nal = &nal[..nal.len() - nal.iter().rev().take_while(|b| **b == 0).count()];
            if nal.is_empty() {
                continue;
            }
            cur.extend([0, 0, 0, 1]);
            cur.extend(nal);
            let picture = match codec {
                VideoCodec::H265 => ((nal[0] >> 1) & 0x3F) < 32,
                _ => matches!(nal[0] & 0x1F, 1 | 5),
            };
            if picture {
                aus.push(std::mem::take(&mut cur));
            }
        }
        aus
    }

    /// Dev tool, not a unit test: `KITE_REC_IN=<file.h264|.h265> KITE_REC_OUT=<dir> cargo test record_file`
    /// feeds an elementary stream through the recorder at 30 fps so the result can be checked with ffprobe.
    #[test]
    fn record_file() {
        let (Ok(input), Ok(out)) = (std::env::var("KITE_REC_IN"), std::env::var("KITE_REC_OUT")) else { return };
        let codec = if input.ends_with("265") || input.ends_with("hevc") { VideoCodec::H265 } else { VideoCodec::H264 };
        let aus = access_units(codec, &std::fs::read(&input).unwrap());
        let rec = VideoRecorder::default();
        rec.set_enabled(true);
        // The first second is pre-roll, the recording starts in the middle of the second group.
        for (i, au) in aus.iter().enumerate() {
            if i == 45 {
                rec.start(Path::new(&out), "record_file_test", Box::new(|_| {})).unwrap();
            }
            rec.feed(codec, au, i as u64 * 3000);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // What a crash would leave: the file as it is on disk while the recording still runs.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let live = rec.status().file.unwrap();
        std::fs::copy(&live, format!("{live}.crash.mkv")).unwrap();
        let files = rec.stop();
        println!("{files:#?} {:?}", rec.status());
        assert_eq!(files.len(), 1);
        assert!(files[0].frames > 0);
    }

    #[test]
    fn preroll_starts_at_the_last_keyframe_and_stop_returns_the_file() {
        let dir = std::env::temp_dir().join(format!("kite-rec-test-{}", unix_ms()));
        let sps = [0x67u8, 0x42, 0xc0, 0x28, 0xda, 0x01, 0xe0, 0x08, 0x9f, 0x97, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xc0, 0xf1, 0x83, 0x2a];
        let key: Vec<u8> = [&[0, 0, 0, 1][..], &sps, &[0, 0, 0, 1, 0x68, 0xce, 0x0f, 0xc8], &[0, 0, 0, 1, 0x65, 1, 2, 3]].concat();
        let delta: Vec<u8> = vec![0, 0, 0, 1, 0x41, 9, 9];
        let rec = VideoRecorder::default();
        // Not enabled: nothing is kept.
        rec.feed(VideoCodec::H264, &key, 0);
        assert!(rec.inner.lock().unwrap().preroll.is_empty());
        rec.set_enabled(true);
        rec.feed(VideoCodec::H264, &delta, 3000); // mid-group: no head, not kept
        rec.feed(VideoCodec::H264, &key, 6000);
        rec.feed(VideoCodec::H264, &delta, 9000);
        rec.feed(VideoCodec::H264, &key, 12000); // a new group replaces the old one
        rec.feed(VideoCodec::H264, &delta, 15000);
        assert_eq!(rec.inner.lock().unwrap().preroll.len(), 2);
        let announced = Arc::new(Mutex::new(Vec::new()));
        let seen = announced.clone();
        rec.start(&dir, "t", Box::new(move |part| seen.lock().unwrap().push(part.clone()))).unwrap();
        rec.feed(VideoCodec::H264, &delta, 18000);
        // A picture in two pieces (same RTP time) becomes one frame.
        rec.feed(VideoCodec::H264, &delta, 21000);
        rec.feed(VideoCodec::H264, &delta, 21000);
        let files = rec.stop();
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].frames, files[0].width, files[0].height), (4, 1920, 1080));
        assert_eq!(files[0].uid.len(), 32);
        assert!(files[0].size > 0 && Path::new(&files[0].path).exists());
        assert!(!rec.status().recording);
        // The part was announced when it was opened, and the file tells what an unclosed one holds.
        let announced = announced.lock().unwrap();
        assert_eq!(announced.len(), 1);
        assert_eq!((announced[0].uid.as_str(), announced[0].end_unix_ms), (files[0].uid.as_str(), 0));
        let read = read_unfinished(&announced[0]).unwrap();
        // The reader knows the time of the last picture, not how long it was shown.
        assert!(read.duration_ms > 0 && read.duration_ms <= files[0].duration_ms);
        assert_eq!((read.width, read.end_unix_ms), (1920, announced[0].start_unix_ms + read.duration_ms));
        let other = RecordedVideo { uid: "00".repeat(16), ..announced[0].clone() };
        assert!(read_unfinished(&other).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
