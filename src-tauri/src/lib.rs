// Kite Ground Control — Universal Ground Control Station
// Licensed under GPL-3.0-only

mod commands;
mod flightlog;
mod mission;
mod msp;
mod scheduler;
mod state;
mod transport;

use commands::connection::{connect, disconnect, list_serial_ports, scan_ble_devices};
use commands::flightlog::{
    flightlog_list, flightlog_get, flightlog_get_track, flightlog_delete,
    flightlog_update_notes, flightlog_update_weather, flightlog_geocode, flightlog_fetch_weather,
    flightlog_default_db_path, flightlog_import_blackbox,
    flightlog_export, flightlog_export_blackbox, flightlog_export_track, flightlog_import_kflight,
    flightlog_kflight_list, flightlog_kflight_get, flightlog_kflight_track,
    flightlog_probe_ardupilot, flightlog_decode_ardupilot_csv,
    flightlog_import_ardupilot,
};
use commands::info::get_app_version;
use commands::mission::{
    mission_get, mission_clear, mission_add_wp, mission_insert_wp,
    mission_remove_wp, mission_update_wp, mission_reorder_wp,
    mission_download, mission_upload, mission_export_xml, mission_import_xml,
    mission_save_file, mission_load_file,
};
use mission::store::MissionStore;
use state::AppState;

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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .manage(MissionStore::new())
        .invoke_handler(tauri::generate_handler![
            list_serial_ports,
            scan_ble_devices,
            connect,
            disconnect,
            get_app_version,
            mission_get,
            mission_clear,
            mission_add_wp,
            mission_insert_wp,
            mission_remove_wp,
            mission_update_wp,
            mission_reorder_wp,
            mission_download,
            mission_upload,
            mission_export_xml,
            mission_import_xml,
            mission_save_file,
            mission_load_file,
            flightlog_list,
            flightlog_get,
            flightlog_get_track,
            flightlog_delete,
            flightlog_update_notes,
            flightlog_update_weather,
            flightlog_geocode,
            flightlog_fetch_weather,
            flightlog_default_db_path,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kite Ground Control");
}
