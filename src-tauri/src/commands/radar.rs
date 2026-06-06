// Radar Commands — configure the foreign-vehicle tracking subsystem and pull the current snapshot.

use std::sync::atomic::Ordering;

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
    // Runtime flag the MSP scheduler polls for ADS-B-via-MSP (the frontend already gates this on
    // INAV-8.0 support + an active MSP link).
    let msp_on = config.enabled && config.adsb.enabled && config.adsb.msp_from_fc;
    state.radar_msp_enabled.store(msp_on, Ordering::Relaxed);

    let mut mgr = state.radar.lock().map_err(|e| e.to_string())?;
    mgr.configure(&config, &app);
    Ok(())
}

/// Update the live ADS-B query centre (map viewport / UAV position). Cheap — no pipeline restart.
#[tauri::command]
pub fn radar_set_center(lat: f64, lon: f64, state: State<'_, AppState>) -> Result<(), String> {
    let mgr = state.radar.lock().map_err(|e| e.to_string())?;
    mgr.set_center(lat, lon);
    Ok(())
}

/// Current consolidated radar state (used on panel open before the next event arrives).
#[tauri::command]
pub fn radar_snapshot(state: State<'_, AppState>) -> Result<RadarSnapshot, String> {
    let mgr = state.radar.lock().map_err(|e| e.to_string())?;
    Ok(mgr.snapshot())
}
