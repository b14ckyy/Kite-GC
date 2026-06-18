// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV Version Model & Feature Gates
// Enables/disables GCS features based on connected FC firmware version.
// Minimum supported version: INAV 7.0.0

use serde::{Deserialize, Serialize};
use std::fmt;

/// Parsed semantic version for INAV firmware
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InavVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl InavVersion {
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse from FC version string (e.g. "7.1.2")
    pub fn parse(version_str: &str) -> Option<Self> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() >= 3 {
            Some(Self {
                major: parts[0].parse().ok()?,
                minor: parts[1].parse().ok()?,
                patch: parts[2].parse().ok()?,
            })
        } else if parts.len() == 2 {
            Some(Self {
                major: parts[0].parse().ok()?,
                minor: parts[1].parse().ok()?,
                patch: 0,
            })
        } else {
            None
        }
    }

    /// Check if this version is at least the given minimum
    pub fn is_at_least(&self, min: InavVersion) -> bool {
        (self.major, self.minor, self.patch) >= (min.major, min.minor, min.patch)
    }
}

impl fmt::Display for InavVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

// ── Minimum INAV versions for specific features ────────────────────

/// Minimum INAV version supported by this GCS
pub const MIN_SUPPORTED_VERSION: InavVersion = InavVersion::new(7, 0, 0);

/// Feature identifiers — each maps to a minimum INAV version.
/// Add new entries here when INAV introduces features we want to gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Feature {
    /// Core telemetry, MSP basics — always available (7.0+)
    CoreTelemetry,
    /// FW Approach / Autoland configuration (INAV 7.1+)
    AutolandConfig,
    /// Geozone editor and display (INAV 8.0+)
    Geozones,
    /// MSP as full RC protocol — MSP_SET_RAW_RC (INAV 8.0+)
    MspRc,
    /// AUX channel control via MSP (INAV 9.1+)
    AuxRc,
    /// ADS-B vehicle list over MSP — MSP2_ADSB_VEHICLE_LIST (INAV 8.0+)
    AdsbMsp,
    /// RC link statistics — MSP2_INAV_GET_LINK_STATS (INAV 9.1+)
    LinkStats,
}

impl Feature {
    /// Minimum INAV version required for this feature
    pub const fn min_version(&self) -> InavVersion {
        match self {
            Feature::CoreTelemetry => InavVersion::new(7, 0, 0),
            Feature::AutolandConfig => InavVersion::new(7, 1, 0),
            Feature::Geozones => InavVersion::new(8, 0, 0),
            Feature::MspRc => InavVersion::new(8, 0, 0),
            Feature::AuxRc => InavVersion::new(9, 1, 0),
            Feature::AdsbMsp => InavVersion::new(8, 0, 0),
            Feature::LinkStats => InavVersion::new(9, 1, 0),
        }
    }

    /// All known features
    pub const ALL: &'static [Feature] = &[
        Feature::CoreTelemetry,
        Feature::AutolandConfig,
        Feature::Geozones,
        Feature::MspRc,
        Feature::AuxRc,
        Feature::AdsbMsp,
        Feature::LinkStats,
    ];
}

/// Set of features available for the connected FC version.
/// Computed once after handshake, then shared with frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureSet {
    /// Parsed INAV version
    pub version: InavVersion,
    /// Individual feature availability
    pub autoland_config: bool,
    pub geozones: bool,
    pub msp_rc: bool,
    pub aux_rc: bool,
    pub adsb_msp: bool,
    pub link_stats: bool,
}

impl FeatureSet {
    /// Determine available features for the given INAV version
    pub fn for_version(version: InavVersion) -> Self {
        Self {
            version,
            autoland_config: version.is_at_least(Feature::AutolandConfig.min_version()),
            geozones: version.is_at_least(Feature::Geozones.min_version()),
            msp_rc: version.is_at_least(Feature::MspRc.min_version()),
            aux_rc: version.is_at_least(Feature::AuxRc.min_version()),
            adsb_msp: version.is_at_least(Feature::AdsbMsp.min_version()),
            link_stats: version.is_at_least(Feature::LinkStats.min_version()),
        }
    }

    /// Check if a specific feature is available
    pub fn has(&self, feature: Feature) -> bool {
        match feature {
            Feature::CoreTelemetry => true,
            Feature::AutolandConfig => self.autoland_config,
            Feature::Geozones => self.geozones,
            Feature::MspRc => self.msp_rc,
            Feature::AuxRc => self.aux_rc,
            Feature::AdsbMsp => self.adsb_msp,
            Feature::LinkStats => self.link_stats,
        }
    }
}

/// Check if the connected version is supported at all
pub fn is_version_supported(version: InavVersion) -> bool {
    version.is_at_least(MIN_SUPPORTED_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        assert_eq!(
            InavVersion::parse("7.1.2"),
            Some(InavVersion::new(7, 1, 2))
        );
        assert_eq!(
            InavVersion::parse("8.0.0"),
            Some(InavVersion::new(8, 0, 0))
        );
        assert_eq!(InavVersion::parse("invalid"), None);
    }

    #[test]
    fn test_version_comparison() {
        let v710 = InavVersion::new(7, 1, 0);
        let v700 = InavVersion::new(7, 0, 0);
        let v800 = InavVersion::new(8, 0, 0);

        assert!(v710.is_at_least(v700));
        assert!(v710.is_at_least(v710));
        assert!(!v700.is_at_least(v710));
        assert!(v800.is_at_least(v710));
    }

    #[test]
    fn test_min_version_rejected() {
        let v600 = InavVersion::new(6, 0, 0);
        assert!(!is_version_supported(v600));
        assert!(is_version_supported(InavVersion::new(7, 0, 0)));
    }

    #[test]
    fn test_feature_set_inav_700() {
        let fs = FeatureSet::for_version(InavVersion::new(7, 0, 0));
        assert!(fs.has(Feature::CoreTelemetry));
        assert!(!fs.has(Feature::AutolandConfig));
        assert!(!fs.has(Feature::Geozones));
        assert!(!fs.has(Feature::MspRc));
        assert!(!fs.has(Feature::AuxRc));
    }

    #[test]
    fn test_feature_set_inav_710() {
        let fs = FeatureSet::for_version(InavVersion::new(7, 1, 0));
        assert!(fs.has(Feature::CoreTelemetry));
        assert!(fs.has(Feature::AutolandConfig));
        assert!(!fs.has(Feature::Geozones));
    }

    #[test]
    fn test_feature_set_inav_800() {
        let fs = FeatureSet::for_version(InavVersion::new(8, 0, 0));
        assert!(fs.has(Feature::AutolandConfig));
        assert!(fs.has(Feature::Geozones));
        assert!(fs.has(Feature::MspRc));
        assert!(!fs.has(Feature::AuxRc));
    }

    #[test]
    fn test_feature_set_inav_910() {
        let fs = FeatureSet::for_version(InavVersion::new(9, 1, 0));
        assert!(fs.has(Feature::AuxRc));
        assert!(fs.has(Feature::MspRc));
        assert!(fs.has(Feature::Geozones));
        assert!(fs.has(Feature::AutolandConfig));
        assert!(fs.has(Feature::LinkStats));
    }

    #[test]
    fn test_link_stats_gated_below_910() {
        assert!(!FeatureSet::for_version(InavVersion::new(9, 0, 0)).has(Feature::LinkStats));
        assert!(!FeatureSet::for_version(InavVersion::new(8, 0, 0)).has(Feature::LinkStats));
    }

    #[test]
    fn test_version_display() {
        assert_eq!(InavVersion::new(7, 1, 2).to_string(), "7.1.2");
    }
}
