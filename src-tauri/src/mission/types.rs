// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mission Waypoint Types
// Data model matching the INAV waypoint system (21-byte MSP_WP struct)

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

/// INAV waypoint action types (MWNP.WPTYPE)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum WpAction {
    Waypoint = 1,
    PosholdUnlim = 2,
    PosholdTime = 3,
    Rth = 4,
    SetPoi = 5,
    Jump = 6,
    SetHead = 7,
    Land = 8,
}

impl WpAction {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Waypoint),
            2 => Some(Self::PosholdUnlim),
            3 => Some(Self::PosholdTime),
            4 => Some(Self::Rth),
            5 => Some(Self::SetPoi),
            6 => Some(Self::Jump),
            7 => Some(Self::SetHead),
            8 => Some(Self::Land),
            _ => None,
        }
    }

    /// Whether this action type has a geographic position on the map
    pub fn has_location(&self) -> bool {
        matches!(
            self,
            Self::Waypoint | Self::PosholdUnlim | Self::PosholdTime | Self::SetPoi | Self::Land
        )
    }

    /// XML action name (for MW mission file format interop)
    pub fn xml_name(&self) -> &'static str {
        match self {
            Self::Waypoint => "WAYPOINT",
            Self::PosholdUnlim => "POSHOLD_UNLIM",
            Self::PosholdTime => "POSHOLD_TIME",
            Self::Rth => "RTH",
            Self::SetPoi => "SET_POI",
            Self::Jump => "JUMP",
            Self::SetHead => "SET_HEAD",
            Self::Land => "LAND",
        }
    }

    pub fn from_xml_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "WAYPOINT" => Some(Self::Waypoint),
            "POSHOLD_UNLIM" | "PH_UNLIM" => Some(Self::PosholdUnlim),
            "POSHOLD_TIME" | "PH_TIME" => Some(Self::PosholdTime),
            "RTH" => Some(Self::Rth),
            "SET_POI" => Some(Self::SetPoi),
            "JUMP" => Some(Self::Jump),
            "SET_HEAD" => Some(Self::SetHead),
            "LAND" => Some(Self::Land),
            _ => None,
        }
    }
}

/// WP end-of-mission flag values
pub const WP_FLAG_NORMAL: u8 = 0x00;
pub const WP_FLAG_LAST: u8 = 0xA5;
pub const WP_FLAG_FBH: u8 = 0x48;

/// P3 bitfield positions
pub const P3_ALT_TYPE: u16 = 1 << 0;
// --- Reference only: uncomment when needed ---
// INAV per-waypoint user-action (UA) p3 bits (bits 1-4 of the p3 bitfield):
// pub const P3_USER_ACTION_1: u16 = 1 << 1;
// pub const P3_USER_ACTION_2: u16 = 1 << 2;
// pub const P3_USER_ACTION_3: u16 = 1 << 3;
// pub const P3_USER_ACTION_4: u16 = 1 << 4;

/// Altitude reference mode (GCS-side). INAV itself only knows REL (p3 bit0=0)
/// and AMSL (p3 bit0=1); AGL is a planning-only mode resolved to AMSL on export.
pub const ALT_MODE_REL: u8 = 0;
pub const ALT_MODE_AMSL: u8 = 1;
pub const ALT_MODE_AGL: u8 = 2;

/// A single INAV waypoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waypoint {
    /// 1-based WP number
    pub number: u8,
    /// Action type
    pub action: WpAction,
    /// Latitude in degrees * 1e7 (WGS84)
    pub lat: i32,
    /// Longitude in degrees * 1e7 (WGS84)
    pub lon: i32,
    /// Altitude in centimetres
    pub altitude: i32,
    /// Parameter 1 (varies by action)
    pub p1: i16,
    /// Parameter 2 (varies by action)
    pub p2: i16,
    /// Parameter 3 (bitfield: bit0=alt_type, bits1-4=user_actions)
    pub p3: i16,
    /// End-of-mission flag (0x00=normal, 0xA5=last, 0x48=FlyByHome)
    pub flag: u8,
    /// GCS-side altitude reference: 0=REL, 1=AMSL, 2=AGL. Authoritative for the
    /// editor; for REL/AMSL it mirrors p3 bit0. AGL holds an above-ground value
    /// in `altitude` and is resolved to AMSL on export. Defaults to 0 for
    /// payloads (e.g. older clients) that don't send it.
    #[serde(default)]
    pub alt_mode: u8,
}

impl Waypoint {
    pub fn new(number: u8, action: WpAction, lat: i32, lon: i32, altitude: i32) -> Self {
        Self {
            number,
            action,
            lat,
            lon,
            altitude,
            p1: 0,
            p2: 0,
            p3: 0,
            flag: WP_FLAG_NORMAL,
            alt_mode: ALT_MODE_REL,
        }
    }

    /// Whether altitude is absolute (AMSL) vs relative to home
    pub fn is_alt_absolute(&self) -> bool {
        (self.p3 as u16) & P3_ALT_TYPE != 0
    }

    /// Derive `alt_mode` from the p3 alt-type bit (REL/AMSL). Used after decoding
    /// from MSP/XML, where AGL never appears (FC only stores REL/AMSL).
    pub fn alt_mode_from_p3(&self) -> u8 {
        if self.is_alt_absolute() { ALT_MODE_AMSL } else { ALT_MODE_REL }
    }

    /// Latitude as floating-point degrees
    pub fn lat_deg(&self) -> f64 {
        self.lat as f64 / 1e7
    }

    /// Longitude as floating-point degrees
    pub fn lon_deg(&self) -> f64 {
        self.lon as f64 / 1e7
    }
}

/// Mission info from MSP_WP_GETINFO
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionInfo {
    /// Maximum number of WPs the FC supports
    pub max_waypoints: u8,
    /// Whether the current mission in FC is valid
    pub is_valid: bool,
    /// Number of waypoints currently stored
    pub wp_count: u8,
}

/// Planned home / launch reference (mwp-compatible `<mwp home-x/home-y>` meta).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HomePt {
    pub lat: f64,
    pub lon: f64,
}

/// A complete mission (collection of waypoints)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Mission {
    /// Waypoints in order (1-based numbering)
    pub waypoints: Vec<Waypoint>,
    /// FC capabilities (populated after MSP_WP_GETINFO)
    pub info: MissionInfo,
    /// Whether the mission has been modified since last save/upload
    pub dirty: bool,
    /// Planned home/launch point parsed from / written to the `.mission` file
    /// `<mwp home-x/home-y>` meta (inter-app compatible with mwp). None if absent.
    #[serde(default)]
    pub home: Option<HomePt>,
}

impl Mission {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all waypoints
    pub fn clear(&mut self) {
        self.waypoints.clear();
        self.dirty = true;
    }

    /// Add a waypoint, auto-setting its number and end-mission flag
    pub fn push(&mut self, mut wp: Waypoint) {
        // Remove last-flag from previous last WP
        if let Some(last) = self.waypoints.last_mut() {
            if last.flag == WP_FLAG_LAST {
                last.flag = WP_FLAG_NORMAL;
            }
        }
        wp.number = (self.waypoints.len() + 1) as u8;
        wp.flag = WP_FLAG_LAST;
        self.waypoints.push(wp);
        self.dirty = true;
    }

    /// Remove waypoint at index, renumber remaining
    pub fn remove(&mut self, index: usize) {
        if index < self.waypoints.len() {
            self.waypoints.remove(index);
            self.renumber();
            self.dirty = true;
        }
    }

    /// Insert waypoint at index, renumber
    pub fn insert(&mut self, index: usize, mut wp: Waypoint) {
        let idx = index.min(self.waypoints.len());
        wp.number = (idx + 1) as u8;
        self.waypoints.insert(idx, wp);
        self.renumber();
        self.dirty = true;
    }

    /// Move waypoint from one index to another
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.waypoints.len() && to < self.waypoints.len() && from != to {
            let wp = self.waypoints.remove(from);
            self.waypoints.insert(to, wp);
            self.renumber();
            self.dirty = true;
        }
    }

    /// Update a waypoint at index
    pub fn update(&mut self, index: usize, wp: Waypoint) {
        if index < self.waypoints.len() {
            self.waypoints[index] = wp;
            self.renumber();
            self.dirty = true;
        }
    }

    /// Renumber all waypoints and fix end-mission flags
    fn renumber(&mut self) {
        for (i, wp) in self.waypoints.iter_mut().enumerate() {
            wp.number = (i + 1) as u8;
            // Clear last-flag for all except actual last
            if wp.flag == WP_FLAG_LAST {
                wp.flag = WP_FLAG_NORMAL;
            }
        }
        // Set last-flag on final WP — unless it's a Fly-by-Home WP, which keeps its
        // 0x48 flag (FBH and LAST share the single flag byte; FBH takes precedence).
        if let Some(last) = self.waypoints.last_mut() {
            if last.flag != WP_FLAG_FBH {
                last.flag = WP_FLAG_LAST;
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wp_action_roundtrip() {
        for v in 1..=8u8 {
            let action = WpAction::from_u8(v).unwrap();
            assert_eq!(action as u8, v);
        }
        assert!(WpAction::from_u8(0).is_none());
        assert!(WpAction::from_u8(9).is_none());
    }

    #[test]
    fn xml_name_roundtrip() {
        for v in 1..=8u8 {
            let action = WpAction::from_u8(v).unwrap();
            let name = action.xml_name();
            assert_eq!(WpAction::from_xml_name(name), Some(action));
        }
    }

    #[test]
    fn mission_push_sets_flags() {
        let mut m = Mission::new();
        m.push(Waypoint::new(0, WpAction::Waypoint, 540000000, -40000000, 5000));
        assert_eq!(m.waypoints[0].number, 1);
        assert_eq!(m.waypoints[0].flag, WP_FLAG_LAST);

        m.push(Waypoint::new(0, WpAction::Waypoint, 540100000, -40100000, 5000));
        assert_eq!(m.waypoints[0].flag, WP_FLAG_NORMAL);
        assert_eq!(m.waypoints[1].number, 2);
        assert_eq!(m.waypoints[1].flag, WP_FLAG_LAST);
    }

    #[test]
    fn renumber_preserves_fbh_on_last_wp() {
        // A Fly-by-Home WP (flag 0x48) as the final waypoint must keep its flag —
        // renumber() must not stamp WP_FLAG_LAST (0xA5) over it.
        let mut m = Mission::new();
        m.push(Waypoint::new(0, WpAction::Waypoint, 540000000, -40000000, 5000));
        m.push(Waypoint::new(0, WpAction::Waypoint, 0, 0, 6000));
        let last = m.waypoints.len() - 1;
        let mut fbh = Waypoint::new(0, WpAction::Waypoint, 0, 0, 6000);
        fbh.flag = WP_FLAG_FBH;
        m.update(last, fbh); // update() triggers renumber()
        assert_eq!(m.waypoints[last].flag, WP_FLAG_FBH);
        assert_eq!(m.waypoints[0].flag, WP_FLAG_NORMAL);
    }

    #[test]
    fn mission_remove_renumbers() {
        let mut m = Mission::new();
        for _ in 0..3 {
            m.push(Waypoint::new(0, WpAction::Waypoint, 0, 0, 0));
        }
        m.remove(1);
        assert_eq!(m.waypoints.len(), 2);
        assert_eq!(m.waypoints[0].number, 1);
        assert_eq!(m.waypoints[1].number, 2);
        assert_eq!(m.waypoints[1].flag, WP_FLAG_LAST);
    }
}
