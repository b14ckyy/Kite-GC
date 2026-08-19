// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! HEARTBEAT `autopilot` + `mavtype` → (fc_variant, platform_type), shared by the live handshake and
//! the tlog importer. It lived in both and drifted, so the same vehicle was identified differently
//! live and after import.
//!
//! `fc_variant` is not a label: `flightmode::classify_ardupilot` picks the mode table from it (looks
//! for "plane", else Copter), and the tables disagree on nearly every number — mode 19 is QLOITER on
//! a Plane and AVOID_ADSB on a Copter. A wrong variant yields a plausible wrong mode name, not a
//! missing one.

use ::mavlink::ardupilotmega::{MavAutopilot, MavType};

/// INAV's `flyingPlatformType_e`, reused for every firmware. Mirrors `helpers/uavIcons.ts`.
pub mod platform {
    pub const MULTIROTOR: u8 = 0;
    pub const AIRPLANE: u8 = 1;
    pub const HELICOPTER: u8 = 2;
    pub const TRICOPTER: u8 = 3;
    pub const ROVER: u8 = 4;
    pub const BOAT: u8 = 5;
    /// Real but unmodelled (submarine, airship, unknown) — renders as the generic arrow.
    pub const OTHER: u8 = 6;
    pub const VTOL: u8 = 7;
}

/// MAV_TYPE 19–25. Matched numerically because the names have already been renamed upstream once
/// (`VTOL_DUOROTOR` → `VTOL_TAILSITTER_DUOROTOR`) and `VTOL_RESERVED5` exists so more can be added
/// inside the range — a dialect bump must not drop a new VTOL type into the Copter fallback.
fn is_vtol(t: MavType) -> bool {
    matches!(t as u8, 19..=25)
}

/// A QuadPlane runs ArduPlane and uses the Plane mode table. Some report `MAV_TYPE_FIXED_WING` (those
/// the `Q_ENABLE` probe in `commands/connection.rs` catches), but ArduPlane reports the specific VTOL
/// type when the frame class says so — and those never reached the probe, because it only runs for
/// fixed-wing. That was issue #40.
fn ardupilot_variant(t: MavType) -> &'static str {
    match t {
        MavType::MAV_TYPE_FIXED_WING => "ArduPlane",
        _ if is_vtol(t) => "ArduPlane",
        // A boat is a Rover frame and uses the Rover mode table.
        MavType::MAV_TYPE_GROUND_ROVER | MavType::MAV_TYPE_SURFACE_BOAT => "ArduRover",
        MavType::MAV_TYPE_SUBMARINE => "ArduSub",
        // Antenna trackers and blimps land here too: own firmware, but Kite has no mode table for
        // either, so a distinct variant string would promise support that does not exist.
        _ => "ArduCopter",
    }
}

fn platform_for(t: MavType) -> u8 {
    match t {
        MavType::MAV_TYPE_FIXED_WING => platform::AIRPLANE,
        _ if is_vtol(t) => platform::VTOL,
        // COAXIAL is a coaxial multirotor (a Copter frame class), not a helicopter.
        MavType::MAV_TYPE_QUADROTOR
        | MavType::MAV_TYPE_COAXIAL
        | MavType::MAV_TYPE_HEXAROTOR
        | MavType::MAV_TYPE_OCTOROTOR
        | MavType::MAV_TYPE_DODECAROTOR
        | MavType::MAV_TYPE_DECAROTOR
        | MavType::MAV_TYPE_GENERIC_MULTIROTOR => platform::MULTIROTOR,
        MavType::MAV_TYPE_TRICOPTER => platform::TRICOPTER,
        MavType::MAV_TYPE_HELICOPTER => platform::HELICOPTER,
        MavType::MAV_TYPE_GROUND_ROVER => platform::ROVER,
        MavType::MAV_TYPE_SURFACE_BOAT => platform::BOAT,
        _ => platform::OTHER,
    }
}

pub fn identify(autopilot: MavAutopilot, t: MavType) -> (String, u8) {
    let variant = match autopilot {
        MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA => ardupilot_variant(t).to_string(),
        MavAutopilot::MAV_AUTOPILOT_PX4 => "PX4".to_string(),
        MavAutopilot::MAV_AUTOPILOT_GENERIC => "Generic".to_string(),
        other => format!("{:?}", other),
    };
    (variant, platform_for(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadplane_vtol_types_are_arduplane() {
        for t in [
            MavType::MAV_TYPE_VTOL_TAILSITTER_DUOROTOR,
            MavType::MAV_TYPE_VTOL_TAILSITTER_QUADROTOR,
            MavType::MAV_TYPE_VTOL_TILTROTOR,
            MavType::MAV_TYPE_VTOL_FIXEDROTOR,
            MavType::MAV_TYPE_VTOL_TAILSITTER,
            MavType::MAV_TYPE_VTOL_TILTWING,
            MavType::MAV_TYPE_VTOL_RESERVED5,
        ] {
            let (variant, platform) = identify(MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA, t);
            assert_eq!(variant, "ArduPlane", "{t:?}");
            assert_eq!(platform, platform::VTOL, "{t:?}");
        }
    }

    /// Issue #40 end to end: the reported tiltrotor sat in custom_mode 19 and Kite showed "Avoid ADSB".
    #[test]
    fn vtol_mode_19_resolves_as_qloiter_not_avoid_adsb() {
        let (variant, _) = identify(
            MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
            MavType::MAV_TYPE_VTOL_TILTROTOR,
        );
        assert_eq!(crate::flightmode::classify_mavlink(19, &variant).primary, "qloiter");
    }

    /// Multirotors used to map to platform 2, which is *Helicopter* in the enum — so every ArduPilot
    /// and PX4 copter was labelled a helicopter and fell through to the generic arrow.
    #[test]
    fn multirotors_are_multirotors() {
        for t in [
            MavType::MAV_TYPE_QUADROTOR,
            MavType::MAV_TYPE_COAXIAL,
            MavType::MAV_TYPE_HEXAROTOR,
            MavType::MAV_TYPE_OCTOROTOR,
            MavType::MAV_TYPE_DODECAROTOR,
            MavType::MAV_TYPE_DECAROTOR,
            MavType::MAV_TYPE_GENERIC_MULTIROTOR,
        ] {
            let (variant, platform) = identify(MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA, t);
            assert_eq!(variant, "ArduCopter", "{t:?}");
            assert_eq!(platform, platform::MULTIROTOR, "{t:?}");
        }
        let ap = MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA;
        assert_eq!(identify(ap, MavType::MAV_TYPE_TRICOPTER).1, platform::TRICOPTER);
        assert_eq!(identify(ap, MavType::MAV_TYPE_HELICOPTER).1, platform::HELICOPTER);
    }

    /// Rover/sub used to emit platform 10 / 12, which are not in the enum — the logbook showed
    /// "Unknown (10)". A boat was identified as ArduCopter.
    #[test]
    fn surface_vehicles_use_the_rover_table_and_real_platform_types() {
        let ap = MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA;
        assert_eq!(identify(ap, MavType::MAV_TYPE_SURFACE_BOAT), ("ArduRover".into(), platform::BOAT));
        assert_eq!(identify(ap, MavType::MAV_TYPE_GROUND_ROVER), ("ArduRover".into(), platform::ROVER));
        assert_eq!(identify(ap, MavType::MAV_TYPE_SUBMARINE), ("ArduSub".into(), platform::OTHER));
    }

    #[test]
    fn fixed_wing_and_px4_are_unchanged() {
        let ap = MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA;
        assert_eq!(identify(ap, MavType::MAV_TYPE_FIXED_WING), ("ArduPlane".into(), platform::AIRPLANE));

        let px4 = MavAutopilot::MAV_AUTOPILOT_PX4;
        assert_eq!(identify(px4, MavType::MAV_TYPE_QUADROTOR), ("PX4".into(), platform::MULTIROTOR));
        // PX4 packs the mode identically for every airframe, so a VTOL keeps the plain "PX4" variant.
        assert_eq!(identify(px4, MavType::MAV_TYPE_VTOL_TILTROTOR), ("PX4".into(), platform::VTOL));
    }
}
