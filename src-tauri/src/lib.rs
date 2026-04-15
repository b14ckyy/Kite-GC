// INAV GCS — Ground Control Station for INAV Flight Controllers
// Licensed under GPL-3.0-only

mod commands;
mod msp;
mod scheduler;
mod state;
mod transport;

use commands::connection::{connect, disconnect, list_serial_ports};
use commands::info::get_app_version;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            list_serial_ports,
            connect,
            disconnect,
            get_app_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running INAV GCS");
}
