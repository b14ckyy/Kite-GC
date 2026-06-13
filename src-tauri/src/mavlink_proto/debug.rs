// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Debug Statistics Tracker
// Only compiled in debug builds (gated via #[cfg(debug_assertions)] in mod.rs).
// MAVLink is push-based, so — unlike the MSP tracker — there is no request/response/
// timeout model. We track every message ID seen in the session, separately per direction
// (RX from the FC, TX from us), with a measured update rate and a "last seen" age.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Per-(message-id, direction) statistics
struct MsgStats {
    name: String,
    is_tx: bool,
    count: u64,
    last_seen: Instant,
    // Rolling 1s window for the per-message rate
    rate_window_start: Instant,
    rate_window_count: u64,
    rate_hz: f64,
}

/// Snapshot of a single MAVLink message's debug info (sent to frontend)
#[derive(Debug, Clone, Serialize)]
pub struct MavMsgDebug {
    pub id: u32,
    pub name: String,
    /// "RX" (from FC) or "TX" (sent by us)
    pub dir: String,
    pub count: u64,
    /// Measured rate in Hz over the last second (0 once the message goes stale)
    pub rate_hz: f64,
    /// Seconds since this message was last seen
    pub last_seen_s: f64,
}

/// Full debug snapshot emitted as a Tauri event
#[derive(Debug, Clone, Serialize)]
pub struct MavlinkDebugSnapshot {
    pub messages: Vec<MavMsgDebug>,
    pub msg_per_sec_rx: f64,
    pub msg_per_sec_tx: f64,
    pub bytes_per_sec_rx: u64,
    pub bytes_per_sec_tx: u64,
}

/// A rate is reported as 0 once nothing has been seen for this long.
const STALE_RATE_SECS: f64 = 2.0;

/// Tracks MAVLink communication statistics in the handler thread
pub struct MavlinkDebugTracker {
    // Keyed by (message_id, is_tx) so HEARTBEAT RX (from FC) and HEARTBEAT TX (our GCS
    // heartbeat) appear as two separate rows.
    stats: HashMap<(u32, bool), MsgStats>,
    // Per-second sliding window for the aggregate throughput counters
    window_start: Instant,
    window_bytes_rx: u64,
    window_bytes_tx: u64,
    window_msg_rx: u64,
    window_msg_tx: u64,
    // Results from last completed 1-second window
    last_bytes_per_sec_rx: u64,
    last_bytes_per_sec_tx: u64,
    last_msg_per_sec_rx: f64,
    last_msg_per_sec_tx: f64,
    // Emit throttle
    last_emit: Instant,
}

impl MavlinkDebugTracker {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            stats: HashMap::new(),
            window_start: now,
            window_bytes_rx: 0,
            window_bytes_tx: 0,
            window_msg_rx: 0,
            window_msg_tx: 0,
            last_bytes_per_sec_rx: 0,
            last_bytes_per_sec_tx: 0,
            last_msg_per_sec_rx: 0.0,
            last_msg_per_sec_tx: 0.0,
            last_emit: now,
        }
    }

    /// Record a received MAVLink frame
    pub fn on_rx(&mut self, msg_id: u32, frame_bytes: usize) {
        self.record(msg_id, false, frame_bytes);
        self.window_bytes_rx += frame_bytes as u64;
        self.window_msg_rx += 1;
    }

    /// Record a transmitted MAVLink frame (our heartbeat, commands, mission items)
    pub fn on_tx(&mut self, msg_id: u32, frame_bytes: usize) {
        self.record(msg_id, true, frame_bytes);
        self.window_bytes_tx += frame_bytes as u64;
        self.window_msg_tx += 1;
    }

    fn record(&mut self, msg_id: u32, is_tx: bool, _frame_bytes: usize) {
        let now = Instant::now();
        let s = self.stats.entry((msg_id, is_tx)).or_insert_with(|| MsgStats {
            name: mavlink_msg_name(msg_id),
            is_tx,
            count: 0,
            last_seen: now,
            rate_window_start: now,
            rate_window_count: 0,
            rate_hz: 0.0,
        });
        s.count += 1;
        s.last_seen = now;
        s.rate_window_count += 1;
        let elapsed = s.rate_window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            s.rate_hz = s.rate_window_count as f64 / elapsed;
            s.rate_window_count = 0;
            s.rate_window_start = now;
        }
    }

    /// Emit debug stats to the frontend at ~60 Hz
    pub fn maybe_emit(&mut self, app_handle: &AppHandle) {
        if self.last_emit.elapsed().as_millis() < 16 {
            return;
        }

        // Roll over the aggregate 1-second window if elapsed
        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.last_bytes_per_sec_rx = (self.window_bytes_rx as f64 / elapsed) as u64;
            self.last_bytes_per_sec_tx = (self.window_bytes_tx as f64 / elapsed) as u64;
            self.last_msg_per_sec_rx = self.window_msg_rx as f64 / elapsed;
            self.last_msg_per_sec_tx = self.window_msg_tx as f64 / elapsed;
            self.window_bytes_rx = 0;
            self.window_bytes_tx = 0;
            self.window_msg_rx = 0;
            self.window_msg_tx = 0;
            self.window_start = Instant::now();
        }

        let mut messages: Vec<MavMsgDebug> = self
            .stats
            .iter()
            .map(|(&(id, _), s)| {
                let age = s.last_seen.elapsed().as_secs_f64();
                MavMsgDebug {
                    id,
                    name: s.name.clone(),
                    dir: if s.is_tx { "TX".into() } else { "RX".into() },
                    count: s.count,
                    // Report 0 once the stream has gone stale, so a stopped message does
                    // not keep showing its last measured rate.
                    rate_hz: if age > STALE_RATE_SECS {
                        0.0
                    } else {
                        (s.rate_hz * 10.0).round() / 10.0
                    },
                    last_seen_s: (age * 10.0).round() / 10.0,
                }
            })
            .collect();

        // Sort: RX before TX, then by message ID
        messages.sort_by(|a, b| {
            a.dir.cmp(&b.dir).then_with(|| a.id.cmp(&b.id))
        });

        let snapshot = MavlinkDebugSnapshot {
            messages,
            msg_per_sec_rx: (self.last_msg_per_sec_rx * 10.0).round() / 10.0,
            msg_per_sec_tx: (self.last_msg_per_sec_tx * 10.0).round() / 10.0,
            bytes_per_sec_rx: self.last_bytes_per_sec_rx,
            bytes_per_sec_tx: self.last_bytes_per_sec_tx,
        };

        let _ = app_handle.emit("debug-mavlink-stats", &snapshot);
        self.last_emit = Instant::now();
    }
}

/// Map a MAVLink message ID to a human-readable name. Covers the common ArduPilot
/// telemetry stream + the messages we send/handle; anything else falls back to MSG_<id>.
fn mavlink_msg_name(id: u32) -> String {
    match id {
        0 => "HEARTBEAT".into(),
        1 => "SYS_STATUS".into(),
        2 => "SYSTEM_TIME".into(),
        4 => "PING".into(),
        20 => "PARAM_REQUEST_READ".into(),
        21 => "PARAM_REQUEST_LIST".into(),
        22 => "PARAM_VALUE".into(),
        23 => "PARAM_SET".into(),
        24 => "GPS_RAW_INT".into(),
        25 => "GPS_STATUS".into(),
        26 => "SCALED_IMU".into(),
        27 => "RAW_IMU".into(),
        29 => "SCALED_PRESSURE".into(),
        30 => "ATTITUDE".into(),
        31 => "ATTITUDE_QUATERNION".into(),
        32 => "LOCAL_POSITION_NED".into(),
        33 => "GLOBAL_POSITION_INT".into(),
        34 => "RC_CHANNELS_SCALED".into(),
        35 => "RC_CHANNELS_RAW".into(),
        36 => "SERVO_OUTPUT_RAW".into(),
        39 => "MISSION_ITEM".into(),
        40 => "MISSION_REQUEST".into(),
        42 => "MISSION_CURRENT".into(),
        43 => "MISSION_REQUEST_LIST".into(),
        44 => "MISSION_COUNT".into(),
        45 => "MISSION_CLEAR_ALL".into(),
        47 => "MISSION_ACK".into(),
        49 => "GPS_GLOBAL_ORIGIN".into(),
        51 => "MISSION_REQUEST_INT".into(),
        62 => "NAV_CONTROLLER_OUTPUT".into(),
        65 => "RC_CHANNELS".into(),
        66 => "REQUEST_DATA_STREAM".into(),
        73 => "MISSION_ITEM_INT".into(),
        74 => "VFR_HUD".into(),
        76 => "COMMAND_LONG".into(),
        77 => "COMMAND_ACK".into(),
        87 => "POSITION_TARGET_GLOBAL_INT".into(),
        111 => "TIMESYNC".into(),
        116 => "SCALED_IMU2".into(),
        125 => "POWER_STATUS".into(),
        129 => "SCALED_IMU3".into(),
        136 => "TERRAIN_REPORT".into(),
        137 => "SCALED_PRESSURE2".into(),
        147 => "BATTERY_STATUS".into(),
        148 => "AUTOPILOT_VERSION".into(),
        152 => "MEMINFO".into(),
        163 => "AHRS".into(),
        165 => "HWSTATUS".into(),
        168 => "WIND".into(),
        178 => "AHRS2".into(),
        182 => "AHRS3".into(),
        193 => "EKF_STATUS_REPORT".into(),
        241 => "VIBRATION".into(),
        242 => "HOME_POSITION".into(),
        244 => "MESSAGE_INTERVAL".into(),
        253 => "STATUSTEXT".into(),
        _ => format!("MSG_{}", id),
    }
}
