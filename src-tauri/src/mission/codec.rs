// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Waypoint Codec
// Encode/decode the 21-byte MSP_WP / MSP_SET_WP payload and MSP_WP_GETINFO response.

use super::types::{MissionInfo, WpAction, Waypoint, ALT_MODE_AMSL, ALT_MODE_REL, P3_ALT_TYPE};

/// Decode a MSP_WP (118) response payload into a Waypoint
/// Payload layout (21 bytes, little-endian):
///   u8  wp_no, u8 action, i32 lat, i32 lon, i32 alt, i16 p1, i16 p2, i16 p3, u8 flag
pub fn decode_wp(payload: &[u8]) -> Result<Waypoint, String> {
    if payload.len() < 21 {
        return Err(format!(
            "MSP_WP payload too short: {} bytes (expected 21)",
            payload.len()
        ));
    }

    let number = payload[0];
    let action_raw = payload[1];
    let action = WpAction::from_u8(action_raw)
        .ok_or_else(|| format!("Unknown WP action: {}", action_raw))?;

    let lat = i32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
    let lon = i32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]);
    let altitude = i32::from_le_bytes([payload[10], payload[11], payload[12], payload[13]]);
    let p1 = i16::from_le_bytes([payload[14], payload[15]]);
    let p2 = i16::from_le_bytes([payload[16], payload[17]]);
    let p3 = i16::from_le_bytes([payload[18], payload[19]]);
    let flag = payload[20];

    Ok(Waypoint {
        number,
        action,
        lat,
        lon,
        altitude,
        p1,
        p2,
        p3,
        flag,
        // FC only stores REL/AMSL; derive the GCS mode from the p3 alt-type bit.
        alt_mode: if (p3 as u16) & P3_ALT_TYPE != 0 { ALT_MODE_AMSL } else { ALT_MODE_REL },
    })
}

/// Encode a Waypoint into a 21-byte MSP_SET_WP (209) payload
pub fn encode_wp(wp: &Waypoint) -> Vec<u8> {
    let mut buf = Vec::with_capacity(21);
    buf.push(wp.number);
    buf.push(wp.action as u8);
    buf.extend_from_slice(&wp.lat.to_le_bytes());
    buf.extend_from_slice(&wp.lon.to_le_bytes());
    buf.extend_from_slice(&wp.altitude.to_le_bytes());
    buf.extend_from_slice(&wp.p1.to_le_bytes());
    buf.extend_from_slice(&wp.p2.to_le_bytes());
    buf.extend_from_slice(&wp.p3.to_le_bytes());
    buf.push(wp.flag);
    debug_assert_eq!(buf.len(), 21);
    buf
}

/// Decode MSP_WP_GETINFO (20) response payload into MissionInfo
/// Payload layout: u8 reserved, u8 maxWaypoints, u8 isValidMission, u8 countBusyPoints
pub fn decode_wp_getinfo(payload: &[u8]) -> Result<MissionInfo, String> {
    if payload.len() < 4 {
        return Err(format!(
            "MSP_WP_GETINFO payload too short: {} bytes (expected 4)",
            payload.len()
        ));
    }
    Ok(MissionInfo {
        max_waypoints: payload[1],
        is_valid: payload[2] != 0,
        wp_count: payload[3],
    })
}

/// Decode MSP_NAV_STATUS (121) response
/// Payload: u8 gps_mode, u8 nav_state, u8 active_wp_action, u8 active_wp_number,
///          u8 nav_error, i16 target_bearing
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavStatus {
    pub gps_mode: u8,
    pub nav_state: u8,
    pub active_wp_action: u8,
    pub active_wp_number: u8,
    pub nav_error: u8,
    pub target_bearing: i16,
}

pub fn decode_nav_status(payload: &[u8]) -> Result<NavStatus, String> {
    if payload.len() < 7 {
        return Err(format!(
            "MSP_NAV_STATUS payload too short: {} bytes (expected 7)",
            payload.len()
        ));
    }
    Ok(NavStatus {
        gps_mode: payload[0],
        nav_state: payload[1],
        active_wp_action: payload[2],
        active_wp_number: payload[3],
        nav_error: payload[4],
        target_bearing: i16::from_le_bytes([payload[5], payload[6]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::types::WP_FLAG_LAST;

    #[test]
    fn encode_decode_roundtrip() {
        let wp = Waypoint {
            number: 1,
            action: WpAction::Waypoint,
            lat: 540353000,
            lon: -45170000,
            altitude: 3500,
            p1: 200,
            p2: 0,
            p3: 1, // absolute altitude
            flag: WP_FLAG_LAST,
            alt_mode: ALT_MODE_AMSL,
        };

        let encoded = encode_wp(&wp);
        assert_eq!(encoded.len(), 21);

        let decoded = decode_wp(&encoded).unwrap();
        assert_eq!(decoded.number, wp.number);
        assert_eq!(decoded.action, wp.action);
        assert_eq!(decoded.lat, wp.lat);
        assert_eq!(decoded.lon, wp.lon);
        assert_eq!(decoded.altitude, wp.altitude);
        assert_eq!(decoded.p1, wp.p1);
        assert_eq!(decoded.p2, wp.p2);
        assert_eq!(decoded.p3, wp.p3);
        assert_eq!(decoded.flag, wp.flag);
    }

    #[test]
    fn decode_wp_getinfo_basic() {
        let payload = [0u8, 120, 1, 5]; // reserved=0, max=120, valid=true, count=5
        let info = decode_wp_getinfo(&payload).unwrap();
        assert_eq!(info.max_waypoints, 120);
        assert!(info.is_valid);
        assert_eq!(info.wp_count, 5);
    }

    #[test]
    fn decode_nav_status_basic() {
        let payload = [3, 21, 1, 4, 0, 0x2C, 0x01]; // bearing = 300
        let status = decode_nav_status(&payload).unwrap();
        assert_eq!(status.gps_mode, 3);
        assert_eq!(status.nav_state, 21);
        assert_eq!(status.active_wp_action, 1);
        assert_eq!(status.active_wp_number, 4);
        assert_eq!(status.nav_error, 0);
        assert_eq!(status.target_bearing, 300);
    }

    #[test]
    fn decode_wp_too_short() {
        assert!(decode_wp(&[0; 20]).is_err());
    }

    #[test]
    fn encode_all_action_types() {
        for v in 1..=8u8 {
            let action = WpAction::from_u8(v).unwrap();
            let wp = Waypoint::new(v, action, 0, 0, 0);
            let encoded = encode_wp(&wp);
            let decoded = decode_wp(&encoded).unwrap();
            assert_eq!(decoded.action, action);
        }
    }
}
