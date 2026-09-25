// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Scheduler Module
// Dedicated thread that owns the serial connection and coordinates all MSP traffic.
// Telemetry slots are time-based, commands and bulk transfers interleave between polls.

pub mod rc_tx;
pub mod telemetry;

// The DebugTracker is compiled into ALL builds now (it used to be a release-time no-op stub), so a
// release `--debug` run can populate the Debug Monitor. Its methods early-return on
// `crate::debug_mode::enabled()`, so when debug mode is off the cost is a single relaxed atomic load
// per call (ADR-008 runtime-gated instead of compiled out).
mod debug;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::link_stats::LinkStats;
use crate::radar;
use crate::radar::source::SourceUpdate;
use crate::transport::Transport;

/// Payload for the `telemetry-fc-link` event (status-icon liveness): `false` while the link is stalled
/// (transport still open but no MSP replies — e.g. the BLE transport silently reconnecting underneath),
/// `true` when telemetry resumes. Lets the UI show a "reconnecting" state without a teardown.
#[derive(Clone, serde::Serialize)]
struct FcLinkAlive {
    alive: bool,
}

/// How often to poll ADS-B from the FC via MSP when enabled (bandwidth-heavy; lowest cadence).
const RADAR_MSP_INTERVAL: Duration = Duration::from_millis(1000);
/// Shared ingest channel + on/off flag for the scheduler-fed ADS-B-via-MSP source.
type RadarIngest = Arc<Mutex<Option<mpsc::Sender<SourceUpdate>>>>;
/// Reply channel of a one-shot UI request.
type OneShotReply = mpsc::Sender<Result<Vec<u8>, String>>;
/// A one-shot request waiting to be (re-)sent: code, payload, reply channel, when the caller asked, and
/// the probe deadline (`None` = normal one-shot, see `SchedulerCommand::MspRequest::timeout`).
type QueuedOneShot = (u16, Vec<u8>, OneShotReply, Instant, Option<Duration>);

/// RC injection cadences (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c). RAW rate is dynamic (RcTxState,
/// user-selectable 10–25 Hz); AUX re-send weave is fixed at 5 Hz.
const RC_AUX_INTERVAL: Duration = Duration::from_millis(200);
/// Frontend-heartbeat deadman (shared with the MAVLink handler) — see rc_tx::RC_DEADMAN.
use rc_tx::RC_DEADMAN;
/// Link-speed probe: for this long after engage, send RAW_RC WITH reply and measure the ACK round-trip.
const RC_PROBE_WINDOW: Duration = Duration::from_secs(2);
/// Per-frame ACK timeout during the probe (ms) — beyond this the frame counts as a timeout.
const RC_PROBE_TIMEOUT_MS: u64 = 300;

/// Result of the post-engage link-speed probe, emitted to the frontend when the link looks too slow
/// for the selected RC rate (`rc-link-slow`).
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RcLinkWarning {
    bad_pct: u32,
    avg_latency_ms: u32,
    raw_rate_hz: u32,
}

pub use telemetry::{TelemetryConfig, TelemetryGroup};

/// Commands sent to the scheduler thread via channel
#[derive(Debug)]
pub enum SchedulerCommand {
    /// Stop the scheduler and return the serial connection
    Stop,
    /// Send a one-shot MSP request and return the response
    MspRequest {
        code: u16,
        payload: Vec<u8>,
        reply: mpsc::Sender<Result<Vec<u8>, String>>,
        /// `None` = the normal one-shot (2 s, one whole-request retry in tunnel mode where allowed).
        /// `Some(t)` = a liveness probe (`MspRequester::probe`): reply deadline `t`, never retried.
        timeout: Option<Duration>,
    },
}

/// How the scheduler runs (docs: Dev-Docs active/MSP_OVER_MAVLINK.md §D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    /// Direct MSP link: telemetry polling, events, stall watchdog, recorder, RC stream, radar.
    Telemetry,
    /// MSP over the MAVLink TUNNEL (INAV 10.0+): on-demand one-shots only. No telemetry slots, no
    /// `telemetry-*` / `telemetry-fc-link` / `connection-lost` / `link-stats` events (the MAVLink handler
    /// owns all of those), no recorder, no RC stream or radar polling, strictly ONE request in flight
    /// (the FC has one tunnel parser per port and replies carry no request id), and a one-shot that
    /// times out is re-sent once as a whole before it fails.
    Tunnel,
}

/// A single MSP request in flight in the pipelined scheduler (see docs active/MSP_PIPELINE.md). Incoming
/// frames are matched back to these **by MSP code** — the scheduler keeps many outstanding at once instead
/// of blocking on each, which is what lets it saturate a high-latency link (mLRS/BLE) the way the INAV
/// Configurator does (same per-message round trip, far higher throughput).
struct Pending {
    /// When the request went out — for the round-trip sample and the reply timeout.
    sent_at: Instant,
    /// Drop/fail the request once this passes with no matching reply.
    deadline: Instant,
    kind: PendingKind,
}

enum PendingKind {
    /// A telemetry poll — decode + emit + feed the recorder on reply.
    Telemetry,
    /// A one-shot UI request — the raw payload goes back through this channel on reply (Err on timeout).
    /// `retry` holds the request payload while one whole-request retry is still allowed (tunnel mode).
    OneShot {
        reply: OneShotReply,
        retry: Option<Vec<u8>>,
        /// When the caller submitted the request (tunnel mode: its outer wait runs from here).
        queued_at: Instant,
        /// This attempt is the whole-request retry (tunnel mode).
        retried: bool,
    },
    /// An ADS-B-via-MSP poll feeding the radar pipeline.
    Radar,
    /// A link-speed probe RAW_RC (sent WITH reply during the post-engage window) — times the ACK.
    RcProbe,
    /// A retried MSP_BOXIDS query — its reply installs the index→permanent-ID mapping.
    BoxIds,
}

/// Max telemetry/one-shot/radar requests in flight at once (protects the FC's MSP RX buffer). RC frames
/// are exempt — they're the control path and always go out at max priority. Tune on-device; the INAV
/// Configurator's implicit cap is ~100 Hz emission + per-code dedup. Tunnel mode uses 1 instead.
const MAX_IN_FLIGHT: usize = 8;

/// Max MSP frames written per scheduler tick — the "soft lock", named after the INAV Configurator's
/// queue (`js/serial_queue.js`), whose executor takes **exactly one** message per run behind a port
/// lock released on write completion. We keep the pipeline: replies are still never waited for and up
/// to `MAX_IN_FLIGHT` stay outstanding. What changes is that a drain (`poll_incoming`) now always runs
/// *between* two writes instead of a whole tick's budget going out back to back, so replies are
/// consumed as the FC produces them rather than piling up in its TX buffer. At the ~125 Hz tick rate
/// (`SCHED_READ_TIMEOUT`) this still allows far more requests per second than any slot set asks for.
/// RC frames are exempt — they're the control path and bypass this budget entirely.
const MAX_WRITES_PER_TICK: usize = 1;
/// How long to wait before asking for MSP_BOXIDS again while we still have no mapping. Without it the
/// activeModes bitmask cannot be decoded at all (`parse_active_modes` refuses to guess), so the whole
/// session shows no flight mode and MSP RC OVERRIDE is never detected — for the price of one 9-byte
/// frame every few seconds we simply keep asking until the FC answers.
const BOXIDS_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Short transport read timeout so the pipelined drain/emit loop ticks ≈100 Hz (like Configurator's
/// executor) instead of being quantized by the coarse idle read timeout.
const SCHED_READ_TIMEOUT: Duration = Duration::from_millis(8);
/// One-shot UI request timeout — matches the old blocking `MSP_RESPONSE_TIMEOUT_MS`.
const MSP_ONESHOT_TIMEOUT_MS: u64 = 2000;
/// How long `MspRequester::msp_request` waits for its reply on a direct MSP link.
const OUTER_WAIT: Duration = Duration::from_secs(5);
/// Tunnel mode: 2 s + 2 s retry, plus queueing behind other one-shots and the 2 s BOXIDS read at start.
const OUTER_WAIT_TUNNEL: Duration = Duration::from_secs(10);
/// Tunnel mode: after a retried / timed-out request of code C completed, a new request of C is held
/// back by ~2×RTT (clamped to this range) so a late duplicate reply lands — and is dropped as stale —
/// before the new request could take it for its own answer.
const STALE_MARGIN_MIN: Duration = Duration::from_millis(200);
const STALE_MARGIN_MAX: Duration = Duration::from_secs(2);
/// Tunnel mode: how long a code stays on the stale watch list (for counting late replies).
const STALE_WATCH_KEEP: Duration = Duration::from_secs(10);
/// Lower bound for the adaptive scale — even a badly congested link keeps polling at the slot floors.
const SCALE_MIN: f64 = 0.05;
/// How often the AIMD scale is re-evaluated from the saturation seen since the last adjust.
const SCALE_ADJUST_INTERVAL: Duration = Duration::from_millis(500);

/// A telemetry slot that tracks when it was last polled
struct TelemetrySlot {
    group: TelemetryGroup,
    codes: Vec<u16>,
    interval: Duration,
    last_poll: Option<Instant>,
    /// For groups with rotating secondary codes, track current index
    rotation_index: usize,
    /// Next code to send in a multi-code slot. The write budget can cut a slot off mid-list, and
    /// without a cursor the next turn would restart at codes[0] — permanently starving everything
    /// behind it (see the poll loop).
    code_cursor: usize,
    /// Scheduling priority (higher = more important, polled first when overloaded)
    priority: u8,
    /// Minimum poll rate (Hz) this slot is never scaled below under link congestion (adaptive floor).
    floor_hz: f64,
}

impl TelemetrySlot {
    fn new(group: TelemetryGroup, codes: Vec<u16>, rate_hz: f64, priority: u8, floor_hz: f64) -> Self {
        Self {
            group,
            codes,
            interval: Duration::from_secs_f64(1.0 / rate_hz),
            last_poll: None,
            rotation_index: 0,
            code_cursor: 0,
            priority,
            floor_hz,
        }
    }

    /// Effective poll interval after adaptive scaling: the configured interval stretched by `1/scale`
    /// (slower when the link can't keep up with total demand), clamped so the rate never drops below the
    /// slot's floor. `scale` is 1.0 when the link keeps up, smaller as it congests — so all slots back off
    /// proportionally, hitting their floors only in the extreme.
    fn effective_interval(&self, scale: f64) -> Duration {
        let stretched = self.interval.as_secs_f64() / scale.clamp(0.01, 1.0);
        let floor_interval = 1.0 / self.floor_hz; // slowest allowed
        Duration::from_secs_f64(stretched.min(floor_interval))
    }

    /// How long this slot is overdue (None if not yet due), against its scaled effective interval.
    fn overdue(&self, scale: f64) -> Option<Duration> {
        let interval = self.effective_interval(scale);
        match self.last_poll {
            None => Some(Duration::from_secs(999)), // never polled = maximally overdue
            Some(last) => {
                let elapsed = last.elapsed();
                if elapsed >= interval {
                    Some(elapsed - interval)
                } else {
                    None
                }
            }
        }
    }

}

/// Handle returned to the caller to interact with the running scheduler
pub struct SchedulerHandle {
    requester: MspRequester,
    thread: Option<thread::JoinHandle<Option<Box<dyn Transport>>>>,
}

impl SchedulerHandle {
    /// A cloneable request handle — lets a command release `AppState::protocol` before a multi-request
    /// transaction (see `state::with_msp`), so a long transfer never blocks `disconnect`.
    pub fn requester(&self) -> MspRequester {
        self.requester.clone()
    }

    /// Stop the scheduler and return the transport for cleanup
    pub fn stop(mut self) -> Option<Box<dyn Transport>> {
        let _ = self.requester.cmd_tx.send(SchedulerCommand::Stop);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .flatten()
    }
}

/// The request side of a running scheduler — clone of its command sender, without the thread handle.
/// Once the scheduler has stopped every request fails fast (the channel is closed).
#[derive(Clone)]
pub struct MspRequester {
    cmd_tx: mpsc::Sender<SchedulerCommand>,
    /// Reply wait of `msp_request` — longer in tunnel mode (see `OUTER_WAIT_TUNNEL`).
    outer_wait: Duration,
    /// One multi-request transaction at a time per connection (see `begin_transaction`).
    txn: Arc<Mutex<()>>,
    /// The scheduler runs in tunnel mode (MSP over MAVLink).
    tunnel: bool,
}

/// Outcome of `MspRequester::probe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The FC answered within the probe timeout.
    Answered,
    /// No answer within the timeout (or the request failed on the wire).
    Silent,
    /// The scheduler is gone — the link was lost or the connection closed.
    SchedulerGone,
}

impl MspRequester {
    /// Send a one-shot MSP command through the scheduler (blocks until response)
    pub fn msp_request(&self, code: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(SchedulerCommand::MspRequest {
                code,
                payload: payload.to_vec(),
                reply: reply_tx,
                timeout: None,
            })
            .map_err(|_| "Scheduler thread gone".to_string())?;
        reply_rx
            .recv_timeout(self.outer_wait)
            .map_err(|_| "Scheduler request timeout".to_string())?
    }

    /// Liveness probe: send `code` (empty payload) with a short reply deadline and no retry.
    pub fn probe(&self, code: u16, timeout: Duration) -> ProbeOutcome {
        let (reply_tx, reply_rx) = mpsc::channel();
        let sent = self.cmd_tx.send(SchedulerCommand::MspRequest {
            code,
            payload: Vec::new(),
            reply: reply_tx,
            timeout: Some(timeout),
        });
        if sent.is_err() {
            return ProbeOutcome::SchedulerGone;
        }
        // Allow for the tunnel's stale-reply hold-off before the probe is even sent.
        match reply_rx.recv_timeout(timeout + STALE_MARGIN_MAX + Duration::from_millis(500)) {
            Ok(Ok(_)) => ProbeOutcome::Answered,
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Timeout) => ProbeOutcome::Silent,
            Err(mpsc::RecvTimeoutError::Disconnected) => ProbeOutcome::SchedulerGone,
        }
    }

    /// The scheduler runs over the MSP-over-MAVLink tunnel (the MAVLink link outlives an FC reboot).
    pub fn is_tunnel(&self) -> bool {
        self.tunnel
    }

    /// Serialise multi-request transactions (a mission transfer, a safehome batch, …) of this connection:
    /// they used to be serialised by holding `AppState::protocol` for the whole command. Not re-entrant —
    /// never nest two transactions on one thread.
    pub fn begin_transaction(&self) -> std::sync::MutexGuard<'_, ()> {
        self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Wait up to `max` for a running transaction to finish (disconnect: let a save complete instead of
    /// stopping the scheduler between its SET requests). `false` = still busy when `max` ran out.
    pub fn wait_idle(&self, max: Duration) -> bool {
        let until = Instant::now() + max;
        loop {
            match self.txn.try_lock() {
                Ok(_) | Err(std::sync::TryLockError::Poisoned(_)) => return true,
                Err(std::sync::TryLockError::WouldBlock) => {}
            }
            if Instant::now() >= until {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Start the MSP scheduler on a dedicated thread
#[allow(clippy::too_many_arguments)] // thread entry context, mirrors scheduler_loop
pub fn start(
    transport: Box<dyn Transport>,
    config: TelemetryConfig,
    app_handle: AppHandle,
    recorder: Option<FlightRecorderHandle>,
    radar_ingest: RadarIngest,
    radar_msp_enabled: Arc<AtomicBool>,
    rc_tx: rc_tx::RcTxHandle,
    mode: SchedulerMode,
) -> SchedulerHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SchedulerCommand>();
    if mode == SchedulerMode::Tunnel {
        // Active from creation — the BOXIDS pre-loop read inside the thread can take up to 2 s.
        crate::transport::tunnel::stats::set_active(true);
    }

    let thread = thread::spawn(move || {
        scheduler_loop(transport, config, app_handle, cmd_rx, recorder, radar_ingest, radar_msp_enabled, rc_tx, mode)
    });

    SchedulerHandle {
        requester: MspRequester {
            cmd_tx,
            outer_wait: if mode == SchedulerMode::Tunnel { OUTER_WAIT_TUNNEL } else { OUTER_WAIT },
            txn: Arc::new(Mutex::new(())),
            tunnel: mode == SchedulerMode::Tunnel,
        },
        thread: Some(thread),
    }
}

/// Main scheduler loop — runs until Stop command received
#[allow(clippy::too_many_arguments)] // cohesive thread entry point; args carry the loop's full context
fn scheduler_loop(
    mut transport: Box<dyn Transport>,
    config: TelemetryConfig,
    app_handle: AppHandle,
    cmd_rx: mpsc::Receiver<SchedulerCommand>,
    recorder: Option<FlightRecorderHandle>,
    radar_ingest: RadarIngest,
    radar_msp_enabled: Arc<AtomicBool>,
    rc_tx: rc_tx::RcTxHandle,
    mode: SchedulerMode,
) -> Option<Box<dyn Transport>> {
    let tunnel = mode == SchedulerMode::Tunnel;
    // Tunnel mode polls nothing — MAVLink streams carry the telemetry.
    let mut slots = if tunnel { Vec::new() } else { build_slots(&config) };
    let max_in_flight = if tunnel { 1 } else { MAX_IN_FLIGHT };
    // Tunnel mode: one-shots wait here until the single in-flight slot is free (FIFO).
    let mut oneshot_queue: VecDeque<QueuedOneShot> = VecDeque::new();
    // Tunnel mode: codes whose retried (or finally timed-out) request completed → when. A reply of such
    // a code arriving with nothing of it pending is a late duplicate (dropped, counted), and a new
    // request of that code is held back by the stale margin (see `STALE_MARGIN_MIN`).
    let mut stale_watch: HashMap<u16, Instant> = HashMap::new();
    // Adaptive rate scaling (AIMD on *measured* saturation, not a predicted ratio): when the in-flight
    // window can't serve all the due slots — or replies time out — the link is the bottleneck, so we ease
    // every slot's rate down proportionally toward its floor; when there's headroom we recover toward full
    // rate. Self-calibrating, so it never throttles while bandwidth is free and degrades *proportionally*
    // rather than starving the low-priority slots. `rtt_ewma` feeds the reply timeout + diagnostics.
    let mut rtt_ewma: Option<f64> = None; // seconds, EWMA of pipelined send→reply round trips
    let mut scale: f64 = 1.0;
    let mut last_scale_adjust = Instant::now();
    let mut saturated_window = false; // any saturation observed since the last scale adjust
    // Requests in flight, keyed by MSP code (FIFO per code → order-preserving match on a reliable stream).
    let mut in_flight: HashMap<u16, VecDeque<Pending>> = HashMap::new();
    let mut radar_last = Instant::now() - RADAR_MSP_INTERVAL;
    let mut rc_raw_last = Instant::now() - rc_tx::RC_RAW_DEFAULT_INTERVAL;
    let mut rc_aux_last = Instant::now() - RC_AUX_INTERVAL;
    // Latest MSP RC OVERRIDE state from the status poll — gates RAW_RC (AUX_RC is independent).
    let mut override_active = false;
    // RC link-speed probe state (first RC_PROBE_WINDOW after RAW streaming starts).
    let mut rc_prev_raw_streaming = false;
    let mut rc_probe_until: Option<Instant> = None;
    let mut rc_probe_sent: u32 = 0;
    let mut rc_probe_slow: u32 = 0;
    let mut rc_probe_timeout: u32 = 0;
    let mut rc_probe_lat = Duration::ZERO;

    // Query MSP_BOXIDS at startup to get the index→permanent_id mapping.
    // INAV's activeModes bitmask uses INDEX-based packing, not permanent box IDs.
    // Without this mapping, we can't correctly decode which modes are active.
    //
    // This attempt is only the fast path — one 2 s request, no retry. Losing it on a lossy air link
    // (mLRS/ELRS right at connect) used to cost the flight modes for the WHOLE session, silently:
    // `parse_active_modes` deliberately reports nothing rather than guessing without the mapping, and
    // `box_active` therefore never sees MSP RC OVERRIDE either. A miss here is no longer terminal —
    // the loop keeps re-asking (see `BOXIDS_RETRY_INTERVAL`) until the FC answers.
    let mut box_ids: Vec<u8> = match transport.msp_request(crate::msp::MSP_BOXIDS, &[]) {
        Ok(msg) => {
            log::info!("MSP_BOXIDS: received {} box IDs", msg.payload.len());
            log::debug!("MSP_BOXIDS payload: {:?}", msg.payload);
            msg.payload
        }
        Err(e) => {
            log::warn!("MSP_BOXIDS query failed ({e}) — retrying from the scheduler loop; flight modes stay blank until it answers");
            Vec::new()
        }
    };
    // Paced from here, so the first retry lands one interval after the failed attempt, not instantly.
    let mut boxids_last_try = Instant::now();

    // Pipelined I/O: tighten the transport read timeout so the drain/emit loop cycles fast (≈100 Hz).
    // (The BOXIDS query above still ran through the blocking `msp_request` handshake path — fine, it's
    // pre-loop and one-shot; from here on the loop owns all reads via `poll_incoming`.)
    transport.set_read_timeout(SCHED_READ_TIMEOUT);

    // Debug tracker for MSP communication stats (no-op in release builds)
    let mut debug_tracker = {
        let polling_codes: Vec<(u16, f64)> = slots.iter()
            .flat_map(|s| {
                // The rotating secondary slot polls ONE of its codes per turn, so each code's real target
                // is the slot rate / code count (e.g. Altitude + Airspeed at 5 Hz → 2.5 Hz each). Every
                // other group polls all its codes each turn, so they get the full slot rate.
                let slot_rate = 1.0 / s.interval.as_secs_f64();
                let per_code_rate = match s.group {
                    TelemetryGroup::PositionSecondary => slot_rate / s.codes.len() as f64,
                    _ => slot_rate,
                };
                s.codes.iter().map(move |&c| (c, per_code_rate))
            })
            .collect();
        debug::DebugTracker::new(
            &polling_codes,
            &[
                crate::msp::MSP_API_VERSION,
                crate::msp::MSP_FC_VARIANT,
                crate::msp::MSP_FC_VERSION,
                crate::msp::MSP_BOARD_INFO,
                crate::msp::MSPV2_INAV_MIXER,
                crate::msp::MSP_BOXIDS,
            ],
        )
    };

    // Always-on link-rate meter (compiled in release too) — feeds the Relay panel's live RX/TX readout,
    // independent of the dev-only Debug Monitor tracker above.
    let mut link_stats = LinkStats::new();

    log::info!(
        "Scheduler started{}: {} telemetry slots (attitude={:.0}Hz, position={:.0}Hz, airspeed={})",
        if tunnel { " (MSP tunnel mode)" } else { "" },
        slots.len(),
        config.attitude_rate_hz,
        config.position_rate_hz,
        if config.airspeed_enabled { "on" } else { "off" },
    );

    // Stall watchdog: warn once at the default log level when the transport stays open but the FC/link
    // stops replying (e.g. a BLE link that quietly stops delivering notifications). See the poll block.
    const STALL_WARN_AFTER: Duration = Duration::from_secs(3);
    let mut last_rx = Instant::now();
    let mut stall_warned = false;

    loop {
        // 0. Bail out if the device is gone (fatal transport error — e.g. USB unplugged). A mere
        //    response timeout (OTA stall) does NOT set this, so we never tear down on a stalled link.
        if transport.is_connection_lost() {
            if tunnel {
                // The MAVLink handler ended (it emits `connection-lost` itself) — just exit, or the
                // frontend would get the event twice.
                log::info!(
                    "Tunnel scheduler: {} gone — exiting. Cause: {}",
                    transport.description(),
                    transport.connection_lost_reason().unwrap_or_else(|| "unknown".to_string()),
                );
                crate::transport::tunnel::stats::set_active(false);
                crate::transport::tunnel::stats::emit_now(&app_handle);
                return Some(transport);
            }
            log::warn!(
                "Scheduler: transport connection lost on {} — tearing down. Cause: {}",
                transport.description(),
                transport
                    .connection_lost_reason()
                    .unwrap_or_else(|| "unknown (no error recorded)".to_string()),
            );
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.shutdown_lost();
                }
            }
            let _ = app_handle.emit("connection-lost", ());
            crate::link_presence::link_down();
            return Some(transport);
        }

        let now = Instant::now();

        // Soft lock: MSP frames written so far in this tick (see `MAX_WRITES_PER_TICK`). Shared by the
        // one-shot, radar and telemetry paths so "one write at a time" holds across all three; the RC
        // stream is deliberately outside it.
        let mut writes_this_tick = 0usize;

        // 1. Commands (non-blocking): stop, or a one-shot UI MSP request → send it and track it in flight
        //    (its reply comes back through the drain, matched by code, and is forwarded on the channel).
        match cmd_rx.try_recv() {
            Ok(SchedulerCommand::Stop) => {
                log::info!("Scheduler stopping");
                if tunnel {
                    crate::transport::tunnel::stats::set_active(false);
                    crate::transport::tunnel::stats::emit_now(&app_handle);
                }
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() {
                        r.shutdown();
                    }
                }
                return Some(transport);
            }
            Ok(SchedulerCommand::MspRequest {
                code,
                payload,
                reply,
                timeout,
            }) if tunnel => {
                oneshot_queue.push_back((code, payload, reply, now, timeout));
            }
            Ok(SchedulerCommand::MspRequest {
                code,
                payload,
                reply,
                timeout,
            }) => {
                debug_tracker.on_request(code, 9 + payload.len());
                link_stats.on_tx(9 + payload.len());
                match transport.msp_send(code, &payload) {
                    Ok(()) => {
                        writes_this_tick += 1;
                        in_flight.entry(code).or_default().push_back(Pending {
                            sent_at: now,
                            deadline: now + timeout.unwrap_or(Duration::from_millis(MSP_ONESHOT_TIMEOUT_MS)),
                            kind: PendingKind::OneShot { reply, retry: None, queued_at: now, retried: false },
                        });
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("Scheduler command channel disconnected");
                return Some(transport);
            }
        }

        // 1b. Tunnel mode: send the next queued one-shot once nothing is in flight. It keeps its payload
        //     for one whole-request retry on timeout (a lost TUNNEL chunk = no reply at all).
        if tunnel && in_flight.values().all(|q| q.is_empty()) && writes_this_tick < MAX_WRITES_PER_TICK {
            // The caller's outer wait has run out → it is gone; sending now would only occupy the slot.
            while oneshot_queue.front().is_some_and(|q| q.3.elapsed() >= OUTER_WAIT_TUNNEL) {
                if let Some((code, _, reply, _, _)) = oneshot_queue.pop_front() {
                    log::debug!("MSP tunnel: 0x{:04X} expired in the queue — dropped unsent", code);
                    crate::transport::tunnel::stats::on_expired();
                    let _ = reply.send(Err(format!("MSP request 0x{:04X} expired in the tunnel queue", code)));
                }
            }
            stale_watch.retain(|_, t| t.elapsed() < STALE_WATCH_KEEP);
            let margin = stale_margin(rtt_ewma);
            let held = oneshot_queue
                .front()
                .and_then(|q| stale_watch.get(&q.0))
                .is_some_and(|t| t.elapsed() < margin);
            if held {
                // FIFO: the head waits out the stale margin of its code; everything behind it waits too.
            } else if let Some((code, payload, reply, queued_at, timeout)) = oneshot_queue.pop_front() {
                debug_tracker.on_request(code, 9 + payload.len());
                link_stats.on_tx(9 + payload.len());
                crate::transport::tunnel::stats::on_request(code);
                match transport.msp_send(code, &payload) {
                    Ok(()) => {
                        writes_this_tick += 1;
                        in_flight.entry(code).or_default().push_back(Pending {
                            sent_at: now,
                            deadline: now + timeout.unwrap_or(Duration::from_millis(MSP_ONESHOT_TIMEOUT_MS)),
                            // No retry for reboot-class codes (the single 2 s wait, then Err) nor for a
                            // probe (its caller polls again itself).
                            kind: PendingKind::OneShot {
                                reply,
                                retry: (timeout.is_none() && tunnel_retryable(code)).then_some(payload),
                                queued_at,
                                retried: false,
                            },
                        });
                    }
                    Err(e) => {
                        crate::transport::tunnel::stats::on_timeout();
                        let _ = reply.send(Err(e));
                    }
                }
            }
        }

        // 1c. RC injection (docs §10 Phase 4c) — HIGHEST priority, fire-and-forget so telemetry waits
        //     behind RC, never the reverse. Two independent gates:
        //       • AUX_RC (CH17–32): streamed on-change whenever ENGAGED — independent of override, so the
        //         GCS can flip the MSP-RC-OVERRIDE switch itself and drive AUX modes.
        //       • RAW_RC (CH1–16): streamed only while ENGAGED **and** MSP-RC-OVERRIDE is active (the FC
        //         ignores RAW_RC otherwise) — this is the takeover of the first 16 channels.
        //     Both require a fresh frontend heartbeat (deadman).
        let rc_now = Instant::now();
        let (rc_enabled, rc_interval, rc_action) = match rc_tx.lock() {
            // Tunnel mode: no RC stream (it would compete for the single in-flight slot).
            _ if tunnel => (false, rc_tx::RC_RAW_DEFAULT_INTERVAL, None),
            Ok(s) => {
                let alive = s.enabled && rc_now.duration_since(s.last_update) < RC_DEADMAN;
                let raw_due = alive
                    && override_active
                    && !s.raw.is_empty()
                    && rc_now.duration_since(rc_raw_last) >= s.raw_interval;
                let aux_due = alive
                    && !s.aux_pending.is_empty()
                    && rc_now.duration_since(rc_aux_last) >= RC_AUX_INTERVAL;
                let action = if raw_due {
                    Some((crate::msp::MSP_SET_RAW_RC, s.raw.clone()))
                } else if aux_due {
                    rc_tx::aux_payload(&s.aux_pending).map(|p| (crate::msp::MSP2_INAV_SET_AUX_RC, p))
                } else {
                    None
                };
                (alive, s.raw_interval, action)
            }
            Err(_) => (false, rc_tx::RC_RAW_DEFAULT_INTERVAL, None),
        };
        let rc_raw_streaming = rc_enabled && override_active;

        // Link-speed probe window: open when RAW starts (override engaged), abort when it stops.
        if rc_raw_streaming && !rc_prev_raw_streaming {
            rc_probe_until = Some(rc_now + RC_PROBE_WINDOW);
            rc_probe_sent = 0;
            rc_probe_slow = 0;
            rc_probe_timeout = 0;
            rc_probe_lat = Duration::ZERO;
        } else if !rc_raw_streaming && rc_prev_raw_streaming {
            rc_probe_until = None;
        }
        rc_prev_raw_streaming = rc_raw_streaming;
        if let Some(until) = rc_probe_until {
            if rc_now >= until {
                rc_probe_until = None;
                if rc_probe_sent > 0 {
                    let bad = rc_probe_slow + rc_probe_timeout;
                    let bad_pct = (bad as f64 / rc_probe_sent as f64) * 100.0;
                    let avg_ms = rc_probe_lat.as_millis() as f64 / rc_probe_sent as f64;
                    eprintln!(
                        "[RC] link probe: sent={rc_probe_sent} slow={rc_probe_slow} timeout={rc_probe_timeout} bad={bad_pct:.0}% avg={avg_ms:.0}ms"
                    );
                    if bad_pct > 30.0 {
                        let _ = app_handle.emit("rc-link-slow", RcLinkWarning {
                            bad_pct: bad_pct.round() as u32,
                            avg_latency_ms: avg_ms.round() as u32,
                            raw_rate_hz: (1000 / rc_interval.as_millis().max(1)) as u32,
                        });
                    }
                }
            }
        }

        if let Some((code, payload)) = rc_action {
            if code == crate::msp::MSP_SET_RAW_RC {
                rc_raw_last = rc_now;
                debug_tracker.on_request(code, 9 + payload.len());
                link_stats.on_tx(9 + payload.len());
                if rc_probe_until.is_some() {
                    // Probe: send WITH reply and time the ACK via the in-flight map (cap-exempt, FIFO per
                    // code). `rc_probe_sent` counts here; the reply/timeout is tallied in the drain/expiry.
                    if transport.msp_send(code, &payload).is_ok() {
                        in_flight.entry(code).or_default().push_back(Pending {
                            sent_at: rc_now,
                            deadline: rc_now + Duration::from_millis(RC_PROBE_TIMEOUT_MS),
                            kind: PendingKind::RcProbe,
                        });
                        rc_probe_sent += 1;
                    }
                } else {
                    // flag=1 → INAV sends no reply for SET_RAW_RC (zero downlink for the stream).
                    let _ = transport.msp_send_no_reply(code, &payload);
                }
            } else {
                rc_aux_last = rc_now;
                debug_tracker.on_request(code, 9 + payload.len());
                link_stats.on_tx(9 + payload.len());
                let _ = transport.msp_send(code, &payload); // fire-and-forget; ACK matched in the drain
            }
        }

        // 3. Radar ADS-B via MSP (opt-in, low cadence) → send + track. Counts against the in-flight cap.
        let tele_in_flight = in_flight
            .values()
            .flatten()
            .filter(|p| !matches!(p.kind, PendingKind::RcProbe))
            .count();
        if !tunnel
            && radar_msp_enabled.load(Ordering::Relaxed)
            && radar_last.elapsed() >= RADAR_MSP_INTERVAL
            && tele_in_flight < max_in_flight
            && writes_this_tick < MAX_WRITES_PER_TICK
            && !in_flight
                .get(&crate::msp::MSP2_ADSB_VEHICLE_LIST)
                .map_or(false, |q| !q.is_empty())
        {
            radar_last = now;
            let code = crate::msp::MSP2_ADSB_VEHICLE_LIST;
            debug_tracker.mark_polling(code, 1.0 / RADAR_MSP_INTERVAL.as_secs_f64());
            debug_tracker.on_request(code, 9);
            link_stats.on_tx(9);
            if transport.msp_send(code, &[]).is_ok() {
                writes_this_tick += 1;
                in_flight.entry(code).or_default().push_back(Pending {
                    sent_at: now,
                    deadline: reply_deadline(now, rtt_ewma),
                    kind: PendingKind::Radar,
                });
            }
        }

        // 3b. Re-ask for MSP_BOXIDS while we still have no mapping. Cheap (one 9-byte frame per
        //     interval), stops the moment a usable reply lands, and shares the same in-flight window
        //     and write budget as everything else, so it can never crowd out telemetry.
        // Tunnel mode: no retry — nothing decodes status flags there (no polling), and a lapsed retry
        // would hold the single in-flight slot against the user's requests.
        if !tunnel
            && box_ids.is_empty()
            && boxids_last_try.elapsed() >= BOXIDS_RETRY_INTERVAL
            && tele_in_flight < max_in_flight
            && writes_this_tick < MAX_WRITES_PER_TICK
            && !in_flight
                .get(&crate::msp::MSP_BOXIDS)
                .map_or(false, |q| !q.is_empty())
        {
            boxids_last_try = now;
            let code = crate::msp::MSP_BOXIDS;
            debug_tracker.on_request(code, 9);
            link_stats.on_tx(9);
            if transport.msp_send(code, &[]).is_ok() {
                writes_this_tick += 1;
                in_flight.entry(code).or_default().push_back(Pending {
                    sent_at: now,
                    deadline: reply_deadline(now, rtt_ewma),
                    kind: PendingKind::BoxIds,
                });
            }
        }

        // 4. Adjust the adaptive scale (AIMD) once per window from the saturation observed since the last
        //    adjust: multiplicative back-off when the link was saturated, gentle additive recovery when it
        //    had headroom. `scale` then stretches every due slot's interval toward its floor (section 5).
        if last_scale_adjust.elapsed() >= SCALE_ADJUST_INTERVAL {
            let prev = scale;
            scale = adjust_scale(scale, saturated_window);
            if (scale - prev).abs() > f64::EPSILON && scale < 0.999 {
                log::debug!(
                    "Adaptive poll scale {:.2} ({}; RTT ~{:.0}ms)",
                    scale,
                    if saturated_window { "saturated → easing toward floors" } else { "headroom → recovering" },
                    rtt_ewma.map(|r| r * 1000.0).unwrap_or(0.0),
                );
            }
            saturated_window = false;
            last_scale_adjust = Instant::now();
        }

        // 5. Emit due telemetry slots, highest priority first, filling the pipe up to the cap (dedup per
        //    code). If the window is full while slots are still due, that's the saturation signal for the
        //    AIMD throttle (section 4) — it eases rates down next window so the pain spreads proportionally
        //    instead of the low-priority slots starving at the cap.
        // Two independent limits: how many replies may be outstanding (`MAX_IN_FLIGHT`, the pipeline
        // depth that keeps slow air links busy) and how many frames may be written before the next drain
        // (`MAX_WRITES_PER_TICK`, the soft lock). The tighter of the two wins.
        let mut budget = max_in_flight
            .saturating_sub(tele_in_flight)
            .min(MAX_WRITES_PER_TICK.saturating_sub(writes_this_tick));
        let mut due: Vec<usize> = (0..slots.len())
            .filter(|&i| slots[i].overdue(scale).is_some())
            .collect();
        if !due.is_empty() {
            // Saturation = *falling behind*, not a momentarily full window: a due slot is flagged only when
            // it's overdue by more than a whole extra interval (it has missed a full cycle). A normal burst
            // — several slots coming due together and briefly filling the window — drains within a tick or
            // two and never reaches this, so it doesn't throttle. Only a link that genuinely can't keep up
            // lets a slot fall a full cycle behind.
            if due.iter().any(|&i| {
                slots[i]
                    .overdue(scale)
                    .map_or(false, |od| od > slots[i].effective_interval(scale))
            }) {
                saturated_window = true;
            }
            due.sort_by(|&a, &b| {
                slots[b]
                    .priority
                    .cmp(&slots[a].priority)
                    .then_with(|| slots[b].overdue(scale).cmp(&slots[a].overdue(scale)))
            });
            for i in due {
                if budget == 0 {
                    break;
                }
                // Rotating secondaries poll one code per turn; every other slot works through all of
                // its codes — but across ticks, resuming at `code_cursor`. MAX_WRITES_PER_TICK is 1, so
                // a slot that restarted its list every turn could only ever emit codes[0]: everything
                // behind it was starved for the whole session. That is what silently killed
                // MSP_SENSOR_STATUS and MSP_NAV_STATUS behind MSPV2_INAV_STATUS in the Status slot,
                // and with NAV_STATUS the active-waypoint highlight. The slot counts as polled (and
                // its interval restarts) only once the cursor has been all the way round.
                let rotating = matches!(slots[i].group, TelemetryGroup::PositionSecondary);
                let codes: Vec<u16> = if rotating {
                    let code = slots[i].codes[slots[i].rotation_index % slots[i].codes.len()];
                    slots[i].rotation_index = slots[i].rotation_index.wrapping_add(1);
                    vec![code]
                } else {
                    slots[i].codes[slots[i].code_cursor..].to_vec()
                };
                let mut emitted = false;
                let mut advanced = 0usize;
                for code in codes {
                    if budget == 0 {
                        break;
                    }
                    advanced += 1;
                    if in_flight.get(&code).map_or(false, |q| !q.is_empty()) {
                        continue; // dedup: this code is already outstanding
                    }
                    debug_tracker.on_request(code, 9);
                    link_stats.on_tx(9);
                    if transport.msp_send(code, &[]).is_ok() {
                        // Kept accurate even though this is the tick's last write path — a future send
                        // added after this block must see a truthful count, not a stale one.
                        #[allow(unused_assignments)]
                        {
                            writes_this_tick += 1;
                        }
                        in_flight.entry(code).or_default().push_back(Pending {
                            sent_at: now,
                            deadline: reply_deadline(now, rtt_ewma),
                            kind: PendingKind::Telemetry,
                        });
                        budget -= 1;
                        emitted = true;
                    }
                }
                if rotating {
                    if emitted {
                        slots[i].last_poll = Some(now);
                    }
                } else {
                    slots[i].code_cursor += advanced;
                    if slots[i].code_cursor >= slots[i].codes.len() {
                        slots[i].code_cursor = 0;
                        slots[i].last_poll = Some(now);
                    }
                }
            }
        }

        // 6. Drain incoming frames and match each to its request by MSP code.
        match transport.poll_incoming() {
            Ok(frames) => {
                for msg in frames {
                    last_rx = Instant::now();
                    if stall_warned && !tunnel {
                        log::warn!("Link recovered — telemetry resumed");
                        stall_warned = false;
                        // Tell the status icon the link is live again (no teardown happened).
                        let _ = app_handle.emit("telemetry-fc-link", FcLinkAlive { alive: true });
                    }
                    let matched = in_flight.get_mut(&msg.code).and_then(|q| q.pop_front());
                    match matched {
                        Some(p) => {
                            let elapsed = p.sent_at.elapsed();
                            let rtt = elapsed.as_secs_f64();
                            rtt_ewma = Some(match rtt_ewma {
                                Some(prev) => prev * 0.8 + rtt * 0.2,
                                None => rtt,
                            });
                            debug_tracker.on_response(msg.code, 9 + msg.payload.len());
                            link_stats.on_rx(9 + msg.payload.len());
                            if tunnel {
                                crate::transport::tunnel::stats::on_reply(9 + msg.payload.len());
                            }
                            match p.kind {
                                PendingKind::Telemetry => dispatch_telemetry(
                                    msg.code,
                                    &msg.payload,
                                    &box_ids,
                                    &recorder,
                                    &app_handle,
                                    config.link_stats_enabled,
                                    &mut override_active,
                                ),
                                PendingKind::OneShot { reply, retried, .. } => {
                                    if tunnel && retried {
                                        // Both attempts may answer; the second answer is stale.
                                        stale_watch.insert(msg.code, Instant::now());
                                    }
                                    let _ = reply.send(Ok(msg.payload));
                                }
                                PendingKind::Radar => {
                                    dispatch_radar(&msg.payload, &radar_ingest, &app_handle)
                                }
                                PendingKind::RcProbe => {
                                    rc_probe_lat += elapsed;
                                    if elapsed > rc_interval {
                                        rc_probe_slow += 1;
                                    }
                                }
                                PendingKind::BoxIds => {
                                    // An empty payload is no mapping — keep asking rather than
                                    // locking in a state that decodes every mode as absent.
                                    if msg.payload.is_empty() {
                                        log::warn!("MSP_BOXIDS answered with an empty payload — still retrying");
                                    } else {
                                        log::warn!(
                                            "MSP_BOXIDS recovered on retry: {} box IDs — flight modes are live",
                                            msg.payload.len()
                                        );
                                        log::debug!("MSP_BOXIDS payload: {:?}", msg.payload);
                                        box_ids = msg.payload;
                                    }
                                }
                            }
                        }
                        None => {
                            if tunnel && stale_watch.contains_key(&msg.code) {
                                log::debug!("MSP tunnel: late reply for 0x{:04X} dropped as stale", msg.code);
                                crate::transport::tunnel::stats::on_stale_reply();
                            }
                            // Unsolicited — e.g. the SET_AUX_RC ACK INAV echoes on a successful weave.
                            if msg.code == crate::msp::MSP2_INAV_SET_AUX_RC {
                                if let Ok(mut s) = rc_tx.lock() {
                                    s.aux_pending.clear();
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // A fatal error already flagged `connection_lost` (torn down at the loop top) — just note.
                log::debug!("poll_incoming: {}", e);
            }
        }

        // 7. Expire timed-out requests: free the window slot; fail one-shots; telemetry re-polls next cycle.
        let expiry_now = Instant::now();
        let mut retries: Vec<QueuedOneShot> = Vec::new();
        for (&code, q) in in_flight.iter_mut() {
            while q.front().map_or(false, |p| expiry_now >= p.deadline) {
                let p = q.pop_front().unwrap();
                match p.kind {
                    // NB: a lapsed telemetry reply is NOT treated as saturation — a lossy link (mLRS)
                    // drops the odd frame even with plenty of headroom, so counting it would ratchet the
                    // throttle down forever. Only genuine window exhaustion (section 5) signals saturation.
                    PendingKind::Telemetry => debug_tracker.on_timeout(code),
                    PendingKind::OneShot { reply, retry, queued_at, .. } => {
                        if tunnel {
                            crate::transport::tunnel::stats::on_timeout();
                        }
                        match retry {
                            // Retry only while the caller is still waiting for it.
                            Some(payload) if queued_at.elapsed() < OUTER_WAIT_TUNNEL => {
                                retries.push((code, payload, reply, queued_at, None))
                            }
                            _ => {
                                if tunnel {
                                    // A lapsed attempt may still answer late — watch the code.
                                    stale_watch.insert(code, expiry_now);
                                }
                                let _ = reply.send(Err(format!(
                                    "MSP response timeout for command 0x{:04X}",
                                    code
                                )));
                            }
                        }
                    }
                    PendingKind::Radar => {
                        debug_tracker.on_timeout(code);
                        let _ = app_handle.emit(
                            radar::ADSB_STATUS_EVENT,
                            &[radar::AdsbStatus { name: "UAV (MSP)".into(), count: 0, ok: false }],
                        );
                    }
                    PendingKind::RcProbe => {
                        rc_probe_timeout += 1;
                        rc_probe_lat += Duration::from_millis(RC_PROBE_TIMEOUT_MS);
                    }
                    // Lapsed retry — the next interval simply asks again.
                    PendingKind::BoxIds => debug_tracker.on_timeout(code),
                }
            }
        }
        in_flight.retain(|_, q| !q.is_empty());
        // Whole-request retry (tunnel mode): the first attempt lapsed, re-send once, then fail for good.
        for (code, payload, reply, queued_at, _) in retries {
            log::debug!("MSP tunnel: 0x{:04X} timed out — retrying once", code);
            debug_tracker.on_request(code, 9 + payload.len());
            link_stats.on_tx(9 + payload.len());
            crate::transport::tunnel::stats::on_retry(code);
            let sent = Instant::now();
            match transport.msp_send(code, &payload) {
                Ok(()) => {
                    in_flight.entry(code).or_default().push_back(Pending {
                        sent_at: sent,
                        deadline: sent + Duration::from_millis(MSP_ONESHOT_TIMEOUT_MS),
                        kind: PendingKind::OneShot { reply, retry: None, queued_at, retried: true },
                    });
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
        }

        // 8. Stall watchdog: warn once (at the default log level) when the transport stays open but the
        //    FC/link stops replying, and tell the status icon (without a teardown — the BLE transport may
        //    be silently reconnecting underneath). Recovery is handled in the drain above.
        if !tunnel && !stall_warned && last_rx.elapsed() >= STALL_WARN_AFTER {
            log::warn!(
                "Link stalled — no MSP reply for {:.0}s (transport still open; FC/link not responding)",
                last_rx.elapsed().as_secs_f32()
            );
            stall_warned = true;
            let _ = app_handle.emit("telemetry-fc-link", FcLinkAlive { alive: false });
        }

        debug_tracker.maybe_emit(&app_handle);
        if tunnel {
            // `link-stats` belongs to the MAVLink handler on this link (it already counts every TUNNEL
            // frame); a second emitter would make the Relay panel's readout flicker between the two.
            crate::transport::tunnel::stats::set_checksum_failures(transport.checksum_errors());
            crate::transport::tunnel::stats::maybe_emit(&app_handle);
        } else {
            link_stats.maybe_emit(&app_handle);
        }
    }
}

/// Build telemetry slots based on config
///
/// Priority levels (higher = polled first when bandwidth is limited):
///   5 = Attitude (most important for real-time display)
///   4 = Status (arming/safety critical)
///   3 = Analog (battery monitoring)
///   2 = PositionPrimary / GPS
///   1 = PositionSecondary (ALT, optional Airspeed)
fn build_slots(config: &TelemetryConfig) -> Vec<TelemetrySlot> {
    use crate::msp::*;

    let mut secondary_codes = vec![MSP_ALTITUDE];
    if config.airspeed_enabled {
        secondary_codes.push(MSPV2_INAV_AIR_SPEED);
    }
    // Adaptive-degradation floors (Hz): the minimum rate each group keeps under a congested link.
    // Attitude and position stay usable (1 Hz) for the horizon + map; everything else floors at 0.5 Hz.
    const FLOOR_ATTITUDE: f64 = 1.0;
    const FLOOR_POSITION: f64 = 1.0;
    const FLOOR_LOW: f64 = 0.5;

    let mut slots = vec![
        TelemetrySlot::new(
            TelemetryGroup::Attitude,
            vec![MSP_ATTITUDE],
            config.attitude_rate_hz,
            5,
            FLOOR_ATTITUDE,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Analog,
            vec![MSPV2_INAV_ANALOG],
            1.0,
            3,
            FLOOR_LOW,
        ),
        // Position: RAW_GPS at the full position rate (fix/lat/lon/alt/speed — needs to be fresh). GPS
        // *statistics* (HDOP, packet counts) change slowly, so they get their own 1 Hz slot below instead
        // of riding at the position rate and burning bandwidth.
        TelemetrySlot::new(
            TelemetryGroup::PositionPrimary,
            vec![MSP_RAW_GPS],
            config.position_rate_hz,
            2,
            FLOOR_POSITION,
        ),
        TelemetrySlot::new(
            TelemetryGroup::GpsStats,
            vec![MSP_GPSSTATISTICS],
            1.0,
            1,
            FLOOR_LOW,
        ),
        TelemetrySlot::new(
            TelemetryGroup::PositionSecondary,
            secondary_codes,
            config.position_rate_hz,
            1,
            FLOOR_POSITION,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Status,
            vec![MSPV2_INAV_STATUS, MSP_SENSOR_STATUS, MSP_NAV_STATUS],
            1.0,
            4,
            FLOOR_LOW,
        ),
        // Throttle + timers (MSP2_INAV_MISC2): own slot at 2 Hz — small message, low priority so it
        // degrades first under load. Drives the Speed widget throttle bar.
        TelemetrySlot::new(
            TelemetryGroup::Misc2,
            vec![MSP2_INAV_MISC2],
            2.0,
            1,
            FLOOR_LOW,
        ),
    ];

    // Wind estimate (INAV 10.0+) — opt-in extra poll, already version-gated in TelemetryConfig. Own
    // 0.5 Hz slot: the estimate shifts over minutes, and riding the rotating secondary slot would have
    // cost altitude a third of its rate for a value that barely moves.
    if config.wind_enabled {
        slots.push(TelemetrySlot::new(
            TelemetryGroup::Wind,
            vec![MSP2_INAV_WIND],
            0.5,
            1,
            FLOOR_LOW,
        ));
    }

    // RC link stats (INAV 9.1+) — own 1 Hz slot, low priority (degrades first under load).
    if config.link_stats_enabled {
        slots.push(TelemetrySlot::new(
            TelemetryGroup::LinkStats,
            vec![MSP2_INAV_GET_LINK_STATS],
            1.0,
            1,
            FLOOR_LOW,
        ));
    }

    slots
}

/// Handle one telemetry reply (pipelined): decode → emit the protocol-agnostic event → feed the flight
/// recorder, plus the Status side-effects (flight-mode classification + MSP-RC-override state) and the
/// pre-9.1 RSSI-only link fallback. Called from the scheduler's drain once an in-flight telemetry
/// request's reply arrives; `override_active` is updated from Status frames for the RC gate.
#[allow(clippy::too_many_arguments)] // dispatch helper; args carry the reply's full decode context
fn dispatch_telemetry(
    code: u16,
    payload_bytes: &[u8],
    box_ids: &[u8],
    recorder: &Option<FlightRecorderHandle>,
    app_handle: &AppHandle,
    link_stats_enabled: bool,
    override_active: &mut bool,
) {
    let event_name = telemetry::event_name_for_code(code);
    let payload = telemetry::decode_telemetry(code, payload_bytes, box_ids);

    // Feed to flight recorder if present
    if let Some(ref rec) = recorder {
        if let Ok(mut r) = rec.lock() {
            telemetry::feed_recorder(code, payload_bytes, &mut r, box_ids);
        }
    }

    if let Err(e) = app_handle.emit(&event_name, &payload) {
        log::warn!("Failed to emit {}: {}", event_name, e);
    }
    // The OS-facing link status (notification + track backfill) — independent of the recorder.
    match payload {
        telemetry::TelemetryPayload::Gps(ref d) => crate::link_status::on_gps(d),
        telemetry::TelemetryPayload::Analog(ref a) => crate::link_status::on_analog(a),
        telemetry::TelemetryPayload::Status(ref s) => crate::link_status::on_status(s),
        _ => {}
    }

    // Flight mode: classify the INAV status bitmask into the canonical model and emit the
    // protocol-agnostic event (+ feed the recorder). The raw flags stay in StatusData as a
    // forensic field only. See docs/active/FLIGHT_MODE_UNIFIED.md.
    if let telemetry::TelemetryPayload::Status(ref s) = payload {
        *override_active = s.msp_rc_override;
        let fm = crate::flightmode::classify_inav(s.flight_mode_flags);
        let _ = app_handle.emit("telemetry-flightmode", &fm);
        crate::link_status::on_flightmode(&fm);
        if let Some(ref rec) = recorder {
            if let Ok(mut r) = rec.lock() { r.on_flightmode(&fm); }
        }
    }

    // RC link: INAV pre-9.1 carries only the configured RSSI channel (0–1023) in ANALOG.
    // Surface it as a normalized RSSI-only link for the RC Link widget. When the FC supports
    // MSP2_INAV_GET_LINK_STATS (9.1+) we poll that richer source instead and suppress this one
    // so the two don't clobber each other on the frontend.
    if !link_stats_enabled {
        if let telemetry::TelemetryPayload::Analog(ref a) = payload {
            let ls = telemetry::LinkStatsData::from_rssi_1023(a.rssi);
            let _ = app_handle.emit("telemetry-linkstats", &ls);
        }
    }
}

/// Reply timeout for a pipelined telemetry/radar request: ~3× the measured round trip, clamped. A lapsed
/// request just frees its window slot and gets re-polled next cycle (the FC dropping the odd reply on a
/// congested link is normal). Falls back to a 150 ms RTT guess before the first sample.
fn reply_deadline(now: Instant, rtt_ewma: Option<f64>) -> Instant {
    let rtt = rtt_ewma.unwrap_or(0.15);
    now + Duration::from_secs_f64((rtt * 3.0).clamp(0.3, 2.0))
}

/// Tunnel mode: how long a new request of a code is held back after a retried / lapsed request of the
/// same code completed — ~2× the measured round trip, clamped to [200 ms, 2 s].
fn stale_margin(rtt_ewma: Option<f64>) -> Duration {
    let rtt = Duration::from_secs_f64(rtt_ewma.unwrap_or(0.0).max(0.0));
    (rtt * 2).clamp(STALE_MARGIN_MIN, STALE_MARGIN_MAX)
}

/// Tunnel mode: requests that are never re-sent after a timeout. A lapsed attempt may have executed on
/// the FC with only its reply chunk lost, so a re-send would repeat a non-idempotent effect:
///  • `MSP_SET_REBOOT` (68, INAV's `MSP_REBOOT`; the geozone "Save to FC" ends with it): the FC reboots
///    once the reply has drained, so a re-send 2 s later can reach the freshly booted FC and reboot it
///    a second time.
/// `MSP_EEPROM_WRITE` (the safehome / geozone / craft-name persist step) is deliberately NOT listed —
/// writing the same config twice is idempotent, so its retry is safe.
const TUNNEL_NO_RETRY: &[u16] = &[crate::msp::MSP_SET_REBOOT];

/// Tunnel mode: may a timed-out one-shot of `code` be re-sent once (see `TUNNEL_NO_RETRY`)?
fn tunnel_retryable(code: u16) -> bool {
    !TUNNEL_NO_RETRY.contains(&code)
}

/// One AIMD step for the adaptive poll scale: multiplicative back-off when the link was saturated over the
/// last window, gentle additive recovery toward 1.0 when it had headroom. Clamped to `[SCALE_MIN, 1.0]`.
/// Self-calibrating — no round-trip/bandwidth estimate — so it never throttles while bandwidth is free and,
/// because it scales every slot's interval uniformly, degrades proportionally instead of starving low prio.
fn adjust_scale(scale: f64, saturated: bool) -> f64 {
    if saturated {
        (scale * 0.9).max(SCALE_MIN) // gentle multiplicative back-off (favour using the bandwidth)
    } else {
        (scale + 0.1).min(1.0) // additive recovery
    }
}

/// Handle one ADS-B-via-MSP reply (pipelined): decode the vehicle list and push it into the radar
/// pipeline. Called from the scheduler's drain when an in-flight Radar request's reply arrives.
fn dispatch_radar(payload_bytes: &[u8], radar_ingest: &RadarIngest, app: &AppHandle) {
    let vehicles = radar::sources::adsb_msp::decode(payload_bytes, radar::now_ms());
    let count = vehicles.len();
    if let Ok(guard) = radar_ingest.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(SourceUpdate {
                source: radar::vehicle::VehicleSource::AdsbMsp,
                vehicles,
            });
        }
    }
    let _ = app.emit(
        radar::ADSB_STATUS_EVENT,
        &[radar::AdsbStatus { name: "UAV (MSP)".into(), count, ok: true }],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(rate_hz: f64, floor_hz: f64) -> TelemetrySlot {
        TelemetrySlot::new(TelemetryGroup::Attitude, vec![0], rate_hz, 5, floor_hz)
    }

    #[test]
    fn effective_interval_full_scale_is_base() {
        // scale 1.0 (link keeps up) → the configured interval is used unchanged.
        let s = slot(5.0, 1.0); // 200 ms base
        assert!((s.effective_interval(1.0).as_secs_f64() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn effective_interval_scales_proportionally() {
        // scale 0.5 → interval doubles (5 Hz → 2.5 Hz), still above the 1 Hz floor.
        let s = slot(5.0, 1.0);
        assert!((s.effective_interval(0.5).as_secs_f64() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn effective_interval_clamped_at_attitude_floor() {
        // Extreme congestion would stretch 200 ms → 4 s, but the 1 Hz floor caps it at 1 s.
        let s = slot(5.0, 1.0);
        assert!((s.effective_interval(0.05).as_secs_f64() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn effective_interval_low_prio_floor_is_half_hz() {
        // Low-priority slots floor at 0.5 Hz → never slower than 2 s, even at minimum scale.
        let s = slot(1.0, 0.5);
        assert!((s.effective_interval(0.01).as_secs_f64() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn scale_backs_off_multiplicatively_when_saturated() {
        assert!((adjust_scale(1.0, true) - 0.9).abs() < 1e-9);
        assert!((adjust_scale(0.5, true) - 0.45).abs() < 1e-9);
    }

    #[test]
    fn scale_recovers_additively_with_headroom() {
        assert!((adjust_scale(0.5, false) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn scale_recovery_clamped_to_one() {
        assert_eq!(adjust_scale(0.95, false), 1.0);
    }

    #[test]
    fn scale_backoff_clamped_to_min() {
        // SCALE_MIN * 0.8 < SCALE_MIN → the floor holds.
        assert_eq!(adjust_scale(SCALE_MIN, true), SCALE_MIN);
    }

    #[test]
    fn stale_margin_is_twice_rtt_clamped() {
        assert_eq!(stale_margin(None), STALE_MARGIN_MIN);
        assert_eq!(stale_margin(Some(0.011)), STALE_MARGIN_MIN); // 11 ms link → 200 ms floor
        assert_eq!(stale_margin(Some(0.3)), Duration::from_millis(600));
        assert_eq!(stale_margin(Some(5.0)), STALE_MARGIN_MAX);
    }

    #[test]
    fn tunnel_never_retries_reboot_but_retries_eeprom_write() {
        assert!(!tunnel_retryable(crate::msp::MSP_SET_REBOOT));
        assert!(tunnel_retryable(crate::msp::MSP_EEPROM_WRITE));
        assert!(tunnel_retryable(crate::msp::MSP_WP));
        assert!(tunnel_retryable(crate::msp::MSP2_COMMON_SET_SETTING));
    }

    #[test]
    fn reply_deadline_clamps_low_and_high() {
        let now = Instant::now();
        // Fast link: 3×5 ms = 15 ms, clamped up to the 300 ms floor.
        assert!((reply_deadline(now, Some(0.005)).duration_since(now).as_secs_f64() - 0.3).abs() < 1e-3);
        // Slow link: 3×1 s = 3 s, clamped down to the 2 s ceiling.
        assert!((reply_deadline(now, Some(1.0)).duration_since(now).as_secs_f64() - 2.0).abs() < 1e-3);
    }
}
