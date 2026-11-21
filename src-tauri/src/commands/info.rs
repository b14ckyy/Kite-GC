// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Info Commands — app version, build info, etc.

/// Return the application version. Sourced from the Tauri package info, which resolves to
/// `package.json` (via `tauri.conf.json`'s `version` path) — the single version source of truth.
#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// True when the in-app Debug Monitor + verbose diagnostics should be available — a debug build, or a
/// release started with `--debug`. The frontend uses this to surface the dev-only UI in release.
#[tauri::command]
pub fn is_debug_mode() -> bool {
    crate::debug_mode::enabled()
}

/// True when the multi-vehicle (fleet) features are on (see `fleet_mode`); loaded once at startup
/// (`stores/fleetMode`).
#[tauri::command]
pub fn is_fleet_mode() -> bool {
    crate::fleet_mode::enabled()
}

/// True when a fleet session records every vehicle (KITE_FLEET_RECORDING); otherwise only the first
/// vehicle is recorded and the frontend says so once a second one appears.
#[tauri::command]
pub fn is_fleet_group_recording() -> bool {
    crate::fleet_mode::group_recording()
}
