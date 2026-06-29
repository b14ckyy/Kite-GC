// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mission Commands — Tauri command handlers for mission planning operations

use tauri::{Emitter, State};

use crate::mavlink_proto::{self, ArduWaypoint};
use crate::mission::store::{MissionStore, mission_from_xml, mission_to_xml};
use crate::mission::types::{HomePt, Mission, MissionInfo, Waypoint, WpAction, ALT_MODE_AGL, ALT_MODE_AMSL, ALT_MODE_REL, P3_ALT_TYPE, WP_FLAG_NORMAL, WP_FLAG_LAST, WP_FLAG_FBH};
use crate::mission::codec;
use crate::terrain::TerrainProvider;

/// Apply the GCS altitude mode to a waypoint. When `alt_mode` is provided it is
/// authoritative and the p3 alt-type bit is kept consistent for REL/AMSL (AGL
/// leaves p3 untouched — it is resolved to AMSL on export). When absent (older
/// callers), the mode is derived from the existing p3 bit so behaviour is
/// unchanged.
fn apply_alt_mode(wp: &mut Waypoint, alt_mode: Option<u8>) {
    match alt_mode {
        Some(ALT_MODE_AMSL) => { wp.alt_mode = ALT_MODE_AMSL; wp.p3 |= P3_ALT_TYPE as i16; }
        Some(ALT_MODE_REL)  => { wp.alt_mode = ALT_MODE_REL;  wp.p3 &= !(P3_ALT_TYPE as i16); }
        Some(m)             => { wp.alt_mode = m; } // AGL — p3 resolved at export
        None                => { wp.alt_mode = wp.alt_mode_from_p3(); }
    }
}
use crate::msp::types::{MSP_WP, MSP_WP_GETINFO, MSP_WP_MISSION_LOAD, MSP_WP_MISSION_SAVE, MSP_SET_WP};
use crate::state::{ActiveProtocol, AppState};

/// Waypoint-download progress, emitted as the `mission-download-progress` event during an FC
/// download (both MSP and MAVLink) so the Mission Manager can show an "x of n" counter.
#[derive(Clone, serde::Serialize)]
struct MissionDownloadProgress {
    current: u16,
    total: u16,
}

/// Event name for `MissionDownloadProgress`.
const MISSION_DOWNLOAD_PROGRESS: &str = "mission-download-progress";

/// Get the current mission snapshot
#[tauri::command]
pub fn mission_get(store: State<'_, MissionStore>) -> Mission {
    store.snapshot()
}

/// Clear the current mission
#[tauri::command]
pub fn mission_clear(store: State<'_, MissionStore>) {
    store.clear();
}

/// Replace the entire active-mission waypoint list in one call. Preserves
/// every field (including `alt_mode`). Used by undo/redo restore, where the
/// snapshot is already a valid, numbered mission.
#[tauri::command]
pub fn mission_set(waypoints: Vec<Waypoint>, store: State<'_, MissionStore>) -> Mission {
    store.set_waypoints(waypoints);
    store.snapshot()
}

/// Add a waypoint to the mission
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub fn mission_add_wp(
    action: u8,
    lat: i32,
    lon: i32,
    altitude: i32,
    p1: i16,
    p2: i16,
    p3: i16,
    alt_mode: Option<u8>,
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let mut wp = Waypoint::new(0, wp_action, lat, lon, altitude);
    wp.p1 = p1;
    wp.p2 = p2;
    wp.p3 = p3;
    apply_alt_mode(&mut wp, alt_mode);
    store.push(wp);
    store.snapshot()
}

/// Insert a waypoint at a specific index
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub fn mission_insert_wp(
    index: usize,
    action: u8,
    lat: i32,
    lon: i32,
    altitude: i32,
    p1: i16,
    p2: i16,
    p3: i16,
    alt_mode: Option<u8>,
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let mut wp = Waypoint::new(0, wp_action, lat, lon, altitude);
    wp.p1 = p1;
    wp.p2 = p2;
    wp.p3 = p3;
    apply_alt_mode(&mut wp, alt_mode);
    store.insert(index, wp);
    store.snapshot()
}

/// Remove a waypoint by index
#[tauri::command]
pub fn mission_remove_wp(index: usize, store: State<'_, MissionStore>) -> Mission {
    store.remove(index);
    store.snapshot()
}

/// Update a waypoint at index
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub fn mission_update_wp(
    index: usize,
    action: u8,
    lat: i32,
    lon: i32,
    altitude: i32,
    p1: i16,
    p2: i16,
    p3: i16,
    flag: u8,
    alt_mode: Option<u8>,
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let mut wp = Waypoint {
        number: 0, // will be renumbered
        action: wp_action,
        lat,
        lon,
        altitude,
        p1,
        p2,
        p3,
        flag,
        alt_mode: ALT_MODE_REL,
    };
    apply_alt_mode(&mut wp, alt_mode);
    store.update(index, wp);
    store.snapshot()
}

/// Reorder a waypoint from one index to another
#[tauri::command]
pub fn mission_reorder_wp(from: usize, to: usize, store: State<'_, MissionStore>) -> Mission {
    store.reorder(from, to);
    store.snapshot()
}

/// Download mission from FC via MSP
/// `(async)` runs off the main thread — each MSP_WP request blocks on the scheduler, so a large
/// mission over a slow telemetry link would otherwise freeze the UI (same fix as the MAVLink path).
#[tauri::command(async)]
pub fn mission_download(
    from_eeprom: bool,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    store: State<'_, MissionStore>,
) -> Result<Mission, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission download not supported via MAVLink yet".into()),
        Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("Mission download not available in telemetry monitoring mode".into()),
        None => return Err("Not connected".into()),
    };

    // Optional: load from EEPROM first
    if from_eeprom {
        handle.msp_request(MSP_WP_MISSION_LOAD, &[0])?;
    }

    // Get mission info
    let info_payload = handle.msp_request(MSP_WP_GETINFO, &[])?;
    let info = codec::decode_wp_getinfo(&info_payload)?;

    log::info!(
        "MSP mission download start: max={}, valid={}, count={}",
        info.max_waypoints, info.is_valid, info.wp_count
    );

    // Download each waypoint
    let mut mission = Mission::new();
    mission.info = info.clone();

    let total = info.wp_count as u16;
    let _ = app.emit(MISSION_DOWNLOAD_PROGRESS, MissionDownloadProgress { current: 0, total });
    for i in 1..=info.wp_count {
        let wp_payload = handle.msp_request(MSP_WP, &[i])?;
        let wp = codec::decode_wp(&wp_payload)?;
        log::debug!("MSP download WP {i}/{total}: {wp:?}");
        mission.waypoints.push(wp);
        let _ = app.emit(MISSION_DOWNLOAD_PROGRESS, MissionDownloadProgress { current: i as u16, total });
    }
    mission.dirty = false;
    log::info!("MSP mission download complete: {} waypoints", mission.waypoints.len());

    store.set(mission.clone());
    Ok(mission)
}

/// Query the FC's mission info (MSP_WP_GETINFO) without downloading the waypoints.
/// Used on connect to decide whether to offer downloading the FC's mission.
/// `(async)` — the MSP_WP_GETINFO request blocks on the scheduler; keep it off the main thread.
#[tauri::command(async)]
pub fn mission_fc_info(state: State<'_, AppState>) -> Result<MissionInfo, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission info not supported via MAVLink yet".into()),
        Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("Mission info not available in telemetry monitoring mode".into()),
        None => return Err("Not connected".into()),
    };
    let info_payload = handle.msp_request(MSP_WP_GETINFO, &[])?;
    codec::decode_wp_getinfo(&info_payload)
}

/// Upload mission to FC via MSP (AGL waypoints resolved to AMSL first)
#[tauri::command]
pub async fn mission_upload(
    save_eeprom: bool,
    state: State<'_, AppState>,
    store: State<'_, MissionStore>,
    terrain: State<'_, TerrainProvider>,
) -> Result<Mission, String> {
    let mission = store.snapshot();
    if mission.waypoints.is_empty() {
        return Err("No waypoints to upload".into());
    }

    // Resolve AGL → AMSL before touching the serial handle (async terrain lookup).
    let resolved = resolve_agl(&mission, &terrain).await;

    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission upload not supported via MAVLink yet".into()),
        Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("Mission upload not available in telemetry monitoring mode".into()),
        None => return Err("Not connected".into()),
    };

    let total = resolved.waypoints.len();
    log::info!("MSP mission upload start: {total} waypoints, save_eeprom={save_eeprom}");

    // Upload each waypoint
    for (i, wp) in resolved.waypoints.iter().enumerate() {
        log::debug!("MSP upload WP {}/{}: {wp:?}", i + 1, total);
        let payload = codec::encode_wp(wp);
        handle.msp_request(MSP_SET_WP, &payload)?;
    }

    // Optional: save to EEPROM (mission ID byte reserved → 0).
    if save_eeprom {
        handle.msp_request(MSP_WP_MISSION_SAVE, &[0])?;
    }

    // Verify by reading back mission info
    let info_payload = handle.msp_request(MSP_WP_GETINFO, &[])?;
    let info = codec::decode_wp_getinfo(&info_payload)?;
    log::info!(
        "MSP mission upload complete: FC reports valid={} wpCount={}{}",
        info.is_valid, info.wp_count, if save_eeprom { " (EEPROM saved)" } else { "" }
    );
    store.set_info(info);
    store.mark_clean();

    Ok(store.snapshot())
}

/// Concatenate per-mission waypoint lists into one INAV multi-mission sequence: global 1-based
/// numbering and each segment's last WP flagged `WP_FLAG_LAST` (0xA5) — or `WP_FLAG_FBH` (0x48) kept
/// when that last WP is already a fly-by-home — with every other WP `WP_FLAG_NORMAL`. INAV stores all
/// missions as one list split on those terminator flags, so a stray mid-segment terminator would break
/// the boundaries; mid-segment flags are therefore forced to NORMAL. Empty missions are skipped.
fn assemble_multi_mission(missions: &[Vec<Waypoint>]) -> Vec<Waypoint> {
    let mut out: Vec<Waypoint> = Vec::new();
    for seg in missions {
        if seg.is_empty() {
            continue;
        }
        let last = seg.len() - 1;
        for (i, wp) in seg.iter().enumerate() {
            let mut w = wp.clone();
            w.flag = if i == last {
                if w.flag == WP_FLAG_FBH { WP_FLAG_FBH } else { WP_FLAG_LAST }
            } else {
                WP_FLAG_NORMAL
            };
            out.push(w);
        }
    }
    for (i, w) in out.iter_mut().enumerate() {
        w.number = (i + 1) as u8;
    }
    out
}

/// Upload one or more missions to the FC as a single INAV multi-mission.
///
/// `missions` is the per-mission waypoint lists in order (slot 1..N); they're concatenated with the
/// proper end-of-mission flags + global numbering (`assemble_multi_mission`), AGL waypoints resolved to
/// AMSL, and uploaded WP-by-WP via MSP_SET_WP. The **active** mission index is intentionally NOT set
/// here — selecting the active mission will be handled by the planned INAV control panel.
/// `(async)` off the main thread — the AGL resolve awaits terrain, then the MSP exchange blocks.
#[tauri::command]
pub async fn mission_upload_multi(
    missions: Vec<Vec<Waypoint>>,
    save_eeprom: bool,
    state: State<'_, AppState>,
    store: State<'_, MissionStore>,
    terrain: State<'_, TerrainProvider>,
) -> Result<Mission, String> {
    let combined = assemble_multi_mission(&missions);
    if combined.is_empty() {
        return Err("No waypoints to upload".into());
    }

    // Resolve AGL → AMSL before touching the serial handle (async terrain lookup).
    let mission = Mission { waypoints: combined, info: MissionInfo::default(), dirty: false, home: None };
    let resolved = resolve_agl(&mission, &terrain).await;

    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission upload not supported via MAVLink yet".into()),
        Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("Mission upload not available in telemetry monitoring mode".into()),
        None => return Err("Not connected".into()),
    };

    let total = resolved.waypoints.len();
    log::info!(
        "MSP multi-mission upload start: {} mission(s), {total} waypoints, save_eeprom={save_eeprom}",
        missions.iter().filter(|m| !m.is_empty()).count()
    );
    for (i, wp) in resolved.waypoints.iter().enumerate() {
        log::debug!("MSP upload WP {}/{}: {wp:?}", i + 1, total);
        handle.msp_request(MSP_SET_WP, &codec::encode_wp(wp))?;
    }

    // Optional: save to EEPROM (mission ID byte reserved → 0).
    if save_eeprom {
        handle.msp_request(MSP_WP_MISSION_SAVE, &[0])?;
    }

    let info_payload = handle.msp_request(MSP_WP_GETINFO, &[])?;
    let info = codec::decode_wp_getinfo(&info_payload)?;
    log::info!(
        "MSP multi-mission upload complete: FC reports valid={} wpCount={}{}",
        info.is_valid, info.wp_count, if save_eeprom { " (EEPROM saved)" } else { "" }
    );
    store.set_info(info);
    store.mark_clean();

    Ok(store.snapshot())
}

/// Read the FC's active multi-mission index (`nav_wp_multi_mission_index`). Defaults to 1 when the
/// setting is absent (single-mission / older firmware). `(async)` — the setting read blocks on the
/// scheduler.
#[tauri::command(async)]
pub fn mission_get_active_index(state: State<'_, AppState>) -> Result<u8, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission info not supported via MAVLink yet".into()),
        Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("Mission info not available in telemetry monitoring mode".into()),
        None => return Err("Not connected".into()),
    };
    Ok(crate::commands::fc_settings::read_uint_setting(handle, "nav_wp_multi_mission_index").unwrap_or(1) as u8)
}

/// Resolve AGL waypoints to AMSL for export. INAV/.mission only understand
/// REL/AMSL, so each AGL waypoint's above-ground value is turned into an
/// absolute altitude: `AMSL = terrain_elevation(lat,lon) + AGL`. Non-AGL
/// waypoints pass through unchanged. Best-effort: if terrain is unavailable for
/// a point, the waypoint is left as-is.
async fn resolve_agl(mission: &Mission, terrain: &TerrainProvider) -> Mission {
    let mut out = mission.clone();
    for wp in out.waypoints.iter_mut() {
        if wp.alt_mode == ALT_MODE_AGL && wp.action.has_location() {
            if let Some(ground) = terrain.elevation(wp.lat_deg(), wp.lon_deg()).await {
                let amsl_m = ground as f64 + wp.altitude as f64 / 100.0;
                wp.altitude = (amsl_m * 100.0).round() as i32;
                wp.p3 |= P3_ALT_TYPE as i16; // mark AMSL
                wp.alt_mode = ALT_MODE_AMSL;
            }
        }
    }
    out
}

/// Export mission as MW XML string (AGL waypoints resolved to AMSL).
/// `home` is the planning-time launch point [lat, lon], written as <mwp> meta.
#[tauri::command]
pub async fn mission_export_xml(
    home: Option<(f64, f64)>,
    store: State<'_, MissionStore>,
    terrain: State<'_, TerrainProvider>,
) -> Result<String, String> {
    let mission = store.snapshot();
    let mut resolved = resolve_agl(&mission, &terrain).await;
    if let Some((lat, lon)) = home {
        resolved.home = Some(HomePt { lat, lon });
    }
    Ok(mission_to_xml(&resolved))
}

/// Export a *library* mission (given its canonical waypoints JSON) to a `.mission` file, without
/// touching the loaded map mission. AGL waypoints are resolved to AMSL, like `mission_save_file`.
#[tauri::command]
pub async fn mission_save_file_from_json(
    path: String,
    waypoints_json: String,
    terrain: State<'_, TerrainProvider>,
) -> Result<(), String> {
    let waypoints: Vec<Waypoint> = serde_json::from_str(&waypoints_json)
        .map_err(|e| format!("Invalid mission JSON: {e}"))?;
    let mission = Mission { waypoints, info: MissionInfo::default(), dirty: false, home: None };
    let resolved = resolve_agl(&mission, &terrain).await;
    let xml = mission_to_xml(&resolved);
    std::fs::write(&path, xml).map_err(|e| format!("Failed to save: {e}"))?;
    Ok(())
}

/// Import mission from MW XML string
#[tauri::command]
pub fn mission_import_xml(xml: String, store: State<'_, MissionStore>) -> Result<Mission, String> {
    let mission = mission_from_xml(&xml)?;
    store.set(mission.clone());
    Ok(mission)
}

/// Save mission to a .mission file (AGL waypoints resolved to AMSL).
/// `home` is the planning-time launch point [lat, lon], written as <mwp> meta.
#[tauri::command]
pub async fn mission_save_file(
    path: String,
    home: Option<(f64, f64)>,
    store: State<'_, MissionStore>,
    terrain: State<'_, TerrainProvider>,
) -> Result<(), String> {
    let mission = store.snapshot();
    let mut resolved = resolve_agl(&mission, &terrain).await;
    if let Some((lat, lon)) = home {
        resolved.home = Some(HomePt { lat, lon });
    }
    let xml = mission_to_xml(&resolved);
    std::fs::write(&path, xml).map_err(|e| format!("Failed to save: {e}"))?;
    store.mark_clean();
    Ok(())
}

/// Load mission from a .mission file
#[tauri::command]
pub fn mission_load_file(path: String, store: State<'_, MissionStore>) -> Result<Mission, String> {
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read: {e}"))?;
    let mission = mission_from_xml(&xml)?;
    store.set(mission.clone());
    Ok(mission)
}

/// Download ArduPilot mission from FC via MAVLink mission microprotocol.
/// Returns the mission as a flat Vec<ArduWaypoint> for the frontend store.
///
/// `(async)` runs this on a worker thread: the mission microprotocol blocks while waiting for each
/// MISSION_ITEM (up to seconds each over a slow SiK link). A plain sync command would run on the
/// main thread and freeze the whole UI for the duration of the download.
#[tauri::command(async)]
pub fn ardu_mission_download(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Vec<ArduWaypoint>, String> {
    // Clone the command sender + sysid while holding the mutex briefly.
    // The actual protocol exchange runs after the lock is released.
    let (cmd_tx, fc_sysid, reserve_home) = {
        let proto = state.protocol.lock().map_err(|e| e.to_string())?;
        match proto.as_ref() {
            // PX4 has no home mission-slot (item 0 is a real waypoint); ArduPilot reserves slot 0 for home.
            Some(ActiveProtocol::Mavlink(h)) => (h.cmd_tx_clone(), h.fc_sysid, !h.fc_variant.eq_ignore_ascii_case("px4")),
            Some(ActiveProtocol::Msp(_)) => return Err("FC is not running MAVLink".into()),
            Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("FC is not running MAVLink".into()),
            None => return Err("Not connected".into()),
        }
    };
    mavlink_proto::mission::download(
        &cmd_tx,
        fc_sysid,
        reserve_home,
        ::mavlink::ardupilotmega::MavMissionType::MAV_MISSION_TYPE_MISSION,
        |current, total| {
            let _ = app.emit(MISSION_DOWNLOAD_PROGRESS, MissionDownloadProgress { current, total });
        },
    )
}

/// Upload an ArduPilot mission to the FC via MAVLink mission microprotocol.
/// `(async)` runs this off the main thread — the upload handshake blocks (same reasoning as
/// `ardu_mission_download`).
#[tauri::command(async)]
pub fn ardu_mission_upload(
    waypoints: Vec<ArduWaypoint>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if waypoints.is_empty() {
        return Err("No waypoints to upload".into());
    }
    let (cmd_tx, fc_sysid, reserve_home) = {
        let proto = state.protocol.lock().map_err(|e| e.to_string())?;
        match proto.as_ref() {
            // PX4 has no home mission-slot (item 0 is a real waypoint); ArduPilot reserves slot 0 for home.
            Some(ActiveProtocol::Mavlink(h)) => (h.cmd_tx_clone(), h.fc_sysid, !h.fc_variant.eq_ignore_ascii_case("px4")),
            Some(ActiveProtocol::Msp(_)) => return Err("FC is not running MAVLink".into()),
            Some(ActiveProtocol::PassiveTelemetry(_)) => return Err("FC is not running MAVLink".into()),
            None => return Err("Not connected".into()),
        }
    };
    mavlink_proto::mission::upload(&cmd_tx, fc_sysid, &waypoints, reserve_home, ::mavlink::ardupilotmega::MavMissionType::MAV_MISSION_TYPE_MISSION)
}

/// Read a text file from disk (used for .waypoints and similar formats)
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read file: {e}"))
}

/// Write text content to a file (used for .waypoints and similar formats)
#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("Failed to write file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(action: WpAction, flag: u8) -> Waypoint {
        let mut w = Waypoint::new(0, action, 0, 0, 0);
        w.flag = flag;
        w
    }

    #[test]
    fn assemble_multi_mission_flags_and_numbering() {
        // Two missions: each segment's last WP must become 0xA5, all others 0x00, numbered globally 1..N.
        let m1 = vec![wp(WpAction::Waypoint, WP_FLAG_NORMAL), wp(WpAction::Waypoint, WP_FLAG_LAST)];
        let m2 = vec![
            wp(WpAction::Waypoint, WP_FLAG_NORMAL),
            wp(WpAction::Waypoint, WP_FLAG_NORMAL),
            wp(WpAction::Waypoint, WP_FLAG_LAST),
        ];
        let out = assemble_multi_mission(&[m1, m2]);

        assert_eq!(out.len(), 5);
        let nums: Vec<u8> = out.iter().map(|w| w.number).collect();
        assert_eq!(nums, vec![1, 2, 3, 4, 5]);
        let flags: Vec<u8> = out.iter().map(|w| w.flag).collect();
        assert_eq!(
            flags,
            vec![WP_FLAG_NORMAL, WP_FLAG_LAST, WP_FLAG_NORMAL, WP_FLAG_NORMAL, WP_FLAG_LAST]
        );
    }

    #[test]
    fn assemble_multi_mission_preserves_fbh_last_and_skips_empty() {
        // An empty mission is skipped; a fly-by-home last WP keeps its 0x48 terminator; a stray
        // mid-segment terminator is forced back to NORMAL so it doesn't split the segment.
        let m1 = vec![
            wp(WpAction::Waypoint, WP_FLAG_LAST), // mid-segment stray terminator → must clear
            wp(WpAction::Land, WP_FLAG_FBH),      // last, FBH → keep 0x48
        ];
        let empty: Vec<Waypoint> = vec![];
        let m2 = vec![wp(WpAction::Waypoint, WP_FLAG_NORMAL)]; // single WP → becomes last (0xA5)
        let out = assemble_multi_mission(&[m1, empty, m2]);

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].flag, WP_FLAG_NORMAL); // stray mid terminator cleared
        assert_eq!(out[1].flag, WP_FLAG_FBH); // FBH preserved
        assert_eq!(out[2].flag, WP_FLAG_LAST); // single WP of m2 terminates it
        assert_eq!(out.iter().map(|w| w.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    }
}
