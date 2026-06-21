// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// HID / joystick input backend for GCS RC control (INAV RC over MSP — see docs/archive/MSP_RC_CONTROL.md).
//
// Phase 1: raw device input only. A dedicated thread polls the selected device at ~50 Hz and streams
// RAW axis/button/hat state to the frontend (`hid-input`), plus the connected-device list
// (`hid-devices`) on hotplug. No channel mapping happens here yet — the frontend's calibration UI
// consumes raw values; mapping → RC channels is a later phase that will move into this thread.
//
// We use **native per-OS backends** (not a gamepad-abstraction library): flight hardware / RC
// transmitters expose many axes + hats that a gamepad model (Xbox layout) misclassifies as buttons.
//   Windows → Windows.Gaming.Input `RawGameController` (raw axes/buttons/switches, no projection).
//   Linux   → evdev (raw ABS axes / BTN keys / HAT axes).
// Each backend yields the same `HidDevice` / `HidSnapshot` shape. The owning thread is the only place
// a backend is touched (the WGI COM objects aren't `Send`), controlled via `HidManager`.

#[cfg(target_os = "linux")]
mod linux;
pub mod profiles;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// ~50 Hz input snapshot cadence — fluid enough for a live stick view / calibration, cheap on the bus.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Sentinel for "no device explicitly selected" stored in the atomic (real ids are small).
const NO_SELECTION: usize = usize::MAX;

/// Shared centre deadband applied to every raw axis (both backends): zero out the small resting offset
/// some controllers report at centre (observed up to ~0.04 on a gamepad) so the centre reads clean and
/// can't leak a stray RC command. ±0.05 on the −1..1 axis scale = 2.5% of the 2.0 full travel — just
/// above the observed error. Scaled (not hard-clipped) so travel past the deadband still reaches ±1
/// with no discontinuity at the edge. Per-channel deadband/expo come later in the mapping layer.
const CENTER_DEADBAND: f32 = 0.05;

fn apply_deadband(v: f32) -> f32 {
    if v.abs() <= CENTER_DEADBAND {
        0.0
    } else {
        v.signum() * (v.abs() - CENTER_DEADBAND) / (1.0 - CENTER_DEADBAND)
    }
}

/// A connected input device, for the frontend device picker.
#[derive(Clone, Serialize, PartialEq)]
pub struct HidDevice {
    /// Stable-per-session id; used by `hid_select_device`.
    pub id: usize,
    pub name: String,
    /// Stable physical identity (OS id / vendor:product) — key for persisted per-device mappings.
    pub uuid: String,
    pub axes: usize,
    pub buttons: usize,
    pub hats: usize,
}

/// One raw axis reading. `code` is the backend-stable index for this control (the durable key a
/// channel mapping binds to). `value` is normalised −1.0…+1.0 (centre 0).
#[derive(Clone, Serialize)]
pub struct HidAxis {
    pub code: u32,
    pub value: f32,
}

/// One raw button reading. `value` is 0.0…1.0 (analog triggers ramp; switches are 0/1).
#[derive(Clone, Serialize)]
pub struct HidButton {
    pub code: u32,
    pub pressed: bool,
    pub value: f32,
}

/// One hat / POV switch. `x`/`y` are −1, 0 or +1 (8-way); +y = up.
#[derive(Clone, Serialize)]
pub struct HidHat {
    pub code: u32,
    pub x: i32,
    pub y: i32,
}

/// Live raw state of the selected device — emitted on `hid-input` each poll tick.
#[derive(Clone, Serialize)]
pub struct HidSnapshot {
    pub id: usize,
    pub axes: Vec<HidAxis>,
    pub buttons: Vec<HidButton>,
    pub hats: Vec<HidHat>,
}

/// Per-OS input source. The owning thread polls one of these; `poll` re-scans for hotplug (the
/// backend throttles its own rescans) and returns the current device list, `snapshot` reads the live
/// raw state of one device.
trait HidBackend {
    fn poll(&mut self) -> Vec<HidDevice>;
    fn snapshot(&mut self, id: usize) -> Option<HidSnapshot>;
}

/// Fallback backend for platforms without an input implementation (e.g. macOS): no devices.
struct NullBackend;
impl HidBackend for NullBackend {
    fn poll(&mut self) -> Vec<HidDevice> {
        Vec::new()
    }
    fn snapshot(&mut self, _id: usize) -> Option<HidSnapshot> {
        None
    }
}

fn make_backend() -> Box<dyn HidBackend> {
    #[cfg(target_os = "windows")]
    {
        return Box::new(windows::WgiBackend::new());
    }
    #[cfg(target_os = "linux")]
    {
        return Box::new(linux::EvdevBackend::new());
    }
    #[allow(unreachable_code)]
    {
        Box::new(NullBackend)
    }
}

/// Shared control surface for the input thread, managed by Tauri.
pub struct HidManager {
    running: Arc<AtomicBool>,
    /// Device id the frontend asked to stream, or `NO_SELECTION` to auto-pick the first connected one.
    selected: Arc<AtomicUsize>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl HidManager {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            selected: Arc::new(AtomicUsize::new(NO_SELECTION)),
            thread: Mutex::new(None),
        }
    }

    /// Start the input thread (idempotent — a second call while running is a no-op).
    pub fn start(&self, app: AppHandle) {
        if self.running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let running = self.running.clone();
        let selected = self.selected.clone();
        let handle = thread::spawn(move || input_loop(app, running, selected));
        *self.thread.lock().unwrap() = Some(handle);
    }

    /// Stop the input thread and wait for it to drain.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    /// Select which device to stream (`hid-input`). The thread picks it up on its next tick.
    pub fn select(&self, id: usize) {
        self.selected.store(id, Ordering::SeqCst);
    }
}

impl Default for HidManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve which device to stream: the explicit selection if still connected, else the first listed.
fn resolve_target(devices: &[HidDevice], selected: usize) -> Option<usize> {
    if selected != NO_SELECTION && devices.iter().any(|d| d.id == selected) {
        return Some(selected);
    }
    devices.first().map(|d| d.id)
}

fn input_loop(app: AppHandle, running: Arc<AtomicBool>, selected: Arc<AtomicUsize>) {
    let mut backend = make_backend();
    let mut last_devices: Vec<HidDevice> = Vec::new();

    while running.load(Ordering::SeqCst) {
        let devices = backend.poll();
        if devices != last_devices {
            let _ = app.emit("hid-devices", &devices);
            last_devices = devices.clone();
        }

        if let Some(target) = resolve_target(&devices, selected.load(Ordering::SeqCst)) {
            if let Some(mut snap) = backend.snapshot(target) {
                for axis in &mut snap.axes {
                    axis.value = apply_deadband(axis.value);
                }
                let _ = app.emit("hid-input", &snap);
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}
