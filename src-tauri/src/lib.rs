// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

mod aero;
mod commands;
mod flightlog;
mod flightmode;
mod hid;
mod link_stats;
mod mavlink_proto;
mod mission;
mod msp;
mod passive_telemetry;
mod radar;
mod scheduler;
mod state;
mod telemetry_forward;
mod terrain;
mod transport;

use commands::connection::{connect, disconnect, list_serial_ports, scan_ble_devices, ble_scan_start, ble_scan_stop};
use commands::flightlog::{
    flightlog_list, flightlog_get, flightlog_get_track, flightlog_delete,
    flightlog_update_notes, flightlog_update_craft_name, flightlog_update_platform_type, flightlog_update_pilot, flightlog_update_weather, flightlog_geocode, flightlog_fetch_weather,
    flightlog_default_db_path, flightlog_default_raw_log_path, flightlog_import_blackbox,
    flightlog_export, flightlog_export_blackbox, flightlog_export_track, flightlog_import_kflight,
    flightlog_kflight_list, flightlog_kflight_get, flightlog_kflight_track,
    flightlog_probe_ardupilot, flightlog_decode_ardupilot_csv,
    flightlog_import_ardupilot, flightlog_import_raw,
    flightlog_link_flights, flightlog_unlink_flight, flightlog_find_linkable,
    flightlog_commit_pending_session, flightlog_discard_pending_session,
    flightlog_continue_pending_session,
    flightlog_scan_orphan_sessions, flightlog_recover_discard, flightlog_recover_save_incomplete,
    flightlog_recover_continue,
    mission_db_save, mission_db_get, mission_db_for_flight, flight_link_mission,
    flight_logged_wp_count, mission_db_geocode, mission_db_find_by_hash, mission_db_update,
    flight_unlink_mission, mission_db_delete, mission_db_flights, mission_db_list,
    mission_db_set_meta,
    battery_db_create, battery_db_update, battery_db_list, battery_db_get,
    battery_db_find_by_serial, battery_db_delete, battery_db_add_usage, battery_db_aggregate,
    battery_db_flights, flight_set_battery_serial, battery_db_set_baseline,
    battery_file_write, battery_file_read,
};
use commands::aero::{aero_fetch, aero_cache_stats, aero_cache_clear};
use commands::hid::{
    hid_start, hid_stop, hid_select_device,
    hid_profiles_dir, hid_profile_list, hid_profile_save, hid_profile_delete,
};
use commands::rc::{
    rc_read_fc_config, rc_set_override_bitmask, rc_read_channels,
    rc_stream_update, rc_stream_set_aux, rc_stream_enable, rc_stream_set_rate,
    rc_stream_set_override, rc_stream_set_manual,
};
use commands::info::get_app_version;
use commands::radar::{radar_configure, radar_set_center, radar_set_node_pos, radar_snapshot};
use commands::terrain::{
    terrain_cache_clear, terrain_cache_stats, terrain_elevation, terrain_elevations, terrain_fan,
    terrain_profile,
};
use terrain::TerrainProvider;
use commands::mission::{
    mission_get, mission_clear, mission_set, mission_add_wp, mission_insert_wp,
    mission_remove_wp, mission_update_wp, mission_reorder_wp,
    mission_download, mission_upload, mission_fc_info, mission_export_xml, mission_import_xml,
    mission_save_file, mission_save_file_from_json, mission_load_file,
    read_text_file, write_text_file,
    ardu_mission_download, ardu_mission_upload,
};
use commands::control::{
    mav_set_mode, mav_arm, mav_takeoff, mav_land, mav_rtl, mav_reposition,
    mav_change_speed, mav_mission_start, mav_mission_pause, mav_mission_set_current,
    mav_set_home_here, mav_abort_landing, mav_set_param,
    mav_guided_change_heading, mav_guided_clear_heading, mav_condition_yaw,
    mav_vtol_transition,
};
use hid::HidManager;
use mission::store::MissionStore;
use state::AppState;
use telemetry_forward::{relay_configure, relay_clear, RelayHub};

/// True when a `.portable` marker file sits next to the executable. Used both to
/// redirect data (`setup_portable_mode`) and to gate plugins whose storage path we
/// cannot redirect in portable mode (e.g. window-state on Windows).
pub fn is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
        .unwrap_or(false)
}

/// Detect portable mode: if a `.portable` marker file exists next to the
/// executable, redirect all application data into a `data/` folder beside
/// the exe.  Must be called **before** `run()` so the WebView picks up the
/// environment variables.
pub fn setup_portable_mode() {
    let exe_dir = match std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        Some(d) => d,
        None => return,
    };

    if !exe_dir.join(".portable").exists() {
        return;
    }

    let data_dir = exe_dir.join("data");
    std::fs::create_dir_all(&data_dir).ok();

    let data_str = data_dir.to_string_lossy().to_string();

    // Windows: redirect WebView2 user-data folder
    #[cfg(target_os = "windows")]
    {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &data_str);
    }

    // Linux: redirect XDG directories so WebKitGTK stores data next to the binary
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("XDG_DATA_HOME", &data_str);
        std::env::set_var("XDG_CONFIG_HOME", &data_str);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    // Persist + restore the main window's size/position/maximized state across launches.
    // The plugin saves to the OS app-config dir, which portable mode cannot redirect on
    // Windows (Known-Folder API, not env-driven) — so only enable it in installed mode.
    // Portable builds trade window-geometry persistence for a clean, system-path-free runtime.
    if !is_portable() {
        use tauri_plugin_window_state::StateFlags;
        // Persist everything EXCEPT the decorations flag: we run with a custom titlebar
        // (`decorations: false` in tauri.conf.json), and the state plugin would otherwise
        // restore a previously-saved `decorations: true` and re-add the native title bar.
        builder = builder.plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::DECORATIONS)
                .build(),
        );
    }

    builder
        .manage(AppState::new())
        .manage(MissionStore::new())
        .manage(TerrainProvider::new())
        .manage(RelayHub::new())
        .manage(HidManager::new())
        .invoke_handler(tauri::generate_handler![
            list_serial_ports,
            scan_ble_devices,
            ble_scan_start,
            ble_scan_stop,
            connect,
            disconnect,
            get_app_version,
            mission_get,
            mission_clear,
            mission_set,
            mission_add_wp,
            mission_insert_wp,
            mission_remove_wp,
            mission_update_wp,
            mission_reorder_wp,
            mission_download,
            mission_upload,
            mission_fc_info,
            mission_export_xml,
            mission_import_xml,
            mission_save_file,
            mission_save_file_from_json,
            mission_load_file,
            read_text_file,
            write_text_file,
            ardu_mission_download,
            ardu_mission_upload,
            mav_set_mode,
            mav_arm,
            mav_takeoff,
            mav_land,
            mav_rtl,
            mav_reposition,
            mav_change_speed,
            mav_mission_start,
            mav_mission_pause,
            mav_mission_set_current,
            mav_set_home_here,
            mav_abort_landing,
            mav_set_param,
            mav_guided_change_heading,
            mav_guided_clear_heading,
            mav_condition_yaw,
            mav_vtol_transition,
            flightlog_list,
            flightlog_get,
            flightlog_get_track,
            flightlog_delete,
            mission_db_save,
            mission_db_get,
            mission_db_for_flight,
            flight_link_mission,
            flight_logged_wp_count,
            mission_db_geocode,
            mission_db_find_by_hash,
            mission_db_update,
            flight_unlink_mission,
            mission_db_delete,
            mission_db_flights,
            mission_db_list,
            mission_db_set_meta,
            battery_db_create,
            battery_db_update,
            battery_db_list,
            battery_db_get,
            battery_db_find_by_serial,
            battery_db_delete,
            battery_db_add_usage,
            battery_db_aggregate,
            battery_db_flights,
            flight_set_battery_serial,
            battery_db_set_baseline,
            battery_file_write,
            battery_file_read,
            flightlog_update_notes,
            flightlog_update_craft_name,
            flightlog_update_platform_type,
            flightlog_update_pilot,
            flightlog_update_weather,
            flightlog_geocode,
            flightlog_fetch_weather,
            flightlog_default_db_path,
            flightlog_default_raw_log_path,
            flightlog_import_blackbox,
            flightlog_export,
            flightlog_export_blackbox,
            flightlog_export_track,
            flightlog_import_kflight,
            flightlog_kflight_list,
            flightlog_kflight_get,
            flightlog_kflight_track,
            flightlog_probe_ardupilot,
            flightlog_decode_ardupilot_csv,
            flightlog_import_ardupilot,
            flightlog_import_raw,
            flightlog_link_flights,
            flightlog_unlink_flight,
            flightlog_find_linkable,
            flightlog_commit_pending_session,
            flightlog_discard_pending_session,
            flightlog_continue_pending_session,
            flightlog_scan_orphan_sessions,
            flightlog_recover_discard,
            flightlog_recover_save_incomplete,
            flightlog_recover_continue,
            terrain_elevation,
            terrain_elevations,
            terrain_profile,
            terrain_fan,
            terrain_cache_stats,
            terrain_cache_clear,
            radar_configure,
            radar_set_center,
            radar_set_node_pos,
            radar_snapshot,
            aero_fetch,
            aero_cache_stats,
            aero_cache_clear,
            relay_configure,
            relay_clear,
            hid_start,
            hid_stop,
            hid_select_device,
            hid_profiles_dir,
            hid_profile_list,
            hid_profile_save,
            hid_profile_delete,
            rc_read_fc_config,
            rc_set_override_bitmask,
            rc_read_channels,
            rc_stream_update,
            rc_stream_set_aux,
            rc_stream_enable,
            rc_stream_set_rate,
            rc_stream_set_override,
            rc_stream_set_manual,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kite Ground Control");
}
