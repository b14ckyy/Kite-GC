// Info Commands — app version, build info, etc.

/// Return application version info
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
