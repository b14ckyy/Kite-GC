// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Unified flight-mode model (see docs/active/FLIGHT_MODE_UNIFIED.md).
//
// Each protocol input adapter classifies its raw mode data into a canonical FlightModeState
// (string ids: one `primary` + zero or more `modifiers`). The pipeline, widget, track-coloring and
// recording/replay consume only this — no protocol awareness downstream. The frontend output
// registry maps ids → label + category (category → colour).

use serde::Serialize;

/// Canonical, protocol-agnostic flight mode: a dominant `primary` mode + active `modifiers`
/// (INAV-style stacking; empty for single-mode protocols like ArduPilot).
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlightModeState {
    pub primary: String,
    pub modifiers: Vec<String>,
}

impl FlightModeState {
    fn primary(id: &str) -> Self {
        Self { primary: id.to_string(), modifiers: Vec::new() }
    }
}

// ── INAV (normalized FLIGHT_MODE bitmask from scheduler::telemetry) ──────────────────────────
// Bit layout matches `box_id_to_flight_mode_bit` / `trackColors.ts` FLIGHT_MODE.
const ANGLE: u32 = 1 << 0;
const HORIZON: u32 = 1 << 1;
const HEADING: u32 = 1 << 2;
const NAV_ALTHOLD: u32 = 1 << 3;
const NAV_RTH: u32 = 1 << 4;
const NAV_POSHOLD: u32 = 1 << 5;
const HEADFREE: u32 = 1 << 6;
const NAV_LAUNCH: u32 = 1 << 7;
const MANUAL: u32 = 1 << 8;
const FAILSAFE: u32 = 1 << 9;
const AUTO_TUNE: u32 = 1 << 10;
const NAV_WP: u32 = 1 << 11;
const NAV_COURSE_HOLD: u32 = 1 << 12;
const FLAPERON: u32 = 1 << 13;
const SOARING: u32 = 1 << 16;
const NAV_FW_AUTOLAND: u32 = 1 << 18;

/// Classify the normalized INAV flight-mode bitmask into the canonical model.
/// Primary follows the same priority order as the old frontend `MODES` list (most specific first);
/// modifiers are the stackable boxes shown as chips.
pub fn classify_inav(flags: u32) -> FlightModeState {
    let primary = if flags & FAILSAFE != 0 && flags & NAV_RTH != 0 {
        "failsafe_rth"
    } else if flags & FAILSAFE != 0 {
        "failsafe"
    } else if flags & NAV_WP != 0 {
        "mission"
    } else if flags & NAV_RTH != 0 {
        "rth"
    } else if flags & NAV_LAUNCH != 0 {
        "launch"
    } else if flags & NAV_POSHOLD != 0 {
        "poshold"
    } else if flags & NAV_COURSE_HOLD != 0 {
        "cruise"
    } else if flags & ANGLE != 0 {
        "angle"
    } else if flags & HORIZON != 0 {
        "horizon"
    } else if flags & MANUAL != 0 {
        "manual"
    } else {
        "acro"
    };

    let mut modifiers = Vec::new();
    if flags & NAV_ALTHOLD != 0 { modifiers.push("althold".to_string()); }
    if flags & HEADING != 0 { modifiers.push("headinghold".to_string()); }
    if flags & HEADFREE != 0 { modifiers.push("headfree".to_string()); }
    if flags & SOARING != 0 { modifiers.push("soaring".to_string()); }
    if flags & AUTO_TUNE != 0 { modifiers.push("autotune".to_string()); }
    if flags & FLAPERON != 0 { modifiers.push("flaperon".to_string()); }
    if flags & NAV_FW_AUTOLAND != 0 { modifiers.push("autoland".to_string()); }

    FlightModeState { primary: primary.to_string(), modifiers }
}

/// Classify an ArduPilot HEARTBEAT custom_mode into the canonical model (no modifiers). The id set
/// mirrors the old frontend `ARDU_PLANE_MODES` / `ARDU_COPTER_MODES` tables; the registry resolves
/// label + category. Plane vs Copter is selected from the fc_variant string.
pub fn classify_ardupilot(custom_mode: u32, variant: &str) -> FlightModeState {
    let is_plane = variant.to_ascii_lowercase().contains("plane");
    let id = if is_plane {
        ardu_plane_mode_id(custom_mode)
    } else {
        ardu_copter_mode_id(custom_mode)
    };
    match id {
        Some(id) => FlightModeState::primary(id),
        None => FlightModeState::primary(&format!("ardu_mode_{}", custom_mode)),
    }
}

fn ardu_plane_mode_id(mode: u32) -> Option<&'static str> {
    Some(match mode {
        0 => "manual",
        1 => "circle",
        2 => "stabilize",
        3 => "training",
        4 => "acro",
        5 => "fbwa",
        6 => "fbwb",
        7 => "cruise",
        8 => "autotune",
        10 => "ardu_auto",
        11 => "rtl",
        12 => "loiter",
        13 => "takeoff",
        14 => "avoid_adsb",
        15 => "guided",
        17 => "qstabilize",
        18 => "qhover",
        19 => "qloiter",
        20 => "qland",
        21 => "qrtl",
        22 => "qautotune",
        23 => "qacro",
        24 => "thermal",
        25 => "loiter_qland",
        _ => return None,
    })
}

fn ardu_copter_mode_id(mode: u32) -> Option<&'static str> {
    Some(match mode {
        0 => "stabilize",
        1 => "acro",
        2 => "althold",
        3 => "ardu_auto",
        4 => "guided",
        5 => "loiter",
        6 => "rtl",
        7 => "circle",
        9 => "land",
        11 => "drift",
        13 => "sport",
        14 => "flip",
        15 => "autotune",
        16 => "poshold",
        17 => "brake",
        18 => "throw",
        19 => "avoid_adsb",
        20 => "guided_nogps",
        21 => "smartrtl",
        22 => "flowhold",
        23 => "follow",
        24 => "zigzag",
        25 => "systemid",
        26 => "autorotate",
        27 => "autortl",
        _ => return None,
    })
}
