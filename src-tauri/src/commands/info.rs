// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Info Commands — app version, build info, etc.

use tauri::Manager;

/// Return the application version. Sourced from the Tauri package info, which resolves to
/// `package.json` (via `tauri.conf.json`'s `version` path) — the single version source of truth.
#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}
