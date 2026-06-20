// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Always-on link throughput meter (RX/TX bytes + messages per second) for the live FC connection.
//
// Unlike the Debug Monitor trackers (`scheduler::debug` / `mavlink_proto::debug`), which carry the
// rich per-message breakdown and are compiled only in debug builds, this lightweight meter is
// compiled into release builds too — so the Relay panel can show the live link rate regardless of
// build profile. It tracks only the four aggregate numbers the panel needs and emits them on the
// `link-stats` event at ~10 Hz. Both the MSP scheduler and the MAVLink handler feed one instance.

use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Aggregate link throughput emitted as the `link-stats` Tauri event.
#[derive(Clone, Serialize)]
pub struct LinkStatsSnapshot {
    pub bytes_per_sec_rx: u64,
    pub bytes_per_sec_tx: u64,
    pub msg_per_sec_rx: f64,
    pub msg_per_sec_tx: f64,
}

/// Sliding 1-second throughput accumulator with a throttled emit.
pub struct LinkStats {
    window_start: Instant,
    bytes_rx: u64,
    bytes_tx: u64,
    msg_rx: u64,
    msg_tx: u64,
    last: LinkStatsSnapshot,
    last_emit: Instant,
}

impl LinkStats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            window_start: now,
            bytes_rx: 0,
            bytes_tx: 0,
            msg_rx: 0,
            msg_tx: 0,
            last: LinkStatsSnapshot {
                bytes_per_sec_rx: 0,
                bytes_per_sec_tx: 0,
                msg_per_sec_rx: 0.0,
                msg_per_sec_tx: 0.0,
            },
            last_emit: now,
        }
    }

    /// Record an outbound frame (request / heartbeat / command).
    #[inline]
    pub fn on_tx(&mut self, frame_bytes: usize) {
        self.bytes_tx += frame_bytes as u64;
        self.msg_tx += 1;
    }

    /// Record an inbound frame (response / telemetry message).
    #[inline]
    pub fn on_rx(&mut self, frame_bytes: usize) {
        self.bytes_rx += frame_bytes as u64;
        self.msg_rx += 1;
    }

    /// Roll the 1-second window if elapsed, then emit the latest snapshot (throttled to ~10 Hz).
    pub fn maybe_emit(&mut self, app_handle: &AppHandle) {
        if self.last_emit.elapsed().as_millis() < 100 {
            return;
        }

        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.last = LinkStatsSnapshot {
                bytes_per_sec_rx: (self.bytes_rx as f64 / elapsed) as u64,
                bytes_per_sec_tx: (self.bytes_tx as f64 / elapsed) as u64,
                msg_per_sec_rx: (self.msg_rx as f64 / elapsed * 10.0).round() / 10.0,
                msg_per_sec_tx: (self.msg_tx as f64 / elapsed * 10.0).round() / 10.0,
            };
            self.bytes_rx = 0;
            self.bytes_tx = 0;
            self.msg_rx = 0;
            self.msg_tx = 0;
            self.window_start = Instant::now();
        }

        let _ = app_handle.emit("link-stats", &self.last);
        self.last_emit = Instant::now();
    }
}

impl Default for LinkStats {
    fn default() -> Self {
        Self::new()
    }
}
