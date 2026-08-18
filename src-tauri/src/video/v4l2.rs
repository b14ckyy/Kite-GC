// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! V4L2 (Video4Linux2) device enumeration for Linux.
//!
//! Reads `/sys/class/video4linux/` to discover USB capture devices that may
//! not be exposed via the browser's `getUserMedia` API (e.g. HDMI capture
//! dongles like MACROSILICON Hagibis). These devices ARE accessible via V4L2
//! and are ingested by ffmpeg (`ffmpeg -f v4l2 -i /dev/videoN …`) into the
//! embedded MJPEG server, which the sinks read directly.

use serde::Serialize;

/// A discovered V4L2 device.
#[derive(Debug, Clone, Serialize)]
pub struct V4l2Device {
    /// e.g. "/dev/video0"
    pub path: String,
    /// Device name from the kernel (e.g. "MACROSILICON Hagibis")
    pub name: String,
    /// e.g. "usb-0000:00:14.0-2/input0"
    pub bus_info: String,
}

/// Enumerate V4L2 video capture devices by scanning `/sys/class/video4linux/`.
///
/// Returns an empty Vec on non-Linux platforms (no-op).
pub fn enumerate() -> Vec<V4l2Device> {
    #[cfg(target_os = "linux")]
    { enumerate_linux() }
    #[cfg(not(target_os = "linux"))]
    { Vec::new() }
}

#[cfg(target_os = "linux")]
fn enumerate_linux() -> Vec<V4l2Device> {
    let mut devices = Vec::new();
    let dir = match std::fs::read_dir("/sys/class/video4linux") {
        Ok(d) => d,
        Err(_) => return devices,
    };

    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Only process `videoN` entries (skip `video4linux` symlinks, etc.)
        if !name_str.starts_with("video") || name_str.len() <= 5 {
            continue;
        }
        // Ensure it's a decimal number after "video"
        if !name_str[5..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let dev_path = format!("/dev/{}", name_str);
        // Read the device name (e.g. "MACROSILICON Hagibis")
        let dev_name = std::fs::read_to_string(entry.path().join("name"))
            .unwrap_or_default()
            .trim()
            .to_string();

        // Read the bus_info from device/input/inputN/name (best-effort)
        let bus_info = read_input_name(&entry.path())
            .unwrap_or_else(|| String::from("unknown"));

        devices.push(V4l2Device {
            path: dev_path,
            name: dev_name,
            bus_info,
        });
    }
    devices
}

#[cfg(target_os = "linux")]
fn read_input_name(sysfs_dir: &std::path::Path) -> Option<String> {
    // Try to find the first input device name under the V4L2 device
    let inputs_dir = sysfs_dir.join("device").join("input");
    if !inputs_dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(&inputs_dir).ok()?;
    for entry in entries.flatten() {
        let name_file = entry.path().join("name");
        if name_file.is_file() {
            if let Ok(name) = std::fs::read_to_string(&name_file) {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

