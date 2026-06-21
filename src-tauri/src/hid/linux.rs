// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Linux HID backend — evdev (pure Rust). Reads the device's raw ABS axes, BTN keys and HAT axes
// directly via EVIOCGABS / EVIOCGKEY ioctls each tick (no event-stream draining needed for a live
// state view). HAT0..3 X/Y absolute-axis pairs are surfaced as hats, the rest as axes, matching the
// Windows backend's shape. See docs/archive/MSP_RC_CONTROL.md §6.
//
// NOTE: compiled only on Linux; it is not built/verified on the Windows dev host. Requires read access
// to /dev/input/event* (the user must be in the `input` group).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use evdev::{AbsoluteAxisCode, Device, KeyCode};

use super::{HidAxis, HidButton, HidDevice, HidHat, HidSnapshot};

const RESCAN_INTERVAL: Duration = Duration::from_millis(1000);
/// evdev HAT axis code range (ABS_HAT0X..ABS_HAT3Y); these are grouped into hats, not listed as axes.
const HAT_RANGE: std::ops::RangeInclusive<u16> = 0x10..=0x17;
/// BTN_* keycodes start here; a joystick/gamepad advertises buttons in 0x120..0x140.
const BTN_BASE: u16 = 0x100;
const JOY_BTN_RANGE: std::ops::Range<u16> = 0x120..0x140;

struct DeviceEntry {
    id: usize,
    dev: Device,
    name: String,
    uuid: String,
    /// Non-hat absolute axis codes (sorted).
    axis_codes: Vec<u16>,
    /// HAT (x_code, y_code) pairs.
    hat_pairs: Vec<(u16, u16)>,
    /// Button keycodes (sorted).
    button_codes: Vec<u16>,
}

pub struct EvdevBackend {
    devices: Vec<DeviceEntry>,
    ids: HashMap<String, usize>,
    next_id: usize,
    last_scan: Option<Instant>,
}

impl EvdevBackend {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            ids: HashMap::new(),
            next_id: 0,
            last_scan: None,
        }
    }

    fn rescan(&mut self) {
        let mut entries = Vec::new();
        for (path, dev) in evdev::enumerate() {
            if !is_joystick(&dev) {
                continue;
            }

            let mut axis_codes = Vec::new();
            let mut hat_pairs = Vec::new();
            if let Some(abs) = dev.supported_absolute_axes() {
                for a in abs.iter() {
                    if !HAT_RANGE.contains(&a.0) {
                        axis_codes.push(a.0);
                    }
                }
                let mut base = *HAT_RANGE.start();
                while base < *HAT_RANGE.end() {
                    let (xc, yc) = (base, base + 1);
                    if abs.contains(AbsoluteAxisCode(xc)) || abs.contains(AbsoluteAxisCode(yc)) {
                        hat_pairs.push((xc, yc));
                    }
                    base += 2;
                }
            }
            axis_codes.sort_unstable();

            let mut button_codes: Vec<u16> = dev
                .supported_keys()
                .map(|keys| keys.iter().map(|k| k.0).filter(|c| *c >= BTN_BASE).collect())
                .unwrap_or_default();
            button_codes.sort_unstable();

            let iid = dev.input_id();
            let uuid = dev
                .unique_name()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| dev.physical_path().map(String::from))
                .unwrap_or_else(|| {
                    format!("{:04x}:{:04x}:{:04x}", iid.vendor(), iid.product(), iid.version())
                });
            let name = dev
                .name()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .unwrap_or_else(|| format!("Joystick ({})", path.display()));

            let id = *self.ids.entry(uuid.clone()).or_insert_with(|| {
                let id = self.next_id;
                self.next_id += 1;
                id
            });

            entries.push(DeviceEntry { id, dev, name, uuid, axis_codes, hat_pairs, button_codes });
        }
        // Stable order so the device list doesn't reshuffle between rescans.
        entries.sort_by_key(|e| e.id);
        self.devices = entries;
    }
}

impl super::HidBackend for EvdevBackend {
    fn poll(&mut self) -> Vec<HidDevice> {
        let due = self.last_scan.map_or(true, |t| t.elapsed() >= RESCAN_INTERVAL);
        if due {
            self.rescan();
            self.last_scan = Some(Instant::now());
        }
        self.devices
            .iter()
            .map(|d| HidDevice {
                id: d.id,
                name: d.name.clone(),
                uuid: d.uuid.clone(),
                axes: d.axis_codes.len(),
                buttons: d.button_codes.len(),
                hats: d.hat_pairs.len(),
            })
            .collect()
    }

    fn snapshot(&mut self, id: usize) -> Option<HidSnapshot> {
        let dev = self.devices.iter().find(|d| d.id == id)?;
        let abs = dev.dev.get_abs_state().ok()?;
        let keys = dev.dev.get_key_state().ok()?;

        let axes = dev
            .axis_codes
            .iter()
            .map(|&c| {
                let info = &abs[c as usize];
                HidAxis { code: c as u32, value: norm(info.value, info.minimum, info.maximum) }
            })
            .collect();

        let hats = dev
            .hat_pairs
            .iter()
            .map(|&(xc, yc)| HidHat {
                code: xc as u32,
                x: sign(abs[xc as usize].value),
                y: -sign(abs[yc as usize].value), // evdev HAT Y: negative = up → flip to +y = up
            })
            .collect();

        let buttons = dev
            .button_codes
            .iter()
            .map(|&c| {
                let pressed = keys.contains(KeyCode(c));
                HidButton { code: c as u32, pressed, value: if pressed { 1.0 } else { 0.0 } }
            })
            .collect();

        Some(HidSnapshot { id, axes, buttons, hats })
    }
}

/// A device is a joystick/gamepad if it has absolute axes AND advertises joystick/gamepad buttons
/// (filters out mice, touchpads and tablets, which also report ABS axes).
fn is_joystick(dev: &Device) -> bool {
    dev.supported_absolute_axes().is_some()
        && dev
            .supported_keys()
            .is_some_and(|keys| keys.iter().any(|k| JOY_BTN_RANGE.contains(&k.0)))
}

/// Map a raw absolute-axis reading to −1.0…+1.0 using its min/max range.
fn norm(value: i32, minimum: i32, maximum: i32) -> f32 {
    let (min, max) = (minimum as f32, maximum as f32);
    if max > min {
        (2.0 * (value as f32 - min) / (max - min) - 1.0).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn sign(v: i32) -> i32 {
    v.signum()
}
