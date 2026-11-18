// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/schema.rs (2026-11-11)
// Deviation: `DType` comes from `super` (Kite's minimal copy in `mod.rs`), not `crate::logs::table`

//! Per-firmware-version knowledge that is NOT in the log header (ADR-009 point 2): event payload
//! layouts, flag / enum names, units of raw values, places where the header and the written data
//! disagree. Chosen by `Firmware revision`.
//!
//! Sources: INAV firmware, `git show <tag>:<path>` (tags 7.0.0, 7.1.0, 7.1.2, 8.0.0, 8.0.1, 9.0.0,
//! 9.0.1, 9.1.0 in the local checkout) and the firmware-researcher's check of the same tags plus
//! 10.0.0-rc2 (2026-10-06):
//! - events: `src/main/blackbox/blackbox.c` `blackboxLogEvent` (9.1.0 lines 2023-2064; byte-identical
//!   in 7.0.0 and 9.1.0), `blackbox_fielddefs.h` `FlightLogEvent` (9.1.0 lines 163-170). INAV writes
//!   ids 0, 13, 14, 30, 40, 255 only — the ids 10-12 and 20 blackbox-tools knows are Cleanflight
//!   autotune/gtune and never appear in INAV logs.
//! - `flightModeFlags` = `boxId_e` bits 0-31 (`src/main/fc/rc_modes.h`), `flightModeFlags2` = bits
//!   32-63 (8.0+), `activeFlightModeFlags` = `flightModeFlags_e`, `stateFlags` = `stateFlags_t`
//!   (`src/main/fc/runtime_config.h`), `failsafePhase` = `failsafePhase_e`
//!   (`src/main/flight/failsafe.h`), disarm reason = `disarmReason_t` (`src/main/fc/fc_core.h`).
//!   Bit positions were never renumbered.
//! - Quirks (header ≠ written data) — see [`InavSchema`] fields.
//! - Units: vbat 0.01 V, amperage 0.01 A (blackbox-tools `INAV_V2`, parser.c:571), gyro deg/s when
//!   `gyro_scale` is 1.0 (INAV always writes `0x3f800000`), GPS altitude m (`llh.alt / 100`,
//!   blackbox.c 9.1.0:1618), GPS eph/epv cm, coordinates 1e-7 deg, power-supply impedance mΩ.
//! - Column types: every frame field the firmware writes is 32 bit (`blackboxWriteSignedVB(int32_t)`
//!   / `blackboxWriteUnsignedVB(uint32_t)`, no 64-bit member in the logged state, blackbox.c
//!   9.1.0), so a field is an `i32` / `u32` column by its header `signed` flag; exceptions in
//!   [`DTYPE_OVERRIDES`].

use super::DType;

/// How one event payload attribute is encoded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PayloadKind {
    /// Unsigned variable byte.
    Uvb,
    /// Unsigned variable byte, a 32-bit microsecond time (the decoder adds its rollover offset).
    UvbTime,
    /// Adjustment function byte; bit 7 flags a float value (`..._FLOAT_VALUE_FLAG`, stripped).
    AdjustmentFunction,
    /// Signed VB when the preceding function byte had bit 7 clear; absent otherwise.
    AdjustmentValue,
    /// Float (little endian) when the preceding function byte had bit 7 set; absent otherwise.
    AdjustmentFloat,
    /// `End of log (disarm reason:N)` + NUL; the value is N.
    EndOfLog,
}

impl PayloadKind {
    /// The `events` column type of an attribute of this kind (the decoded value's own type).
    pub fn dtype(self) -> DType {
        match self {
            PayloadKind::Uvb | PayloadKind::AdjustmentFunction => DType::U32,
            PayloadKind::UvbTime => DType::I64,
            PayloadKind::AdjustmentValue | PayloadKind::EndOfLog => DType::I32,
            PayloadKind::AdjustmentFloat => DType::F32,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EventAttr {
    /// Column name in the `events` group.
    pub column: &'static str,
    pub kind: PayloadKind,
    pub unit: Option<&'static str>,
}

/// What the frame decoder must do beyond recording an event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventEffect {
    None,
    /// `LOGGING_RESUME`: move the iteration/time reference for frame validation.
    LoggingResume,
    /// `LOG_END`: the log ends after this event.
    LogEnd,
}

#[derive(Debug)]
pub struct EventLayout {
    pub id: u8,
    // Shown by the event-marker UI (Milestone 2); the decoder only needs `id`.
    #[allow(dead_code)]
    pub name: &'static str,
    pub attrs: &'static [EventAttr],
    pub effect: EventEffect,
}

/// Most attributes an event layout has (sizes the decoder's per-event scratch).
pub const MAX_EVENT_ATTRS: usize = 3;

pub struct InavSchema {
    pub label: &'static str,
    pub events: &'static [EventLayout],
    /// Bit → name of `flightModeFlags` (index 0-31) and `flightModeFlags2` (index 32-63 = bit 0-31
    /// of `flightModeFlags2`); also the bits of event 30. "" = unused bit.
    // The flag / enum tables are consumed by the flag and event-marker UI (Milestone 2).
    #[allow(dead_code)]
    pub rc_mode_names: &'static [&'static str],
    /// Bit → name of `activeFlightModeFlags`.
    #[allow(dead_code)]
    pub active_flight_mode_names: &'static [&'static str],
    /// Bit → name of `stateFlags`.
    #[allow(dead_code)]
    pub state_flag_names: &'static [&'static str],
    /// Value → name of `failsafePhase`.
    #[allow(dead_code)]
    pub failsafe_phase_names: &'static [&'static str],
    /// Value → name of the `LOG_END` disarm reason.
    #[allow(dead_code)]
    pub disarm_reason_names: &'static [&'static str],
    /// `(frame, field, predictor)` replacing the header's predictor.
    /// P `accVib`: declared AVERAGE_2, written as the delta to the previous frame (7.1.0+;
    /// blackbox.c 9.1.0 lines 315 vs 1246, 7.1.0 lines 313 vs 1190) → PREVIOUS. The field does not
    /// exist in 7.0, so the override is harmless there.
    pub predictor_overrides: &'static [(char, &'static str, i32)],
    /// S frames are decoded without any predictor: `rxUpdateRate` and `escTemperature` declare
    /// PREVIOUS but are written absolute (blackbox.c 9.1.0 lines 442/469 vs 1333/1358; 7.1.2 lines
    /// 1297/1299; all tags).
    pub slow_frames_absolute: bool,
    /// 7.0-8.0.x: I-frames write `motor[0]` under the MOTORS include flag while the header lists
    /// `motor[n]` only for `motorCount ≥ n` (blackbox.c 8.0.0 line 1000 vs the
    /// `AT_LEAST_MOTORS_1` header condition). With no motors and the flag set, every I-frame carries
    /// one UVB the header does not declare. Fixed in 9.0.0. HEURISTIC: applied when the header has
    /// no `motor[0]` (the include flag itself is not in the header).
    pub motor0_without_header: bool,
    /// Header S order `escTemperature, droneCANBusOffCount, terrainAGL, terrainAMSL`, write order
    /// `escTemperature, terrainAGL, terrainAMSL, droneCANBusOffCount` (targets with USE_DRONECAN
    /// and USE_TERRAIN; firmware-researcher, 10.0.0-rc2 blackbox.c). Also seen in a 9.1.0 dev build
    /// (`INAV 9.1.0 (0dab6eb2) MATEKH743` in Kite GC's DB: the bus-off column held zigzag AGL, the
    /// AGL column ~354 m AMSL, the AMSL column 0). Applied only when the header lists the three
    /// fields consecutively in that order; remapped after the S frame is decoded.
    pub slow_terrain_remap: bool,
    /// Every field INAV can write, per group: `(group, name, unit)`; a name ending in `[` matches
    /// all indices (`motor[` → `motor[0]` … `motor[7]`). `unit` = what one raw unit means, `None` =
    /// dimensionless or not verified.
    pub fields: &'static [(&'static str, &'static str, Option<&'static str>)],
    /// `(group, field, column type)` replacing the header-`signed` rule of [`Self::field_dtype`].
    pub dtype_overrides: &'static [(&'static str, &'static str, DType)],
}

impl InavSchema {
    pub fn event(&self, id: u8) -> Option<&'static EventLayout> {
        self.events.iter().find(|e| e.id == id)
    }

    /// `Some(unit)` for a field this schema knows (the unit itself may be `None`), `None` for a
    /// field name it does not know. `gyro_scale` decides whether raw gyro is deg/s.
    pub fn field_unit(&self, group: &str, field: &str, gyro_scale: f32) -> Option<Option<String>> {
        let (_, _, unit) = self.fields.iter().find(|(g, name, _)| {
            *g == group && (*name == field || (name.ends_with('[') && field.starts_with(name)))
        })?;
        if group == "main" && field.starts_with("gyroADC[") {
            return Some((gyro_scale == 1.0).then(|| "deg/s".to_owned()));
        }
        Some(unit.map(str::to_owned))
    }

    /// Column type of a frame field: [`Self::dtype_overrides`], else `i32` / `u32` by the header's
    /// `signed` flag (the firmware's 32-bit value, exact).
    pub fn field_dtype(&self, group: &str, field: &str, signed: bool) -> DType {
        self.dtype_overrides
            .iter()
            .find(|(g, name, _)| *g == group && *name == field)
            .map_or(if signed { DType::I32 } else { DType::U32 }, |o| o.2)
    }
}

// --- Event layouts — identical for 7.0.0 … 10.0.0-rc2. ------------------------------------------

const EVENTS: &[EventLayout] = &[
    EventLayout {
        id: 0,
        name: "SYNC_BEEP",
        attrs: &[EventAttr {
            column: "beepTime",
            kind: PayloadKind::UvbTime,
            unit: Some("us"),
        }],
        effect: EventEffect::None,
    },
    EventLayout {
        id: 13,
        name: "INFLIGHT_ADJUSTMENT",
        attrs: &[
            EventAttr {
                column: "adjustmentFunction",
                kind: PayloadKind::AdjustmentFunction,
                unit: None,
            },
            EventAttr {
                column: "adjustmentValue",
                kind: PayloadKind::AdjustmentValue,
                unit: None,
            },
            EventAttr {
                column: "adjustmentFloatValue",
                kind: PayloadKind::AdjustmentFloat,
                unit: None,
            },
        ],
        effect: EventEffect::None,
    },
    EventLayout {
        id: 14,
        name: "LOGGING_RESUME",
        attrs: &[
            EventAttr {
                column: "logIteration",
                kind: PayloadKind::Uvb,
                unit: None,
            },
            EventAttr {
                column: "currentTime",
                kind: PayloadKind::UvbTime,
                unit: Some("us"),
            },
        ],
        effect: EventEffect::LoggingResume,
    },
    EventLayout {
        id: 30,
        name: "FLIGHTMODE",
        attrs: &[
            EventAttr {
                column: "flightModeFlags",
                kind: PayloadKind::Uvb,
                unit: None,
            },
            EventAttr {
                column: "lastFlightModeFlags",
                kind: PayloadKind::Uvb,
                unit: None,
            },
        ],
        effect: EventEffect::None,
    },
    EventLayout {
        id: 40,
        name: "IMU_FAILURE",
        // 1 = quaternion reset to the last good value, 2 = reset from the accelerometer (imu.c).
        attrs: &[EventAttr {
            column: "imuErrorCode",
            kind: PayloadKind::Uvb,
            unit: None,
        }],
        effect: EventEffect::None,
    },
    EventLayout {
        id: 255,
        name: "LOG_END",
        attrs: &[EventAttr {
            column: "disarmReason",
            kind: PayloadKind::EndOfLog,
            unit: None,
        }],
        effect: EventEffect::LogEnd,
    },
];

// --- Flag / enum tables. ------------------------------------------------------------------------

/// `boxId_e` 7.0: 0-54, 23 = KILLSWITCH.
const RC_MODES_V7_0: &[&str] = &[
    "ARM",
    "ANGLE",
    "HORIZON",
    "NAVALTHOLD",
    "HEADINGHOLD",
    "HEADFREE",
    "HEADADJ",
    "CAMSTAB",
    "NAVRTH",
    "NAVPOSHOLD",
    "MANUAL",
    "BEEPERON",
    "LEDLOW",
    "LIGHTS",
    "NAVLAUNCH",
    "OSD",
    "TELEMETRY",
    "BLACKBOX",
    "FAILSAFE",
    "NAVWP",
    "AIRMODE",
    "HOMERESET",
    "GCSNAV",
    "KILLSWITCH",
    "SURFACE",
    "FLAPERON",
    "TURNASSIST",
    "AUTOTRIM",
    "AUTOTUNE",
    "CAMERA1",
    "CAMERA2",
    "CAMERA3",
    "OSDALT1",
    "OSDALT2",
    "OSDALT3",
    "NAVCOURSEHOLD",
    "BRAKING",
    "USER1",
    "USER2",
    "FPVANGLEMIX",
    "LOITERDIRCHN",
    "MSPRCOVERRIDE",
    "PREARM",
    "TURTLE",
    "NAVCRUISE",
    "AUTOLEVEL",
    "PLANWPMISSION",
    "SOARING",
    "USER3",
    "USER4",
    "CHANGEMISSION",
    "BEEPERMUTE",
    "MULTIFUNCTION",
    "MIXERPROFILE",
    "MIXERTRANSITION",
];

/// `boxId_e` 7.1: + 55 ANGLEHOLD.
const RC_MODES_V7_1: &[&str] = &[
    "ARM",
    "ANGLE",
    "HORIZON",
    "NAVALTHOLD",
    "HEADINGHOLD",
    "HEADFREE",
    "HEADADJ",
    "CAMSTAB",
    "NAVRTH",
    "NAVPOSHOLD",
    "MANUAL",
    "BEEPERON",
    "LEDLOW",
    "LIGHTS",
    "NAVLAUNCH",
    "OSD",
    "TELEMETRY",
    "BLACKBOX",
    "FAILSAFE",
    "NAVWP",
    "AIRMODE",
    "HOMERESET",
    "GCSNAV",
    "KILLSWITCH",
    "SURFACE",
    "FLAPERON",
    "TURNASSIST",
    "AUTOTRIM",
    "AUTOTUNE",
    "CAMERA1",
    "CAMERA2",
    "CAMERA3",
    "OSDALT1",
    "OSDALT2",
    "OSDALT3",
    "NAVCOURSEHOLD",
    "BRAKING",
    "USER1",
    "USER2",
    "FPVANGLEMIX",
    "LOITERDIRCHN",
    "MSPRCOVERRIDE",
    "PREARM",
    "TURTLE",
    "NAVCRUISE",
    "AUTOLEVEL",
    "PLANWPMISSION",
    "SOARING",
    "USER3",
    "USER4",
    "CHANGEMISSION",
    "BEEPERMUTE",
    "MULTIFUNCTION",
    "MIXERPROFILE",
    "MIXERTRANSITION",
    "ANGLEHOLD",
];

/// `boxId_e` 8.0 … 9.1: 23 removed, + 56-59 gimbal.
const RC_MODES_V8: &[&str] = &[
    "ARM",
    "ANGLE",
    "HORIZON",
    "NAVALTHOLD",
    "HEADINGHOLD",
    "HEADFREE",
    "HEADADJ",
    "CAMSTAB",
    "NAVRTH",
    "NAVPOSHOLD",
    "MANUAL",
    "BEEPERON",
    "LEDLOW",
    "LIGHTS",
    "NAVLAUNCH",
    "OSD",
    "TELEMETRY",
    "BLACKBOX",
    "FAILSAFE",
    "NAVWP",
    "AIRMODE",
    "HOMERESET",
    "GCSNAV",
    "",
    "SURFACE",
    "FLAPERON",
    "TURNASSIST",
    "AUTOTRIM",
    "AUTOTUNE",
    "CAMERA1",
    "CAMERA2",
    "CAMERA3",
    "OSDALT1",
    "OSDALT2",
    "OSDALT3",
    "NAVCOURSEHOLD",
    "BRAKING",
    "USER1",
    "USER2",
    "FPVANGLEMIX",
    "LOITERDIRCHN",
    "MSPRCOVERRIDE",
    "PREARM",
    "TURTLE",
    "NAVCRUISE",
    "AUTOLEVEL",
    "PLANWPMISSION",
    "SOARING",
    "USER3",
    "USER4",
    "CHANGEMISSION",
    "BEEPERMUTE",
    "MULTIFUNCTION",
    "MIXERPROFILE",
    "MIXERTRANSITION",
    "ANGLEHOLD",
    "GIMBALTLOCK",
    "GIMBALRLOCK",
    "GIMBALCENTER",
    "GIMBALHTRK",
];

/// `boxId_e` 10.0: + 60-63 (firmware-researcher, 10.0.0-rc2 `rc_modes.h`).
const RC_MODES_V10: &[&str] = &[
    "ARM",
    "ANGLE",
    "HORIZON",
    "NAVALTHOLD",
    "HEADINGHOLD",
    "HEADFREE",
    "HEADADJ",
    "CAMSTAB",
    "NAVRTH",
    "NAVPOSHOLD",
    "MANUAL",
    "BEEPERON",
    "LEDLOW",
    "LIGHTS",
    "NAVLAUNCH",
    "OSD",
    "TELEMETRY",
    "BLACKBOX",
    "FAILSAFE",
    "NAVWP",
    "AIRMODE",
    "HOMERESET",
    "GCSNAV",
    "",
    "SURFACE",
    "FLAPERON",
    "TURNASSIST",
    "AUTOTRIM",
    "AUTOTUNE",
    "CAMERA1",
    "CAMERA2",
    "CAMERA3",
    "OSDALT1",
    "OSDALT2",
    "OSDALT3",
    "NAVCOURSEHOLD",
    "BRAKING",
    "USER1",
    "USER2",
    "FPVANGLEMIX",
    "LOITERDIRCHN",
    "MSPRCOVERRIDE",
    "PREARM",
    "TURTLE",
    "NAVCRUISE",
    "AUTOLEVEL",
    "PLANWPMISSION",
    "SOARING",
    "USER3",
    "USER4",
    "CHANGEMISSION",
    "BEEPERMUTE",
    "MULTIFUNCTION",
    "MIXERPROFILE",
    "MIXERTRANSITION",
    "ANGLEHOLD",
    "GIMBALTLOCK",
    "GIMBALRLOCK",
    "GIMBALCENTER",
    "GIMBALHTRK",
    "AUTOSPEED",
    "TERRAINAGLHOLD",
    "INFLIGHTMENU",
    "THRUSTREVERSE",
];

/// `flightModeFlags_e` 7.0: bits 0-16.
const ACTIVE_MODES_V7_0: &[&str] = &[
    "ANGLE_MODE",
    "HORIZON_MODE",
    "HEADING_MODE",
    "NAV_ALTHOLD_MODE",
    "NAV_RTH_MODE",
    "NAV_POSHOLD_MODE",
    "HEADFREE_MODE",
    "NAV_LAUNCH_MODE",
    "MANUAL_MODE",
    "FAILSAFE_MODE",
    "AUTO_TUNE",
    "NAV_WP_MODE",
    "NAV_COURSE_HOLD_MODE",
    "FLAPERON",
    "TURN_ASSISTANT",
    "TURTLE_MODE",
    "SOARING_MODE",
];

/// `flightModeFlags_e` 7.1: + ANGLEHOLD_MODE, NAV_FW_AUTOLAND.
const ACTIVE_MODES_V7_1: &[&str] = &[
    "ANGLE_MODE",
    "HORIZON_MODE",
    "HEADING_MODE",
    "NAV_ALTHOLD_MODE",
    "NAV_RTH_MODE",
    "NAV_POSHOLD_MODE",
    "HEADFREE_MODE",
    "NAV_LAUNCH_MODE",
    "MANUAL_MODE",
    "FAILSAFE_MODE",
    "AUTO_TUNE",
    "NAV_WP_MODE",
    "NAV_COURSE_HOLD_MODE",
    "FLAPERON",
    "TURN_ASSISTANT",
    "TURTLE_MODE",
    "SOARING_MODE",
    "ANGLEHOLD_MODE",
    "NAV_FW_AUTOLAND",
];

/// `flightModeFlags_e` 8.0+: + NAV_SEND_TO.
const ACTIVE_MODES_V8: &[&str] = &[
    "ANGLE_MODE",
    "HORIZON_MODE",
    "HEADING_MODE",
    "NAV_ALTHOLD_MODE",
    "NAV_RTH_MODE",
    "NAV_POSHOLD_MODE",
    "HEADFREE_MODE",
    "NAV_LAUNCH_MODE",
    "MANUAL_MODE",
    "FAILSAFE_MODE",
    "AUTO_TUNE",
    "NAV_WP_MODE",
    "NAV_COURSE_HOLD_MODE",
    "FLAPERON",
    "TURN_ASSISTANT",
    "TURTLE_MODE",
    "SOARING_MODE",
    "ANGLEHOLD_MODE",
    "NAV_FW_AUTOLAND",
    "NAV_SEND_TO",
];

/// `stateFlags_t` 7.0: bit 10 unused, no TAILSITTER.
const STATE_FLAGS_V7_0: &[&str] = &[
    "GPS_FIX_HOME",
    "GPS_FIX",
    "CALIBRATE_MAG",
    "SMALL_ANGLE",
    "FIXED_WING_LEGACY",
    "ANTI_WINDUP",
    "FLAPERON_AVAILABLE",
    "NAV_MOTOR_STOP_OR_IDLE",
    "COMPASS_CALIBRATED",
    "ACCELEROMETER_CALIBRATED",
    "",
    "NAV_CRUISE_BRAKING",
    "NAV_CRUISE_BRAKING_BOOST",
    "NAV_CRUISE_BRAKING_LOCKED",
    "NAV_EXTRA_ARMING_SAFETY_BYPASSED",
    "AIRMODE_ACTIVE",
    "ESC_SENSOR_ENABLED",
    "AIRPLANE",
    "MULTIROTOR",
    "ROVER",
    "BOAT",
    "ALTITUDE_CONTROL",
    "MOVE_FORWARD_ONLY",
    "SET_REVERSIBLE_MOTORS_FORWARD",
    "FW_HEADING_USE_YAW",
    "ANTI_WINDUP_DEACTIVATED",
    "LANDING_DETECTED",
    "IN_FLIGHT_EMERG_REARM",
];

/// `stateFlags_t` 7.1: + 28 TAILSITTER, bit 10 still unused.
const STATE_FLAGS_V7_1: &[&str] = &[
    "GPS_FIX_HOME",
    "GPS_FIX",
    "CALIBRATE_MAG",
    "SMALL_ANGLE",
    "FIXED_WING_LEGACY",
    "ANTI_WINDUP",
    "FLAPERON_AVAILABLE",
    "NAV_MOTOR_STOP_OR_IDLE",
    "COMPASS_CALIBRATED",
    "ACCELEROMETER_CALIBRATED",
    "",
    "NAV_CRUISE_BRAKING",
    "NAV_CRUISE_BRAKING_BOOST",
    "NAV_CRUISE_BRAKING_LOCKED",
    "NAV_EXTRA_ARMING_SAFETY_BYPASSED",
    "AIRMODE_ACTIVE",
    "ESC_SENSOR_ENABLED",
    "AIRPLANE",
    "MULTIROTOR",
    "ROVER",
    "BOAT",
    "ALTITUDE_CONTROL",
    "MOVE_FORWARD_ONLY",
    "SET_REVERSIBLE_MOTORS_FORWARD",
    "FW_HEADING_USE_YAW",
    "ANTI_WINDUP_DEACTIVATED",
    "LANDING_DETECTED",
    "IN_FLIGHT_EMERG_REARM",
    "TAILSITTER",
];

/// `stateFlags_t` 8.0+: + 10 GPS_ESTIMATED_FIX.
const STATE_FLAGS_V8: &[&str] = &[
    "GPS_FIX_HOME",
    "GPS_FIX",
    "CALIBRATE_MAG",
    "SMALL_ANGLE",
    "FIXED_WING_LEGACY",
    "ANTI_WINDUP",
    "FLAPERON_AVAILABLE",
    "NAV_MOTOR_STOP_OR_IDLE",
    "COMPASS_CALIBRATED",
    "ACCELEROMETER_CALIBRATED",
    "GPS_ESTIMATED_FIX",
    "NAV_CRUISE_BRAKING",
    "NAV_CRUISE_BRAKING_BOOST",
    "NAV_CRUISE_BRAKING_LOCKED",
    "NAV_EXTRA_ARMING_SAFETY_BYPASSED",
    "AIRMODE_ACTIVE",
    "ESC_SENSOR_ENABLED",
    "AIRPLANE",
    "MULTIROTOR",
    "ROVER",
    "BOAT",
    "ALTITUDE_CONTROL",
    "MOVE_FORWARD_ONLY",
    "SET_REVERSIBLE_MOTORS_FORWARD",
    "FW_HEADING_USE_YAW",
    "ANTI_WINDUP_DEACTIVATED",
    "LANDING_DETECTED",
    "IN_FLIGHT_EMERG_REARM",
    "TAILSITTER",
];

/// `failsafePhase_e`, identical in all tags.
const FAILSAFE_PHASES: &[&str] = &[
    "IDLE",
    "RX_LOSS_DETECTED",
    "RX_LOSS_IDLE",
    "RETURN_TO_HOME",
    "LANDING",
    "LANDED",
    "RX_LOSS_MONITORING",
    "RX_LOSS_RECOVERED",
];

/// `disarmReason_t` 7.x: 5 = KILLSWITCH.
const DISARM_REASONS_V7: &[&str] = &[
    "NONE",
    "TIMEOUT",
    "STICKS",
    "SWITCH_3D",
    "SWITCH",
    "KILLSWITCH",
    "FAILSAFE",
    "NAVIGATION",
    "LANDING",
];

/// `disarmReason_t` 8.0+: 5 removed.
const DISARM_REASONS_V8: &[&str] = &[
    "NONE",
    "TIMEOUT",
    "STICKS",
    "SWITCH_3D",
    "SWITCH",
    "",
    "FAILSAFE",
    "NAVIGATION",
    "LANDING",
];

const OVERRIDES: &[(char, &str, i32)] = &[('P', "accVib", 1)];

/// `loopIteration` is the main group's row key: `u64`, so the INC predictor's `prev + 1 + skipped`
/// (not wrapped to 32 bit, as in blackbox-tools) is kept as decoded.
const DTYPE_OVERRIDES: &[(&str, &str, DType)] = &[("main", "loopIteration", DType::U64)];

/// Every field of the INAV 9.1.0 tables (blackbox.c `blackboxMainFields`, `blackboxGpsGFields`,
/// `blackboxGpsHFields`, `blackboxSlowFields`, all conditions and `#ifdef`s) plus the 10.0.0-rc2
/// additions (`fwAutoSpeedP/I`, `gyroRaw2[0..2]`, S `droneCANBusOffCount`, `terrainAGL`,
/// `terrainAMSL`). 7.x/8.x names are a subset. Units: firmware-researcher 2026-10-06 (INAV 9.1.0
/// source, `loadMainState` / `loadSlowState` / `writeGPSFrame` and the sensor drivers); `None` =
/// dimensionless, an enum / bit field, or a firmware-internal scale. Notes: `axisP/I/D/F` are PID
/// outputs on the mixer's µs scale; `mcSurface*` and `navSurf` are never assigned (always 0);
/// `rcCommand[0..2]` are ±500 stick units (yaw sign inverted); `servo[k]` is output
/// `servo[k + minServoIndex]`; `navPos` is NEU relative to the GPS origin (first fix after boot),
/// not to home; `surfaceRaw` −1 = out of range, −2 = failure; temperatures −1250 = invalid;
/// `terrainAGL` −1 / `terrainAMSL` `i32::MIN` = none; `escTemperature` unit from the ESC protocol
/// (unverified). An exact name must come before a `[` prefix of the same field.
const FIELDS: &[(&str, &str, Option<&str>)] = &[
    ("main", "loopIteration", None),
    ("main", "time", Some("us")),
    ("main", "axisRate[", Some("deg/s")),
    ("main", "axisP[", None),
    ("main", "axisI[", None),
    ("main", "axisD[", None),
    ("main", "axisF[", None),
    ("main", "fwAltP", Some("deg/10")),
    ("main", "fwAltI", Some("deg/10")),
    ("main", "fwAltD", Some("deg/10")),
    ("main", "fwAltOut", Some("deg/10")),
    ("main", "fwPosP", Some("deg/10")),
    ("main", "fwPosI", Some("deg/10")),
    ("main", "fwPosD", Some("deg/10")),
    ("main", "fwPosOut", Some("deg/10")),
    ("main", "fwAutoSpeedP", Some("us")),
    ("main", "fwAutoSpeedI", Some("us")),
    ("main", "mcPosAxisP[", Some("cm/s")),
    // X/Y: requested acceleration; Z: throttle correction.
    ("main", "mcVelAxisP[2]", Some("us")),
    ("main", "mcVelAxisI[2]", Some("us")),
    ("main", "mcVelAxisD[2]", Some("us")),
    ("main", "mcVelAxisFF[2]", Some("us")),
    ("main", "mcVelAxisOut[2]", Some("us")),
    ("main", "mcVelAxisP[", Some("cm/s²")),
    ("main", "mcVelAxisI[", Some("cm/s²")),
    ("main", "mcVelAxisD[", Some("cm/s²")),
    ("main", "mcVelAxisFF[", Some("cm/s²")),
    ("main", "mcVelAxisOut[", Some("cm/s²")),
    ("main", "mcSurfaceP", None),
    ("main", "mcSurfaceI", None),
    ("main", "mcSurfaceD", None),
    ("main", "mcSurfaceOut", None),
    ("main", "rcData[", Some("us")),
    ("main", "rcCommand[3]", Some("us")),
    ("main", "rcCommand[", None),
    ("main", "vbat", Some("V/100")),
    ("main", "amperage", Some("A/100")),
    ("main", "magADC[", None),
    ("main", "BaroAlt", Some("cm")),
    ("main", "AirSpeed", Some("cm/s")),
    ("main", "surfaceRaw", Some("cm")),
    ("main", "rssi", None),
    ("main", "gyroADC[", None),
    ("main", "gyroRaw[", Some("deg/s")),
    ("main", "gyroRaw2[", Some("deg/s")),
    ("main", "gyroPeakRoll[", Some("Hz")),
    ("main", "gyroPeakPitch[", Some("Hz")),
    ("main", "gyroPeakYaw[", Some("Hz")),
    ("main", "accSmooth[", Some("g/acc_1G")),
    ("main", "accVib", Some("g/acc_1G")),
    ("main", "attitude[", Some("deg/10")),
    ("main", "debug[", None),
    ("main", "motor[", Some("us")),
    ("main", "servo[", Some("us")),
    ("main", "navState", None),
    ("main", "navFlags", None),
    ("main", "navEPH", Some("cm")),
    ("main", "navEPV", Some("cm")),
    ("main", "navPos[", Some("cm")),
    ("main", "navVel[", Some("cm/s")),
    ("main", "navTgtVel[", Some("cm/s")),
    ("main", "navTgtPos[", Some("cm")),
    ("main", "navTgtHdg", Some("deg/100")),
    ("main", "navSurf", None),
    ("main", "navAcc[", Some("cm/s²")),
    ("gps", "time", Some("us")),
    ("gps", "GPS_fixType", None),
    ("gps", "GPS_numSat", None),
    ("gps", "GPS_coord[", Some("1e-7deg")),
    ("gps", "GPS_altitude", Some("m")),
    ("gps", "GPS_speed", Some("cm/s")),
    ("gps", "GPS_ground_course", Some("deg/10")),
    ("gps", "GPS_hdop", Some("1/100")),
    ("gps", "GPS_eph", Some("cm")),
    ("gps", "GPS_epv", Some("cm")),
    ("gps", "GPS_velned[", Some("cm/s")),
    ("home", "GPS_home[", Some("1e-7deg")),
    ("slow", "activeWpNumber", None),
    ("slow", "flightModeFlags", None),
    ("slow", "flightModeFlags2", None),
    ("slow", "activeFlightModeFlags", None),
    ("slow", "stateFlags", None),
    ("slow", "failsafePhase", None),
    ("slow", "rxSignalReceived", None),
    ("slow", "rxFlightChannelsValid", None),
    ("slow", "rxUpdateRate", Some("Hz")),
    ("slow", "hwHealthStatus", None),
    ("slow", "powerSupplyImpedance", Some("mOhm")),
    ("slow", "sagCompensatedVBat", Some("V/100")),
    ("slow", "wind[", Some("cm/s")),
    ("slow", "mspOverrideFlags", None),
    ("slow", "IMUTemperature", Some("degC/10")),
    ("slow", "baroTemperature", Some("degC/10")),
    ("slow", "sens0Temp", Some("degC/10")),
    ("slow", "sens1Temp", Some("degC/10")),
    ("slow", "sens2Temp", Some("degC/10")),
    ("slow", "sens3Temp", Some("degC/10")),
    ("slow", "sens4Temp", Some("degC/10")),
    ("slow", "sens5Temp", Some("degC/10")),
    ("slow", "sens6Temp", Some("degC/10")),
    ("slow", "sens7Temp", Some("degC/10")),
    ("slow", "escRPM", Some("rpm")),
    ("slow", "escTemperature", Some("degC")),
    ("slow", "droneCANBusOffCount", None),
    ("slow", "terrainAGL", Some("cm")),
    ("slow", "terrainAMSL", Some("cm")),
];

// --- Version lines. -----------------------------------------------------------------------------

static INAV_7_0: InavSchema = INAV_7_0_BASE;

static INAV_7_1: InavSchema = InavSchema {
    label: "INAV 7.1",
    rc_mode_names: RC_MODES_V7_1,
    active_flight_mode_names: ACTIVE_MODES_V7_1,
    state_flag_names: STATE_FLAGS_V7_1,
    ..INAV_7_0_BASE
};

/// 8.0 (= 8.0.1).
static INAV_8_0: InavSchema = InavSchema {
    label: "INAV 8.0",
    rc_mode_names: RC_MODES_V8,
    active_flight_mode_names: ACTIVE_MODES_V8,
    state_flag_names: STATE_FLAGS_V8,
    disarm_reason_names: DISARM_REASONS_V8,
    ..INAV_7_0_BASE
};

/// 9.0 (= 9.0.1) and 9.1: the 8.0 tables, the MOTORS-flag quirk fixed.
static INAV_9: InavSchema = InavSchema {
    label: "INAV 9.x",
    rc_mode_names: RC_MODES_V8,
    active_flight_mode_names: ACTIVE_MODES_V8,
    state_flag_names: STATE_FLAGS_V8,
    disarm_reason_names: DISARM_REASONS_V8,
    motor0_without_header: false,
    slow_terrain_remap: true,
    ..INAV_7_0_BASE
};

/// 10.0 (checked by the firmware-researcher against 10.0.0-rc2; no 10.x tag locally).
static INAV_10: InavSchema = InavSchema {
    label: "INAV 10.x",
    rc_mode_names: RC_MODES_V10,
    active_flight_mode_names: ACTIVE_MODES_V8,
    state_flag_names: STATE_FLAGS_V8,
    disarm_reason_names: DISARM_REASONS_V8,
    motor0_without_header: false,
    slow_terrain_remap: true,
    ..INAV_7_0_BASE
};

/// Base for struct-update syntax (a `static` cannot be used as the base of another one).
const INAV_7_0_BASE: InavSchema = InavSchema {
    label: "INAV 7.0",
    events: EVENTS,
    rc_mode_names: RC_MODES_V7_0,
    active_flight_mode_names: ACTIVE_MODES_V7_0,
    state_flag_names: STATE_FLAGS_V7_0,
    failsafe_phase_names: FAILSAFE_PHASES,
    disarm_reason_names: DISARM_REASONS_V7,
    predictor_overrides: OVERRIDES,
    slow_frames_absolute: true,
    motor0_without_header: true,
    slow_terrain_remap: false,
    fields: FIELDS,
    dtype_overrides: DTYPE_OVERRIDES,
};

/// The schema for a parsed `Firmware revision` (`(major, minor, patch)`). Versions outside the
/// known lines use the nearest one: below 7.0 (unsupported, ADR-009) or unparsable → 7.0; above
/// 10 → 10.x. The caller logs a warning for those.
pub fn schema_for(version: Option<(u8, u8, u8)>) -> (&'static InavSchema, bool) {
    match version {
        Some((7, 0, _)) => (&INAV_7_0, true),
        Some((7, _, _)) => (&INAV_7_1, true),
        Some((8, _, _)) => (&INAV_8_0, true),
        Some((9, _, _)) => (&INAV_9, true),
        Some((10, _, _)) => (&INAV_10, true),
        Some((major, _, _)) if major > 10 => (&INAV_10, false),
        _ => (&INAV_7_0, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_selection() {
        assert_eq!(schema_for(Some((9, 1, 0))).0.label, "INAV 9.x");
        assert_eq!(schema_for(Some((9, 0, 1))).0.label, "INAV 9.x");
        assert_eq!(schema_for(Some((8, 0, 1))).0.label, "INAV 8.0");
        assert_eq!(schema_for(Some((7, 1, 2))).0.label, "INAV 7.1");
        assert_eq!(schema_for(Some((7, 0, 0))).0.label, "INAV 7.0");
        assert_eq!(schema_for(Some((10, 0, 0))).0.label, "INAV 10.x");
        assert!(!schema_for(Some((11, 0, 0))).1);
        assert!(!schema_for(Some((6, 1, 0))).1);
        assert!(!schema_for(None).1);
        assert!(schema_for(Some((8, 0, 1))).0.motor0_without_header);
        assert!(!schema_for(Some((9, 0, 0))).0.motor0_without_header);
        assert!(schema_for(Some((10, 0, 0))).0.slow_terrain_remap);
        assert!(schema_for(Some((9, 1, 0))).0.slow_terrain_remap);
        assert!(!schema_for(Some((8, 0, 1))).0.slow_terrain_remap);
    }

    #[test]
    fn tables_are_bit_aligned() {
        // Positions are the enum values: spot-check the ones that moved between lines.
        assert_eq!(RC_MODES_V7_0.len(), 55);
        assert_eq!(RC_MODES_V7_0[23], "KILLSWITCH");
        assert_eq!(RC_MODES_V7_1[55], "ANGLEHOLD");
        assert_eq!(RC_MODES_V8[23], "");
        assert_eq!(RC_MODES_V8[24], "SURFACE");
        assert_eq!(RC_MODES_V8[59], "GIMBALHTRK");
        assert_eq!(RC_MODES_V10[63], "THRUSTREVERSE");
        assert_eq!(STATE_FLAGS_V8[10], "GPS_ESTIMATED_FIX");
        assert_eq!(STATE_FLAGS_V7_1[28], "TAILSITTER");
        assert_eq!(STATE_FLAGS_V7_0.len(), 28);
        assert_eq!(ACTIVE_MODES_V8[19], "NAV_SEND_TO");
        assert_eq!(DISARM_REASONS_V8[8], "LANDING");
        for s in [&INAV_7_0, &INAV_7_1, &INAV_8_0, &INAV_9, &INAV_10] {
            assert!(s.events.iter().all(|e| e.attrs.len() <= MAX_EVENT_ATTRS));
            assert_eq!(s.failsafe_phase_names.len(), 8);
            assert!(s.rc_mode_names.len() <= 64 && s.state_flag_names.len() <= 32);
            assert!(s.active_flight_mode_names.len() <= 32 && s.disarm_reason_names.len() == 9);
        }
    }

    #[test]
    fn units() {
        let s = schema_for(Some((9, 1, 0))).0;
        let unit = |g, f| s.field_unit(g, f, 1.0);
        assert_eq!(
            s.field_unit("main", "gyroADC[1]", 1.0),
            Some(Some("deg/s".into()))
        );
        assert_eq!(s.field_unit("main", "gyroADC[1]", 0.5), Some(None));
        assert_eq!(unit("gps", "GPS_coord[0]"), Some(Some("1e-7deg".into())));
        assert_eq!(unit("main", "vbat"), Some(Some("V/100".into())));
        assert_eq!(unit("main", "motor[7]"), Some(Some("us".into())));
        assert_eq!(unit("main", "axisP[0]"), Some(None));
        assert_eq!(unit("main", "rcCommand[3]"), Some(Some("us".into())));
        assert_eq!(unit("main", "rcCommand[1]"), Some(None));
        assert_eq!(unit("main", "mcVelAxisFF[2]"), Some(Some("us".into())));
        assert_eq!(unit("main", "mcVelAxisFF[0]"), Some(Some("cm/s²".into())));
        assert_eq!(unit("main", "noSuchField"), None);
        assert_eq!(unit("slow", "vbat"), None);
    }

    #[test]
    fn column_types() {
        let s = schema_for(Some((9, 1, 0))).0;
        assert_eq!(s.field_dtype("main", "loopIteration", false), DType::U64);
        assert_eq!(s.field_dtype("main", "gyroADC[0]", true), DType::I32);
        assert_eq!(s.field_dtype("slow", "flightModeFlags", false), DType::U32);
        assert_eq!(s.field_dtype("gps", "GPS_coord[0]", true), DType::I32);
        assert_eq!(s.field_dtype("slow", "loopIteration", false), DType::U32);
        let kinds: Vec<DType> = s
            .event(13)
            .unwrap()
            .attrs
            .iter()
            .map(|a| a.kind.dtype())
            .collect();
        assert_eq!(kinds, [DType::U32, DType::I32, DType::F32]);
        assert_eq!(s.event(0).unwrap().attrs[0].kind.dtype(), DType::I64);
        // The decoder builds one `events` column per attribute name: one type per name.
        let attrs: Vec<&EventAttr> = s.events.iter().flat_map(|e| e.attrs).collect();
        for a in &attrs {
            assert_ne!(a.column, "type");
            for b in &attrs {
                assert!(a.column != b.column || a.kind.dtype() == b.kind.dtype());
            }
        }
    }
}
