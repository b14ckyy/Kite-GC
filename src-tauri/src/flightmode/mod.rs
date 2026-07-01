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

/// Synthetic INAV "arming disabled" bits the passive decoders (CRSF / S.Port) set on *disarmed* telemetry
/// so the toolbar arming indicator can show ready vs not-ready — these links carry only a coarse mode
/// signal, not INAV's real armingFlags bitfield. Real INAV MSP never uses bits 0/1 (armingFlag_e starts at
/// ARMED = bit 2), so reusing them is safe; the frontend maps them to reasons in `helpers/arming.ts`.
pub const ARM_DISABLE_INITIALISING: u32 = 1 << 0; // CRSF "WAIT" (still initialising)
pub const ARM_DISABLE_BLOCKED: u32 = 1 << 1; // CRSF "!ERR" / S.Port arming-disabled

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

/// Dispatch a MAVLink HEARTBEAT `custom_mode` to the right per-autopilot classifier, selected by the
/// `fc_variant` string. PX4 packs the mode (main + sub) into `custom_mode` completely differently from
/// ArduPilot's flat per-vehicle table, so they need separate decoders. Used by the live MAVLink handler
/// and the tlog importer (both have the variant from the HEARTBEAT autopilot field).
pub fn classify_mavlink(custom_mode: u32, variant: &str) -> FlightModeState {
    if variant.eq_ignore_ascii_case("PX4") {
        classify_px4(custom_mode)
    } else {
        classify_ardupilot(custom_mode, variant)
    }
}

/// Classify a PX4 HEARTBEAT `custom_mode` into the canonical model (no modifiers). PX4 uses
/// `union px4_custom_mode`: `main_mode` in bits 16-23 and `sub_mode` in bits 24-31. The sub_mode
/// only refines AUTO and POSCTL; every other mode is identified by main_mode alone. Ids mirror the
/// frontend `MODE_REGISTRY` (`px4_*` plus the shared manual/acro/mission/land/takeoff entries).
pub fn classify_px4(custom_mode: u32) -> FlightModeState {
    let main_mode = ((custom_mode >> 16) & 0xFF) as u8;
    let sub_mode = ((custom_mode >> 24) & 0xFF) as u8;
    match px4_mode_id(main_mode, sub_mode) {
        Some(id) => FlightModeState::primary(id),
        None => FlightModeState::primary(&format!("px4_mode_{}_{}", main_mode, sub_mode)),
    }
}

/// PX4_CUSTOM_MAIN_MODE_* (+ PX4_CUSTOM_SUB_MODE_AUTO_* / _POSCTL_*) → canonical id.
/// Values from PX4 `commander/px4_custom_mode.h`; labels match the QGC mode names.
fn px4_mode_id(main_mode: u8, sub_mode: u8) -> Option<&'static str> {
    Some(match main_mode {
        1 => "manual",            // MANUAL
        2 => "px4_altitude",      // ALTCTL
        3 => match sub_mode {     // POSCTL
            1 => "px4_orbit",     //   POSCTL_ORBIT
            _ => "px4_position",  //   POSCTL_POSCTL / _SLOW
        },
        4 => match sub_mode {     // AUTO
            1 => "px4_ready",         //   AUTO_READY
            2 => "takeoff",           //   AUTO_TAKEOFF
            3 => "px4_hold",          //   AUTO_LOITER  → "Hold"
            4 => "mission",           //   AUTO_MISSION
            5 => "px4_return",        //   AUTO_RTL     → "Return"
            6 => "land",              //   AUTO_LAND
            8 => "px4_follow_me",     //   AUTO_FOLLOW_TARGET
            9 => "px4_precland",      //   AUTO_PRECLAND
            10 => "px4_vtol_takeoff", //   AUTO_VTOL_TAKEOFF
            _ => return None,
        },
        5 => "acro",              // ACRO
        6 => "px4_offboard",      // OFFBOARD
        7 => "px4_stabilized",    // STABILIZED
        8 => "px4_rattitude",     // RATTITUDE
        10 => "px4_termination",  // TERMINATION
        _ => return None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a PX4 custom_mode the way the firmware packs it: main_mode in bits 16-23,
    /// sub_mode in bits 24-31.
    fn px4_mode(main: u8, sub: u8) -> u32 {
        ((sub as u32) << 24) | ((main as u32) << 16)
    }

    #[test]
    fn px4_main_modes() {
        assert_eq!(classify_px4(px4_mode(1, 0)).primary, "manual");
        assert_eq!(classify_px4(px4_mode(2, 0)).primary, "px4_altitude");
        assert_eq!(classify_px4(px4_mode(3, 0)).primary, "px4_position");
        assert_eq!(classify_px4(px4_mode(5, 0)).primary, "acro");
        assert_eq!(classify_px4(px4_mode(6, 0)).primary, "px4_offboard");
        assert_eq!(classify_px4(px4_mode(7, 0)).primary, "px4_stabilized");
    }

    #[test]
    fn px4_auto_sub_modes() {
        assert_eq!(classify_px4(px4_mode(4, 2)).primary, "takeoff");
        assert_eq!(classify_px4(px4_mode(4, 3)).primary, "px4_hold");
        assert_eq!(classify_px4(px4_mode(4, 4)).primary, "mission");
        assert_eq!(classify_px4(px4_mode(4, 5)).primary, "px4_return");
        assert_eq!(classify_px4(px4_mode(4, 6)).primary, "land");
        assert_eq!(classify_px4(px4_mode(3, 1)).primary, "px4_orbit");
    }

    #[test]
    fn px4_unknown_falls_back() {
        assert_eq!(classify_px4(px4_mode(99, 0)).primary, "px4_mode_99_0");
        assert_eq!(classify_px4(px4_mode(4, 99)).primary, "px4_mode_4_99");
    }

    #[test]
    fn dispatch_routes_by_variant() {
        // PX4 variant → PX4 table (main=4/sub=4 = Mission).
        assert_eq!(classify_mavlink(px4_mode(4, 4), "PX4").primary, "mission");
        // ArduPilot variant → flat table (raw custom_mode 4 = Guided on Copter).
        assert_eq!(classify_mavlink(4, "ArduCopter").primary, "guided");
        // The same raw value 4 under PX4 is main_mode 0 (no high bytes) → unknown.
        assert_eq!(classify_mavlink(4, "PX4").primary, "px4_mode_0_0");
    }
}
