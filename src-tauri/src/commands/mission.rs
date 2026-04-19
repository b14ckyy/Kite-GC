// Mission Commands — Tauri command handlers for mission planning operations

use tauri::State;

use crate::mission::store::{MissionStore, mission_from_xml, mission_to_xml};
use crate::mission::types::{Mission, Waypoint, WpAction};
use crate::mission::codec;
use crate::msp::types::{MSP_WP, MSP_WP_GETINFO, MSP_WP_MISSION_LOAD, MSP_WP_MISSION_SAVE, MSP_SET_WP};
use crate::state::{ActiveProtocol, AppState};

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

/// Add a waypoint to the mission
#[tauri::command]
pub fn mission_add_wp(
    action: u8,
    lat: i32,
    lon: i32,
    altitude: i32,
    p1: i16,
    p2: i16,
    p3: i16,
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let mut wp = Waypoint::new(0, wp_action, lat, lon, altitude);
    wp.p1 = p1;
    wp.p2 = p2;
    wp.p3 = p3;
    store.push(wp);
    store.snapshot()
}

/// Insert a waypoint at a specific index
#[tauri::command]
pub fn mission_insert_wp(
    index: usize,
    action: u8,
    lat: i32,
    lon: i32,
    altitude: i32,
    p1: i16,
    p2: i16,
    p3: i16,
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let mut wp = Waypoint::new(0, wp_action, lat, lon, altitude);
    wp.p1 = p1;
    wp.p2 = p2;
    wp.p3 = p3;
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
    store: State<'_, MissionStore>,
) -> Mission {
    let wp_action = WpAction::from_u8(action).unwrap_or(WpAction::Waypoint);
    let wp = Waypoint {
        number: 0, // will be renumbered
        action: wp_action,
        lat,
        lon,
        altitude,
        p1,
        p2,
        p3,
        flag,
    };
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
#[tauri::command]
pub fn mission_download(
    from_eeprom: bool,
    state: State<'_, AppState>,
    store: State<'_, MissionStore>,
) -> Result<Mission, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission download not supported via MAVLink yet".into()),
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
        "Mission download: max={}, valid={}, count={}",
        info.max_waypoints, info.is_valid, info.wp_count
    );

    // Download each waypoint
    let mut mission = Mission::new();
    mission.info = info.clone();

    for i in 1..=info.wp_count {
        let wp_payload = handle.msp_request(MSP_WP, &[i])?;
        let wp = codec::decode_wp(&wp_payload)?;
        mission.waypoints.push(wp);
    }
    mission.dirty = false;

    store.set(mission.clone());
    Ok(mission)
}

/// Upload mission to FC via MSP
#[tauri::command]
pub fn mission_upload(
    save_eeprom: bool,
    state: State<'_, AppState>,
    store: State<'_, MissionStore>,
) -> Result<Mission, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(ActiveProtocol::Mavlink(_)) => return Err("Mission upload not supported via MAVLink yet".into()),
        None => return Err("Not connected".into()),
    };

    let mission = store.snapshot();
    if mission.waypoints.is_empty() {
        return Err("No waypoints to upload".into());
    }

    log::info!("Mission upload: {} waypoints", mission.waypoints.len());

    // Upload each waypoint
    for wp in &mission.waypoints {
        let payload = codec::encode_wp(wp);
        handle.msp_request(MSP_SET_WP, &payload)?;
    }

    // Optional: save to EEPROM
    if save_eeprom {
        handle.msp_request(MSP_WP_MISSION_SAVE, &[0])?;
    }

    // Verify by reading back mission info
    let info_payload = handle.msp_request(MSP_WP_GETINFO, &[])?;
    let info = codec::decode_wp_getinfo(&info_payload)?;
    store.set_info(info);
    store.mark_clean();

    Ok(store.snapshot())
}

/// Export mission as MW XML string
#[tauri::command]
pub fn mission_export_xml(store: State<'_, MissionStore>) -> String {
    let mission = store.snapshot();
    mission_to_xml(&mission)
}

/// Import mission from MW XML string
#[tauri::command]
pub fn mission_import_xml(xml: String, store: State<'_, MissionStore>) -> Result<Mission, String> {
    let mission = mission_from_xml(&xml)?;
    store.set(mission.clone());
    Ok(mission)
}

/// Save mission to a .mission file
#[tauri::command]
pub fn mission_save_file(path: String, store: State<'_, MissionStore>) -> Result<(), String> {
    let mission = store.snapshot();
    let xml = mission_to_xml(&mission);
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
