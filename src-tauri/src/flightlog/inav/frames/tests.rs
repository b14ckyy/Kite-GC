// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/frames/tests.rs (2026-11-11)
// Deviation: the `decode` and `Column` imports are gone (the Analyzer's table layer is not ported); `log_slice` comes from Kite's `telemetry`
// Deviation: `table_groups_and_time_bases` and `values_beyond_f32_precision_are_exact` left out — they check the Analyzer's table layer (time bases, column types)
// Deviation: `multi_log_split` splits with `telemetry::log_slice` and decodes each slice with `run` (was `decode`)

//! Synthetic logs, built with the firmware's writer logic: history rule, INC with skipped frames,
//! validation/resync, time rollover, events, multi-log split and the table's time bases.

use super::super::header::parse_header;
use super::super::schema::schema_for;
use super::super::stream::tests::{write_svb, write_uvb};
use super::*;
use super::super::telemetry::log_slice;

/// Header: I = loopIteration (UVB), time (UVB), a (signed SVB); P = INC, STRAIGHT_LINE, PREVIOUS;
/// S = flags (UVB) + `extra` S fields of UVB (to build an over-long frame).
fn header(p_interval: &str, s_extra: usize) -> Vec<u8> {
    let mut s_names = vec!["flags".to_string()];
    s_names.extend((0..s_extra).map(|k| format!("x{k}")));
    let n = s_names.len();
    let zeros = vec!["0"; n].join(",");
    let ones = vec!["1"; n].join(",");
    format!(
        "H Product:Blackbox flight data recorder by Nicholas Sherlock\n\
         H Data version:2\n\
         H I interval:4\n\
         H Field I name:loopIteration,time,a\n\
         H Field I signed:0,0,1\n\
         H Field I predictor:0,0,0\n\
         H Field I encoding:1,1,0\n\
         H Field P predictor:6,2,1\n\
         H Field P encoding:9,0,0\n\
         H Field S name:{}\n\
         H Field S signed:{zeros}\n\
         H Field S predictor:{zeros}\n\
         H Field S encoding:{ones}\n\
         H Firmware type:Cleanflight\n\
         H Firmware revision:INAV 9.1.0 (d17f2b3e) TEST\n\
         H P interval:{p_interval}\n",
        s_names.join(",")
    )
    .into_bytes()
}

/// Mirrors the firmware's main-frame history to write I/P frames.
struct Writer {
    bytes: Vec<u8>,
    prev: Option<(u32, u32, i32)>,
    prev2: Option<(u32, u32, i32)>,
}

impl Writer {
    fn new(header: Vec<u8>) -> Self {
        Writer {
            bytes: header,
            prev: None,
            prev2: None,
        }
    }

    fn i(&mut self, iteration: u32, time: u32, a: i32) -> &mut Self {
        self.bytes.push(b'I');
        write_uvb(&mut self.bytes, iteration);
        write_uvb(&mut self.bytes, time);
        write_svb(&mut self.bytes, a);
        self.prev = Some((iteration, time, a));
        self.prev2 = self.prev;
        self
    }

    /// P frame for `time`/`a` (the iteration is implied by INC).
    fn p(&mut self, iteration: u32, time: u32, a: i32) -> &mut Self {
        let (_, t1, a1) = self.prev.unwrap();
        let (_, t2, _) = self.prev2.unwrap();
        self.bytes.push(b'P');
        // Firmware: 32-bit arithmetic, `time - (2·prev − prev2)` as a signed VB.
        let predicted = t1.wrapping_mul(2).wrapping_sub(t2);
        write_svb(&mut self.bytes, time.wrapping_sub(predicted) as i32);
        write_svb(&mut self.bytes, a - a1);
        self.prev2 = self.prev;
        self.prev = Some((iteration, time, a));
        self
    }

    fn s(&mut self, values: &[u32]) -> &mut Self {
        self.bytes.push(b'S');
        for &v in values {
            write_uvb(&mut self.bytes, v);
        }
        self
    }

    fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(b);
        self
    }

    fn flight_mode(&mut self, flags: u32, last: u32) -> &mut Self {
        self.bytes.extend_from_slice(&[b'E', 30]);
        write_uvb(&mut self.bytes, flags);
        write_uvb(&mut self.bytes, last);
        self
    }

    fn log_end(&mut self, reason: u32) -> &mut Self {
        self.bytes.extend_from_slice(&[b'E', 255]);
        self.bytes
            .extend_from_slice(format!("End of log (disarm reason:{reason})").as_bytes());
        self.bytes.push(0);
        self
    }
}

#[derive(Default)]
struct Collect {
    main: Vec<Vec<i64>>,
    slow: Vec<Vec<i64>>,
    events: Vec<(u8, [Option<EventValue>; MAX_EVENT_ATTRS])>,
}

impl FrameSink for Collect {
    fn main_frame(&mut self, values: &[i64]) {
        self.main.push(values.to_vec());
    }
    fn slow_frame(&mut self, values: &[i64]) {
        self.slow.push(values.to_vec());
    }
    fn gps_frame(&mut self, _: &[i64], _: Option<i64>, _: bool) {}
    fn home_frame(&mut self, _: &[i64], _: Option<i64>, _: bool) {}
    fn event(&mut self, e: &Event) {
        self.events.push((e.layout.id, e.values));
    }
}

fn run(bytes: &[u8]) -> (Collect, [KindCounters; 6]) {
    let h = parse_header(bytes).unwrap();
    let (schema, _) = schema_for(h.firmware_version);
    let mut dec = FrameDecoder::new(&h, schema, 0).unwrap();
    let mut sink = Collect::default();
    dec.run(bytes, h.data_start, &mut sink);
    (sink, dec.counters)
}

fn rows(c: &Collect) -> Vec<(i64, i64, i64)> {
    c.main.iter().map(|r| (r[0], r[1], r[2])).collect()
}

#[test]
fn i_frame_history_and_straight_line() {
    let mut w = Writer::new(header("1/1", 0));
    // After the I-frame both history slots are the I-frame: the first P predicts time 1000.
    w.i(0, 1000, 5)
        .p(1, 2000, 7)
        .p(2, 3000, 4)
        .p(3, 4100, 4)
        .i(4, 5000, -3)
        .p(5, 6000, -1);
    let (c, counters) = run(&w.bytes);
    assert_eq!(
        rows(&c),
        [
            (0, 1000, 5),
            (1, 2000, 7),
            (2, 3000, 4),
            (3, 4100, 4),
            (4, 5000, -3),
            (5, 6000, -1)
        ]
    );
    assert_eq!(counters[FrameKind::I.index()].ok, 2);
    assert_eq!(counters[FrameKind::P.index()].ok, 4);
}

#[test]
fn inc_counts_skipped_iterations() {
    // P interval 1/2, I interval 4: iterations 0, 2 | 4, 6 are logged.
    let mut w = Writer::new(header("1/2", 0));
    w.i(0, 0, 0).p(2, 2000, 1).i(4, 4000, 2).p(6, 6000, 3);
    let (c, _) = run(&w.bytes);
    assert_eq!(
        rows(&c),
        [(0, 0, 0), (2, 2000, 1), (4, 4000, 2), (6, 6000, 3)]
    );
}

#[test]
fn over_long_frame_is_corrupt_and_p_frames_wait_for_an_i_frame() {
    let mut w = Writer::new(header("1/1", 300));
    w.i(0, 1000, 1).p(1, 2000, 2);
    let mut big = vec![0u32; 301];
    big[0] = 9;
    w.s(&big);
    // The firmware keeps writing P-frames; the decoder must drop them until the next I-frame.
    w.p(2, 3000, 3).i(4, 5000, 4).p(5, 6000, 5);
    let (c, counters) = run(&w.bytes);
    assert_eq!(
        rows(&c),
        [(0, 1000, 1), (1, 2000, 2), (4, 5000, 4), (5, 6000, 5)]
    );
    assert_eq!(counters[FrameKind::S.index()].corrupt, 1);
    assert_eq!(counters[FrameKind::P.index()].dropped, 1);
    assert!(c.slow.is_empty());
}

#[test]
fn bad_next_marker_resyncs() {
    let mut w = Writer::new(header("1/1", 0));
    w.i(0, 1000, 1)
        .p(1, 2000, 2)
        .raw(&[0x01, 0x02])
        .p(2, 3000, 3)
        .i(4, 5000, 4);
    let (c, counters) = run(&w.bytes);
    // The P before the garbage has no valid successor → corrupt; the P after it is dropped.
    assert_eq!(rows(&c), [(0, 1000, 1), (4, 5000, 4)]);
    assert_eq!(counters[FrameKind::P.index()].corrupt, 1);
    assert_eq!(counters[FrameKind::P.index()].dropped, 1);
}

#[test]
fn implausible_jump_invalidates_until_a_plausible_i_frame() {
    let mut w = Writer::new(header("1/1", 0));
    // Time jumps 20 s between P frames → invalid; the next I-frame (checked against the last
    // accepted frame, 2000 µs) is again out of range; the clean log resumes only near it.
    w.i(0, 1000, 1)
        .p(1, 2000, 2)
        .p(2, 20_002_000, 3)
        .p(3, 20_003_000, 3);
    let (c, counters) = run(&w.bytes);
    assert_eq!(rows(&c), [(0, 1000, 1), (1, 2000, 2)]);
    assert_eq!(counters[FrameKind::P.index()].dropped, 2);
}

#[test]
fn time_rollover() {
    let mut w = Writer::new(header("1/1", 0));
    w.i(0, 0xFFFF_FF00, 0)
        .p(1, 0x0000_0100, 0)
        .p(2, 0x0000_0300, 0)
        .i(4, 0x0000_0500, 0);
    let (c, _) = run(&w.bytes);
    let t: Vec<i64> = c.main.iter().map(|r| r[1]).collect();
    assert_eq!(
        t,
        [0xFFFF_FF00, 0x1_0000_0100, 0x1_0000_0300, 0x1_0000_0500]
    );
}

#[test]
fn events_flight_mode_and_log_end() {
    let mut w = Writer::new(header("1/1", 0));
    w.i(0, 1000, 1)
        .flight_mode(0b101, 0b001)
        .s(&[7])
        .p(1, 2000, 2)
        .raw(&[b'E', 99]) // unknown type followed by a frame start: dropped, stream stays valid
        .p(2, 3000, 3)
        .log_end(4)
        .raw(b"Ijunk after the end");
    let (c, counters) = run(&w.bytes);
    assert_eq!(rows(&c), [(0, 1000, 1), (1, 2000, 2), (2, 3000, 3)]);
    assert_eq!(c.events.len(), 2);
    assert_eq!(c.events[0].0, 30);
    assert_eq!(
        c.events[0].1,
        [Some(EventValue::U32(5)), Some(EventValue::U32(1)), None]
    );
    assert_eq!(c.events[1].0, 255);
    assert_eq!(c.events[1].1, [Some(EventValue::I32(4)), None, None]);
    assert_eq!(c.slow, [vec![7]]);
    assert_eq!(counters[FrameKind::E.index()].dropped, 1);
    assert_eq!(counters[FrameKind::I.index()].corrupt, 0);
}

#[test]
fn multi_log_split() {
    let mut first = Writer::new(header("1/1", 0));
    first.i(0, 1000, 1).log_end(0);
    let mut second = Writer::new(header("1/1", 0));
    second.i(0, 7000, 9).p(1, 8000, 10);
    let mut bytes = first.bytes.clone();
    bytes.extend_from_slice(&second.bytes);
    let times = |index: usize| {
        let (range, _) = log_slice(&bytes, index).unwrap();
        let (c, _) = run(&bytes[range]);
        c.main.iter().map(|r| r[1]).collect::<Vec<i64>>()
    };
    assert_eq!(times(0), [1000]);
    assert_eq!(times(1), [7000, 8000]);
    assert!(log_slice(&bytes, 2).is_err());
}

/// A log with I fields `loopIteration,time` (unsigned UVB) + signed SVB fields, and four S fields
/// (signed SVB, unsigned UVB, signed SVB, signed SVB).
fn custom_log(revision: &str, i_fields: &str, s_fields: &str) -> Vec<u8> {
    let extra = i_fields.split(',').count() - 2;
    let rep = |v: &str| vec![v; extra].join(",");
    format!(
        "H Product:Blackbox flight data recorder by Nicholas Sherlock
         H Data version:2
         H I interval:4
         H Field I name:{i_fields}
         H Field I signed:0,0,{}
         H Field I predictor:0,0,{}
         H Field I encoding:1,1,{}
         H Field P predictor:6,2,{}
         H Field P encoding:9,0,{}
         H Field S name:{s_fields}
         H Field S signed:1,0,1,1
         H Field S predictor:1,0,0,0
         H Field S encoding:0,1,0,0
         H Firmware revision:{revision}
",
        rep("1"),
        rep("0"),
        rep("0"),
        rep("1"),
        rep("0"),
    )
    .into_bytes()
}

#[test]
fn inav_8_undeclared_motor0_is_skipped() {
    // 8.0.x without motors in the header: the I-frame still carries motor[0] − minthrottle as one
    // UVB between the last pre-motor field (debug[0]) and the servos.
    let header = custom_log(
        "INAV 8.0.1 (abc) T",
        "loopIteration,time,debug[0],servo[0]",
        "flags,a,b,c",
    );
    for (revision, skip_expected) in [("8.0.1", true), ("9.0.0", false)] {
        let mut bytes = String::from_utf8(header.clone())
            .unwrap()
            .replace("8.0.1", revision)
            .into_bytes();
        bytes.push(b'I');
        write_uvb(&mut bytes, 0);
        write_uvb(&mut bytes, 1000);
        write_svb(&mut bytes, -5);
        if skip_expected {
            write_uvb(&mut bytes, 300); // the undeclared motor[0]
        }
        write_svb(&mut bytes, 77);
        bytes.push(b'I');
        let (c, _) = run(&bytes);
        assert_eq!(c.main, [vec![0, 1000, -5, 77]], "{revision}");
    }
}

#[test]
fn inav_10_terrain_fields_are_remapped() {
    // Header order escTemperature, droneCANBusOffCount, terrainAGL, terrainAMSL; written in the
    // order escTemperature, terrainAGL, terrainAMSL, droneCANBusOffCount.
    let mut bytes = custom_log(
        "INAV 10.0.0 (abc) T",
        "loopIteration,time,a",
        "escTemperature,droneCANBusOffCount,terrainAGL,terrainAMSL",
    );
    bytes.push(b'S');
    write_svb(&mut bytes, -3); // escTemperature
    write_svb(&mut bytes, -1234); // terrainAGL
    write_svb(&mut bytes, 56_789); // terrainAMSL
    write_uvb(&mut bytes, 7); // droneCANBusOffCount
    bytes.push(b'I');
    write_uvb(&mut bytes, 0);
    write_uvb(&mut bytes, 1000);
    write_svb(&mut bytes, 1);
    let (c, _) = run(&bytes);
    assert_eq!(c.slow, [vec![-3, 7, -1234, 56_789]]);
}
