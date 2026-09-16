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
#[cfg(target_os = "macos")]
mod macos;
pub mod profiles;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
    #[cfg(target_os = "macos")]
    {
        return Box::new(macos::IOKitBackend::new());
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

/// How long a listed device may deliver no reading before we log it once (a wiring hint for the log —
/// on Windows, Windows.Gaming.Input only reports input while Kite owns the foreground window).
const SILENT_WARN_AFTER: Duration = Duration::from_secs(5);

fn input_loop(app: AppHandle, running: Arc<AtomicBool>, selected: Arc<AtomicUsize>) {
    let mut backend = make_backend();
    let mut last_devices: Vec<HidDevice> = Vec::new();
    // Diagnostics: which target we are streaming, whether it has produced a reading yet, when it was
    // selected, and the last axis vector we logged (1 Hz debug trace while the values change).
    let mut cur_target: Option<usize> = None;
    let mut got_reading = false;
    let mut since = Instant::now();
    let mut warned_silent = false;
    let mut last_trace = Instant::now();
    let mut last_axes: Vec<i32> = Vec::new();

    log::info!("[hid] input thread started");
    while running.load(Ordering::SeqCst) {
        let devices = backend.poll();
        if devices != last_devices {
            if devices.is_empty() {
                log::info!("[hid] no game controllers connected");
            } else {
                for d in &devices {
                    log::info!(
                        "[hid] device id={} '{}' axes={} buttons={} hats={} ({})",
                        d.id, d.name, d.axes, d.buttons, d.hats, d.uuid
                    );
                }
            }
            let _ = app.emit("hid-devices", &devices);
            last_devices = devices.clone();
        }

        let target = resolve_target(&devices, selected.load(Ordering::SeqCst));
        if target != cur_target {
            cur_target = target;
            got_reading = false;
            warned_silent = false;
            since = Instant::now();
            if let Some(id) = target {
                log::info!("[hid] streaming device id={}", id);
            }
        }

        if let Some(target) = target {
            if let Some(mut snap) = backend.snapshot(target) {
                for axis in &mut snap.axes {
                    axis.value = apply_deadband(axis.value);
                }
                if !got_reading {
                    got_reading = true;
                    log::info!(
                        "[hid] first reading from device id={} ({} axes, {} buttons, {} hats)",
                        target, snap.axes.len(), snap.buttons.len(), snap.hats.len()
                    );
                }
                // 1 Hz trace of the axis vector (percent) while it changes — enough to see in the log
                // whether stick movement reaches the backend at all.
                let axes_pct: Vec<i32> = snap.axes.iter().map(|a| (a.value * 100.0).round() as i32).collect();
                if axes_pct != last_axes && last_trace.elapsed() >= Duration::from_secs(1) {
                    let pressed: Vec<u32> = snap.buttons.iter().filter(|b| b.pressed).map(|b| b.code + 1).collect();
                    log::debug!("[hid] axes%={:?} buttons={:?}", axes_pct, pressed);
                    last_axes = axes_pct;
                    last_trace = Instant::now();
                }
                let _ = app.emit("hid-input", &snap);
            } else if !got_reading && !warned_silent && since.elapsed() >= SILENT_WARN_AFTER {
                warned_silent = true;
                log::warn!(
                    "[hid] device id={} is listed but has delivered no reading for {}s — on Windows, input is only reported while the Kite window is in the foreground",
                    target, SILENT_WARN_AFTER.as_secs()
                );
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
    log::info!("[hid] input thread stopped");
}
