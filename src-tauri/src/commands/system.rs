// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// System power commands — cross-platform AC/battery detection (Windows + Linux + macOS via
// starship-battery, Android via sysfs). Used by the low-power 3D "auto" mode to cap the render frame
// rate on battery — which matters most on a tablet in the field, where the 3D globe is the single
// largest drain and the pack is the flight's clock.

/// Whether the host is currently running on battery (i.e. a battery is present and discharging).
/// Returns false when on AC, fully charged, or there's no battery (desktop) — anything that isn't a
/// clear "discharging" state. Detection failures also report false (treat as AC → no cap).
#[cfg(not(any(target_os = "ios", target_os = "android")))]
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

/// iOS has no starship-battery backend. An iPad is always battery-powered, so report `true` - the
/// low-power 3D "auto" mode then caps the render frame rate, which is the right default on a tablet.
#[cfg(target_os = "ios")]
#[tauri::command]
pub fn system_on_battery() -> bool {
    true
}

/// Android: read the battery state straight from sysfs.
///
/// `starship-battery` has no Android target, and the platform's own `BatteryManager` would mean a JNI
/// round-trip for a value the kernel already publishes as text. The power-supply class is world-readable
/// on Android and reports exactly the state we need — `Charging` / `Discharging` / `Full` /
/// `Not charging` / `Unknown`.
///
/// The node name is not fixed (`battery` on most devices, `bms` on some Qualcomm ones, and a few use a
/// vendor name), so scan the class directory for the first supply of type `Battery` rather than hardcoding
/// one path. Anything unreadable reports false — same "treat as AC, don't cap" fallback as the desktop
/// path, since a wrongly capped frame rate is worse than a missed optimisation.
/// Whether the device's ACTIVE network runs over Wi-Fi (Android: ConnectivityManager, VPN
/// underlays included where the system exposes them; unknown → true, the safe default for
/// the one caller). Used to pause continuous GCS location updates while an RTSP stream
/// runs — fused location's periodic Wi-Fi scans take the radio off-channel and burst-drop
/// RTP every ~10 s (measured on the Teclast M11). Non-Android platforms report false: the
/// pause only exists there.
#[tauri::command]
pub fn system_active_net_is_wifi() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::net::active_net_is_wifi().unwrap_or(true)
    }
    #[cfg(not(target_os = "android"))]
    false
}

#[cfg(target_os = "android")]
#[tauri::command]
pub fn system_on_battery() -> bool {
    const CLASS_DIR: &str = "/sys/class/power_supply";

    let entries = match std::fs::read_dir(CLASS_DIR) {
        Ok(e) => e,
        Err(e) => {
            log::debug!("power_supply class unreadable: {e}");
            return false;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // `type` distinguishes the battery from the USB / AC / wireless supplies in the same directory.
        match std::fs::read_to_string(path.join("type")) {
            Ok(kind) if kind.trim() == "Battery" => {}
            _ => continue,
        }
        if let Ok(status) = std::fs::read_to_string(path.join("status")) {
            if status.trim() == "Discharging" {
                return true;
            }
        }
    }
    false
}
