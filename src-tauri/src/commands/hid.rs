// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// HID / joystick input commands — start/stop the input thread and pick the streamed device. The
// thread emits `hid-devices` (connected list) and `hid-input` (live raw axis/button state). See
// `crate::hid` and docs/active/RC_CONTROL.md.

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
