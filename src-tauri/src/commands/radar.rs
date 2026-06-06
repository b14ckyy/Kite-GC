// Radar Commands — configure the foreign-vehicle tracking subsystem and pull the current snapshot.

use tauri::{AppHandle, State};

use crate::radar::{RadarConfig, RadarSnapshot};
use crate::state::AppState;

/// Apply a radar config: start/stop the pipeline and its sources. Idempotent — call on every
/// `settings.radar` change (incl. the master switch).
#[tauri::command]
pub fn radar_configure(
    config: RadarConfig,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let mut mgr = state.radar.lock().map_err(|e| e.to_string())?;
    mgr.configure(&config, &app);
    Ok(())
}

/// Current consolidated radar state (used on panel open before the next event arrives).
#[tauri::command]
pub fn radar_snapshot(state: State<'_, AppState>) -> Result<RadarSnapshot, String> {
    let mgr = state.radar.lock().map_err(|e| e.to_string())?;
    Ok(mgr.snapshot())
}
