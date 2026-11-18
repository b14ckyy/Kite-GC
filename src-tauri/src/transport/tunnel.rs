// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP-over-MAVLink tunnel transport (INAV 10.0+, docs: Dev-Docs active/MSP_OVER_MAVLINK.md).
//
// A `ByteTransport` that carries the framed MSP byte stream inside MAVLink TUNNEL (#385) messages
// through the running MAVLink handler — the handler stays the only reader/writer of the real link.
//   write: the MSP frame is cut into ≤128-byte chunks, each sent as one TUNNEL (payload type 0x8001,
//          target = FC sysid / component 1) back-to-back via `MavlinkCommand::SendRaw`; the handler
//          frames it with its own MAVLink sequence number.
//   read:  TUNNEL payloads addressed to Kite arrive on an mpsc channel fed by the handler and are
//          served as a plain byte stream (with a carry-over when the caller's buffer is smaller).
// The channel closing (handler thread ended) reads as `Disconnected`.
//
// Also home of the tunnel's Debug Monitor tracker (`debug-tunnel-stats`), fed by this transport, the
// tunnel-mode MSP scheduler and the connect-time probe.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::{ByteTransport, TransportError};
use crate::mavlink_proto::handler::MavlinkCommand;
use crate::mavlink_proto::tunnel::{
    encode_chunk, TUNNEL_CHUNK_MAX, TUNNEL_CRC_EXTRA, TUNNEL_MSG_ID,
};

/// Default read timeout (same as the network/serial transports' construction default).
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(50);
/// Target component of every request: MAV_COMP_ID_AUTOPILOT1 (INAV accepts 0 or 1).
const TARGET_COMPONENT: u8 = 1;

pub struct TunnelTransport {
    cmd_tx: mpsc::Sender<MavlinkCommand>,
    rx: mpsc::Receiver<Vec<u8>>,
    fc_sysid: u8,
    /// Description of the MAVLink link underneath (for `description()`).
    link_desc: String,
    /// Bytes of the last received chunk that did not fit the caller's buffer.
    carry: Vec<u8>,
    carry_pos: usize,
    read_timeout: Duration,
    /// Unregister the handler's tunnel receiver on drop (false in unit tests without a handler).
    registered: bool,
}

impl TunnelTransport {
    /// Register a tunnel receiver with the running MAVLink handler and return the transport.
    /// `link_desc` = the description of the MAVLink handler's own transport (for logging).
    pub fn open(cmd_tx: mpsc::Sender<MavlinkCommand>, fc_sysid: u8, link_desc: &str) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        cmd_tx
            .send(MavlinkCommand::RegisterTunnelReceiver(tx))
            .map_err(|_| "MAVLink handler gone — cannot open the MSP tunnel".to_string())?;
        let mut t = Self::from_parts(cmd_tx, rx, fc_sysid);
        t.link_desc = link_desc.to_string();
        t.registered = true;
        Ok(t)
    }

    fn from_parts(cmd_tx: mpsc::Sender<MavlinkCommand>, rx: mpsc::Receiver<Vec<u8>>, fc_sysid: u8) -> Self {
        Self {
            cmd_tx,
            rx,
            fc_sysid,
            link_desc: String::new(),
            carry: Vec::new(),
            carry_pos: 0,
            read_timeout: DEFAULT_READ_TIMEOUT,
            registered: false,
        }
    }

    /// Serve buffered carry-over bytes into `buf`; returns how many were copied.
    fn take_carry(&mut self, buf: &mut [u8]) -> usize {
        let rest = &self.carry[self.carry_pos..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.carry_pos += n;
        if self.carry_pos >= self.carry.len() {
            self.carry.clear();
            self.carry_pos = 0;
        }
        n
    }
}

impl ByteTransport for TunnelTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.carry_pos < self.carry.len() {
            return Ok(self.take_carry(buf));
        }
        match self.rx.recv_timeout(self.read_timeout) {
            Ok(chunk) => {
                stats::on_chunk_rx(chunk.len());
                log::debug!("MSP tunnel RX chunk: {} bytes", chunk.len());
                self.carry = chunk;
                self.carry_pos = 0;
                Ok(self.take_carry(buf))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(0),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        stats::on_write_start();
        for chunk in data.chunks(TUNNEL_CHUNK_MAX) {
            let payload = encode_chunk(self.fc_sysid, TARGET_COMPONENT, chunk);
            self.cmd_tx
                .send(MavlinkCommand::SendRaw {
                    msg_id: TUNNEL_MSG_ID,
                    crc_extra: TUNNEL_CRC_EXTRA,
                    payload: payload.to_vec(),
                })
                .map_err(|_| TransportError::Disconnected)?;
            stats::on_chunk_tx(chunk.len());
            log::debug!("MSP tunnel TX chunk: {} bytes", chunk.len());
        }
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }

    fn description(&self) -> String {
        format!("MSP/MAV tunnel over {}", self.link_desc)
    }
}

impl Drop for TunnelTransport {
    fn drop(&mut self) {
        if self.registered {
            // Fails silently when the handler is already gone — nothing left to unregister then.
            let _ = self.cmd_tx.send(MavlinkCommand::UnregisterTunnelReceiver);
        }
    }
}

// ── Debug Monitor tracker ────────────────────────────────────────────

/// Tunnel diagnostics for the Debug Monitor's Tunnel tab (`debug-tunnel-stats`). One process-wide
/// instance: the transport, the tunnel-mode scheduler, the probe and the dev request command all feed
/// it from different threads. Compiled into all builds; every recorder early-returns unless
/// `debug_mode::enabled()` (same runtime gate as the MSP / MAVLink trackers).
pub mod stats {
    use super::*;

    /// Snapshot emitted as `debug-tunnel-stats`.
    #[derive(Debug, Clone, Default, Serialize)]
    pub struct TunnelStatsSnapshot {
        /// A tunnel-mode MSP scheduler is running on the current link.
        pub active: bool,
        pub chunks_tx: u64,
        pub chunks_rx: u64,
        pub bytes_tx: u64,
        pub bytes_rx: u64,
        /// MSP requests / replies through the tunnel scheduler (one-shots incl. retries).
        pub requests: u64,
        pub replies: u64,
        /// Requests that got no reply in time (each attempt counts).
        pub timeouts: u64,
        /// Whole-request retries sent after a first timeout.
        pub retries: u64,
        /// Late replies dropped as stale (a code whose retried request had already completed).
        pub stale_replies: u64,
        /// Queued requests dropped unsent because their caller had already given up.
        pub expired: u64,
        /// MSP frames the MSP parser dropped on a checksum mismatch (a lost / corrupted chunk).
        pub checksum_failures: u32,
        /// Largest framed MSP reply (header + payload + checksum) and the TUNNEL chunks it arrived in.
        pub largest_reply_bytes: u32,
        pub largest_reply_chunks: u32,
        /// TUNNEL chunks received since the last request went out (the multi-chunk reply test).
        pub last_request_chunks: u32,
        /// MSP code currently in flight (at most one in tunnel mode).
        pub in_flight: Option<u16>,
        /// Last connect-time probe: "" (none yet) | "ok" | "no_reply" | "rejected".
        pub probe_result: String,
        pub probe_rtt_ms: Option<u64>,
        /// Human-readable probe detail (the attempt that answered, or why it was rejected).
        pub probe_detail: String,
    }

    struct Tracker {
        snap: TunnelStatsSnapshot,
        last_emit: Option<Instant>,
    }

    static TRACKER: Mutex<Tracker> = Mutex::new(Tracker {
        snap: TunnelStatsSnapshot {
            active: false,
            chunks_tx: 0,
            chunks_rx: 0,
            bytes_tx: 0,
            bytes_rx: 0,
            requests: 0,
            replies: 0,
            timeouts: 0,
            retries: 0,
            stale_replies: 0,
            expired: 0,
            checksum_failures: 0,
            largest_reply_bytes: 0,
            largest_reply_chunks: 0,
            last_request_chunks: 0,
            in_flight: None,
            probe_result: String::new(),
            probe_rtt_ms: None,
            probe_detail: String::new(),
        },
        last_emit: None,
    });

    /// Emission throttle: ≤ 2 Hz.
    const EMIT_INTERVAL: Duration = Duration::from_millis(500);

    fn with<F: FnOnce(&mut TunnelStatsSnapshot)>(f: F) {
        if !crate::debug_mode::enabled() {
            return;
        }
        if let Ok(mut t) = TRACKER.lock() {
            f(&mut t.snap);
        }
    }

    /// Clear every counter (new connection).
    pub fn reset() {
        with(|s| *s = TunnelStatsSnapshot::default());
    }

    pub fn set_active(active: bool) {
        with(|s| s.active = active);
    }

    pub(super) fn on_write_start() {
        with(|s| s.last_request_chunks = 0);
    }

    pub(super) fn on_chunk_tx(bytes: usize) {
        with(|s| {
            s.chunks_tx += 1;
            s.bytes_tx += bytes as u64;
        });
    }

    pub(super) fn on_chunk_rx(bytes: usize) {
        with(|s| {
            s.chunks_rx += 1;
            s.bytes_rx += bytes as u64;
            s.last_request_chunks += 1;
        });
    }

    pub fn on_request(code: u16) {
        with(|s| {
            s.requests += 1;
            s.in_flight = Some(code);
        });
    }

    /// A reply matched its request; `framed_bytes` = the MSP frame as it crossed the tunnel.
    pub fn on_reply(framed_bytes: usize) {
        with(|s| {
            s.replies += 1;
            s.in_flight = None;
            if framed_bytes as u32 >= s.largest_reply_bytes {
                s.largest_reply_bytes = framed_bytes as u32;
                s.largest_reply_chunks = s.last_request_chunks;
            }
        });
    }

    pub fn on_timeout() {
        with(|s| {
            s.timeouts += 1;
            s.in_flight = None;
        });
    }

    pub fn on_retry(code: u16) {
        with(|s| {
            s.retries += 1;
            s.requests += 1;
            s.in_flight = Some(code);
        });
    }

    pub fn on_stale_reply() {
        with(|s| s.stale_replies += 1);
    }

    pub fn on_expired() {
        with(|s| s.expired += 1);
    }

    pub fn set_checksum_failures(n: u32) {
        with(|s| s.checksum_failures = n);
    }

    pub fn set_probe(result: &str, rtt_ms: Option<u64>, detail: String) {
        with(|s| {
            s.probe_result = result.to_string();
            s.probe_rtt_ms = rtt_ms;
            s.probe_detail = detail;
        });
    }

    /// The current snapshot (Debug Monitor tab opened after connect).
    pub fn snapshot() -> TunnelStatsSnapshot {
        TRACKER.lock().map(|t| t.snap.clone()).unwrap_or_default()
    }

    /// TUNNEL chunks received since the last request was written.
    pub fn last_request_chunks() -> u32 {
        TRACKER.lock().map(|t| t.snap.last_request_chunks).unwrap_or(0)
    }

    /// Emit the snapshot, throttled to ≤ 2 Hz.
    pub fn maybe_emit(app: &AppHandle) {
        if !crate::debug_mode::enabled() {
            return;
        }
        let snap = match TRACKER.lock() {
            Ok(mut t) => {
                if t.last_emit.is_some_and(|l| l.elapsed() < EMIT_INTERVAL) {
                    return;
                }
                t.last_emit = Some(Instant::now());
                t.snap.clone()
            }
            Err(_) => return,
        };
        let _ = app.emit("debug-tunnel-stats", &snap);
    }

    /// Emit the snapshot now (probe finished, scheduler stopped) regardless of the throttle.
    pub fn emit_now(app: &AppHandle) {
        if !crate::debug_mode::enabled() {
            return;
        }
        let snap = match TRACKER.lock() {
            Ok(mut t) => {
                t.last_emit = Some(Instant::now());
                t.snap.clone()
            }
            Err(_) => return,
        };
        let _ = app.emit("debug-tunnel-stats", &snap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mavlink_proto::tunnel::{self, PAYLOAD_TYPE_INAV_MSP};

    fn pair(fc_sysid: u8) -> (TunnelTransport, mpsc::Receiver<MavlinkCommand>, mpsc::Sender<Vec<u8>>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (data_tx, data_rx) = mpsc::channel();
        (TunnelTransport::from_parts(cmd_tx, data_rx, fc_sysid), cmd_rx, data_tx)
    }

    #[test]
    fn write_splits_into_128_byte_chunks() {
        let (mut t, cmd_rx, _data_tx) = pair(7);
        let frame: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8 + 1).collect();
        t.write_bytes(&frame).unwrap();

        let mut sizes = Vec::new();
        let mut rebuilt = Vec::new();
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                MavlinkCommand::SendRaw { msg_id, crc_extra, payload } => {
                    assert_eq!((msg_id, crc_extra), (385, 147));
                    let raw = tunnel::decode(&payload).expect("valid TUNNEL payload");
                    assert_eq!(raw.payload_type, PAYLOAD_TYPE_INAV_MSP);
                    assert_eq!((raw.target_system, raw.target_component), (7, 1));
                    sizes.push(raw.payload.len());
                    rebuilt.extend_from_slice(&raw.payload);
                }
                _ => panic!("unexpected handler command"),
            }
        }
        assert_eq!(sizes, vec![128, 128, 44]);
        assert_eq!(rebuilt, frame);
    }

    #[test]
    fn read_reassembles_through_a_small_buffer() {
        let (mut t, _cmd_rx, data_tx) = pair(1);
        let frame: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        for c in frame.chunks(128) {
            data_tx.send(c.to_vec()).unwrap();
        }
        t.set_read_timeout(Duration::from_millis(5));
        let mut out = Vec::new();
        let mut buf = [0u8; 50];
        loop {
            let n = t.read_bytes(&mut buf).unwrap();
            if n == 0 {
                break; // timeout: channel drained
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(out, frame);
    }

    #[test]
    fn read_reports_disconnect_when_handler_is_gone() {
        let (mut t, _cmd_rx, data_tx) = pair(1);
        drop(data_tx);
        let mut buf = [0u8; 16];
        assert!(matches!(t.read_bytes(&mut buf), Err(TransportError::Disconnected)));
    }
}
