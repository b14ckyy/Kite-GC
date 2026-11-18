// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/frames.rs (2026-11-11)

//! The frame loop: marker byte → I/P/S/G/H/E frame, field decoding with encodings + predictors,
//! frame validation and resynchronisation. Logic after blackbox-tools v9.0.0 (`1918a75`)
//! `src/parser.c`: `parseFrame` (709-835), `parseIntraframe`…`parseSlowFrame` (884-923),
//! `completeIntraframe`…`completeSlowFrame` (1294-1466), the timestamp rollover (1263-1291) and the
//! data state of `flightLogParse` (1561-1672). Events are parsed through the schema layer instead
//! of the C decoder's fixed list, so the event types INAV writes no longer desynchronise the stream.
//!
//! Decoded values go to a [`FrameSink`] as `i64` slices — no allocation per frame.

use super::encodings::{self, Encoding};
use super::header::{is_frame_marker, FrameDef, InavHeader};
use super::predictors::{self, FrameRate, PredictContext, Predictor};
use super::schema::{EventEffect, EventLayout, InavSchema, PayloadKind, MAX_EVENT_ATTRS};
use super::stream::Stream;

/// A frame longer than this is corrupt (`FLIGHT_LOG_MAX_FRAME_LENGTH`).
pub const MAX_FRAME_LENGTH: usize = 256;
/// Largest plausible iteration jump between accepted main frames (`MAXIMUM_ITERATION_JUMP_BETWEEN_FRAMES`).
const MAX_ITERATION_JUMP: u32 = 5000;
/// Largest plausible time jump between accepted main frames, µs (`MAXIMUM_TIME_JUMP_BETWEEN_FRAMES`).
const MAX_TIME_JUMP_US: i64 = 10_000_000;
/// Fixed positions of `loopIteration` and `time` in the main frame (`FLIGHT_LOG_FIELD_INDEX_*`).
pub const INDEX_ITERATION: usize = 0;
pub const INDEX_TIME: usize = 1;
/// Corrupt-run warnings logged at `warn` level per log; later ones go to `debug`.
const MAX_CORRUPT_WARNINGS: u32 = 20;
/// Longest `End of log (…)` text searched for its NUL terminator.
const MAX_END_MESSAGE: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameKind {
    I,
    P,
    S,
    G,
    H,
    E,
}

impl FrameKind {
    pub const ALL: [FrameKind; 6] = [
        FrameKind::I,
        FrameKind::P,
        FrameKind::S,
        FrameKind::G,
        FrameKind::H,
        FrameKind::E,
    ];

    fn from_marker(b: u8) -> Option<Self> {
        Some(match b {
            b'I' => FrameKind::I,
            b'P' => FrameKind::P,
            b'S' => FrameKind::S,
            b'G' => FrameKind::G,
            b'H' => FrameKind::H,
            b'E' => FrameKind::E,
            _ => return None,
        })
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn marker(self) -> char {
        ['I', 'P', 'S', 'G', 'H', 'E'][self.index()]
    }
}

/// Counters per frame kind (see `table::DecodeStats` for the meaning).
#[derive(Clone, Copy, Default, Debug)]
pub struct KindCounters {
    pub ok: u64,
    pub corrupt: u64,
    pub dropped: u64,
    pub bytes: u64,
}

/// One decoded event attribute, in the type of its `events` column ([`PayloadKind::dtype`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EventValue {
    U32(u32),
    I32(i32),
    I64(i64),
    F32(f32),
}

/// One decoded event. `values[k]` belongs to `layout.attrs[k]`; `None` where the payload had none.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub layout: &'static EventLayout,
    pub values: [Option<EventValue>; MAX_EVENT_ATTRS],
}

/// Receives accepted frames in stream order.
pub trait FrameSink {
    /// A main (I or P) frame that passed validation; `values[INDEX_TIME]` is the 64-bit time.
    fn main_frame(&mut self, values: &[i64]);
    fn slow_frame(&mut self, values: &[i64]);
    /// `time_us`: the frame's own `time` field (rollover applied) when the header has one, else the
    /// time of the last accepted main frame; `None` before the first. `home_valid`: the last `H`
    /// frame arrived while the main stream was valid (blackbox-tools prints only those).
    fn gps_frame(&mut self, values: &[i64], time_us: Option<i64>, home_valid: bool);
    fn home_frame(&mut self, values: &[i64], time_us: Option<i64>, home_valid: bool);
    fn event(&mut self, event: &Event);
}

/// Field definitions resolved to predictors/encodings.
struct CompiledDef {
    count: usize,
    signed: Vec<bool>,
    predictor: Vec<Predictor>,
    encoding: Vec<Encoding>,
    /// For a TAG8_8SVB field: how many consecutive fields its group holds (`parseFrame`).
    svb_group: Vec<usize>,
    /// Read and discard one undeclared UVB before this field index (`== count`: after the last);
    /// see [`InavSchema::motor0_without_header`].
    extra_uvb_at: Option<usize>,
}

impl CompiledDef {
    fn new(def: &FrameDef, frame: char, schema: &InavSchema) -> Result<Self, String> {
        let count = def.count();
        let mut predictor = Vec::with_capacity(count);
        let mut encoding = Vec::with_capacity(count);
        for k in 0..count {
            let name = &def.names[k];
            let pred_id = schema
                .predictor_overrides
                .iter()
                .find(|(f, n, _)| *f == frame && n == name)
                .map_or(def.predictor[k], |(_, _, p)| *p);
            let mut pred = Predictor::from_id(pred_id).ok_or_else(|| {
                format!("unsupported predictor {pred_id} for {frame} field {name}")
            })?;
            if frame == 'S' && schema.slow_frames_absolute {
                pred = Predictor::Zero;
            }
            // INC reads nothing, so its encoding is irrelevant (the C parser never looks at it).
            let enc = if pred == Predictor::Inc {
                Encoding::Null
            } else {
                let id = def.encoding[k];
                Encoding::from_id(id)
                    .ok_or_else(|| format!("unsupported encoding {id} for {frame} field {name}"))?
            };
            predictor.push(pred);
            encoding.push(enc);
        }
        if frame == 'G' {
            predictors::resolve_home_pairs(&mut predictor);
        }
        let svb_group = (0..count)
            .map(|i| {
                let mut j = i + 1;
                while j < i + 8 && j < count && encoding[j] == Encoding::Tag8_8Svb {
                    j += 1;
                }
                j - i
            })
            .collect();
        Ok(CompiledDef {
            count,
            signed: def.signed.clone(),
            predictor,
            encoding,
            svb_group,
            extra_uvb_at: None,
        })
    }
}

/// History references for one frame decode.
struct History<'a> {
    prev: Option<&'a [i64]>,
    prev2: Option<&'a [i64]>,
    skipped: u32,
}

/// `parseFrame`: decode the fields of one frame into `out`.
fn decode_fields(
    s: &mut Stream,
    def: &CompiledDef,
    out: &mut [i64],
    hist: &History,
    ctx: &PredictContext,
    data_version: i32,
) {
    let mut vals = [0i64; 8];
    let mut extra_pending = def.extra_uvb_at.is_some();
    let mut i = 0;
    while i < def.count {
        if extra_pending && def.extra_uvb_at.is_some_and(|at| i >= at) {
            s.read_uvb();
            extra_pending = false;
        }
        if def.predictor[i] == Predictor::Inc {
            out[i] = predictors::increment(i, hist.prev, hist.skipped);
            i += 1;
            continue;
        }
        let group = match def.encoding[i] {
            Encoding::Tag8_4S16 => {
                if data_version < 2 {
                    encodings::read_tag8_4s16_v1(s, &mut vals);
                } else {
                    encodings::read_tag8_4s16_v2(s, &mut vals);
                }
                4
            }
            Encoding::Tag2_3S32 => {
                encodings::read_tag2_3s32(s, &mut vals);
                3
            }
            Encoding::Tag8_8Svb => {
                let n = def.svb_group[i];
                encodings::read_tag8_8svb(s, &mut vals, n);
                n
            }
            single => {
                let raw = match single {
                    Encoding::SignedVb => i64::from(s.read_svb()),
                    Encoding::UnsignedVb => i64::from(s.read_uvb()),
                    Encoding::Neg14Bit => encodings::read_neg_14bit(s),
                    _ => 0, // Null
                };
                let v =
                    predictors::apply(def.predictor[i], i, raw, out, hist.prev, hist.prev2, ctx);
                // The firmware's fields are 32 bit: wrap to the declared signedness (parseFrame).
                out[i] = if def.signed[i] {
                    i64::from(v as i32)
                } else {
                    i64::from(v as u32)
                };
                i += 1;
                continue;
            }
        };
        // Group encodings: no 32-bit wrap (as in the C parser); a group running past the last
        // field is read in full, the surplus values are discarded.
        for &raw in vals.iter().take(group) {
            if i < def.count {
                let v =
                    predictors::apply(def.predictor[i], i, raw, out, hist.prev, hist.prev2, ctx);
                out[i] = v;
            }
            i += 1;
        }
    }
    if extra_pending {
        s.read_uvb();
    }
}

/// Field names the firmware writes after the motors in the main frame (`blackboxMainFields`
/// order, 7.0.0-8.0.1): the undeclared `motor[0]` UVB sits before the first of them.
const AFTER_MOTORS: &[&str] = &[
    "servo[",
    "navState",
    "navFlags",
    "navEPH",
    "navEPV",
    "navPos[",
    "navVel[",
    "navTgtVel[",
    "navTgtPos[",
    "navTgtHdg",
    "navSurf",
    "navAcc[",
];

/// Indices of the 10.0 S fields whose write order differs from the header order.
#[derive(Clone, Copy, Debug)]
struct TerrainRemap {
    bus_off: usize,
    agl: usize,
    amsl: usize,
}

impl TerrainRemap {
    /// Present only when the header lists the three fields consecutively in the 10.0 order.
    fn find(def: &FrameDef) -> Option<Self> {
        let bus_off = def.index_of("droneCANBusOffCount")?;
        let agl = def.index_of("terrainAGL")?;
        let amsl = def.index_of("terrainAMSL")?;
        (agl == bus_off + 1 && amsl == agl + 1).then_some(TerrainRemap { bus_off, agl, amsl })
    }

    /// The header slots hold: bus-off slot = UVB-read zigzag(AGL), AGL slot = AMSL, AMSL slot =
    /// SVB-read busOff (firmware-researcher, INAV 10.0.0-rc2 blackbox.c).
    fn apply(&self, buf: &mut [i64]) {
        let agl = i64::from(super::stream::zigzag_decode(buf[self.bus_off] as u32));
        let amsl = buf[self.agl];
        let raw = buf[self.amsl] as i32;
        let bus_off = i64::from(((raw << 1) ^ (raw >> 31)) as u32);
        buf[self.agl] = agl;
        buf[self.amsl] = amsl;
        buf[self.bus_off] = bus_off;
    }
}

/// Decoder state for one log.
pub struct FrameDecoder {
    schema: &'static InavSchema,
    i_def: CompiledDef,
    p_def: CompiledDef,
    s_def: CompiledDef,
    g_def: CompiledDef,
    h_def: CompiledDef,
    rate: FrameRate,
    data_version: i32,
    minthrottle: i64,
    vbatref: i64,
    motor_output_low: i64,
    motor0: Option<usize>,
    home_idx: [Option<usize>; 2],
    g_time: Option<usize>,
    terrain_remap: Option<TerrainRemap>,

    /// Ring of three main frames; `cur` is decoded into, `prev`/`prev2` are the history.
    history: [Vec<i64>; 3],
    cur: usize,
    prev: Option<usize>,
    prev2: Option<usize>,
    main_valid: bool,
    last_iteration: Option<u32>,
    /// `-1` = none yet (C `lastMainFrameTime`).
    last_time: i64,
    /// Time of the last main frame handed to the sink.
    last_emitted_time: Option<i64>,
    rollover: [i64; 2],
    last_skipped: u32,

    home: [i64; 2],
    home_valid: bool,
    slow_buf: Vec<i64>,
    gps_buf: Vec<i64>,
    home_buf: Vec<i64>,
    pending_event: Option<Event>,

    pub counters: [KindCounters; 6],
    /// Base offset of the log within the file (log messages report file offsets).
    base_offset: usize,
    corrupt_run: Option<(usize, u32)>,
    corrupt_warnings: u32,
    unknown_events_warned: [bool; 256],
}

impl FrameDecoder {
    pub fn new(
        header: &InavHeader,
        schema: &'static InavSchema,
        base_offset: usize,
    ) -> Result<Self, String> {
        let mut i_def = CompiledDef::new(&header.i, 'I', schema)?;
        let p_def = CompiledDef::new(&header.p, 'P', schema)?;
        let s_def = CompiledDef::new(&header.s, 'S', schema)?;
        let g_def = CompiledDef::new(&header.g, 'G', schema)?;
        let h_def = CompiledDef::new(&header.h, 'H', schema)?;
        if i_def.count <= INDEX_TIME {
            return Err("main frame has fewer than two fields (loopIteration, time)".into());
        }
        if header.i.names[INDEX_ITERATION] != "loopIteration"
            || header.i.names[INDEX_TIME] != "time"
        {
            log::warn!(
                "INAV: main fields 0/1 are {:?}/{:?}, expected loopIteration/time — decoding them as such",
                header.i.names[INDEX_ITERATION],
                header.i.names[INDEX_TIME]
            );
        }
        let motor0 = header.i.index_of("motor[0]");
        if schema.motor0_without_header && motor0.is_none() {
            let at = header
                .i
                .names
                .iter()
                .position(|n| AFTER_MOTORS.iter().any(|a| n.starts_with(a)))
                .unwrap_or(i_def.count);
            i_def.extra_uvb_at = Some(at);
            log::info!(
                "INAV: {} log without motor[0] — assuming the undeclared motor[0] value in I-frames                  before field {at} (heuristic)",
                schema.label
            );
        }
        let terrain_remap = if schema.slow_terrain_remap {
            TerrainRemap::find(&header.s)
        } else {
            None
        };
        let uses_motor0 = i_def.predictor.contains(&Predictor::Motor0)
            || p_def.predictor.contains(&Predictor::Motor0);
        if uses_motor0 && motor0.is_none() {
            return Err("a field predicts from motor[0] but the log has no motor[0]".into());
        }
        let home_idx = [
            header.h.index_of("GPS_home[0]"),
            header.h.index_of("GPS_home[1]"),
        ];
        if g_def
            .predictor
            .iter()
            .any(|p| matches!(p, Predictor::HomeCoord(_)))
            && home_idx.iter().any(Option::is_none)
        {
            return Err(
                "GPS fields predict from the home position but the log has no GPS_home fields"
                    .into(),
            );
        }
        let n = i_def.count;
        Ok(FrameDecoder {
            schema,
            rate: FrameRate::new(
                header.i_interval,
                header.p_interval_num,
                header.p_interval_denom,
            ),
            data_version: header.data_version,
            minthrottle: header.minthrottle,
            vbatref: header.vbatref,
            motor_output_low: header.motor_output.0,
            motor0,
            home_idx,
            g_time: header.g.index_of("time"),
            terrain_remap,
            history: [vec![0; n], vec![0; n], vec![0; n]],
            cur: 0,
            prev: None,
            prev2: None,
            main_valid: false,
            last_iteration: None,
            last_time: -1,
            last_emitted_time: None,
            rollover: [0; 2],
            last_skipped: 0,
            home: [0; 2],
            home_valid: false,
            slow_buf: vec![0; s_def.count],
            gps_buf: vec![0; g_def.count],
            home_buf: vec![0; h_def.count],
            pending_event: None,
            i_def,
            p_def,
            s_def,
            g_def,
            h_def,
            counters: [KindCounters::default(); 6],
            base_offset,
            corrupt_run: None,
            corrupt_warnings: 0,
            unknown_events_warned: [false; 256],
        })
    }

    fn context(&self) -> PredictContext {
        PredictContext {
            minthrottle: self.minthrottle,
            vbatref: self.vbatref,
            motor_output_low: self.motor_output_low,
            motor0: self.motor0,
            home: self.home,
            last_main_time: self.prev.map(|p| self.history[p][INDEX_TIME]),
        }
    }

    /// Decode every frame of `log` (one log's bytes) from `data_start` on.
    pub fn run(&mut self, log: &[u8], data_start: usize, sink: &mut impl FrameSink) {
        let mut s = Stream::new(log);
        s.set_pos(data_start);
        let mut last: Option<(FrameKind, usize)> = None;
        let mut premature_eof = false;
        loop {
            let command = s.read_byte();
            if let Some((kind, start)) = last {
                // −1: one byte of the next frame (or the EOF probe) is already read.
                let size = s.pos().saturating_sub(1).saturating_sub(start);
                let next_is_frame = command.is_some_and(is_frame_marker);
                let looks_complete = next_is_frame || (!premature_eof && command.is_none());
                if size <= MAX_FRAME_LENGTH && looks_complete {
                    let accepted = self.complete(kind, sink);
                    let c = &mut self.counters[kind.index()];
                    if accepted {
                        c.ok += 1;
                        c.bytes += size as u64;
                        self.end_corrupt_run(start);
                    } else {
                        c.dropped += 1;
                    }
                } else {
                    // Corrupt: resynchronise from the byte after this frame's marker.
                    self.main_valid = false;
                    self.counters[kind.index()].corrupt += 1;
                    let run = self.corrupt_run.get_or_insert((start, 0));
                    run.1 += 1;
                    s.set_pos(start + 1);
                    s.clear_eof();
                    last = None;
                    premature_eof = false;
                    continue;
                }
            }
            let Some(marker) = command else {
                break;
            };
            let start = s.pos() - 1;
            let kind = FrameKind::from_marker(marker);
            match kind {
                Some(k) => self.parse(k, &mut s, start),
                // Not a frame start: garbage between frames.
                None => self.main_valid = false,
            }
            if s.eof() {
                premature_eof = true;
            }
            last = kind.map(|k| (k, start));
        }
        self.end_corrupt_run(s.end());
    }

    fn end_corrupt_run(&mut self, at: usize) {
        let Some((start, frames)) = self.corrupt_run.take() else {
            return;
        };
        let (from, to) = (self.base_offset + start, self.base_offset + at);
        if self.corrupt_warnings < MAX_CORRUPT_WARNINGS {
            log::warn!(
                "INAV: corrupt data at bytes {from}..{to} ({frames} frame start(s) rejected), resynchronised"
            );
        } else {
            log::debug!(
                "INAV: corrupt data at bytes {from}..{to} ({frames} frame start(s) rejected)"
            );
        }
        self.corrupt_warnings += 1;
    }

    fn parse(&mut self, kind: FrameKind, s: &mut Stream, start: usize) {
        match kind {
            FrameKind::I | FrameKind::P => self.parse_main(kind, s),
            FrameKind::S => {
                let mut buf = std::mem::take(&mut self.slow_buf);
                self.parse_standalone(&self.s_def, s, &mut buf);
                self.slow_buf = buf;
            }
            FrameKind::G => {
                let mut buf = std::mem::take(&mut self.gps_buf);
                self.parse_standalone(&self.g_def, s, &mut buf);
                self.gps_buf = buf;
            }
            FrameKind::H => {
                let mut buf = std::mem::take(&mut self.home_buf);
                self.parse_standalone(&self.h_def, s, &mut buf);
                self.home_buf = buf;
            }
            FrameKind::E => self.pending_event = self.parse_event(s, start),
        }
    }

    fn parse_main(&mut self, kind: FrameKind, s: &mut Stream) {
        let mut out = std::mem::take(&mut self.history[self.cur]);
        let ctx = self.context();
        let (def, hist) = if kind == FrameKind::I {
            let hist = History {
                prev: self.prev.map(|p| &self.history[p][..]),
                prev2: None,
                skipped: 0,
            };
            (&self.i_def, hist)
        } else {
            self.last_skipped = self.rate.skipped_after(self.last_iteration);
            let hist = History {
                prev: self.prev.map(|p| &self.history[p][..]),
                prev2: self.prev2.map(|p| &self.history[p][..]),
                skipped: self.last_skipped,
            };
            (&self.p_def, hist)
        };
        decode_fields(s, def, &mut out, &hist, &ctx, self.data_version);
        self.history[self.cur] = out;
    }

    /// S/G/H frames: no history (`parseSlowFrame` & co. pass no previous frame).
    fn parse_standalone(&self, def: &CompiledDef, s: &mut Stream, out: &mut [i64]) {
        let hist = History {
            prev: None,
            prev2: None,
            skipped: 0,
        };
        decode_fields(s, def, out, &hist, &self.context(), self.data_version);
    }

    /// An event per the schema; `None` for an unknown type or an implausible payload (the frame
    /// is then dropped, or treated as corrupt when no frame start follows — as `parseEventFrame`
    /// does with types it does not know).
    fn parse_event(&mut self, s: &mut Stream, start: usize) -> Option<Event> {
        let id = s.read_u8();
        let Some(layout) = self.schema.event(id) else {
            // Worth a warning only in a healthy stream; while resynchronising through garbage
            // any 'E' byte looks like an event.
            if self.main_valid && !self.unknown_events_warned[usize::from(id)] {
                self.unknown_events_warned[usize::from(id)] = true;
                log::warn!(
                    "INAV: unknown event type {id} at byte {} ({}), skipped",
                    self.base_offset + start,
                    self.schema.label
                );
            } else {
                log::debug!(
                    "INAV: unknown event type {id} at byte {}, skipped",
                    self.base_offset + start
                );
            }
            return None;
        };
        let mut values = [None; MAX_EVENT_ATTRS];
        let mut float_value = false;
        for (slot, attr) in values.iter_mut().zip(layout.attrs) {
            *slot = match attr.kind {
                PayloadKind::Uvb => Some(EventValue::U32(s.read_uvb())),
                PayloadKind::UvbTime => {
                    Some(EventValue::I64(i64::from(s.read_uvb()) + self.rollover[0]))
                }
                PayloadKind::AdjustmentFunction => {
                    let b = s.read_u8();
                    float_value = b > 127;
                    Some(EventValue::U32(u32::from(b & 0x7F)))
                }
                PayloadKind::AdjustmentValue => {
                    (!float_value).then(|| EventValue::I32(s.read_svb()))
                }
                PayloadKind::AdjustmentFloat => {
                    float_value.then(|| EventValue::F32(s.read_f32_le()))
                }
                PayloadKind::EndOfLog => {
                    let reason = read_end_of_log(s)?;
                    s.truncate_here();
                    reason.map(EventValue::I32)
                }
            };
        }
        Some(Event { layout, values })
    }

    fn complete(&mut self, kind: FrameKind, sink: &mut impl FrameSink) -> bool {
        match kind {
            FrameKind::I => self.complete_main(true, sink),
            FrameKind::P => self.complete_main(false, sink),
            FrameKind::E => match self.pending_event.take() {
                Some(event) => {
                    if event.layout.effect == EventEffect::LoggingResume {
                        // Accept the jump into the future (`completeEventFrame`).
                        if let [Some(EventValue::U32(iteration)), Some(EventValue::I64(time)), ..] =
                            event.values
                        {
                            self.last_iteration = Some(iteration);
                            self.last_time = time;
                        }
                    }
                    sink.event(&event);
                    true
                }
                None => false,
            },
            FrameKind::H => {
                for (k, idx) in self.home_idx.iter().enumerate() {
                    if let Some(i) = idx {
                        self.home[k] = self.home_buf[*i];
                    }
                }
                self.home_valid = self.main_valid;
                sink.home_frame(&self.home_buf, self.last_emitted_time, self.home_valid);
                true
            }
            FrameKind::G => {
                let time = match self.g_time {
                    Some(t) => {
                        self.gps_buf[t] = self.apply_rollover(self.gps_buf[t], 1);
                        Some(self.gps_buf[t])
                    }
                    None => self.last_emitted_time,
                };
                sink.gps_frame(&self.gps_buf, time, self.home_valid);
                true
            }
            FrameKind::S => {
                if let Some(remap) = self.terrain_remap {
                    remap.apply(&mut self.slow_buf);
                }
                sink.slow_frame(&self.slow_buf);
                true
            }
        }
    }

    /// `completeIntraframe` / `completeInterframe`.
    fn complete_main(&mut self, intra: bool, sink: &mut impl FrameSink) -> bool {
        let t = self.history[self.cur][INDEX_TIME];
        self.history[self.cur][INDEX_TIME] = self.apply_rollover(t, 0);
        if intra {
            // Only an I-frame can re-validate the stream; it is checked against the last accepted
            // frame when there is one.
            if self.last_iteration.is_some() && !self.plausible() {
                self.invalidate();
            } else {
                self.main_valid = true;
            }
        } else if self.main_valid && !self.plausible() {
            self.invalidate();
        }
        if !self.main_valid {
            return false;
        }
        let frame = &self.history[self.cur];
        self.last_iteration = Some(frame[INDEX_ITERATION] as u32);
        self.last_time = frame[INDEX_TIME];
        self.last_emitted_time = Some(frame[INDEX_TIME]);
        sink.main_frame(frame);
        if intra {
            // Both history slots become the I-frame: nothing older may be used for prediction.
            self.prev = Some(self.cur);
            self.prev2 = Some(self.cur);
        } else {
            self.prev2 = self.prev;
            self.prev = Some(self.cur);
        }
        self.cur = (self.cur + 1) % 3;
        true
    }

    /// `flightLogValidateMainFrameValues`: iteration and time did not move backwards and did not
    /// jump implausibly far.
    fn plausible(&self) -> bool {
        let frame = &self.history[self.cur];
        let iteration = frame[INDEX_ITERATION] as u32;
        let time = frame[INDEX_TIME];
        let last_iteration = self.last_iteration.unwrap_or(u32::MAX);
        iteration >= last_iteration
            && iteration < last_iteration.wrapping_add(MAX_ITERATION_JUMP)
            && time >= self.last_time
            && time < self.last_time + MAX_TIME_JUMP_US
    }

    /// `flightLogInvalidateStream`.
    fn invalidate(&mut self) {
        self.main_valid = false;
        self.prev = None;
        self.prev2 = None;
    }

    /// `flightLogDetectAndApplyTimestampRollover`: the 32-bit µs clock wrapped when time appears to
    /// go backwards modulo 2^32 but by a small forward step.
    fn apply_rollover(&mut self, timestamp: i64, id: usize) -> i64 {
        if self.last_time != -1 {
            let t = timestamp as u32;
            let last = self.last_time as u32;
            if t < last && t.wrapping_sub(last) < MAX_TIME_JUMP_US as u32 {
                self.rollover[id] += 1 << 32;
            }
        }
        i64::from(timestamp as u32) + self.rollover[id]
    }
}

/// `End of log (disarm reason:N)` + NUL. `None` = not an end-of-log text (the event is invalid);
/// `Some(None)` = the text without a reason (older firmware writes `End of log` only).
fn read_end_of_log(s: &mut Stream) -> Option<Option<i32>> {
    let mut text = Vec::with_capacity(MAX_END_MESSAGE);
    loop {
        match s.read_byte()? {
            0 => break,
            b if text.len() < MAX_END_MESSAGE => text.push(b),
            _ => return None,
        }
    }
    let text = std::str::from_utf8(&text).ok()?;
    let rest = text.strip_prefix("End of log")?;
    let reason = rest
        .split_once("reason:")
        .and_then(|(_, r)| r.trim_end_matches(')').trim().parse().ok());
    Some(reason)
}

#[cfg(test)]
mod tests;
