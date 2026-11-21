// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
//
// Frame decoder ported from FrSky-Bluetooth-Trainer (test_frsky_ble.py),
// Copyright (c) 2026 garfield2412, MIT License:
// https://github.com/garfield2412/FrSky-Bluetooth-Trainer/blob/main/LICENSE

// FrSky Bluetooth trainer input: the radio (trainer slave) notifies its channels on FFF0 / FFF6.
// Design: Dev-Docs active/FRSKY_BT_TRAINER.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use std::time::{Duration, Instant};

use serde::Serialize;

use super::HidDevice;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
use super::{HidAxis, HidSnapshot};

/// Type byte, three bytes per channel pair, checksum; the radio may send any even channel count.
const FRAME_MIN: usize = 5;
const FRAME_MAX: usize = 38;
const DELIMITER: u8 = 0x7E;
const ESCAPE: u8 = 0x7D;
const TRAINER_FRAME: u8 = 0x80;

/// Byte-stream decoder; a frame may span several notifications.
#[derive(Default)]
pub struct TrainerDecoder {
    frame: Option<Vec<u8>>,
    escaped: bool,
    /// Non-empty frames dropped for length, header or checksum.
    pub rejected: u64,
}

impl TrainerDecoder {
    pub fn feed(&mut self, data: &[u8], out: &mut Vec<Vec<u16>>) {
        for &byte in data {
            if byte == DELIMITER {
                // A closing delimiter also opens the next frame, so an empty body is normal.
                if let Some(frame) = self.frame.take() {
                    if !frame.is_empty() {
                        let valid = !self.escaped
                            && frame.len() >= FRAME_MIN
                            && (frame.len() - 2) % 3 == 0
                            && frame[0] == TRAINER_FRAME
                            && frame.iter().fold(0u8, |acc, b| acc ^ b) == 0;
                        if valid {
                            out.push(decode_channels(&frame));
                        } else {
                            self.rejected += 1;
                        }
                    }
                }
                self.frame = Some(Vec::with_capacity(FRAME_MAX));
                self.escaped = false;
            } else if let Some(frame) = self.frame.as_mut() {
                if self.escaped {
                    frame.push(byte ^ 0x20);
                    self.escaped = false;
                } else if byte == ESCAPE {
                    self.escaped = true;
                } else {
                    frame.push(byte);
                }
                if frame.len() > FRAME_MAX {
                    self.frame = None;
                    self.rejected += 1;
                }
            }
        }
    }
}

/// Two 12-bit channels per three bytes, between the type byte and the checksum.
fn decode_channels(frame: &[u8]) -> Vec<u16> {
    let mut channels = Vec::with_capacity((frame.len() - 2) / 3 * 2);
    for group in frame[1..frame.len() - 1].chunks_exact(3) {
        let (first, middle, last) = (group[0] as u16, group[1] as u16, group[2] as u16);
        channels.push(first | ((middle & 0xF0) << 4));
        channels.push(((middle & 0x0F) << 4) | (last >> 4) | ((last & 0x0F) << 8));
    }
    channels
}

/// Id of the trainer radio in the HID device list; real backend ids are small indices.
pub const DEVICE_ID: usize = 1_000_000;
/// Without a valid frame for this long the radio leaves the device list, like an unplugged joystick.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
const STALL_AFTER: Duration = Duration::from_millis(500);

#[cfg(not(any(target_os = "ios", target_os = "android")))]
struct Feed {
    device: HidDevice,
    at: Instant,
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
static FEED: Mutex<Option<Feed>> = Mutex::new(None);
/// Set by the HID thread while the trainer radio is the streamed device.
static SELECTED: AtomicBool = AtomicBool::new(false);

pub fn set_selected(selected: bool) {
    SELECTED.store(selected, Ordering::Relaxed);
}

/// The radio as a HID device, only while its frames are fresh.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn device() -> Option<HidDevice> {
    let feed = FEED.lock().ok()?;
    feed.as_ref()
        .filter(|f| f.at.elapsed() < STALL_AFTER)
        .map(|f| f.device.clone())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn device() -> Option<HidDevice> {
    None
}

/// Channels as raw axes, 988..2012 µs on −1..1; no centre deadband for a radio.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn snapshot(channels: &[u16]) -> HidSnapshot {
    let axes = channels
        .iter()
        .enumerate()
        .map(|(i, &us)| HidAxis {
            code: i as u32,
            value: ((us as f32 - 1500.0) / 512.0).clamp(-1.0, 1.0),
        })
        .collect();
    HidSnapshot {
        id: DEVICE_ID,
        axes,
        buttons: Vec::new(),
        hats: Vec::new(),
    }
}

/// Emitted as `trainer-status` (4 Hz and on every state change).
#[derive(Clone, Serialize, Default)]
pub struct TrainerStatus {
    /// searching | connecting | waiting | streaming | stalled | stopped | foreign (`device_id` sends
    /// other data, e.g. telemetry, and was given up for this start)
    pub state: &'static str,
    /// BLE id of the radio this status is about.
    pub device_id: String,
    pub device: String,
    pub error: Option<String>,
    pub characteristic: String,
    pub channels: Vec<u16>,
    pub frames: u64,
    pub rejected: u64,
    pub notifications: u64,
    pub interval_last_ms: f32,
    pub interval_mean_ms: f32,
    pub interval_p95_ms: f32,
    pub interval_max_ms: f32,
    pub frame_age_ms: Option<u32>,
    /// Gaps of more than `STALL_AFTER` between valid frames, since start.
    pub stalls: u32,
    /// Sessions that reached a first frame after an earlier one had, since start.
    pub reconnects: u32,
    pub last_raw: String,
    /// Last notification that closed a rejected frame.
    pub last_rejected_raw: String,
    /// A module that demands a passkey; it is paired with the FrSky PIN before every connection.
    pub pairing: bool,
}

/// A remembered radio as the frontend hands it over.
#[derive(serde::Deserialize)]
pub struct TrainerDevice {
    pub id: String,
    #[serde(default)]
    pub pairing: bool,
}

/// Owns the stop handle of the running trainer task.
#[derive(Default)]
pub struct TrainerManager {
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    devices: Mutex<Vec<String>>,
}

impl TrainerManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// True for a radio the trainer task looks for or holds; it cannot serve telemetry at the same time.
    pub fn holds(&self, device_id: &str) -> bool {
        self.devices
            .lock()
            .map(|l| l.iter().any(|d| d == device_id))
            .unwrap_or(false)
    }

    pub fn release(&self, device_id: &str) {
        if let Ok(mut list) = self.devices.lock() {
            list.retain(|d| d != device_id);
        }
    }

    /// Dropping the sender ends the task.
    pub fn stop(&self) {
        if let Ok(mut guard) = self.stop.lock() {
            *guard = None;
        }
        if let Ok(mut list) = self.devices.lock() {
            list.clear();
        }
    }

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    pub fn start(&self, app: tauri::AppHandle, devices: Vec<TrainerDevice>) -> Result<(), String> {
        let mut ids = Vec::new();
        for d in devices {
            if d.pairing {
                crate::transport::ble_pairing::flag(&d.id);
            }
            // A radio the telemetry link holds is left alone; that link hands it over itself.
            if !crate::transport::ble::is_listen_device(&d.id) {
                ids.push(d.id);
            }
        }
        if ids.is_empty() {
            return Err("No radio to look for (in use as the telemetry link)".into());
        }
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.stop.lock().map_err(|e| e.to_string())? = Some(tx);
        *self.devices.lock().map_err(|e| e.to_string())? = ids.clone();
        tauri::async_runtime::spawn(link::run(app, ids, rx));
        Ok(())
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub fn start(
        &self,
        _app: tauri::AppHandle,
        _devices: Vec<TrainerDevice>,
    ) -> Result<(), String> {
        Err("Bluetooth trainer input is not available on this platform".into())
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod link {
    use super::*;
    use std::collections::VecDeque;

    const RECONNECT_AFTER: Duration = Duration::from_secs(5);
    const INTERVAL_WINDOW: usize = 256;

    /// Frame timing and counters of one start; survives the reconnects.
    struct Stats {
        status: TrainerStatus,
        decoder: TrainerDecoder,
        intervals: VecDeque<f32>,
        first_frame: Option<Instant>,
        last_frame: Option<Instant>,
        session_frames: u64,
        session_notifications: u64,
        foreign: bool,
        sessions_with_frames: u32,
        stalled: bool,
        logged_rejects: u32,
        frames_out: Vec<Vec<u16>>,
    }

    impl Stats {
        fn new() -> Self {
            Self {
                status: TrainerStatus::default(),
                decoder: TrainerDecoder::default(),
                intervals: VecDeque::with_capacity(INTERVAL_WINDOW),
                first_frame: None,
                last_frame: None,
                session_frames: 0,
                session_notifications: 0,
                foreign: false,
                sessions_with_frames: 0,
                stalled: false,
                logged_rejects: 0,
                frames_out: Vec::new(),
            }
        }

        fn begin_session(&mut self) {
            self.decoder = TrainerDecoder {
                rejected: self.decoder.rejected,
                ..Default::default()
            };
            self.intervals.clear();
            self.first_frame = None;
            self.last_frame = None;
            self.session_frames = 0;
            self.session_notifications = 0;
            self.stalled = false;
            self.status.interval_last_ms = 0.0;
            self.status.interval_max_ms = 0.0;
        }

        fn on_notification(&mut self, data: &[u8], now: Instant) {
            self.status.notifications += 1;
            self.session_notifications += 1;
            self.status.last_raw = data
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            self.frames_out.clear();
            let rejected = self.decoder.rejected;
            self.decoder.feed(data, &mut self.frames_out);
            if self.decoder.rejected != rejected {
                self.status.last_rejected_raw = self.status.last_raw.clone();
                if self.logged_rejects < 16 {
                    self.logged_rejects += 1;
                    log::info!("[trainer] rejected frame in: {}", self.status.last_raw);
                }
            }
            for channels in self.frames_out.drain(..) {
                if let Some(prev) = self.last_frame {
                    let ms = now.duration_since(prev).as_secs_f32() * 1000.0;
                    self.status.interval_last_ms = ms;
                    self.status.interval_max_ms = self.status.interval_max_ms.max(ms);
                    if self.intervals.len() == INTERVAL_WINDOW {
                        self.intervals.pop_front();
                    }
                    self.intervals.push_back(ms);
                } else {
                    self.first_frame = Some(now);
                    self.sessions_with_frames += 1;
                    self.status.reconnects = self.sessions_with_frames - 1;
                }
                self.last_frame = Some(now);
                self.session_frames += 1;
                self.status.frames += 1;
                self.status.channels = channels;
                self.stalled = false;
            }
        }

        /// Refreshes the derived fields; returns the age of the last valid frame.
        fn refresh(&mut self, now: Instant, session_start: Instant) -> Duration {
            let age = now.duration_since(self.last_frame.unwrap_or(session_start));
            self.status.rejected = self.decoder.rejected;
            self.status.frame_age_ms = self
                .last_frame
                .map(|t| now.duration_since(t).as_millis() as u32);
            if let (Some(first), Some(last)) = (self.first_frame, self.last_frame) {
                if self.session_frames > 1 {
                    self.status.interval_mean_ms = last.duration_since(first).as_secs_f32()
                        * 1000.0
                        / (self.session_frames - 1) as f32;
                }
            }
            if !self.intervals.is_empty() {
                let mut sorted: Vec<f32> = self.intervals.iter().copied().collect();
                sorted.sort_by(|a, b| a.total_cmp(b));
                let rank = (95 * sorted.len()).div_ceil(100);
                self.status.interval_p95_ms = sorted[rank - 1];
            }
            if self.last_frame.is_some() && age > STALL_AFTER {
                if !self.stalled {
                    self.stalled = true;
                    self.status.stalls += 1;
                }
                self.status.state = "stalled";
            } else if self.last_frame.is_some() {
                self.status.state = "streaming";
            }
            age
        }
    }

    use crate::transport::ble_pairing;
    use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
    use btleplug::platform::{Manager, Peripheral};
    use futures::StreamExt;
    use tauri::{AppHandle, Emitter, Manager as _};

    const SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x0000fff0_0000_1000_8000_00805f9b34fb);
    const CHARACTERISTIC: uuid::Uuid =
        uuid::Uuid::from_u128(0x0000fff6_0000_1000_8000_00805f9b34fb);
    const FIND_TIMEOUT: Duration = Duration::from_secs(10);
    const RETRY_DELAY: Duration = Duration::from_secs(2);
    const STATUS_INTERVAL: Duration = Duration::from_millis(250);
    /// Data without a single trainer frame for this long is something else (telemetry): release.
    const FOREIGN_AFTER: Duration = Duration::from_secs(2);
    const FOREIGN_NOTIFICATIONS: u64 = 10;
    /// The older FrSky module (TI CC254x profile) carries these beside FFF6; the radios with their
    /// own Bluetooth do not. That module demands a passkey and hangs up without one.
    const PIN_MODULE_MARKERS: [uuid::Uuid; 2] = [
        uuid::Uuid::from_u128(0x0000fff4_0000_1000_8000_00805f9b34fb),
        uuid::Uuid::from_u128(0x0000fff5_0000_1000_8000_00805f9b34fb),
    ];

    fn emit(app: &AppHandle, stats: &Stats) {
        let _ = app.emit("trainer-status", &stats.status);
    }

    fn set_state(app: &AppHandle, stats: &mut Stats, state: &'static str) {
        stats.status.state = state;
        emit(app, stats);
    }

    fn clear_feed() {
        if let Ok(mut feed) = FEED.lock() {
            *feed = None;
        }
    }

    /// Refreshes the HID device entry and, while the radio is the streamed device, emits the frame.
    fn publish(app: &AppHandle, device_id: &str, stats: &Stats) {
        let channels = &stats.status.channels;
        if let Ok(mut feed) = FEED.lock() {
            *feed = Some(Feed {
                device: HidDevice {
                    id: DEVICE_ID,
                    name: format!("{} (Bluetooth)", stats.status.device),
                    uuid: format!("frsky-bt:{device_id}"),
                    axes: channels.len(),
                    buttons: 0,
                    hats: 0,
                },
                at: Instant::now(),
            });
        }
        if SELECTED.load(Ordering::Relaxed) {
            let _ = app.emit("hid-input", snapshot(channels));
        }
    }

    pub async fn run(
        app: AppHandle,
        mut ids: Vec<String>,
        mut stop: tokio::sync::oneshot::Receiver<()>,
    ) {
        let mut stats = Stats::new();
        let mut peripheral: Option<Peripheral> = None;
        let mut bond: Option<ble_pairing::Bond> = None;
        // btleplug also lists radios seen earlier: one that failed to connect is tried last next time.
        let mut failed: Option<String> = None;
        let mut gave_up = false;
        log::info!("[trainer] started for {ids:?}");
        loop {
            let result = tokio::select! {
                _ = &mut stop => break,
                r = session(&app, &ids, failed.as_deref(), &mut peripheral, &mut bond, &mut stats) => r,
            };
            failed = (result.is_err() && stats.status.state == "connecting")
                .then(|| stats.status.device_id.clone());
            if let Err(e) = result {
                log::warn!("[trainer] {e}");
                stats.status.error = Some(e);
            }
            if let Some(p) = peripheral.take() {
                let _ = p.disconnect().await;
            }
            if let Some(b) = bond.take() {
                b.release().await;
            }
            clear_feed();
            stats.status.channels.clear();
            if stats.foreign {
                stats.foreign = false;
                let id = stats.status.device_id.clone();
                ids.retain(|d| *d != id);
                app.state::<TrainerManager>().release(&id);
                set_state(&app, &mut stats, "foreign");
                if ids.is_empty() {
                    gave_up = true;
                    break;
                }
            }
            set_state(&app, &mut stats, "searching");
            tokio::select! {
                _ = &mut stop => break,
                _ = tokio::time::sleep(RETRY_DELAY) => {}
            }
        }
        if let Some(p) = peripheral.take() {
            let _ = p.disconnect().await;
        }
        if let Some(b) = bond.take() {
            b.release().await;
        }
        clear_feed();
        stats.status.channels.clear();
        if gave_up {
            app.state::<TrainerManager>().stop();
            log::info!("[trainer] ended: no radio left that sends trainer frames");
        } else {
            set_state(&app, &mut stats, "stopped");
            log::info!("[trainer] stopped");
        }
    }

    /// One connection; returns with the reason when the link is lost or delivers nothing.
    async fn session(
        app: &AppHandle,
        ids: &[String],
        failed: Option<&str>,
        slot: &mut Option<Peripheral>,
        bond: &mut Option<ble_pairing::Bond>,
        stats: &mut Stats,
    ) -> Result<(), String> {
        set_state(app, stats, "searching");
        let manager = Manager::new()
            .await
            .map_err(|e| format!("BLE manager init failed: {e}"))?;
        let adapter = manager
            .adapters()
            .await
            .map_err(|e| format!("no BLE adapters found: {e}"))?
            .into_iter()
            .next()
            .ok_or_else(|| "no BLE adapter available".to_string())?;

        adapter
            .start_scan(ScanFilter { services: vec![] })
            .await
            .map_err(|e| format!("BLE scan failed: {e}"))?;
        let deadline = Instant::now() + FIND_TIMEOUT;
        let found = loop {
            let list = adapter
                .peripherals()
                .await
                .map_err(|e| format!("list peripherals: {e}"))?;
            let mut known: Vec<_> = list
                .into_iter()
                .filter(|p| ids.iter().any(|d| *d == p.id().to_string()))
                .collect();
            known.sort_by_key(|p| failed == Some(p.id().to_string().as_str()));
            if let Some(p) = known.into_iter().next() {
                break Some(p);
            }
            if Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let _ = adapter.stop_scan().await;
        let peripheral = found.ok_or_else(|| "no remembered radio in the BLE scan".to_string())?;
        *slot = Some(peripheral.clone());
        let device_id = peripheral.id().to_string();
        let device_id = device_id.as_str();
        if stats.status.device_id != device_id {
            stats.status.device_id = device_id.to_string();
            stats.status.device.clear();
        }

        stats.status.pairing = ble_pairing::flagged(device_id);
        if stats.status.pairing {
            set_state(app, stats, "connecting");
            match ble_pairing::pair(device_id, ble_pairing::FRSKY_PIN).await {
                Ok(b) => *bond = Some(b),
                Err(e) => log::warn!("[trainer] {e}; connecting unpaired"),
            }
        }

        // A reconnect often comes without the advertised name; the modules pad theirs with CR LF.
        let name = peripheral
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.local_name);
        match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => stats.status.device = n.to_string(),
            None if stats.status.device.is_empty() => stats.status.device = device_id.to_string(),
            None => {}
        }
        set_state(app, stats, "connecting");
        peripheral
            .connect()
            .await
            .map_err(|e| format!("BLE connect failed: {e}"))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| format!("BLE service discovery failed: {e}"))?;

        // Without FFF6 every vendor notify characteristic is taken, so an unknown radio still shows
        // its raw bytes. SIG services stay out: subscribing there raises a pairing prompt on Windows.
        let all = peripheral.characteristics();
        for c in &all {
            log::info!(
                "[trainer] GATT {} / {} {:?}",
                c.service_uuid,
                c.uuid,
                c.properties
            );
        }
        let pin_module = PIN_MODULE_MARKERS.iter().all(|m| {
            all.iter()
                .any(|c| c.service_uuid == SERVICE && c.uuid == *m)
        });
        if pin_module && !stats.status.pairing {
            ble_pairing::flag(device_id);
            stats.status.pairing = true;
            return Err("module demands a passkey: reconnecting with PIN pairing".into());
        }
        let mut targets: Vec<_> = all
            .iter()
            .filter(|c| c.service_uuid == SERVICE && c.uuid == CHARACTERISTIC)
            .cloned()
            .collect();
        if targets.is_empty() {
            log::warn!(
                "[trainer] no FFF0 / FFF6 on '{}', listening on every vendor notify characteristic",
                stats.status.device
            );
            targets = all
                .iter()
                .filter(|c| {
                    c.properties
                        .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
                        && !crate::transport::ble::is_standard_gatt_service(c.service_uuid)
                })
                .cloned()
                .collect();
        }
        let mut subscribed = Vec::new();
        for c in &targets {
            match peripheral.subscribe(c).await {
                Ok(()) => subscribed.push(c.uuid.to_string()),
                Err(e) => log::warn!("[trainer] subscribe to {} failed: {e}", c.uuid),
            }
        }
        if subscribed.is_empty() {
            return Err("no notify characteristic to listen on".into());
        }
        stats.status.characteristic = subscribed.join(", ");
        let mut notifications = peripheral
            .notifications()
            .await
            .map_err(|e| format!("BLE notification stream failed: {e}"))?;

        stats.begin_session();
        stats.status.error = None;
        set_state(app, stats, "waiting");
        log::info!(
            "[trainer] connected to '{}', listening on {}",
            stats.status.device,
            stats.status.characteristic
        );

        let session_start = Instant::now();
        let mut logged_raw = 0u32;
        let mut tick = tokio::time::interval(STATUS_INTERVAL);
        loop {
            tokio::select! {
                n = notifications.next() => {
                    let Some(n) = n else { return Err("Bluetooth link lost".into()) };
                    let first = stats.session_frames == 0;
                    let before = stats.status.frames;
                    stats.on_notification(&n.value, Instant::now());
                    if stats.status.frames != before {
                        publish(app, device_id, stats);
                    }
                    if logged_raw < 8 {
                        logged_raw += 1;
                        log::info!("[trainer] raw {} ({} bytes): {}", n.uuid, n.value.len(), stats.status.last_raw);
                    }
                    if first && stats.session_frames > 0 {
                        log::info!("[trainer] first frame, {} channels", stats.status.channels.len());
                    }
                }
                _ = tick.tick() => {
                    let age = stats.refresh(Instant::now(), session_start);
                    emit(app, stats);
                    if age > RECONNECT_AFTER {
                        return Err(format!("no valid trainer frame for {} s", RECONNECT_AFTER.as_secs()));
                    }
                    if stats.session_frames == 0
                        && stats.session_notifications >= FOREIGN_NOTIFICATIONS
                        && session_start.elapsed() >= FOREIGN_AFTER
                    {
                        stats.foreign = true;
                        return Err("the radio sends data, but no trainer frames; released".into());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unescaped body (type, packed channels, checksum) for the given channel values.
    fn body(channels: &[u16]) -> Vec<u8> {
        let mut b = vec![TRAINER_FRAME];
        for pair in channels.chunks(2) {
            let (a, c) = (pair[0], pair[1]);
            b.push((a & 0xFF) as u8);
            b.push((((a >> 8) & 0x0F) << 4) as u8 | ((c >> 4) & 0x0F) as u8);
            b.push(((c & 0x0F) << 4) as u8 | ((c >> 8) & 0x0F) as u8);
        }
        let checksum = b.iter().fold(0u8, |acc, x| acc ^ x);
        b.push(checksum);
        b
    }

    fn wire(body: &[u8]) -> Vec<u8> {
        let mut w = vec![DELIMITER];
        for &x in body {
            if x == DELIMITER || x == ESCAPE {
                w.push(ESCAPE);
                w.push(x ^ 0x20);
            } else {
                w.push(x);
            }
        }
        w.push(DELIMITER);
        w
    }

    fn decode(chunks: &[&[u8]]) -> (Vec<Vec<u16>>, u64) {
        let mut d = TrainerDecoder::default();
        let mut out = Vec::new();
        for c in chunks {
            d.feed(c, &mut out);
        }
        (out, d.rejected)
    }

    const CH8: [u16; 8] = [988, 1500, 2012, 1234, 1000, 2000, 1501, 1499];

    #[test]
    fn decodes_8_and_16_channels() {
        let (out, rejected) = decode(&[&wire(&body(&CH8))]);
        assert_eq!(out, vec![CH8.to_vec()]);
        assert_eq!(rejected, 0);

        let ch16: Vec<u16> = (0..16).map(|i| 988 + i * 64).collect();
        let (out, _) = decode(&[&wire(&body(&ch16))]);
        assert_eq!(out, vec![ch16]);
    }

    #[test]
    fn decodes_other_channel_counts() {
        for n in [2usize, 6, 10, 12, 24] {
            let ch: Vec<u16> = (0..n as u16).map(|i| 1000 + i * 40).collect();
            assert_eq!(decode(&[&wire(&body(&ch))]), (vec![ch], 0), "{n} channels");
        }
    }

    #[test]
    fn frame_split_across_notifications() {
        let w = wire(&body(&CH8));
        let (out, rejected) = decode(&[&w[..5], &w[5..9], &w[9..]]);
        assert_eq!(out, vec![CH8.to_vec()]);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn back_to_back_frames_share_no_state() {
        let mut w = wire(&body(&CH8));
        w.extend(wire(&body(&CH8)));
        let (out, rejected) = decode(&[&w]);
        assert_eq!(out.len(), 2);
        assert_eq!(rejected, 0);
    }

    #[test]
    fn escaped_bytes_round_trip() {
        // 0x7E / 0x7D as the low byte of a channel force the escape path.
        let ch = [0x57E, 0x57D, 1500, 1500, 1500, 1500, 1500, 1500];
        let w = wire(&body(&ch));
        assert!(w[1..w.len() - 1].contains(&ESCAPE));
        let (out, _) = decode(&[&w]);
        assert_eq!(out, vec![ch.to_vec()]);
    }

    #[test]
    fn rejects_bad_checksum_length_and_type() {
        let mut bad = body(&CH8);
        bad[3] ^= 0x01;
        assert_eq!(decode(&[&wire(&bad)]), (vec![], 1));

        let mut odd = body(&CH8);
        odd.insert(1, 0);
        assert_eq!(decode(&[&wire(&odd)]), (vec![], 1));

        let mut other = body(&CH8);
        other[0] = 0x81;
        let last = other.len() - 1;
        other[last] ^= 0x01;
        assert_eq!(decode(&[&wire(&other)]), (vec![], 1));
    }

    #[test]
    fn overlong_garbage_does_not_block_the_next_frame() {
        let mut w = vec![DELIMITER];
        w.extend([0x11u8; 40]);
        w.extend(wire(&body(&CH8)));
        let (out, rejected) = decode(&[&w]);
        assert_eq!(out, vec![CH8.to_vec()]);
        assert_eq!(rejected, 1);
    }
}
