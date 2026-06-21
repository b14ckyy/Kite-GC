// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// HID / joystick input commands — start/stop the input thread and pick the streamed device. The
// thread emits `hid-devices` (connected list) and `hid-input` (live raw axis/button state). See
// `crate::hid` and docs/archive/MSP_RC_CONTROL.md.

use tauri::{AppHandle, State};

use crate::hid::HidManager;

/// Start streaming HID input (idempotent). Emits the device list immediately, then live snapshots.
#[tauri::command]
pub fn hid_start(app: AppHandle, manager: State<'_, HidManager>) {
    manager.start(app);
}

/// Stop the HID input thread.
#[tauri::command]
pub fn hid_stop(manager: State<'_, HidManager>) {
    manager.stop();
}

/// Choose which connected device to stream on `hid-input`.
#[tauri::command]
pub fn hid_select_device(id: usize, manager: State<'_, HidManager>) {
    manager.select(id);
}

// ── RC control profiles (Documents/KiteGC/HID-Profiles) ──────────────────────────────────────────

/// Absolute path of the profiles directory (created if missing) — for display in the UI.
#[tauri::command]
pub fn hid_profiles_dir() -> Result<String, String> {
    crate::hid::profiles::dir_path()
}

/// Raw JSON text of every saved profile (the frontend parses + sorts them).
#[tauri::command]
pub fn hid_profile_list() -> Result<Vec<String>, String> {
    crate::hid::profiles::list()
}

/// Save (overwrite) a profile by name.
#[tauri::command]
pub fn hid_profile_save(name: String, json: String) -> Result<(), String> {
    crate::hid::profiles::save(&name, &json)
}

/// Delete a profile by name.
#[tauri::command]
pub fn hid_profile_delete(name: String) -> Result<(), String> {
    crate::hid::profiles::delete(&name)
}
