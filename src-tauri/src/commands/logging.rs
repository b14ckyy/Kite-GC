// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Logging Commands — runtime log-level control + log file location.
// The logger itself is installed in `lib.rs` before the Tauri builder; these commands let the
// frontend apply the user's persisted level on startup and expose the file for "open log folder".

use crate::logging;

/// Set the active log level. Accepts "off" / "error" / "warning" / "debug" (case-insensitive);
/// anything else falls back to "warning".
#[tauri::command]
pub fn set_log_level(level: String) {
    logging::set_level(logging::level_from_str(&level));
}

/// Absolute path of the current log file, or `None` if logging could not be initialized.
#[tauri::command]
pub fn get_log_path() -> Option<String> {
    logging::log_path().map(|p| p.to_string_lossy().to_string())
}
