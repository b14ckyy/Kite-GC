// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV MSP handshake — identity, version gate, feature set, craft info and home.
// Shared by the serial/network MSP connect path and the MSP-over-MAVLink tunnel (INAV 10.0+). Runs
// on a blocking `Transport` before any scheduler owns it.

use crate::transport::Transport;

use super::features::is_version_supported;
use super::{
    FcInfo, FeatureSet, InavVersion, MspMessage, MSP_API_VERSION, MSP_BLACKBOX_CONFIG, MSP_BOARD_INFO,
    MSP_FC_VARIANT, MSP_FC_VERSION, MSP_NAME, MSP_UID, MSP_WP, MSPV2_INAV_MIXER,
};

/// INAV's RTH home from `MSP_WP #0` (deg / deg / m).
#[derive(Debug, Clone, Copy)]
pub struct HomeFix {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
}

/// Result of a successful INAV handshake.
pub struct MspHandshake {
    pub fc_info: FcInfo,
    /// Home set on the FC, `None` when unset (lat == lon == 0) or not answered.
    pub home: Option<HomeFix>,
}

/// One request with up to `attempts` tries (whole-request retry). `attempts == 1` is exactly one
/// `msp_request`, error text unchanged. A lost transport is never retried.
fn request(
    transport: &mut dyn Transport,
    code: u16,
    payload: &[u8],
    attempts: u32,
) -> Result<MspMessage, String> {
    let attempts = attempts.max(1);
    let mut attempt = 1;
    loop {
        match transport.msp_request(code, payload) {
            Ok(msg) => return Ok(msg),
            Err(e) if attempt >= attempts || transport.is_connection_lost() => return Err(e),
            Err(e) => {
                log::debug!("MSP handshake: 0x{:04X} attempt {}/{} failed ({}) — retrying", code, attempt, attempts, e);
                attempt += 1;
            }
        }
    }
}

/// Run the INAV MSP handshake: API_VERSION, FC_VARIANT (must be "INAV"), FC_VERSION, BOARD_INFO,
/// version gate + feature set, MIXER, NAME, UID, BLACKBOX_CONFIG, WP #0 (home).
///
/// `attempts` = tries per request (1 on a direct MSP link; the MAVLink tunnel passes 2 because a lost
/// TUNNEL chunk means no reply at all). Returns `Err` for a non-INAV / unsupported firmware or a failed
/// identity request; the informational requests (mixer, name, …) only log on failure.
pub fn run(transport: &mut dyn Transport, attempts: u32) -> Result<MspHandshake, String> {
    let mut fc_info = FcInfo::default();

    // 1) MSP_API_VERSION → [mspProtocol, apiVersionMajor, apiVersionMinor]
    let resp = request(transport, MSP_API_VERSION, &[], attempts)?;
    if resp.payload.len() >= 3 {
        fc_info.msp_protocol = resp.payload[0];
        fc_info.api_version = format!("{}.{}", resp.payload[1], resp.payload[2]);
    }

    // 2) MSP_FC_VARIANT → 4-byte identifier string (e.g. "INAV")
    let resp = request(transport, MSP_FC_VARIANT, &[], attempts)?;
    fc_info.fc_variant = String::from_utf8_lossy(&resp.payload).trim().to_string();

    // 3) MSP_FC_VERSION → [major, minor, patch]
    let resp = request(transport, MSP_FC_VERSION, &[], attempts)?;
    if resp.payload.len() >= 3 {
        fc_info.fc_version = format!(
            "{}.{}.{}",
            resp.payload[0], resp.payload[1], resp.payload[2]
        );
    }

    // 4) MSP_BOARD_INFO → board identifier (4 bytes) + hw revision (u16 LE)
    let resp = request(transport, MSP_BOARD_INFO, &[], attempts)?;
    if resp.payload.len() >= 4 {
        fc_info.board_id = String::from_utf8_lossy(&resp.payload[..4])
            .trim()
            .to_string();
    }
    if resp.payload.len() >= 6 {
        fc_info.hardware_revision =
            (resp.payload[4] as u16) | ((resp.payload[5] as u16) << 8);
    }

    // ── Version check & feature detection ────────────────────────────
    if fc_info.fc_variant != "INAV" {
        return Err(format!(
            "Unsupported firmware variant: '{}'. Only INAV is currently supported.",
            fc_info.fc_variant
        ));
    }

    let version = InavVersion::parse(&fc_info.fc_version).ok_or_else(|| {
        format!("Cannot parse firmware version: '{}'", fc_info.fc_version)
    })?;

    if !is_version_supported(version) {
        return Err(format!(
            "INAV {} is not supported. Minimum required version is 7.0.0.",
            version
        ));
    }

    let feature_set = FeatureSet::for_version(version);
    log::info!(
        "Feature gates for INAV {}: autoland={}, geozones={}, msp_rc={}, aux_rc={}",
        version,
        feature_set.autoland_config,
        feature_set.geozones,
        feature_set.msp_rc,
        feature_set.aux_rc
    );
    fc_info.features = Some(feature_set);

    // 5) MSP2_INAV_MIXER → platform type and mixer preset
    match request(transport, MSPV2_INAV_MIXER, &[], attempts) {
        Ok(resp) => {
            if resp.payload.len() >= 7 {
                fc_info.platform_type = resp.payload[3];
                fc_info.mixer_preset =
                    (resp.payload[5] as i16) | ((resp.payload[6] as i16) << 8);
            }
        }
        Err(e) => {
            log::warn!("Failed to query mixer config: {}", e);
        }
    }

    // 6) MSP_NAME → craft name configured in the FC
    match request(transport, MSP_NAME, &[], attempts) {
        Ok(resp) => {
            fc_info.craft_name = String::from_utf8_lossy(&resp.payload).trim().to_string();
        }
        Err(e) => {
            log::warn!("Failed to query craft name: {}", e);
        }
    }

    // 6b) MSP_UID → the MCU's 96-bit unique id (three little-endian u32 words), rendered as 24 hex
    // chars. Informational: reconnect identity for the platform-type override, stored per flight.
    match request(transport, MSP_UID, &[], attempts) {
        Ok(resp) if resp.payload.len() >= 12 => {
            fc_info.fc_uid = Some(resp.payload[..12].iter().map(|b| format!("{:02X}", b)).collect());
        }
        Ok(_) => log::warn!("MSP_UID: short reply"),
        Err(e) => log::warn!("Failed to query MSP_UID: {}", e),
    }

    // 6c) MSP_BLACKBOX_CONFIG → [supported, device, …]; device 0 = NONE. Seeds the vehicle library's
    // "blackbox available" flag when the craft is saved from the UAV Info panel.
    match request(transport, MSP_BLACKBOX_CONFIG, &[], attempts) {
        Ok(resp) if resp.payload.len() >= 2 => {
            fc_info.blackbox = Some(resp.payload[0] != 0 && resp.payload[1] != 0);
        }
        Ok(_) => log::debug!("MSP_BLACKBOX_CONFIG: short reply"),
        Err(e) => log::debug!("MSP_BLACKBOX_CONFIG not answered: {}", e),
    }

    // 7) Home position — MSP_WP #0 is INAV's RTH home (GPS_home, lat/lon in deg·1e7). One-shot at
    //    connect so a mid-flight connect / app restart recovers Home; the live arm-transition path
    //    only sets it when we actually witness the arm. Raw-parse the 21-byte WP payload (the home
    //    WP's action byte isn't a normal nav action, so we don't go through decode_wp). lat==lon==0
    //    means no home is set yet (on the ground, pre-arm) → skip; arm will set it live.
    let mut home = None;
    match request(transport, MSP_WP, &[0], attempts) {
        Ok(resp) if resp.payload.len() >= 14 => {
            let p = &resp.payload;
            let lat_e7 = i32::from_le_bytes([p[2], p[3], p[4], p[5]]);
            let lon_e7 = i32::from_le_bytes([p[6], p[7], p[8], p[9]]);
            let alt_cm = i32::from_le_bytes([p[10], p[11], p[12], p[13]]);
            if lat_e7 != 0 || lon_e7 != 0 {
                let fix = HomeFix {
                    lat: lat_e7 as f64 / 1e7,
                    lon: lon_e7 as f64 / 1e7,
                    alt: alt_cm as f64 / 100.0,
                };
                log::info!("Home from FC (MSP_WP 0): {:.7}, {:.7}", fix.lat, fix.lon);
                home = Some(fix);
            } else {
                log::info!("MSP_WP(0): no home set on FC yet");
            }
        }
        Ok(_) => log::warn!("MSP_WP(0) home response too short"),
        Err(e) => log::warn!("Failed to query home (MSP_WP 0): {}", e),
    }

    let transport_desc = transport.description();
    log::info!(
        "Connected to {} {} v{} via {} (board: {}, API: {}, platform: {})",
        fc_info.fc_variant,
        fc_info.fc_version,
        fc_info.api_version,
        transport_desc,
        fc_info.board_id,
        fc_info.api_version,
        fc_info.platform_type,
    );

    Ok(MspHandshake { fc_info, home })
}
