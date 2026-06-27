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

/// Record a one-line settings snapshot in the current session's log header. Called once by the
/// frontend after it loads the persisted settings (the backend can't see them at startup).
#[tauri::command]
pub fn log_session_settings(summary: String) {
    logging::log_session_settings(&summary);
}
