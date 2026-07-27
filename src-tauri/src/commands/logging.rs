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

/// Write a frontend diagnostic into the application log file.
///
/// Whole state machines live in the UI — the video source router and its RTSP reconnect loop above
/// all — so the answer to "why did the stream abort?" was only ever a `console.warn` in DevTools. A
/// tester on a Raspberry Pi has neither DevTools nor a console: a release build's stdout goes nowhere.
/// This is the bridge, so those events land in the same file the Diagnostics page hands out.
///
/// `level` accepts "error" / "warn" / "info" / "debug" (anything else is treated as info). Records are
/// tagged `ui::<area>` so a log reader can tell frontend lines from backend ones at a glance.
#[tauri::command]
pub fn log_frontend(level: String, area: String, message: String) {
    let lvl = match level.to_ascii_lowercase().as_str() {
        "error" => log::Level::Error,
        "warn" | "warning" => log::Level::Warn,
        "debug" => log::Level::Debug,
        _ => log::Level::Info,
    };
    // `log!` needs a literal target, so build the record by hand — which also means applying the level
    // gate ourselves (the macros normally do that before the logger is ever called).
    if lvl > log::max_level() {
        return;
    }
    let target = format!("ui::{area}");
    log::logger().log(
        &log::Record::builder()
            .level(lvl)
            .target(&target)
            .args(format_args!("{message}"))
            .build(),
    );
}
