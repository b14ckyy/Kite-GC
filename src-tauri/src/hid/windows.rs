// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Windows HID backend — Windows.Gaming.Input `RawGameController`. Unlike the WGI *Gamepad* projection
// (which forces an Xbox layout and misclassifies HOTAS / RC-transmitter axes as buttons), the raw
// controller exposes the device's true axes / buttons / switches. We read the live reading every tick.
// See docs/active/RC_CONTROL.md §6.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Gaming::Input::{GameControllerSwitchPosition, RawGameController};

use super::{HidAxis, HidButton, HidDevice, HidHat, HidSnapshot};

/// How often to re-enumerate controllers for hotplug (the live reading is polled every tick regardless).
const RESCAN_INTERVAL: Duration = Duration::from_millis(500);

struct DeviceEntry {
    id: usize,
    ctrl: RawGameController,
    name: String,
    uuid: String,
    axes: usize,
    buttons: usize,
    switches: usize,
}

pub struct WgiBackend {
    devices: Vec<DeviceEntry>,
    /// Stable id per physical controller (NonRoamableId → id), kept across rescans / reconnects.
    ids: HashMap<String, usize>,
    next_id: usize,
    last_scan: Option<Instant>,
}

impl WgiBackend {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            ids: HashMap::new(),
            next_id: 0,
            last_scan: None,
        }
    }

    /// Re-enumerate connected controllers, preserving stable ids. Returns true if the device set changed.
    fn rescan(&mut self) {
        let controllers = match RawGameController::RawGameControllers() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[hid] RawGameControllers() failed: {e}");
                return;
            }
        };
        let count = controllers.Size().unwrap_or(0);

        let mut entries = Vec::new();
        // Intentionally index with GetAt instead of into_iter() — the iterator can crash under some
        // hosts (see gilrs issue 132).
        for i in 0..count {
            let Ok(ctrl) = controllers.GetAt(i) else { continue };
            let uuid = ctrl
                .NonRoamableId()
                .map(|h| h.to_string())
                .unwrap_or_default();
            let name = ctrl
                .DisplayName()
                .map(|h| h.to_string())
                .unwrap_or_else(|_| "Game controller".into());

            let id = *self.ids.entry(uuid.clone()).or_insert_with(|| {
                let id = self.next_id;
                self.next_id += 1;
                id
            });

            entries.push(DeviceEntry {
                id,
                axes: ctrl.AxisCount().unwrap_or(0).max(0) as usize,
                buttons: ctrl.ButtonCount().unwrap_or(0).max(0) as usize,
                switches: ctrl.SwitchCount().unwrap_or(0).max(0) as usize,
                ctrl,
                name,
                uuid,
            });
        }
        self.devices = entries;
    }
}

impl super::HidBackend for WgiBackend {
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
                axes: d.axes,
                buttons: d.buttons,
                hats: d.switches,
            })
            .collect()
    }

    fn snapshot(&mut self, id: usize) -> Option<HidSnapshot> {
        let dev = self.devices.iter().find(|d| d.id == id)?;

        let mut buttons = vec![false; dev.buttons];
        let mut switches = vec![GameControllerSwitchPosition::Center; dev.switches];
        let mut axes = vec![0.0_f64; dev.axes];
        dev.ctrl
            .GetCurrentReading(&mut buttons, &mut switches, &mut axes)
            .ok()?;

        Some(HidSnapshot {
            id,
            // WGI axes are 0.0..1.0 (centre 0.5) — remap to the −1..1 convention the UI/mapping use.
            axes: axes
                .iter()
                .enumerate()
                .map(|(i, v)| HidAxis { code: i as u32, value: (*v * 2.0 - 1.0) as f32 })
                .collect(),
            buttons: buttons
                .iter()
                .enumerate()
                .map(|(i, &p)| HidButton { code: i as u32, pressed: p, value: if p { 1.0 } else { 0.0 } })
                .collect(),
            hats: switches
                .iter()
                .enumerate()
                .map(|(i, &s)| {
                    let (x, y) = switch_xy(s);
                    HidHat { code: i as u32, x, y }
                })
                .collect(),
        })
    }
}

/// Map a WGI 8-way switch position to (x, y) ∈ {−1, 0, 1}; +y = up.
fn switch_xy(s: GameControllerSwitchPosition) -> (i32, i32) {
    match s {
        GameControllerSwitchPosition::Up => (0, 1),
        GameControllerSwitchPosition::UpRight => (1, 1),
        GameControllerSwitchPosition::Right => (1, 0),
        GameControllerSwitchPosition::DownRight => (1, -1),
        GameControllerSwitchPosition::Down => (0, -1),
        GameControllerSwitchPosition::DownLeft => (-1, -1),
        GameControllerSwitchPosition::Left => (-1, 0),
        GameControllerSwitchPosition::UpLeft => (-1, 1),
        _ => (0, 0),
    }
}
