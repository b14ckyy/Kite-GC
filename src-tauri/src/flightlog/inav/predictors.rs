// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/predictors.rs (2026-11-11)

//! Field predictors (`blackbox_fielddefs.h` `FlightLogFieldPredictor`, INAV 9.1.0 lines 114-144):
//! the decoded value is the raw stream value plus the prediction. Logic after blackbox-tools v9.0.0
//! (`1918a75`) `parser.c` `applyPrediction` (lines 630-700), the INC branch of `parseFrame` and
//! `shouldHaveFrame` / `countIntentionallySkippedFrames` (lines 622-625, 841-856).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Predictor {
    /// 0 — no prediction.
    Zero,
    /// 1 — previous frame's value.
    Previous,
    /// 2 — `2·prev − prev2`.
    StraightLine,
    /// 3 — `(prev + prev2) / 2` (integer division, truncating toward zero as in C).
    Average2,
    /// 4 — header `minthrottle`.
    MinThrottle,
    /// 5 — `motor[0]` of the frame being decoded.
    Motor0,
    /// 6 — nothing is read; `prev + 1 + skipped frames`.
    Inc,
    /// 7 — GPS home coordinate; `HomeCoord(1)` is the second of a pair (`HOME_COORD_1`).
    HomeCoord(u8),
    /// 8 — 1500.
    Plus1500,
    /// 9 — header `vbatref`.
    VbatRef,
    /// 10 — `time` of the last main frame.
    LastMainFrameTime,
    /// 11 — header `motorOutput` low (blackbox-tools knows it; INAV does not write it).
    MinMotor,
}

impl Predictor {
    pub fn from_id(id: i32) -> Option<Self> {
        Some(match id {
            0 => Predictor::Zero,
            1 => Predictor::Previous,
            2 => Predictor::StraightLine,
            3 => Predictor::Average2,
            4 => Predictor::MinThrottle,
            5 => Predictor::Motor0,
            6 => Predictor::Inc,
            7 => Predictor::HomeCoord(0),
            8 => Predictor::Plus1500,
            9 => Predictor::VbatRef,
            10 => Predictor::LastMainFrameTime,
            11 => Predictor::MinMotor,
            _ => return None,
        })
    }
}

/// Home-coordinate predictors come in pairs (lat, lon) with the same id; the second of a pair
/// predicts from `GPS_home[1]` (`flightLogParse` rewrite to `HOME_COORD_1`, parser.c:1587-1592).
pub fn resolve_home_pairs(predictors: &mut [Predictor]) {
    for i in 1..predictors.len() {
        if predictors[i - 1] == Predictor::HomeCoord(0) && predictors[i] == Predictor::HomeCoord(0)
        {
            predictors[i] = Predictor::HomeCoord(1);
        }
    }
}

/// Everything a prediction may refer to besides the frame history.
pub struct PredictContext {
    pub minthrottle: i64,
    pub vbatref: i64,
    pub motor_output_low: i64,
    /// Index of `motor[0]` in the main frame.
    pub motor0: Option<usize>,
    /// The last committed GPS home (from an `H` frame), `[lat, lon]`; zero before the first.
    pub home: [i64; 2],
    /// `time` of the last accepted main frame while the main stream is valid.
    pub last_main_time: Option<i64>,
}

/// The value of field `index` given its raw stream value. `current` is the frame being decoded
/// (fields before `index` are final), `prev`/`prev2` the history (`None` when there is none —
/// the history predictors then predict 0, as the C code does).
pub fn apply(
    predictor: Predictor,
    index: usize,
    raw: i64,
    current: &[i64],
    prev: Option<&[i64]>,
    prev2: Option<&[i64]>,
    ctx: &PredictContext,
) -> i64 {
    let prediction = match predictor {
        Predictor::Zero | Predictor::Inc => 0,
        Predictor::MinThrottle => ctx.minthrottle,
        Predictor::Plus1500 => 1500,
        Predictor::Motor0 => ctx.motor0.map_or(0, |m| current[m]),
        Predictor::VbatRef => ctx.vbatref,
        Predictor::Previous => prev.map_or(0, |p| p[index]),
        Predictor::StraightLine => match prev {
            Some(p) => 2 * p[index] - prev2.unwrap_or(p)[index],
            None => 0,
        },
        Predictor::Average2 => match prev {
            Some(p) => (p[index] + prev2.unwrap_or(p)[index]) / 2,
            None => 0,
        },
        Predictor::HomeCoord(k) => ctx.home[usize::from(k.min(1))],
        Predictor::LastMainFrameTime => ctx.last_main_time.unwrap_or(0),
        Predictor::MinMotor => ctx.motor_output_low,
    };
    raw + prediction
}

/// INC (6): `prev + 1 + skipped` (`parseFrame`).
pub fn increment(index: usize, prev: Option<&[i64]>, skipped: u32) -> i64 {
    i64::from(skipped) + 1 + prev.map_or(0, |p| p[index])
}

/// Logging rate from the header: which loop iterations carry a main frame.
#[derive(Clone, Copy, Debug)]
pub struct FrameRate {
    i_interval: u32,
    p_num: u32,
    p_denom: u32,
}

impl FrameRate {
    /// Guarded against 0 (the firmware never writes it; the C decoder would divide by zero / loop).
    pub fn new(i_interval: u32, p_num: u32, p_denom: u32) -> Self {
        FrameRate {
            i_interval: i_interval.max(1),
            p_num: p_num.max(1),
            p_denom: p_denom.max(1),
        }
    }

    /// `shouldHaveFrame`: does iteration `index` carry a logged main frame?
    pub fn should_have_frame(&self, index: u32) -> bool {
        // The C expression mixes `int32_t` with `unsigned int` operands, so it is evaluated unsigned.
        let phase = index % self.i_interval;
        phase.wrapping_add(self.p_num).wrapping_sub(1) % self.p_denom < self.p_num
    }

    /// `countIntentionallySkippedFrames`: iterations after `last` that are not logged before the
    /// next logged one; 0 when no main frame was accepted yet.
    pub fn skipped_after(&self, last: Option<u32>) -> u32 {
        let Some(last) = last else {
            return 0;
        };
        let mut count = 0;
        let mut index = last.wrapping_add(1);
        // At most one I interval can pass without a logged frame (every I-frame iteration is).
        while !self.should_have_frame(index) && count <= self.i_interval {
            count += 1;
            index = index.wrapping_add(1);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PredictContext {
        PredictContext {
            minthrottle: 1050,
            vbatref: 1260,
            motor_output_low: 1000,
            motor0: Some(2),
            home: [473_000_000, 85_000_000],
            last_main_time: Some(5_000_000),
        }
    }

    #[test]
    fn every_predictor() {
        let c = ctx();
        let prev = [10i64, 100, 7];
        let prev2 = [9i64, 80, 5];
        let current = [0i64, 0, 1234];
        let p = Some(&prev[..]);
        let p2 = Some(&prev2[..]);
        assert_eq!(apply(Predictor::Zero, 1, 5, &current, p, p2, &c), 5);
        assert_eq!(apply(Predictor::Previous, 1, 5, &current, p, p2, &c), 105);
        assert_eq!(
            apply(Predictor::StraightLine, 1, 5, &current, p, p2, &c),
            2 * 100 - 80 + 5
        );
        assert_eq!(
            apply(Predictor::Average2, 1, 5, &current, p, p2, &c),
            90 + 5
        );
        // C integer division truncates toward zero: (−3 + −4) / 2 = −3.
        let (n1, n2) = ([0i64, -3], [0i64, -4]);
        assert_eq!(
            apply(
                Predictor::Average2,
                1,
                0,
                &current,
                Some(&n1),
                Some(&n2),
                &c
            ),
            -3
        );
        assert_eq!(
            apply(Predictor::MinThrottle, 0, -50, &current, None, None, &c),
            1000
        );
        assert_eq!(
            apply(Predictor::Motor0, 1, 6, &current, None, None, &c),
            1240
        );
        assert_eq!(
            apply(Predictor::HomeCoord(0), 0, -7, &current, None, None, &c),
            472_999_993
        );
        assert_eq!(
            apply(Predictor::HomeCoord(1), 0, 3, &current, None, None, &c),
            85_000_003
        );
        assert_eq!(
            apply(Predictor::Plus1500, 0, -20, &current, None, None, &c),
            1480
        );
        assert_eq!(
            apply(Predictor::VbatRef, 0, -60, &current, None, None, &c),
            1200
        );
        assert_eq!(
            apply(
                Predictor::LastMainFrameTime,
                0,
                250,
                &current,
                None,
                None,
                &c
            ),
            5_000_250
        );
        assert_eq!(
            apply(Predictor::MinMotor, 0, 5, &current, None, None, &c),
            1005
        );
        // No history: history predictors predict 0.
        assert_eq!(
            apply(Predictor::Previous, 1, 5, &current, None, None, &c),
            5
        );
        assert_eq!(
            apply(Predictor::StraightLine, 1, 5, &current, None, None, &c),
            5
        );
        // After an I-frame both history slots are that frame: straight line = previous.
        assert_eq!(
            apply(Predictor::StraightLine, 1, 5, &current, p, p, &c),
            105
        );
    }

    #[test]
    fn home_pairs() {
        let mut p = [
            Predictor::Zero,
            Predictor::HomeCoord(0),
            Predictor::HomeCoord(0),
            Predictor::Zero,
        ];
        resolve_home_pairs(&mut p);
        assert_eq!(p[1], Predictor::HomeCoord(0));
        assert_eq!(p[2], Predictor::HomeCoord(1));
    }

    #[test]
    fn inc_counts_skipped_frames() {
        // P interval 1/5, I interval 32: iterations 0, 5, 10, … 30, then 32 (next I frame) …
        let rate = FrameRate::new(32, 1, 5);
        let logged: Vec<u32> = (0..40).filter(|&i| rate.should_have_frame(i)).collect();
        assert_eq!(logged, [0, 5, 10, 15, 20, 25, 30, 32, 37]);
        assert_eq!(rate.skipped_after(None), 0);
        assert_eq!(rate.skipped_after(Some(0)), 4);
        assert_eq!(rate.skipped_after(Some(30)), 1);
        let prev = [0i64];
        assert_eq!(increment(0, Some(&prev), rate.skipped_after(Some(0))), 5);
        // Every frame logged: INC is +1.
        let all = FrameRate::new(32, 1, 1);
        assert_eq!(all.skipped_after(Some(7)), 0);
        assert_eq!(increment(0, Some(&[7]), 0), 8);
        assert_eq!(increment(0, None, 0), 1);
    }
}
