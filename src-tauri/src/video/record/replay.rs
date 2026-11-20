// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Plays a flight's recordings in step with the log replay. The replay clock of the frontend leads: this
//! thread only follows the flight time it is told, and hands the pictures that are due to the decode sink.
//! Where no recording covers the time (before, after, between two files, file gone) there is a gap.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::mkv_read::{Frame, MkvFile};
use crate::video::rtsp::VideoCodec;

/// The reported time may differ this much from the running clock before it is re-anchored: the frontend
/// reports in 100 ms steps, and following every step would stutter.
const CLOCK_TOLERANCE_MS: f64 = 300.0;
/// Further ahead than this is a jump (seek to the keyframe before), not a wait.
const SKIP_AHEAD_MS: i64 = 1500;
/// Above this speed only keyframes are shown: no decoder keeps up with 10x at 60 fps.
const KEYFRAMES_ONLY_ABOVE: f64 = 2.5;
/// Pictures handed over per second while catching up from a keyframe to the target. The Linux sink's
/// input queue drops when it is full (120 access units), and a stateless decoder never recovers from a
/// dropped one, so it is fed slower.
#[cfg(not(target_os = "linux"))]
const CATCH_UP_FPS: u64 = 400;
#[cfg(target_os = "linux")]
const CATCH_UP_FPS: u64 = 120;
/// A new seek waits this long after the last one, so a drag over the timeline is not a seek per pixel.
const SEEK_MIN_GAP: Duration = Duration::from_millis(120);
/// Shown pictures are at least this far apart: a sink presents at the display's rate, and what arrives
/// faster (2x playback) would queue up and fall behind.
const SHOW_MIN_GAP: Duration = Duration::from_millis(15);
/// Pausing reports the replay's last 10 Hz step, which lies a little behind the picture on screen. That
/// is no reason to go back to the keyframe.
const PAUSE_BACK_TOLERANCE_MS: i64 = 600;
/// The frontend's replay clock is a 100 ms timer and can run slow against this thread's; when the
/// difference is corrected, the picture holds for that long rather than restart at the keyframe.
const PLAY_BACK_TOLERANCE_MS: i64 = 1200;

/// One recording of the flight with its two sync anchors (video time ↔ flight time, ms).
#[derive(Clone, Debug)]
pub struct ReplayPart {
    pub path: PathBuf,
    pub a_video_ms: i64,
    pub a_flight_ms: i64,
    pub b_video_ms: i64,
    pub b_flight_ms: i64,
}

impl ReplayPart {
    /// Video time for a flight time inside this part.
    fn video_ms(&self, flight_ms: f64) -> Option<i64> {
        if flight_ms < self.a_flight_ms as f64 || flight_ms > self.b_flight_ms as f64 {
            return None;
        }
        let span = (self.b_flight_ms - self.a_flight_ms) as f64;
        let k = if span > 0.0 { (flight_ms - self.a_flight_ms as f64) / span } else { 0.0 };
        Some(self.a_video_ms + (k * (self.b_video_ms - self.a_video_ms) as f64).round() as i64)
    }
}

/// Index of the part that covers `flight_ms`.
pub fn part_at(parts: &[ReplayPart], flight_ms: f64) -> Option<usize> {
    parts.iter().position(|p| p.video_ms(flight_ms).is_some())
}

#[derive(Clone, Copy)]
struct Clock {
    flight_ms: f64,
    at: Instant,
    playing: bool,
    rate: f64,
    /// Set by the stop of a running replay, cleared by the next move.
    just_paused: bool,
}

impl Clock {
    fn now(&self) -> f64 {
        if self.playing { self.flight_ms + self.at.elapsed().as_secs_f64() * 1000.0 * self.rate } else { self.flight_ms }
    }
}

pub struct Replay {
    stop: Arc<AtomicBool>,
    /// The picture for the reported position is on screen. Cleared by a move, set by the thread.
    ready: Arc<AtomicBool>,
    /// Show the current picture again from this moment on (see `refresh`).
    redo: Arc<Mutex<Option<Instant>>>,
    clock: Arc<Mutex<Clock>>,
    parts: Vec<ReplayPart>,
    thread: Option<JoinHandle<()>>,
}

impl Replay {
    /// `push` hands one access unit to the decode sink (time in 90 kHz ticks, rising); `false` = decode
    /// it without showing it.
    pub fn start(parts: Vec<ReplayPart>, push: impl Fn(Vec<u8>, u64, bool) + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let clock = Arc::new(Mutex::new(Clock { flight_ms: 0.0, at: Instant::now(), playing: false, rate: 1.0, just_paused: false }));
        let ready = Arc::new(AtomicBool::new(false));
        let redo = Arc::new(Mutex::new(None));
        let thread = {
            let (stop, ready, redo, clock, parts) = (stop.clone(), ready.clone(), redo.clone(), clock.clone(), parts.clone());
            std::thread::Builder::new()
                .name("video-replay".into())
                .spawn(move || run(parts, clock, stop, ready, redo, push))
                .ok()
        };
        Self { stop, ready, redo, clock, parts, thread }
    }

    /// The replay's position. Returns whether a recording covers it (`false` = gap) and whether its
    /// picture is on screen already.
    pub fn sync(&self, flight_ms: f64, playing: bool, rate: f64) -> (bool, bool) {
        let mut c = self.clock.lock().unwrap();
        if (c.now() - flight_ms).abs() > CLOCK_TOLERANCE_MS * rate.max(1.0) {
            self.ready.store(false, Ordering::SeqCst);
        }
        let steady = c.playing && playing && (c.rate - rate).abs() < 1e-6 && (c.now() - flight_ms).abs() < CLOCK_TOLERANCE_MS * rate.max(1.0);
        if !steady {
            let just_paused = !playing && (c.playing || (c.just_paused && (c.flight_ms - flight_ms).abs() < 1.0));
            *c = Clock { flight_ms, at: Instant::now(), playing, rate, just_paused };
        }
        (part_at(&self.parts, flight_ms).is_some(), self.ready.load(Ordering::SeqCst))
    }
}

impl Replay {
    /// A video surface came on screen: a sink shows nothing on a surface until the next picture, and a
    /// paused replay sends none. Decode the current one again, once the sink has the surface.
    pub fn refresh(&self) {
        *self.redo.lock().unwrap() = Some(Instant::now() + Duration::from_millis(80));
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Open {
    part: usize,
    file: MkvFile,
    /// Time of the last picture handed over; `None` right after a seek.
    shown_ms: Option<i64>,
    /// Read ahead of the clock.
    pending: Option<Frame>,
}

fn run(
    parts: Vec<ReplayPart>,
    clock: Arc<Mutex<Clock>>,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    redo: Arc<Mutex<Option<Instant>>>,
    push: impl Fn(Vec<u8>, u64, bool),
) {
    let started = Instant::now();
    let mut open: Option<Open> = None;
    let mut bad_part: Option<usize> = None;
    let mut last_seek = Instant::now() - SEEK_MIN_GAP;
    let mut last_shown = Instant::now() - SHOW_MIN_GAP;
    // Every picture gets a time of its own: the sink tells the hidden ones apart by it.
    let mut last_ts = 0u64;
    let mut hand_over = |f: Frame, show: bool| {
        last_ts = ((started.elapsed().as_micros() as u64) * 9 / 100).max(last_ts + 1);
        push(f.annexb, last_ts, show);
        if show {
            ready.store(true, Ordering::SeqCst);
        }
    };

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(4));
        let c = *clock.lock().unwrap();
        let flight_ms = c.now();
        let Some(index) = part_at(&parts, flight_ms) else { continue };
        if bad_part == Some(index) {
            continue;
        }
        if open.as_ref().map(|o| o.part) != Some(index) {
            match MkvFile::open(&parts[index].path) {
                Ok(file) => open = Some(Open { part: index, file, shown_ms: None, pending: None }),
                Err(e) => {
                    log::warn!("[video] replay: {} cannot be read: {e}", parts[index].path.display());
                    bad_part = Some(index);
                    open = None;
                    continue;
                }
            }
        }
        let o = open.as_mut().unwrap();
        let Some(target) = parts[index].video_ms(flight_ms) else { continue };
        let mut forced = false;
        {
            let mut r = redo.lock().unwrap();
            if r.is_some_and(|at| Instant::now() >= at) {
                *r = None;
                o.shown_ms = None;
                o.pending = None;
                forced = true;
            }
        }
        let keyframes_only = c.playing && c.rate > KEYFRAMES_ONLY_ABOVE;

        if keyframes_only {
            // Fast forward: the newest keyframe at or before the target, once.
            let key = o.file.keyframe_before(target);
            if o.shown_ms != Some(key) {
                if o.file.seek_before(key).is_err() {
                    continue;
                }
                o.pending = None;
                if let Ok(Some(f)) = o.file.next_frame() {
                    o.shown_ms = Some(f.ms);
                    hand_over(f, true);
                }
            }
            continue;
        }

        // A running replay whose clock was set back a little waits for it instead of going back.
        let back_ok = if c.just_paused {
            PAUSE_BACK_TOLERANCE_MS
        } else if c.playing {
            PLAY_BACK_TOLERANCE_MS
        } else {
            0
        };
        let jump = match o.shown_ms {
            None => true,
            Some(shown) => target < shown - back_ok || target - shown > SKIP_AHEAD_MS,
        };
        if jump {
            if !forced && last_seek.elapsed() < SEEK_MIN_GAP {
                continue;
            }
            last_seek = Instant::now();
            if o.file.seek_before(target).is_err() {
                continue;
            }
            o.pending = None;
            o.shown_ms = None;
        }
        // Everything that is due. After a seek that is the run from the keyframe to the target: the
        // decoder needs every picture of it, but only the last one is for the screen.
        let mut handed = 0u64;
        loop {
            let next = match o.pending.take() {
                Some(f) => Some(f),
                None => o.file.next_frame().unwrap_or(None),
            };
            let Some(f) = next else { break };
            if f.ms > target {
                if o.shown_ms.is_none() {
                    // The target lies before the file's first picture: that one is the nearest.
                    o.shown_ms = Some(target);
                    hand_over(f, true);
                } else {
                    o.pending = Some(f);
                }
                break;
            }
            // One picture of look-ahead tells whether this is the newest that is due.
            o.pending = o.file.next_frame().unwrap_or(None);
            let newest = o.pending.as_ref().is_none_or(|n| n.ms > target);
            let show = newest && (c.rate <= 1.0 || last_shown.elapsed() >= SHOW_MIN_GAP);
            if show {
                last_shown = Instant::now();
            }
            o.shown_ms = Some(f.ms);
            hand_over(f, show);
            handed += 1;
            if handed > 2 {
                std::thread::sleep(Duration::from_micros(1_000_000 / CATCH_UP_FPS));
            }
            if stop.load(Ordering::SeqCst) || handed > 2 * CATCH_UP_FPS {
                break;
            }
        }
    }
}

/// What the frontend needs to show the picture: taken from the first readable part.
pub struct ReplayInfo {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    /// The first keyframe: the Linux sink chooses its decode route from the parameter sets.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub first_au: Vec<u8>,
}

pub fn probe(parts: &[ReplayPart]) -> Option<ReplayInfo> {
    parts.iter().find_map(|p| {
        let mut f = MkvFile::open(&p.path).ok()?;
        let first = f.next_frame().ok()??;
        Some(ReplayInfo { codec: f.codec, width: f.width, height: f.height, first_au: first.annexb })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(a_flight: i64, b_flight: i64, video_len: i64) -> ReplayPart {
        ReplayPart { path: PathBuf::new(), a_video_ms: 0, a_flight_ms: a_flight, b_video_ms: video_len, b_flight_ms: b_flight }
    }

    #[test]
    fn flight_time_maps_onto_the_part_that_covers_it() {
        let parts = [part(-2_000, 30_000, 32_000), part(45_000, 60_000, 15_030)];
        assert_eq!(part_at(&parts, -3_000.0), None);
        assert_eq!(part_at(&parts, 0.0), Some(0));
        assert_eq!(parts[0].video_ms(0.0), Some(2_000));
        assert_eq!(part_at(&parts, 40_000.0), None); // between the files
        assert_eq!(parts[1].video_ms(60_000.0), Some(15_030)); // the anchors absorb the clock drift
        assert_eq!(part_at(&parts, 60_001.0), None);
    }
}
