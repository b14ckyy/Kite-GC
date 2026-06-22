// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Safehome + fixed-wing autoland config (INAV). See docs/active/AUTOLAND_SAFEHOME.md.
//
// `safehome_read_all` always reads the 8 safehome slots + the two radius settings (so the map can show
// them on any INAV ≥7.0); the per-site approaches + nav_fw_land_* settings are read only when the
// autoland feature is present (≥7.1) — older firmware lacks MSP2_INAV_FW_APPROACH. `safehome_write_all`
// is the "Save to FC" batch: all safehomes + approaches + settings in one go, then a single
// MSP_EEPROM_WRITE to persist. Editing/writing is a ≥7.1 path only (the frontend gates the button).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::fc_settings::{read_uint_setting, set_uint_setting};
use crate::msp::{
    MSP2_INAV_FW_APPROACH, MSP2_INAV_SAFEHOME, MSP2_INAV_SET_FW_APPROACH, MSP2_INAV_SET_SAFEHOME,
    MSP_EEPROM_WRITE,
};
use crate::scheduler::SchedulerHandle;
use crate::state::{ActiveProtocol, AppState};

/// Number of safehome slots in INAV (indices 0..7). FW_APPROACH shares these indices (8+ = mission
/// LAND waypoints, handled separately later).
const MAX_SAFE_HOMES: u8 = 8;

/// One safehome point (lat/lon in degrees × 1e7).
#[derive(Serialize, Deserialize, Clone)]
pub struct SafeHome {
    pub index: u8,
    pub enabled: bool,
    pub lat: i32,
    pub lon: i32,
}

/// Per-site fixed-wing approach config (`fwapproach`). Headings: positive = bidirectional, negative =
/// exclusive direction, 0 = off. `approach_direction` 0 = left turns, 1 = right turns.
#[derive(Serialize, Deserialize, Clone)]
pub struct FwApproach {
    pub index: u8,
    pub approach_alt_cm: i32,
    pub land_alt_cm: i32,
    pub approach_direction: u8,
    pub heading1: i16,
    pub heading2: i16,
    pub sea_level_ref: bool,
}

/// Global autoland / safehome settings (read by name; `None` when the firmware lacks the setting).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AutolandSettings {
    pub approach_length_cm: Option<u32>,
    pub pitch2throttle_mod: Option<u16>,
    pub glide_alt_cm: Option<u16>,
    pub flare_alt_cm: Option<u16>,
    pub glide_pitch_deg: Option<u8>,
    pub flare_pitch_deg: Option<u8>,
    pub max_tailwind_cms: Option<u16>,
    pub safehome_usage_mode: Option<u8>,
    pub rth_allow_landing: Option<u8>,
}

/// Full safehome + autoland config snapshot for the frontend.
#[derive(Serialize, Deserialize, Clone)]
pub struct SafeHomeConfig {
    pub safehomes: Vec<SafeHome>,
    /// Per-site approaches (slots 0..7). Empty when autoland isn't available (<7.1).
    pub approaches: Vec<FwApproach>,
    /// `safehome_max_distance` in **cm** (green ring; ÷100 for metres). Read on all versions.
    pub safehome_max_distance_cm: Option<u32>,
    /// `nav_fw_loiter_radius` in **cm** (yellow ring; ÷100 for metres). Read on all versions.
    pub loiter_radius_cm: Option<u32>,
    pub autoland: AutolandSettings,
    /// True when the autoland feature is available (≥7.1) — drives editing + approach overlay.
    pub has_autoland: bool,
}

/// Resolve the MSP scheduler handle, erroring for non-MSP / disconnected links.
fn msp_handle<'a>(
    proto: &'a Option<ActiveProtocol>,
) -> Result<&'a SchedulerHandle, String> {
    match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => Ok(h),
        Some(_) => Err("FC is not running MSP (INAV)".into()),
        None => Err("Not connected".into()),
    }
}

/// Read all safehomes + radius settings (always) + approaches + autoland settings (≥7.1).
#[tauri::command(async)]
pub fn safehome_read_all(state: State<'_, AppState>) -> Result<SafeHomeConfig, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = msp_handle(&proto)?;

    let has_autoland = {
        let info = state.fc_info.lock().map_err(|e| e.to_string())?;
        info.as_ref()
            .and_then(|fc| fc.features.as_ref())
            .map(|f| f.autoland_config)
            .unwrap_or(false)
    };

    // Safehomes — per-index: resp = [idx, enabled, lat(4), lon(4)] = 10 bytes.
    let mut safehomes = Vec::with_capacity(MAX_SAFE_HOMES as usize);
    for i in 0..MAX_SAFE_HOMES {
        let r = handle.msp_request(MSP2_INAV_SAFEHOME, &[i])?;
        if r.len() >= 10 {
            safehomes.push(SafeHome {
                index: r[0],
                enabled: r[1] != 0,
                lat: i32::from_le_bytes([r[2], r[3], r[4], r[5]]),
                lon: i32::from_le_bytes([r[6], r[7], r[8], r[9]]),
            });
        }
    }

    let safehome_max_distance_cm = read_uint_setting(handle, "safehome_max_distance").map(|v| v as u32);
    let loiter_radius_cm = read_uint_setting(handle, "nav_fw_loiter_radius").map(|v| v as u32);

    let (approaches, autoland) = if has_autoland {
        let mut a = Vec::with_capacity(MAX_SAFE_HOMES as usize);
        for i in 0..MAX_SAFE_HOMES {
            // resp = [idx, approachAlt(4), landAlt(4), dir, hdg1(2), hdg2(2), seaLevel] = 15 bytes.
            let r = handle.msp_request(MSP2_INAV_FW_APPROACH, &[i])?;
            if r.len() >= 15 {
                a.push(FwApproach {
                    index: r[0],
                    approach_alt_cm: i32::from_le_bytes([r[1], r[2], r[3], r[4]]),
                    land_alt_cm: i32::from_le_bytes([r[5], r[6], r[7], r[8]]),
                    approach_direction: r[9],
                    heading1: i16::from_le_bytes([r[10], r[11]]),
                    heading2: i16::from_le_bytes([r[12], r[13]]),
                    sea_level_ref: r[14] != 0,
                });
            }
        }
        let autoland = AutolandSettings {
            approach_length_cm: read_uint_setting(handle, "nav_fw_land_approach_length").map(|v| v as u32),
            pitch2throttle_mod: read_uint_setting(handle, "nav_fw_land_final_approach_pitch2throttle_mod").map(|v| v as u16),
            glide_alt_cm: read_uint_setting(handle, "nav_fw_land_glide_alt").map(|v| v as u16),
            flare_alt_cm: read_uint_setting(handle, "nav_fw_land_flare_alt").map(|v| v as u16),
            glide_pitch_deg: read_uint_setting(handle, "nav_fw_land_glide_pitch").map(|v| v as u8),
            flare_pitch_deg: read_uint_setting(handle, "nav_fw_land_flare_pitch").map(|v| v as u8),
            max_tailwind_cms: read_uint_setting(handle, "nav_fw_land_max_tailwind").map(|v| v as u16),
            safehome_usage_mode: read_uint_setting(handle, "safehome_usage_mode").map(|v| v as u8),
            rth_allow_landing: read_uint_setting(handle, "nav_rth_allow_landing").map(|v| v as u8),
        };
        (a, autoland)
    } else {
        (Vec::new(), AutolandSettings::default())
    };

    eprintln!(
        "[SAFEHOME] read {} safehomes, {} approaches, autoland={} (max_dist={:?}cm loiter={:?}cm)",
        safehomes.len(), approaches.len(), has_autoland, safehome_max_distance_cm, loiter_radius_cm
    );

    Ok(SafeHomeConfig {
        safehomes,
        approaches,
        safehome_max_distance_cm,
        loiter_radius_cm,
        autoland,
        has_autoland,
    })
}

/// "Save to FC": write the whole config as a batch (all safehomes + approaches + settings), then a
/// single EEPROM write to persist. ≥7.1 path (the frontend only exposes the button there).
#[tauri::command(async)]
pub fn safehome_write_all(config: SafeHomeConfig, state: State<'_, AppState>) -> Result<(), String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = msp_handle(&proto)?;

    // Safehomes: [idx, enabled, lat(4), lon(4)].
    for sh in &config.safehomes {
        let mut p = Vec::with_capacity(10);
        p.push(sh.index);
        p.push(sh.enabled as u8);
        p.extend_from_slice(&sh.lat.to_le_bytes());
        p.extend_from_slice(&sh.lon.to_le_bytes());
        handle.msp_request(MSP2_INAV_SET_SAFEHOME, &p)?;
    }

    // Approaches: [idx, approachAlt(4), landAlt(4), dir, hdg1(2), hdg2(2), seaLevel].
    for ap in &config.approaches {
        let mut p = Vec::with_capacity(15);
        p.push(ap.index);
        p.extend_from_slice(&ap.approach_alt_cm.to_le_bytes());
        p.extend_from_slice(&ap.land_alt_cm.to_le_bytes());
        p.push(ap.approach_direction);
        p.extend_from_slice(&ap.heading1.to_le_bytes());
        p.extend_from_slice(&ap.heading2.to_le_bytes());
        p.push(ap.sea_level_ref as u8);
        handle.msp_request(MSP2_INAV_SET_FW_APPROACH, &p)?;
    }

    // Global settings (native width preserved by set_uint_setting). Only write the ones we have.
    let a = &config.autoland;
    let writes: [(&str, Option<u64>); 11] = [
        ("safehome_max_distance", config.safehome_max_distance_cm.map(|v| v as u64)),
        ("nav_fw_loiter_radius", config.loiter_radius_cm.map(|v| v as u64)),
        ("nav_fw_land_approach_length", a.approach_length_cm.map(|v| v as u64)),
        ("nav_fw_land_final_approach_pitch2throttle_mod", a.pitch2throttle_mod.map(|v| v as u64)),
        ("nav_fw_land_glide_alt", a.glide_alt_cm.map(|v| v as u64)),
        ("nav_fw_land_flare_alt", a.flare_alt_cm.map(|v| v as u64)),
        ("nav_fw_land_glide_pitch", a.glide_pitch_deg.map(|v| v as u64)),
        ("nav_fw_land_flare_pitch", a.flare_pitch_deg.map(|v| v as u64)),
        ("nav_fw_land_max_tailwind", a.max_tailwind_cms.map(|v| v as u64)),
        ("safehome_usage_mode", a.safehome_usage_mode.map(|v| v as u64)),
        ("nav_rth_allow_landing", a.rth_allow_landing.map(|v| v as u64)),
    ];
    for (name, value) in writes {
        if let Some(v) = value {
            set_uint_setting(handle, name, v)?;
        }
    }

    // Persist everything to EEPROM (single write).
    handle.msp_request(MSP_EEPROM_WRITE, &[])?;
    eprintln!(
        "[SAFEHOME] saved {} safehomes + {} approaches + settings to FC (EEPROM written)",
        config.safehomes.len(), config.approaches.len()
    );
    Ok(())
}
