// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Non-macOS stand-in for the CoreLocation module (location_macos.rs), so the command exists on every
// target and `generate_handler!` needs no platform branches.
//
// Nothing to implement here on purpose: Windows (WebView2) and Linux (WebKitGTK) both expose the Web
// Geolocation API, so `helpers/userLocation.ts` resolves the operator's position in the WebView and
// never calls this. macOS is the only desktop target where that API is missing.

/// Always `None` off macOS: the frontend uses `navigator.geolocation` there instead.
#[tauri::command]
pub fn location_os_last() -> Option<()> {
    None
}
