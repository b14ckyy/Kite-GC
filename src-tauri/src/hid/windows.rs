// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Windows HID backend — Windows.Gaming.Input `RawGameController`. Unlike the WGI *Gamepad* projection
// (which forces an Xbox layout and misclassifies HOTAS / RC-transmitter axes as buttons), the raw
// controller exposes the device's true axes / buttons / switches. We read the live reading every tick.
// See docs/archive/MSP_RC_CONTROL.md §6.
//
// XInput fallback: on some machines (seen on Windows 11 26100 with the newer `dc1-controller` Xbox
// driver stack) WGI *lists* an Xbox-class pad — 6 axes / 14 buttons / 0 switches, generic display name —
// but `GetCurrentReading` never returns a report (timestamp stays 0) and `Gamepad.Gamepads` is empty,
// while the classic XInput API reads the very same pad fine. So an Xbox-class entry that stays silent
// AND is absent from `Gamepad.Gamepads` (the discriminator — a working stack lists it there) is read
// through XInput instead, with the WGI object kept only for identity (name, stable id). The
// XInput layout is emitted in WGI's raw order (LX LY RX RY LT RT; Menu View A B X Y DPad up/down/left/right
// LB RB LS RS) so a mapping learned on one source keeps working if the other one takes over.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Gaming::Input::{Gamepad, GameControllerSwitchPosition, RawGameController};
use windows::Win32::UI::Input::XboxController::{
    XInputGetState, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_BUTTON_FLAGS,
    XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
    XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB, XINPUT_GAMEPAD_RIGHT_SHOULDER,
    XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE,
};

use super::{HidAxis, HidButton, HidDevice, HidHat, HidSnapshot};

/// How often to re-enumerate controllers for hotplug (the live reading is polled every tick regardless).
const RESCAN_INTERVAL: Duration = Duration::from_millis(500);

/// Where a device's live state comes from. Starts as `Wgi`; an Xbox-class pad that never produces a
/// WGI reading is switched to `XInput(user)` and stays there for the session (stable control indexes).
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Wgi,
    XInput(u32),
}

struct DeviceEntry {
    id: usize,
    ctrl: RawGameController,
    name: String,
    uuid: String,
    axes: usize,
    buttons: usize,
    switches: usize,
    source: Source,
    /// When this entry was created / last re-armed — the XInput fallback kicks in after
    /// `XINPUT_FALLBACK_AFTER` of WGI silence (a WGI pad that is merely at rest gets that long to report).
    since: Instant,
}

/// How long an Xbox-class pad may stay silent on WGI before we read it through XInput instead.
const XINPUT_FALLBACK_AFTER: Duration = Duration::from_millis(1500);

/// XInput button order = WGI's raw order for Xbox pads (see module docs).
const XINPUT_BUTTONS: [XINPUT_GAMEPAD_BUTTON_FLAGS; 14] = [
    XINPUT_GAMEPAD_START,
    XINPUT_GAMEPAD_BACK,
    XINPUT_GAMEPAD_A,
    XINPUT_GAMEPAD_B,
    XINPUT_GAMEPAD_X,
    XINPUT_GAMEPAD_Y,
    XINPUT_GAMEPAD_DPAD_UP,
    XINPUT_GAMEPAD_DPAD_DOWN,
    XINPUT_GAMEPAD_DPAD_LEFT,
    XINPUT_GAMEPAD_DPAD_RIGHT,
    XINPUT_GAMEPAD_LEFT_SHOULDER,
    XINPUT_GAMEPAD_RIGHT_SHOULDER,
    XINPUT_GAMEPAD_LEFT_THUMB,
    XINPUT_GAMEPAD_RIGHT_THUMB,
];

/// The WGI raw shape of an Xbox-class pad (XInput layout): 6 axes, 14 buttons, no switches.
fn is_xbox_layout(axes: usize, buttons: usize, switches: usize) -> bool {
    axes == 6 && buttons == 14 && switches == 0
}

/// The distinctive symptom of the broken stack: WGI lists the pad as a RawGameController but NOT as a
/// Gamepad. A working stack puts every Xbox-class pad into `Gamepad.Gamepads` within a few hundred
/// milliseconds, so on a working stack this stays false and the XInput path is never taken — a pad
/// that merely rests (timestamp 0 until its first input) is left to WGI. Sampled in `rescan()`.
fn wgi_gamepads_empty() -> bool {
    Gamepad::Gamepads().map(|v| v.Size().unwrap_or(0) == 0).unwrap_or(true)
}

/// Live XInput state of `user` (0..=3), or None when nothing is connected there.
fn xinput_state(user: u32) -> Option<XINPUT_STATE> {
    let mut st = XINPUT_STATE::default();
    // SAFETY: `st` is a valid, writable XINPUT_STATE for the duration of the call.
    let rc = unsafe { XInputGetState(user, &mut st) };
    (rc == 0).then_some(st)
}

/// Snapshot an XInput user in the WGI-compatible shape (sticks −1..1; triggers rest at −1, matching
/// WGI's 0..1 → −1..1 remap of trigger axes).
fn xinput_snapshot(id: usize, user: u32) -> Option<HidSnapshot> {
    let g = xinput_state(user)?.Gamepad;
    let stick = |v: i16| (v as f32 / 32767.0).clamp(-1.0, 1.0);
    let trigger = |v: u8| v as f32 / 255.0 * 2.0 - 1.0;
    let axes = [
        stick(g.sThumbLX),
        stick(g.sThumbLY),
        stick(g.sThumbRX),
        stick(g.sThumbRY),
        trigger(g.bLeftTrigger),
        trigger(g.bRightTrigger),
    ];
    Some(HidSnapshot {
        id,
        axes: axes.iter().enumerate().map(|(i, &v)| HidAxis { code: i as u32, value: v }).collect(),
        buttons: XINPUT_BUTTONS
            .iter()
            .enumerate()
            .map(|(i, flag)| {
                let p = g.wButtons.0 & flag.0 != 0;
                HidButton { code: i as u32, pressed: p, value: if p { 1.0 } else { 0.0 } }
            })
            .collect(),
        hats: Vec::new(),
    })
}

pub struct WgiBackend {
    devices: Vec<DeviceEntry>,
    /// Stable id per physical controller (NonRoamableId → id), kept across rescans / reconnects.
    ids: HashMap<String, usize>,
    next_id: usize,
    last_scan: Option<Instant>,
    /// `Gamepad.Gamepads().Size() == 0`, sampled once per rescan (500 ms) — the XInput fallback's
    /// discriminator. Cached so a resting Xbox-layout entry does not cost a WinRT call every 20 ms tick.
    gamepads_empty: bool,
}

impl WgiBackend {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            ids: HashMap::new(),
            next_id: 0,
            last_scan: None,
            gamepads_empty: false,
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

        // Reuse the existing controller objects for devices still present — recreating them resets
        // GetCurrentReading to a zero/no-reading state, which would re-trigger the startup "no valid
        // reading yet" gate every rescan. Only added devices get a fresh object.
        let mut prev: HashMap<String, DeviceEntry> =
            self.devices.drain(..).map(|d| (d.uuid.clone(), d)).collect();

        let mut entries = Vec::new();
        // Intentionally index with GetAt instead of into_iter() — the iterator can crash under some
        // hosts (see gilrs issue 132).
        for i in 0..count {
            let Ok(ctrl) = controllers.GetAt(i) else { continue };
            let uuid = ctrl
                .NonRoamableId()
                .map(|h| h.to_string())
                .unwrap_or_default();

            if let Some(existing) = prev.remove(&uuid) {
                entries.push(existing); // keep the live controller (and its reading)
                continue;
            }

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
                source: Source::Wgi,
                since: Instant::now(),
            });
        }
        self.devices = entries;
        self.gamepads_empty = wgi_gamepads_empty();
    }
}

impl super::HidBackend for WgiBackend {
    fn poll(&mut self) -> Vec<HidDevice> {
        let due = self.last_scan.is_none_or(|t| t.elapsed() >= RESCAN_INTERVAL);
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
        // Which XInput user slot the k-th Xbox-class entry maps to: the k-th connected XInput user.
        // (WGI exposes no XInput index; with one pad — the common case — this is exact.)
        let xbox_rank = self
            .devices
            .iter()
            .take_while(|d| d.id != id)
            .filter(|d| is_xbox_layout(d.axes, d.buttons, d.switches))
            .count();
        let self_gamepads_empty = self.gamepads_empty;
        let dev = self.devices.iter_mut().find(|d| d.id == id)?;

        if let Source::XInput(user) = dev.source {
            if let Some(snap) = xinput_snapshot(id, user) {
                return Some(snap);
            }
            // The XInput slot went away (unplugged / re-enumerated): back to WGI, grace period re-armed.
            log::info!("[hid] '{}' lost its XInput slot {user} — back to Windows.Gaming.Input", dev.name);
            dev.source = Source::Wgi;
            dev.since = Instant::now();
        }

        let mut buttons = vec![false; dev.buttons];
        let mut switches = vec![GameControllerSwitchPosition::Center; dev.switches];
        let mut axes = vec![0.0_f64; dev.axes];
        let timestamp = dev
            .ctrl
            .GetCurrentReading(&mut buttons, &mut switches, &mut axes)
            .ok()?;

        // WGI returns a zero-initialised reading (all axes 0.0 → −1.0 after remap) with timestamp 0
        // until the device delivers its first report — some controllers stay silent at rest. Suppress
        // it so we never surface/stream bogus neutral values at startup; the first input (any movement)
        // produces a real reading. evdev (Linux) reads the kernel's cached state, so it isn't affected.
        if timestamp == 0 {
            // Xbox-class pad, still silent after the grace period AND missing from Gamepad.Gamepads → the
            // broken stack: read it through XInput (module docs). A resting pad on a working stack is in
            // Gamepads and stays on WGI.
            if is_xbox_layout(dev.axes, dev.buttons, dev.switches) && dev.since.elapsed() >= XINPUT_FALLBACK_AFTER && self_gamepads_empty {
                if let Some(user) = (0..4u32).filter(|u| xinput_state(*u).is_some()).nth(xbox_rank) {
                    log::info!(
                        "[hid] '{}' delivers no Windows.Gaming.Input readings — reading it through XInput (user {user})",
                        dev.name
                    );
                    dev.source = Source::XInput(user);
                    return xinput_snapshot(id, user);
                }
            }
            return None;
        }

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
