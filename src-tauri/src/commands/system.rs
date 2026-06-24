// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// System power commands — cross-platform AC/battery detection (Windows + Linux + macOS via
// starship-battery). Used by the low-power 3D "auto" mode to cap the render frame rate on battery.

/// Whether the host is currently running on battery (i.e. a battery is present and discharging).
/// Returns false when on AC, fully charged, or there's no battery (desktop) — anything that isn't a
/// clear "discharging" state. Detection failures also report false (treat as AC → no cap).
#[tauri::command]
pub fn system_on_battery() -> bool {
    let manager = match starship_battery::Manager::new() {
        Ok(m) => m,
        Err(e) => {
            log::debug!("battery manager unavailable: {e}");
            return false;
        }
    };
    let batteries = match manager.batteries() {
        Ok(b) => b,
        Err(e) => {
            log::debug!("battery enumeration failed: {e}");
            return false;
        }
    };
    for battery in batteries.flatten() {
        if battery.state() == starship_battery::State::Discharging {
            return true;
        }
    }
    false
}
