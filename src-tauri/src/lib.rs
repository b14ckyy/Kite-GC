// INAV GCS — Ground Control Station for INAV Flight Controllers
// Licensed under GPL-3.0-only

mod commands;
mod mission;
mod msp;
mod scheduler;
mod state;
mod transport;

use commands::connection::{connect, disconnect, list_serial_ports};
use commands::info::get_app_version;
use commands::mission::{
    mission_get, mission_clear, mission_add_wp, mission_insert_wp,
    mission_remove_wp, mission_update_wp, mission_reorder_wp,
    mission_download, mission_upload, mission_export_xml, mission_import_xml,
    mission_save_file, mission_load_file,
};
use mission::store::MissionStore;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .manage(MissionStore::new())
        .invoke_handler(tauri::generate_handler![
            list_serial_ports,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running INAV GCS");
}
