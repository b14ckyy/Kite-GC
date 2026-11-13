// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/coverage_test.rs (2026-11-11)
// Deviation: decodes through `FrameDecoder` + the `Collect` sink (`decode_frames`) instead of `decode_observed`; the frame counters replace `table.stats`
// Deviation: `check_table` (the Analyzer's table layer) left out, with its import of `logs::table`

//! Synthetic full-coverage test: a log that declares EVERY field INAV 9.1.0 can write (all
//! `FLIGHT_LOG_FIELD_CONDITION_*` true, every `#ifdef` on), plus random subsets of the condition
//! groups, encoded with the firmware's writer logic and decoded — every value of every frame must
//! come back exactly. Independent of what the real test logs happen to contain.
//!
//! The field tables are generated from INAV 9.1.0 `src/main/blackbox/blackbox.c`
//! (`blackboxMainFields` lines 205-399, `blackboxGpsGFields` 402-417, `blackboxGpsHFields` 420-423,
//! `blackboxSlowFields` 427-470). The writer mirrors `writeIntraframe` (852-1055), `writeInterframe`
//! (1085-1311: one `blackboxWriteTag8_8SVB` call for the whole optional group, so with all nine
//! optional fields present the ninth has no header bit — quirk documented in `schema.rs`; the test
//! keeps that field's delta non-zero, as the decoder, like blackbox-tools, groups 8 + 1),
//! `writeSlowFrame` (1313-1365), `writeGPSHomeFrame`/`writeGPSFrame` (1582-1628),
//! `blackboxLogEvent` (2023-2064) and the frame order of `blackboxLogIteration` (2122-2172), with
//! the `blackbox_encoding.c` encoders (`encodings::tests`).

use super::frames::{FrameDecoder, KindCounters};
use super::header::parse_header;
use super::encodings::tests::{write_tag2_3s32, write_tag8_4s16, write_tag8_8svb};
use super::frames::{Event, EventValue, FrameSink};
use super::schema::schema_for;
use super::stream::tests::{write_svb, write_uvb};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cond {
    Always,
    PidD(u8),
    FixedWingNav,
    McNav,
    RcData,
    RcCommand,
    Vbat,
    Amperage,
    Mag,
    Baro,
    Pitot,
    Surface,
    Rssi,
    GyroRaw,
    GyroPeaksRoll,
    GyroPeaksPitch,
    GyroPeaksYaw,
    Acc,
    Attitude,
    Debug,
    Motors(u8),
    Servos(u8),
    NavPos,
    NavAcc,
}

/// One main field: name, signed, I predictor, I encoding, P predictor, P encoding (header values),
/// condition.
struct M(&'static str, bool, i32, i32, i32, i32, Cond);

/// `blackboxMainFields`, INAV 9.1.0 (generated from the source).
const MAIN_9_1: &[M] = &[
    M("loopIteration", false, 0, 1, 6, 9, Cond::Always),
    M("time", false, 0, 1, 2, 0, Cond::Always),
    M("axisRate[0]", true, 0, 0, 1, 0, Cond::Always),
    M("axisRate[1]", true, 0, 0, 1, 0, Cond::Always),
    M("axisRate[2]", true, 0, 0, 1, 0, Cond::Always),
    M("axisP[0]", true, 0, 0, 1, 0, Cond::Always),
    M("axisP[1]", true, 0, 0, 1, 0, Cond::Always),
    M("axisP[2]", true, 0, 0, 1, 0, Cond::Always),
    M("axisI[0]", true, 0, 0, 1, 7, Cond::Always),
    M("axisI[1]", true, 0, 0, 1, 7, Cond::Always),
    M("axisI[2]", true, 0, 0, 1, 7, Cond::Always),
    M("axisD[0]", true, 0, 0, 1, 0, Cond::PidD(0)),
    M("axisD[1]", true, 0, 0, 1, 0, Cond::PidD(1)),
    M("axisD[2]", true, 0, 0, 1, 0, Cond::PidD(2)),
    M("axisF[0]", true, 0, 0, 1, 0, Cond::Always),
    M("axisF[1]", true, 0, 0, 1, 0, Cond::Always),
    M("axisF[2]", true, 0, 0, 1, 0, Cond::Always),
    M("fwAltP", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwAltI", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwAltD", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwAltOut", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwPosP", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwPosI", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwPosD", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("fwPosOut", true, 0, 0, 1, 0, Cond::FixedWingNav),
    M("mcPosAxisP[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcPosAxisP[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcPosAxisP[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisP[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisP[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisP[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisI[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisI[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisI[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisD[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisD[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisD[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisFF[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisFF[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisFF[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisOut[0]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisOut[1]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcVelAxisOut[2]", true, 0, 0, 1, 0, Cond::McNav),
    M("mcSurfaceP", true, 0, 0, 1, 0, Cond::McNav),
    M("mcSurfaceI", true, 0, 0, 1, 0, Cond::McNav),
    M("mcSurfaceD", true, 0, 0, 1, 0, Cond::McNav),
    M("mcSurfaceOut", true, 0, 0, 1, 0, Cond::McNav),
    M("rcData[0]", true, 0, 0, 1, 8, Cond::RcData),
    M("rcData[1]", true, 0, 0, 1, 8, Cond::RcData),
    M("rcData[2]", true, 0, 0, 1, 8, Cond::RcData),
    M("rcData[3]", true, 0, 0, 1, 8, Cond::RcData),
    M("rcCommand[0]", true, 0, 0, 1, 8, Cond::RcCommand),
    M("rcCommand[1]", true, 0, 0, 1, 8, Cond::RcCommand),
    M("rcCommand[2]", true, 0, 0, 1, 8, Cond::RcCommand),
    M("rcCommand[3]", false, 4, 1, 1, 8, Cond::RcCommand),
    M("vbat", false, 9, 3, 1, 6, Cond::Vbat),
    M("amperage", true, 0, 0, 1, 6, Cond::Amperage),
    M("magADC[0]", true, 0, 0, 1, 6, Cond::Mag),
    M("magADC[1]", true, 0, 0, 1, 6, Cond::Mag),
    M("magADC[2]", true, 0, 0, 1, 6, Cond::Mag),
    M("BaroAlt", true, 0, 0, 1, 6, Cond::Baro),
    M("AirSpeed", true, 0, 0, 1, 6, Cond::Pitot),
    M("surfaceRaw", true, 0, 0, 1, 6, Cond::Surface),
    M("rssi", false, 0, 1, 1, 6, Cond::Rssi),
    M("gyroADC[0]", true, 0, 0, 3, 0, Cond::Always),
    M("gyroADC[1]", true, 0, 0, 3, 0, Cond::Always),
    M("gyroADC[2]", true, 0, 0, 3, 0, Cond::Always),
    M("gyroRaw[0]", true, 0, 0, 3, 0, Cond::GyroRaw),
    M("gyroRaw[1]", true, 0, 0, 3, 0, Cond::GyroRaw),
    M("gyroRaw[2]", true, 0, 0, 3, 0, Cond::GyroRaw),
    M("gyroPeakRoll[0]", false, 0, 1, 3, 0, Cond::GyroPeaksRoll),
    M("gyroPeakRoll[1]", false, 0, 1, 3, 0, Cond::GyroPeaksRoll),
    M("gyroPeakRoll[2]", false, 0, 1, 3, 0, Cond::GyroPeaksRoll),
    M("gyroPeakPitch[0]", false, 0, 1, 3, 0, Cond::GyroPeaksPitch),
    M("gyroPeakPitch[1]", false, 0, 1, 3, 0, Cond::GyroPeaksPitch),
    M("gyroPeakPitch[2]", false, 0, 1, 3, 0, Cond::GyroPeaksPitch),
    M("gyroPeakYaw[0]", false, 0, 1, 3, 0, Cond::GyroPeaksYaw),
    M("gyroPeakYaw[1]", false, 0, 1, 3, 0, Cond::GyroPeaksYaw),
    M("gyroPeakYaw[2]", false, 0, 1, 3, 0, Cond::GyroPeaksYaw),
    M("accSmooth[0]", true, 0, 0, 3, 0, Cond::Acc),
    M("accSmooth[1]", true, 0, 0, 3, 0, Cond::Acc),
    M("accSmooth[2]", true, 0, 0, 3, 0, Cond::Acc),
    M("accVib", false, 0, 1, 3, 0, Cond::Acc),
    M("attitude[0]", true, 0, 0, 3, 0, Cond::Attitude),
    M("attitude[1]", true, 0, 0, 3, 0, Cond::Attitude),
    M("attitude[2]", true, 0, 0, 3, 0, Cond::Attitude),
    M("debug[0]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[1]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[2]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[3]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[4]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[5]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[6]", true, 0, 0, 3, 0, Cond::Debug),
    M("debug[7]", true, 0, 0, 3, 0, Cond::Debug),
    M("motor[0]", false, 4, 1, 3, 0, Cond::Motors(1)),
    M("motor[1]", false, 5, 0, 3, 0, Cond::Motors(2)),
    M("motor[2]", false, 5, 0, 3, 0, Cond::Motors(3)),
    M("motor[3]", false, 5, 0, 3, 0, Cond::Motors(4)),
    M("motor[4]", false, 5, 0, 3, 0, Cond::Motors(5)),
    M("motor[5]", false, 5, 0, 3, 0, Cond::Motors(6)),
    M("motor[6]", false, 5, 0, 3, 0, Cond::Motors(7)),
    M("motor[7]", false, 5, 0, 3, 0, Cond::Motors(8)),
    M("servo[0]", false, 8, 0, 3, 0, Cond::Servos(1)),
    M("servo[1]", false, 8, 0, 3, 0, Cond::Servos(2)),
    M("servo[2]", false, 8, 0, 3, 0, Cond::Servos(3)),
    M("servo[3]", false, 8, 0, 3, 0, Cond::Servos(4)),
    M("servo[4]", false, 8, 0, 3, 0, Cond::Servos(5)),
    M("servo[5]", false, 8, 0, 3, 0, Cond::Servos(6)),
    M("servo[6]", false, 8, 0, 3, 0, Cond::Servos(7)),
    M("servo[7]", false, 8, 0, 3, 0, Cond::Servos(8)),
    M("servo[8]", false, 8, 0, 3, 0, Cond::Servos(9)),
    M("servo[9]", false, 8, 0, 3, 0, Cond::Servos(10)),
    M("servo[10]", false, 8, 0, 3, 0, Cond::Servos(11)),
    M("servo[11]", false, 8, 0, 3, 0, Cond::Servos(12)),
    M("servo[12]", false, 8, 0, 3, 0, Cond::Servos(13)),
    M("servo[13]", false, 8, 0, 3, 0, Cond::Servos(14)),
    M("servo[14]", false, 8, 0, 3, 0, Cond::Servos(15)),
    M("servo[15]", false, 8, 0, 3, 0, Cond::Servos(16)),
    M("servo[16]", false, 8, 0, 3, 0, Cond::Servos(17)),
    M("servo[17]", false, 8, 0, 3, 0, Cond::Servos(18)),
    M("servo[18]", false, 8, 0, 3, 0, Cond::Servos(19)),
    M("servo[19]", false, 8, 0, 3, 0, Cond::Servos(20)),
    M("servo[20]", false, 8, 0, 3, 0, Cond::Servos(21)),
    M("servo[21]", false, 8, 0, 3, 0, Cond::Servos(22)),
    M("servo[22]", false, 8, 0, 3, 0, Cond::Servos(23)),
    M("servo[23]", false, 8, 0, 3, 0, Cond::Servos(24)),
    M("servo[24]", false, 8, 0, 3, 0, Cond::Servos(25)),
    M("servo[25]", false, 8, 0, 3, 0, Cond::Servos(26)),
    M("navState", true, 0, 0, 1, 0, Cond::Always),
    M("navFlags", false, 0, 0, 1, 0, Cond::Always),
    M("navEPH", true, 0, 0, 1, 0, Cond::NavPos),
    M("navEPV", true, 0, 0, 1, 0, Cond::NavPos),
    M("navPos[0]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navPos[1]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navPos[2]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navVel[0]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navVel[1]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navVel[2]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navTgtVel[0]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navTgtVel[1]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navTgtVel[2]", true, 0, 0, 3, 0, Cond::NavPos),
    M("navTgtPos[0]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navTgtPos[1]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navTgtPos[2]", true, 0, 0, 1, 0, Cond::NavPos),
    M("navTgtHdg", false, 0, 0, 1, 0, Cond::NavPos),
    M("navSurf", true, 0, 0, 1, 0, Cond::NavPos),
    M("navAcc[0]", true, 0, 0, 3, 0, Cond::NavAcc),
    M("navAcc[1]", true, 0, 0, 3, 0, Cond::NavAcc),
    M("navAcc[2]", true, 0, 0, 3, 0, Cond::NavAcc),
];

/// `blackboxSlowFields`, INAV 9.1.0: name, signed, header predictor, encoding, behind an `#ifdef`.
const SLOW_9_1: &[(&str, bool, i32, i32, bool)] = &[
    ("activeWpNumber", false, 0, 1, false),
    ("flightModeFlags", false, 0, 1, false),
    ("flightModeFlags2", false, 0, 1, false),
    ("activeFlightModeFlags", false, 0, 1, false),
    ("stateFlags", false, 0, 1, false),
    ("failsafePhase", false, 0, 7, false),
    ("rxSignalReceived", false, 0, 7, false),
    ("rxFlightChannelsValid", false, 0, 7, false),
    ("rxUpdateRate", false, 1, 1, false),
    ("hwHealthStatus", false, 0, 1, false),
    ("powerSupplyImpedance", false, 0, 1, false),
    ("sagCompensatedVBat", false, 0, 1, false),
    ("wind[0]", true, 0, 0, false),
    ("wind[1]", true, 0, 0, false),
    ("wind[2]", true, 0, 0, false),
    ("mspOverrideFlags", false, 0, 1, true),
    ("IMUTemperature", true, 0, 0, false),
    ("baroTemperature", true, 0, 0, true),
    ("sens0Temp", true, 0, 0, true),
    ("sens1Temp", true, 0, 0, true),
    ("sens2Temp", true, 0, 0, true),
    ("sens3Temp", true, 0, 0, true),
    ("sens4Temp", true, 0, 0, true),
    ("sens5Temp", true, 0, 0, true),
    ("sens6Temp", true, 0, 0, true),
    ("sens7Temp", true, 0, 0, true),
    ("escRPM", false, 0, 1, true),
    ("escTemperature", true, 1, 0, true),
];

/// `blackboxGpsGFields`, INAV 9.1.0: name, signed, predictor, encoding (`time` only when not every
/// iteration is logged — always the case here, P interval 1/2).
const GPS_9_1: &[(&str, bool, i32, i32)] = &[
    ("time", false, 10, 1),
    ("GPS_fixType", false, 0, 1),
    ("GPS_numSat", false, 0, 1),
    ("GPS_coord[0]", true, 7, 0),
    ("GPS_coord[1]", true, 7, 0),
    ("GPS_altitude", true, 0, 0),
    ("GPS_speed", false, 0, 1),
    ("GPS_ground_course", false, 0, 1),
    ("GPS_hdop", false, 0, 1),
    ("GPS_eph", false, 0, 1),
    ("GPS_epv", false, 0, 1),
    ("GPS_velned[0]", true, 0, 0),
    ("GPS_velned[1]", true, 0, 0),
    ("GPS_velned[2]", true, 0, 0),
];

const MINTHROTTLE: i64 = 1000;
const VBATREF: i64 = 1600;
const I_INTERVAL: u32 = 8;

/// Deterministic pseudo-random numbers (no fixtures, no flakes).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + i64::from(self.next()) % (hi - lo + 1)
    }
    fn chance(&mut self, percent: u32) -> bool {
        self.next() % 100 < percent
    }
    /// Half zeros, a quarter small (1-byte VB), a quarter anywhere in `lo..=hi` — keeps the full
    /// 149-field I-frame under the decoder's 256-byte frame limit.
    fn mixed(&mut self, lo: i64, hi: i64) -> i64 {
        match self.next() % 4 {
            0 | 1 => 0i64.clamp(lo, hi),
            2 => self.range(-60, 60).clamp(lo, hi),
            _ => self.range(lo, hi),
        }
    }
}

/// Which condition groups the simulated FC logs.
struct Config {
    groups: Vec<Cond>,
    motors: u8,
    servos: u8,
    pid_d: [bool; 3],
    gps: bool,
    slow_optional: bool,
}

impl Config {
    fn full() -> Self {
        use Cond::*;
        Config {
            groups: vec![
                FixedWingNav,
                McNav,
                RcData,
                RcCommand,
                Vbat,
                Amperage,
                Mag,
                Baro,
                Pitot,
                Surface,
                Rssi,
                GyroRaw,
                GyroPeaksRoll,
                GyroPeaksPitch,
                GyroPeaksYaw,
                Acc,
                Attitude,
                Debug,
                NavPos,
                NavAcc,
            ],
            motors: 8,
            servos: 26,
            pid_d: [true; 3],
            gps: true,
            slow_optional: true,
        }
    }

    fn random(rng: &mut Rng) -> Self {
        let mut c = Config::full();
        c.groups.retain(|_| rng.chance(50));
        c.motors = rng.range(0, 8) as u8;
        c.servos = rng.range(0, 26) as u8;
        c.pid_d = [rng.chance(50), rng.chance(50), rng.chance(50)];
        c.gps = rng.chance(50);
        c.slow_optional = rng.chance(50);
        c
    }

    fn logs(&self, cond: Cond) -> bool {
        match cond {
            Cond::Always => true,
            Cond::PidD(k) => self.pid_d[usize::from(k)],
            Cond::Motors(n) => self.motors >= n,
            Cond::Servos(n) => self.servos >= n,
            other => self.groups.contains(&other),
        }
    }
}

/// The log under construction plus the firmware-side state the writers need.
struct Fw<'a> {
    out: Vec<u8>,
    main: Vec<&'a M>,
    slow: Vec<(&'static str, bool, i32, i32, bool)>,
    /// `blackboxHistory[1]` / `[2]` (main-frame values in header order).
    prev: Vec<i64>,
    prev2: Vec<i64>,
    home: [i64; 2],
    /// `blackboxHistory[1]->time` for the G-frame time prediction.
    last_main_time: i64,
    /// Added to every main-frame time (a logging pause moves the clock on).
    time_offset: i64,
    /// Largest frame written (must stay within the decoder's 256-byte limit).
    max_frame: usize,
    expected: Collect,
}

#[derive(Default, Debug, PartialEq)]
struct Collect {
    main: Vec<Vec<i64>>,
    slow: Vec<Vec<i64>>,
    gps: Vec<(Option<i64>, Vec<i64>)>,
    home: Vec<Vec<i64>>,
    events: Vec<(u8, Vec<Option<EventValue>>)>,
}

impl FrameSink for Collect {
    fn main_frame(&mut self, values: &[i64]) {
        self.main.push(values.to_vec());
    }
    fn slow_frame(&mut self, values: &[i64]) {
        self.slow.push(values.to_vec());
    }
    fn gps_frame(&mut self, values: &[i64], time_us: Option<i64>, _: bool) {
        self.gps.push((time_us, values.to_vec()));
    }
    fn home_frame(&mut self, values: &[i64], _: Option<i64>, _: bool) {
        self.home.push(values.to_vec());
    }
    fn event(&mut self, event: &Event) {
        let n = event.layout.attrs.len();
        self.events
            .push((event.layout.id, event.values[..n].to_vec()));
    }
}

fn header_line(out: &mut Vec<u8>, key: &str, value: &str) {
    out.extend_from_slice(format!("H {key}:{value}\n").as_bytes());
}

fn join<T: ToString>(items: impl Iterator<Item = T>) -> String {
    items.map(|v| v.to_string()).collect::<Vec<_>>().join(",")
}

impl<'a> Fw<'a> {
    fn new(cfg: &Config) -> Fw<'a> {
        let main: Vec<&M> = MAIN_9_1.iter().filter(|f| cfg.logs(f.6)).collect();
        let slow: Vec<_> = SLOW_9_1
            .iter()
            .copied()
            .filter(|s| cfg.slow_optional || !s.4)
            .collect();
        let mut out = Vec::new();
        header_line(
            &mut out,
            "Product",
            "Blackbox flight data recorder by Nicholas Sherlock",
        );
        header_line(&mut out, "Data version", "2");
        header_line(&mut out, "I interval", &I_INTERVAL.to_string());
        header_line(&mut out, "Field I name", &join(main.iter().map(|f| f.0)));
        header_line(
            &mut out,
            "Field I signed",
            &join(main.iter().map(|f| u8::from(f.1))),
        );
        header_line(
            &mut out,
            "Field I predictor",
            &join(main.iter().map(|f| f.2)),
        );
        header_line(
            &mut out,
            "Field I encoding",
            &join(main.iter().map(|f| f.3)),
        );
        header_line(
            &mut out,
            "Field P predictor",
            &join(main.iter().map(|f| f.4)),
        );
        header_line(
            &mut out,
            "Field P encoding",
            &join(main.iter().map(|f| f.5)),
        );
        if cfg.gps {
            header_line(&mut out, "Field H name", "GPS_home[0],GPS_home[1]");
            header_line(&mut out, "Field H signed", "1,1");
            header_line(&mut out, "Field H predictor", "0,0");
            header_line(&mut out, "Field H encoding", "0,0");
            header_line(&mut out, "Field G name", &join(GPS_9_1.iter().map(|g| g.0)));
            header_line(
                &mut out,
                "Field G signed",
                &join(GPS_9_1.iter().map(|g| u8::from(g.1))),
            );
            header_line(
                &mut out,
                "Field G predictor",
                &join(GPS_9_1.iter().map(|g| g.2)),
            );
            header_line(
                &mut out,
                "Field G encoding",
                &join(GPS_9_1.iter().map(|g| g.3)),
            );
        }
        header_line(&mut out, "Field S name", &join(slow.iter().map(|s| s.0)));
        header_line(
            &mut out,
            "Field S signed",
            &join(slow.iter().map(|s| u8::from(s.1))),
        );
        header_line(
            &mut out,
            "Field S predictor",
            &join(slow.iter().map(|s| s.2)),
        );
        header_line(
            &mut out,
            "Field S encoding",
            &join(slow.iter().map(|s| s.3)),
        );
        header_line(&mut out, "Firmware type", "Cleanflight");
        header_line(
            &mut out,
            "Firmware revision",
            "INAV 9.1.0 (00000000) SYNTHETIC",
        );
        header_line(&mut out, "P interval", "1/2");
        header_line(&mut out, "minthrottle", &MINTHROTTLE.to_string());
        header_line(&mut out, "maxthrottle", "2000");
        header_line(&mut out, "gyro_scale", "0x3f800000");
        header_line(&mut out, "motorOutput", "1000,2000");
        header_line(&mut out, "acc_1G", "4096");
        header_line(&mut out, "vbatref", &VBATREF.to_string());
        let n = main.len();
        Fw {
            out,
            main,
            slow,
            prev: vec![0; n],
            prev2: vec![0; n],
            home: [0; 2],
            last_main_time: 0,
            time_offset: 0,
            max_frame: 0,
            expected: Collect::default(),
        }
    }

    /// Values a real FC could log for each main field (ranges keep the firmware's int16 fields and
    /// the TAG8_4S16 / NEG_14BIT deltas in range; zeros and negatives included).
    fn main_values(&self, rng: &mut Rng, iteration: u32) -> Vec<i64> {
        self.main
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let name = f.0;
                match name {
                    "loopIteration" => i64::from(iteration),
                    "time" => {
                        1_000_000
                            + self.time_offset
                            + i64::from(iteration) * 1000
                            + rng.range(0, 40)
                    }
                    "vbat" => VBATREF + rng.mixed(-600, 600),
                    "rssi" => (self.prev[k] + rng.range(1, 50)) % 1024, // always changes (8 + 1 group)
                    "navTgtHdg" => rng.mixed(0, 36000),
                    _ if name.starts_with("rcData[") => rng.range(900, 2100),
                    "rcCommand[3]" => rng.range(MINTHROTTLE, 2000),
                    _ if name.starts_with("rcCommand[") => rng.range(-500, 500),
                    _ if name.starts_with("motor[") => MINTHROTTLE + rng.mixed(0, 1000),
                    _ if name.starts_with("servo[") => 1500 + rng.mixed(-500, 500),
                    _ if name.starts_with("debug[") => rng.mixed(-1_000_000, 1_000_000),
                    _ if f.1 => rng.mixed(-5000, 5000),
                    _ => rng.mixed(0, 5000),
                }
            })
            .collect()
    }

    fn write_i(&mut self, v: &[i64]) {
        let start = self.out.len();
        self.out.push(b'I');
        let motor0 = self.main.iter().position(|f| f.0 == "motor[0]");
        for (k, f) in self.main.iter().enumerate() {
            let raw = match f.2 {
                4 => v[k] - MINTHROTTLE,
                5 => v[k] - v[motor0.unwrap()],
                8 => v[k] - 1500,
                9 => {
                    // NEG_14BIT: `(vbatReference - vbat) & 0x3FFF` as unsigned VB.
                    write_uvb(&mut self.out, ((VBATREF - v[k]) & 0x3FFF) as u32);
                    continue;
                }
                _ => v[k],
            };
            match f.3 {
                0 => write_svb(&mut self.out, raw as i32),
                1 => write_uvb(&mut self.out, raw as u32),
                e => panic!("unexpected I encoding {e}"),
            }
        }
        self.prev = v.to_vec();
        self.prev2 = v.to_vec();
        self.last_main_time = v[1];
        self.max_frame = self.max_frame.max(self.out.len() - start);
        self.expected.main.push(v.to_vec());
    }

    fn write_p(&mut self, v: &[i64]) {
        let start = self.out.len();
        self.out.push(b'P');
        let (prev, prev2) = (&self.prev, &self.prev2);
        let delta = |k: usize, f: &M| -> i64 {
            // The firmware writes accVib as a delta to the previous frame despite its header
            // predictor AVERAGE_2 (schema quirk).
            let predictor = if f.0 == "accVib" { 1 } else { f.4 };
            v[k] - match predictor {
                0 => 0,
                1 => prev[k],
                2 => 2 * prev[k] - prev2[k],
                3 => (prev[k] + prev2[k]) / 2,
                p => panic!("unexpected P predictor {p}"),
            }
        };
        let mut k = 0;
        while k < self.main.len() {
            let f = self.main[k];
            if f.4 == 6 {
                k += 1; // INC: nothing written
                continue;
            }
            let run = self.main[k..].iter().take_while(|g| g.5 == f.5).count();
            match f.5 {
                0 => {
                    write_svb(&mut self.out, delta(k, f) as i32);
                    k += 1;
                }
                7 => {
                    let d = [0, 1, 2].map(|j| delta(k + j, self.main[k + j]) as i32);
                    write_tag2_3s32(&mut self.out, d);
                    k += 3;
                }
                8 => {
                    let d = [0, 1, 2, 3].map(|j| delta(k + j, self.main[k + j]) as i32);
                    write_tag8_4s16(&mut self.out, d);
                    k += 4;
                }
                6 => {
                    // One call for the whole optional group (`optionalFieldCount` ≤ 9).
                    let d: Vec<i32> = (k..k + run)
                        .map(|j| delta(j, self.main[j]) as i32)
                        .collect();
                    write_tag8_8svb(&mut self.out, &d);
                    k += run;
                }
                e => panic!("unexpected P encoding {e}"),
            }
        }
        self.prev2 = std::mem::replace(&mut self.prev, v.to_vec());
        self.last_main_time = v[1];
        self.max_frame = self.max_frame.max(self.out.len() - start);
        self.expected.main.push(v.to_vec());
    }

    fn write_s(&mut self, rng: &mut Rng) {
        let v: Vec<i64> = self
            .slow
            .iter()
            .map(|s| match s.0 {
                "flightModeFlags" | "flightModeFlags2" | "activeFlightModeFlags" | "stateFlags" => {
                    i64::from(rng.next() | (rng.next() & 0x8000_0000))
                }
                "failsafePhase" => rng.range(0, 7),
                "rxSignalReceived" | "rxFlightChannelsValid" => rng.range(0, 1),
                _ if s.1 => rng.range(-500, 500),
                _ => rng.range(0, 5000),
            })
            .collect();
        self.out.push(b'S');
        let mut k = 0;
        while k < v.len() {
            match self.slow[k].3 {
                0 => write_svb(&mut self.out, v[k] as i32),
                1 => write_uvb(&mut self.out, v[k] as u32),
                7 => {
                    write_tag2_3s32(
                        &mut self.out,
                        [v[k] as i32, v[k + 1] as i32, v[k + 2] as i32],
                    );
                    k += 2;
                }
                e => panic!("unexpected S encoding {e}"),
            }
            k += 1;
        }
        self.expected.slow.push(v);
    }

    fn write_home(&mut self, rng: &mut Rng) {
        self.home = [
            rng.range(470_000_000, 480_000_000),
            rng.range(80_000_000, 90_000_000),
        ];
        self.out.push(b'H');
        write_svb(&mut self.out, self.home[0] as i32);
        write_svb(&mut self.out, self.home[1] as i32);
        self.expected.home.push(self.home.to_vec());
    }

    /// A G frame at `time`.
    fn write_gps(&mut self, rng: &mut Rng, time: i64) {
        let v: Vec<i64> = GPS_9_1
            .iter()
            .map(|g| match g.0 {
                "time" => time,
                "GPS_fixType" => rng.range(0, 3),
                "GPS_numSat" => rng.range(0, 30),
                "GPS_coord[0]" => self.home[0] + rng.range(-1_000_000, 1_000_000),
                "GPS_coord[1]" => self.home[1] + rng.range(-1_000_000, 1_000_000),
                "GPS_altitude" => rng.range(-100, 5000),
                "GPS_ground_course" => rng.range(0, 3599),
                _ if g.1 => rng.range(-3000, 3000),
                _ => rng.range(0, 9999),
            })
            .collect();
        self.out.push(b'G');
        for (k, g) in GPS_9_1.iter().enumerate() {
            let raw = match g.2 {
                10 => v[k] - self.last_main_time,
                7 => v[k] - self.home[if g.0.ends_with("[1]") { 1 } else { 0 }],
                _ => v[k],
            };
            match g.3 {
                0 => write_svb(&mut self.out, raw as i32),
                _ => write_uvb(&mut self.out, raw as u32),
            }
        }
        self.expected.gps.push((Some(time), v));
    }

    fn event(&mut self, id: u8, payload: &[u8], expected: &[Option<EventValue>]) {
        self.out.extend_from_slice(&[b'E', id]);
        self.out.extend_from_slice(payload);
        self.expected.events.push((id, expected.to_vec()));
    }
}

fn uvb(v: u32) -> Vec<u8> {
    let mut b = Vec::new();
    write_uvb(&mut b, v);
    b
}

/// Simulate `blackboxLogIteration` over iterations 0..`span` (I interval 8, P interval 1/2), a
/// logging resume to iteration 1000, and the end of log; decode and compare everything.
fn simulate(cfg: &Config, seed: u64) {
    let mut rng = Rng(seed);
    let mut fw = Fw::new(cfg);
    let log_iterations = |fw: &mut Fw, rng: &mut Rng, from: u32, to: u32| {
        for it in from..to {
            let p_index = it % I_INTERVAL;
            let v = fw.main_values(rng, it);
            let time = v[1];
            if p_index == 0 {
                if rng.chance(60) {
                    fw.write_s(rng);
                }
                fw.write_i(&v);
                continue;
            }
            // Events come first in a P iteration (arming beep, flight-mode change).
            if it % 6 == 2 {
                fw.event(0, &uvb(time as u32), &[Some(EventValue::I64(time))]);
                fw.event(
                    30,
                    &[uvb(0x8000_0005), uvb(1)].concat(),
                    &[Some(EventValue::U32(0x8000_0005)), Some(EventValue::U32(1))],
                );
            }
            let logged = p_index.is_multiple_of(2); // P interval 1/2
            if logged {
                if rng.chance(40) {
                    fw.write_s(rng);
                }
                fw.write_p(&v);
            }
            if cfg.gps && (fw.expected.home.is_empty() || rng.chance(10)) {
                fw.write_home(rng);
                fw.write_gps(rng, time);
            } else if cfg.gps && rng.chance(50) {
                fw.write_gps(rng, time);
            }
        }
    };
    log_iterations(&mut fw, &mut rng, 0, 13);
    // In-flight adjustments: integer and float value (function byte bit 7 = float).
    fw.event(
        13,
        &[uvb(5), {
            let mut b = Vec::new();
            write_svb(&mut b, -42);
            b
        }]
        .concat(),
        &[Some(EventValue::U32(5)), Some(EventValue::I32(-42)), None],
    );
    fw.event(
        13,
        &[&[7u8 | 0x80][..], &1.25f32.to_le_bytes()].concat(),
        &[Some(EventValue::U32(7)), None, Some(EventValue::F32(1.25))],
    );
    fw.event(40, &uvb(2), &[Some(EventValue::U32(2))]);
    log_iterations(&mut fw, &mut rng, 13, 26);
    // Logging paused, resumed at iteration 1000 (an I-frame iteration) 20 s later.
    fw.time_offset = 20_000_000;
    let v = fw.main_values(&mut rng, 1000);
    let resume_time = v[1] as u32;
    fw.event(
        14,
        &[uvb(1000), uvb(resume_time)].concat(),
        &[
            Some(EventValue::U32(1000)),
            Some(EventValue::I64(i64::from(resume_time))),
        ],
    );
    fw.write_i(&v);
    log_iterations(&mut fw, &mut rng, 1001, 1020);
    let reason = b"End of log (disarm reason:3)\0";
    fw.event(255, reason, &[Some(EventValue::I32(3))]);

    assert!(
        fw.max_frame <= 256,
        "synthetic frame of {} bytes exceeds the 256-byte frame limit",
        fw.max_frame
    );
    let mut got = Collect::default();
    let counters = decode_frames(&fw.out, &mut got);
    let total = |f: fn(&KindCounters) -> u64| counters.iter().map(f).sum::<u64>();
    assert_eq!(total(|c| c.corrupt), 0, "corrupt frames");
    assert_eq!(total(|c| c.dropped), 0, "dropped frames");
    let names: Vec<&str> = fw.main.iter().map(|f| f.0).collect();
    assert_eq!(got.main.len(), fw.expected.main.len(), "main frame count");
    for (r, (a, b)) in got.main.iter().zip(&fw.expected.main).enumerate() {
        for k in 0..names.len() {
            assert_eq!(a[k], b[k], "main row {r} field {}", names[k]);
        }
    }
    assert_eq!(got.slow, fw.expected.slow, "S frames");
    assert_eq!(got.home, fw.expected.home, "H frames");
    assert_eq!(got.gps, fw.expected.gps, "G frames");
    assert_eq!(got.events, fw.expected.events, "events");
}

/// One log through the frame decoder: the Analyzer's `decode_observed` without its table.
fn decode_frames(bytes: &[u8], sink: &mut Collect) -> [KindCounters; 6] {
    let header = parse_header(bytes).unwrap();
    let (schema, _) = schema_for(header.firmware_version);
    let mut decoder = FrameDecoder::new(&header, schema, 0).unwrap();
    decoder.run(bytes, header.data_start, sink);
    decoder.counters
}

#[test]
fn every_inav_9_1_field_round_trips() {
    let cfg = Config::full();
    assert_eq!(
        MAIN_9_1.iter().filter(|f| cfg.logs(f.6)).count(),
        MAIN_9_1.len()
    );
    assert_eq!(MAIN_9_1.len(), 149);
    simulate(&cfg, 1);
    simulate(&cfg, 2);
}

#[test]
fn random_condition_subsets_round_trip() {
    let mut pick = Rng(0x5eed);
    for seed in 0..12 {
        let cfg = Config::random(&mut pick);
        simulate(&cfg, 100 + seed);
    }
}
