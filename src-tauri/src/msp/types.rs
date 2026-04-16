// MSP Message Types and Constants
// Reference: INAV Configurator MSPCodes.js

use serde::{Deserialize, Serialize};

/// MSP protocol directions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MspDirection {
    Request,  // '<' — from GCS to FC
    Response, // '>' — from FC to GCS
    Error,    // '!' — error response from FC
}

/// MSP protocol version
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MspVersion {
    V1,
    V2,
}

/// A decoded MSP message
#[derive(Debug, Clone)]
pub struct MspMessage {
    pub version: MspVersion,
    pub direction: MspDirection,
    pub code: u16,
    pub payload: Vec<u8>,
}

// ── MSP v1 command codes ────────────────────────────────────────────
pub const MSP_API_VERSION: u16 = 1;
pub const MSP_FC_VARIANT: u16 = 2;
pub const MSP_FC_VERSION: u16 = 3;
pub const MSP_BOARD_INFO: u16 = 4;
pub const MSP_BUILD_INFO: u16 = 5;
pub const MSP_NAME: u16 = 10;
pub const MSP_STATUS: u16 = 101;
pub const MSP_RAW_IMU: u16 = 102;
pub const MSP_SERVO: u16 = 103;
pub const MSP_MOTOR: u16 = 104;
pub const MSP_RC: u16 = 105;
pub const MSP_RAW_GPS: u16 = 106;
pub const MSP_COMP_GPS: u16 = 107;
pub const MSP_ATTITUDE: u16 = 108;
pub const MSP_ALTITUDE: u16 = 109;
pub const MSP_ANALOG: u16 = 110;
pub const MSP_ACTIVEBOXES: u16 = 113;
pub const MSP_STATUS_EX: u16 = 150;
pub const MSP_SENSOR_STATUS: u16 = 151;
pub const MSP_BATTERY_STATE: u16 = 130;
pub const MSP_SET_REBOOT: u16 = 68;
pub const MSP_EEPROM_WRITE: u16 = 250;
pub const MSP_UID: u16 = 160;
pub const MSP_GPS_SV_INFO: u16 = 164;
pub const MSP_GPSSTATISTICS: u16 = 166;

// ── Mission / Waypoint MSP v1 command codes ─────────────────────────
pub const MSP_WP_MISSION_SAVE: u16 = 18;
pub const MSP_WP_MISSION_LOAD: u16 = 19;
pub const MSP_WP_GETINFO: u16 = 20;
pub const MSP_WP: u16 = 118;
pub const MSP_NAV_STATUS: u16 = 121;
pub const MSP_SET_WP: u16 = 209;

// ── INAV MSP v2 command codes ───────────────────────────────────────
pub const MSPV2_INAV_STATUS: u16 = 0x2000;
pub const MSPV2_INAV_ANALOG: u16 = 0x2002;
pub const MSPV2_INAV_MISC: u16 = 0x2003;
pub const MSPV2_INAV_BATTERY_CONFIG: u16 = 0x2005;
pub const MSPV2_INAV_AIR_SPEED: u16 = 0x2009;
pub const MSPV2_INAV_MIXER: u16 = 0x2010;

// ── Jumbo frame threshold ───────────────────────────────────────────
pub const JUMBO_FRAME_MIN_SIZE: u8 = 255;

// ── FC info returned after handshake ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FcInfo {
    /// MSP protocol version (e.g. 0)
    pub msp_protocol: u8,
    /// API version string (e.g. "2.5")
    pub api_version: String,
    /// Flight controller variant (e.g. "INAV")
    pub fc_variant: String,
    /// Flight controller firmware version (e.g. "7.1.2")
    pub fc_version: String,
    /// Board identifier (e.g. "MATF", "SPRF")
    pub board_id: String,
    /// Hardware revision
    pub hardware_revision: u16,
    /// Platform type from mixer config (0=Multirotor, 1=Airplane, 2=Helicopter, etc.)
    pub platform_type: u8,
    /// Applied mixer preset ID
    pub mixer_preset: i16,
    /// Version-dependent feature availability
    pub features: Option<super::features::FeatureSet>,
}
