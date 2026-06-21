// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Scheduler Module
// Dedicated thread that owns the serial connection and coordinates all MSP traffic.
// Telemetry slots are time-based, commands and bulk transfers interleave between polls.

pub mod rc_tx;
pub mod telemetry;

#[cfg(debug_assertions)]
mod debug;

// In release builds, DebugTracker is a zero-sized no-op struct.
// All tracking calls are inlined away by the compiler.
#[cfg(not(debug_assertions))]
mod debug {
    pub struct DebugTracker;
    impl DebugTracker {
        pub fn new(_polling: &[(u16, f64)], _handshake: &[u16]) -> Self { Self }
        #[inline(always)]
        pub fn mark_polling(&mut self, _: u16, _: f64) {}
        #[inline(always)]
        pub fn on_request(&mut self, _: u16, _: usize) {}
        #[inline(always)]
        pub fn on_response(&mut self, _: u16, _: usize) {}
        #[inline(always)]
        pub fn on_timeout(&mut self, _: u16) {}
        #[inline(always)]
        pub fn maybe_emit(&mut self, _: &tauri::AppHandle) {}
    }
}

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

/// How often to poll ADS-B from the FC via MSP when enabled (bandwidth-heavy; lowest cadence).
const RADAR_MSP_INTERVAL: Duration = Duration::from_millis(1000);
/// Shared ingest channel + on/off flag for the scheduler-fed ADS-B-via-MSP source.
type RadarIngest = Arc<Mutex<Option<mpsc::Sender<SourceUpdate>>>>;

/// RC injection cadences (docs/active/RC_CONTROL.md §10 Phase 4c). RAW rate is dynamic (RcTxState,
/// user-selectable 10–25 Hz); AUX re-send weave is fixed at 5 Hz.
const RC_AUX_INTERVAL: Duration = Duration::from_millis(200);
/// Frontend-heartbeat deadman: no fresh RC frame for this long → stop streaming (FC failsafes).
const RC_DEADMAN: Duration = Duration::from_millis(500);
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
    },
}

/// A telemetry slot that tracks when it was last polled
struct TelemetrySlot {
    group: TelemetryGroup,
    codes: Vec<u16>,
    interval: Duration,
    last_poll: Option<Instant>,
    /// For groups with rotating secondary codes, track current index
    rotation_index: usize,
    /// Scheduling priority (higher = more important, polled first when overloaded)
    priority: u8,
}

impl TelemetrySlot {
    fn new(group: TelemetryGroup, codes: Vec<u16>, rate_hz: f64, priority: u8) -> Self {
        Self {
            group,
            codes,
            interval: Duration::from_secs_f64(1.0 / rate_hz),
            last_poll: None,
            rotation_index: 0,
            priority,
        }
    }

    /// How long this slot is overdue (None if not yet due)
    fn overdue(&self) -> Option<Duration> {
        match self.last_poll {
            None => Some(Duration::from_secs(999)), // never polled = maximally overdue
            Some(last) => {
                let elapsed = last.elapsed();
                if elapsed >= self.interval {
                    Some(elapsed - self.interval)
                } else {
                    None
                }
            }
        }
    }

    /// Time until next poll is due
    fn time_until_due(&self) -> Duration {
        match self.last_poll {
            None => Duration::ZERO,
            Some(last) => {
                let elapsed = last.elapsed();
                if elapsed >= self.interval {
                    Duration::ZERO
                } else {
                    self.interval - elapsed
                }
            }
        }
    }
}

/// Handle returned to the caller to interact with the running scheduler
pub struct SchedulerHandle {
    cmd_tx: mpsc::Sender<SchedulerCommand>,
    thread: Option<thread::JoinHandle<Option<Box<dyn Transport>>>>,
}

impl SchedulerHandle {
    /// Send a one-shot MSP command through the scheduler (blocks until response)
    pub fn msp_request(&self, code: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(SchedulerCommand::MspRequest {
                code,
                payload: payload.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| "Scheduler thread gone".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Scheduler request timeout".to_string())?
    }

    /// Stop the scheduler and return the transport for cleanup
    pub fn stop(mut self) -> Option<Box<dyn Transport>> {
        let _ = self.cmd_tx.send(SchedulerCommand::Stop);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .flatten()
    }
}

/// Start the MSP scheduler on a dedicated thread
pub fn start(
    transport: Box<dyn Transport>,
    config: TelemetryConfig,
    app_handle: AppHandle,
    recorder: Option<FlightRecorderHandle>,
    radar_ingest: RadarIngest,
    radar_msp_enabled: Arc<AtomicBool>,
    rc_tx: rc_tx::RcTxHandle,
) -> SchedulerHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SchedulerCommand>();

    let thread = thread::spawn(move || {
        scheduler_loop(transport, config, app_handle, cmd_rx, recorder, radar_ingest, radar_msp_enabled, rc_tx)
    });

    SchedulerHandle {
        cmd_tx,
        thread: Some(thread),
    }
}

/// Main scheduler loop — runs until Stop command received
fn scheduler_loop(
    mut transport: Box<dyn Transport>,
    config: TelemetryConfig,
    app_handle: AppHandle,
    cmd_rx: mpsc::Receiver<SchedulerCommand>,
    recorder: Option<FlightRecorderHandle>,
    radar_ingest: RadarIngest,
    radar_msp_enabled: Arc<AtomicBool>,
    rc_tx: rc_tx::RcTxHandle,
) -> Option<Box<dyn Transport>> {
    let mut slots = build_slots(&config);
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

    // Query MSP_BOXIDS once at startup to get the index→permanent_id mapping.
    // INAV's activeModes bitmask uses INDEX-based packing, not permanent box IDs.
    // Without this mapping, we can't correctly decode which modes are active.
    let box_ids: Vec<u8> = match transport.msp_request(crate::msp::MSP_BOXIDS, &[]) {
        Ok(msg) => {
            eprintln!("[MSP-BOXIDS] Received {} box IDs: {:?}", msg.payload.len(), msg.payload);
            msg.payload
        }
        Err(e) => {
            log::warn!("Failed to query MSP_BOXIDS: {} — flight mode detection may be inaccurate", e);
            eprintln!("[MSP-BOXIDS] Query FAILED: {} — using empty mapping", e);
            Vec::new()
        }
    };

    // Debug tracker for MSP communication stats (no-op in release builds)
    let mut debug_tracker = {
        let polling_codes: Vec<(u16, f64)> = slots.iter()
            .flat_map(|s| {
                let rate = 1.0 / s.interval.as_secs_f64();
                s.codes.iter().map(move |&c| (c, rate))
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
            ],
        )
    };

    // Always-on link-rate meter (compiled in release too) — feeds the Relay panel's live RX/TX readout,
    // independent of the dev-only Debug Monitor tracker above.
    let mut link_stats = LinkStats::new();

    log::info!(
        "Scheduler started: {} telemetry slots (attitude={:.0}Hz, position={:.0}Hz, airspeed={})",
        slots.len(),
        config.attitude_rate_hz,
        config.position_rate_hz,
        if config.airspeed_enabled { "on" } else { "off" },
    );

    loop {
        // 0. Bail out if the device is gone (fatal transport error — e.g. USB unplugged). A mere
        //    response timeout (OTA stall) does NOT set this, so we never tear down on a stalled link.
        if transport.is_connection_lost() {
            log::warn!("Scheduler: transport connection lost (device gone) — tearing down");
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.shutdown_lost();
                }
            }
            let _ = app_handle.emit("connection-lost", ());
            return Some(transport);
        }

        // 0b. RC AUX_RC ACK (async): the FC echoes 0x2230 on a successful SET_AUX_RC. It surfaces as an
        //     unsolicited reply during a telemetry read — once seen, stop re-sending until the next change.
        for code in transport.take_unsolicited_codes() {
            if code == crate::msp::MSP2_INAV_SET_AUX_RC {
                if let Ok(mut s) = rc_tx.lock() {
                    s.aux_pending.clear();
                }
            }
        }

        // 1. Check for stop/commands (non-blocking)
        match cmd_rx.try_recv() {
            Ok(SchedulerCommand::Stop) => {
                log::info!("Scheduler stopping");
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
            }) => {
                let result = do_msp_request(&mut *transport, code, &payload, &mut debug_tracker, &mut link_stats);
                let _ = reply.send(result);
                debug_tracker.maybe_emit(&app_handle);
                link_stats.maybe_emit(&app_handle);
                continue; // re-check commands before polling
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("Scheduler command channel disconnected");
                return Some(transport);
            }
        }

        // 1b. ADS-B from the FC via MSP (opt-in, lowest cadence) → radar pipeline.
        if radar_msp_enabled.load(Ordering::Relaxed) && radar_last.elapsed() >= RADAR_MSP_INTERVAL {
            radar_last = Instant::now();
            poll_radar_adsb(&mut *transport, &radar_ingest, &app_handle, &mut debug_tracker, &mut link_stats);
            debug_tracker.maybe_emit(&app_handle);
            link_stats.maybe_emit(&app_handle);
            continue;
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
                    // Probe: send WITH reply and measure the ACK round-trip.
                    let t0 = Instant::now();
                    match transport.msp_request_timeout(code, &payload, RC_PROBE_TIMEOUT_MS) {
                        Ok(msg) => {
                            let lat = t0.elapsed();
                            rc_probe_lat += lat;
                            if lat > rc_interval {
                                rc_probe_slow += 1;
                            }
                            link_stats.on_rx(9 + msg.payload.len());
                            debug_tracker.on_response(code, 9 + msg.payload.len());
                        }
                        Err(_) => {
                            rc_probe_timeout += 1;
                            rc_probe_lat += Duration::from_millis(RC_PROBE_TIMEOUT_MS);
                        }
                    }
                    rc_probe_sent += 1;
                } else {
                    // flag=1 → INAV sends no reply for SET_RAW_RC (zero downlink for the stream).
                    let _ = transport.msp_send_no_reply(code, &payload);
                }
            } else {
                rc_aux_last = rc_now;
                debug_tracker.on_request(code, 9 + payload.len());
                link_stats.on_tx(9 + payload.len());
                let _ = transport.msp_send(code, &payload); // fire-and-forget; ACK observed at 0b
            }
            debug_tracker.maybe_emit(&app_handle);
            link_stats.maybe_emit(&app_handle);
            continue;
        }

        // 2. Find most overdue telemetry slot (priority-based adaptive degradation:
        //    when multiple slots are overdue, highest priority wins.
        //    This naturally degrades low-priority groups first — GPS before Attitude.)
        let most_overdue = slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.overdue().map(|d| (i, slot.priority, d)))
            .max_by(|(_, p1, d1), (_, p2, d2)| {
                p1.cmp(p2).then_with(|| d1.cmp(d2))
            })
            .map(|(i, _, _)| i);

        if let Some(idx) = most_overdue {
            if let Some(ov) = poll_slot(&mut *transport, &mut slots[idx], &app_handle, &mut debug_tracker, &mut link_stats, &recorder, &box_ids, config.link_stats_enabled) {
                override_active = ov;
            }
            debug_tracker.maybe_emit(&app_handle);
            link_stats.maybe_emit(&app_handle);
            continue;
        }

        // 3. Nothing overdue — check for commands with short timeout
        let min_wait = slots
            .iter()
            .map(|s| s.time_until_due())
            .min()
            .unwrap_or(Duration::from_millis(100));

        // Wait for a command or until next slot is due. While RAW is streaming (engaged + override),
        // also cap to the next RAW_RC deadline so the stream never sleeps through its slot.
        let mut wait = min_wait.min(Duration::from_millis(50));
        if override_active {
            if let Ok(s) = rc_tx.lock() {
                if s.enabled && Instant::now().duration_since(s.last_update) < RC_DEADMAN {
                    wait = wait.min(s.raw_interval.saturating_sub(Instant::now().duration_since(rc_raw_last)));
                }
            }
        }
        match cmd_rx.recv_timeout(wait) {
            Ok(SchedulerCommand::Stop) => {
                log::info!("Scheduler stopping");
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
            }) => {
                let result = do_msp_request(&mut *transport, code, &payload, &mut debug_tracker, &mut link_stats);
                let _ = reply.send(result);
                debug_tracker.maybe_emit(&app_handle);
                link_stats.maybe_emit(&app_handle);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::warn!("Scheduler command channel disconnected");
                return Some(transport);
            }
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

    let mut slots = vec![
        TelemetrySlot::new(
            TelemetryGroup::Attitude,
            vec![MSP_ATTITUDE],
            config.attitude_rate_hz,
            5,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Analog,
            vec![MSPV2_INAV_ANALOG],
            1.0,
            3,
        ),
        TelemetrySlot::new(
            TelemetryGroup::PositionPrimary,
            vec![MSP_RAW_GPS, MSP_GPSSTATISTICS],
            config.position_rate_hz,
            2,
        ),
        TelemetrySlot::new(
            TelemetryGroup::PositionSecondary,
            secondary_codes,
            config.position_rate_hz,
            1,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Status,
            vec![MSPV2_INAV_STATUS, MSP_SENSOR_STATUS, MSP_NAV_STATUS],
            1.0,
            4,
        ),
    ];

    // RC link stats (INAV 9.1+) — own 1 Hz slot, low priority (degrades first under load).
    if config.link_stats_enabled {
        slots.push(TelemetrySlot::new(
            TelemetryGroup::LinkStats,
            vec![MSP2_INAV_GET_LINK_STATS],
            1.0,
            1,
        ));
    }

    slots
}

/// Poll a single telemetry slot and emit events. Returns `Some(msp_rc_override)` when this poll decoded
/// an INAV status frame — the RC injection gate uses it to decide whether RAW_RC takes effect.
fn poll_slot(
    transport: &mut dyn Transport,
    slot: &mut TelemetrySlot,
    app_handle: &AppHandle,
    tracker: &mut debug::DebugTracker,
    link_stats: &mut LinkStats,
    recorder: &Option<FlightRecorderHandle>,
    box_ids: &[u8],
    link_stats_enabled: bool,
) -> Option<bool> {
    let mut override_flag = None;
    // Mark poll time BEFORE the request — the interval measures request-to-request,
    // not reply-to-request. This prevents reply latency from artificially slowing
    // the poll rate. If the reply takes longer than the interval, the slot becomes
    // immediately overdue and gets polled again on the next loop iteration.
    slot.last_poll = Some(Instant::now());

    let codes_to_poll: Vec<u16> = match slot.group {
        TelemetryGroup::PositionSecondary => {
            // Rotating: pick one secondary code per cycle
            let code = slot.codes[slot.rotation_index % slot.codes.len()];
            slot.rotation_index = slot.rotation_index.wrapping_add(1);
            vec![code]
        }
        _ => slot.codes.clone(),
    };

    for code in codes_to_poll {
        tracker.on_request(code, 9); // MSP V2 request frame = 9 bytes (empty payload)
        link_stats.on_tx(9);
        match transport.msp_request(code, &[]) {
            Ok(msg) => {
                tracker.on_response(code, 9 + msg.payload.len());
                link_stats.on_rx(9 + msg.payload.len());
                let event_name = telemetry::event_name_for_code(code);
                let payload = telemetry::decode_telemetry(code, &msg.payload, box_ids);

                // Feed to flight recorder if present
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() {
                        telemetry::feed_recorder(code, &msg.payload, &mut r, box_ids);
                    }
                }

                if let Err(e) = app_handle.emit(&event_name, &payload) {
                    log::warn!("Failed to emit {}: {}", event_name, e);
                }

                // Flight mode: classify the INAV status bitmask into the canonical model and emit the
                // protocol-agnostic event (+ feed the recorder). The raw flags stay in StatusData as a
                // forensic field only. See docs/active/FLIGHT_MODE_UNIFIED.md.
                if let telemetry::TelemetryPayload::Status(ref s) = payload {
                    override_flag = Some(s.msp_rc_override);
                    let fm = crate::flightmode::classify_inav(s.flight_mode_flags);
                    let _ = app_handle.emit("telemetry-flightmode", &fm);
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
            Err(e) => {
                tracker.on_timeout(code);
                log::debug!("Telemetry poll 0x{:04X} failed: {}", code, e);
            }
        }
    }
    override_flag
}

/// Run a one-shot MSP request, recording it in the debug tracker so all MSP traffic shows up there.
fn do_msp_request(
    transport: &mut dyn Transport,
    code: u16,
    payload: &[u8],
    tracker: &mut debug::DebugTracker,
    link_stats: &mut LinkStats,
) -> Result<Vec<u8>, String> {
    tracker.on_request(code, 9 + payload.len());
    link_stats.on_tx(9 + payload.len());
    let resp = transport.msp_request(code, payload);
    match &resp {
        Ok(msg) => {
            tracker.on_response(code, 9 + msg.payload.len());
            link_stats.on_rx(9 + msg.payload.len());
        }
        Err(_) => tracker.on_timeout(code),
    }
    resp.map(|msg| msg.payload)
}

/// Poll the FC's ADS-B vehicle list (MSP2_ADSB_VEHICLE_LIST) and push it into the radar pipeline.
fn poll_radar_adsb(
    transport: &mut dyn Transport,
    radar_ingest: &RadarIngest,
    app: &AppHandle,
    tracker: &mut debug::DebugTracker,
    link_stats: &mut LinkStats,
) {
    let code = crate::msp::MSP2_ADSB_VEHICLE_LIST;
    tracker.mark_polling(code, 1.0 / RADAR_MSP_INTERVAL.as_secs_f64());
    tracker.on_request(code, 9);
    link_stats.on_tx(9);
    match transport.msp_request(code, &[]) {
        Ok(msg) => {
            tracker.on_response(code, 9 + msg.payload.len());
            link_stats.on_rx(9 + msg.payload.len());
            let vehicles = radar::sources::adsb_msp::decode(&msg.payload, radar::now_ms());
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
        Err(e) => {
            tracker.on_timeout(code);
            log::debug!("Radar ADS-B (MSP) poll failed: {}", e);
            let _ = app.emit(
                radar::ADSB_STATUS_EVENT,
                &[radar::AdsbStatus { name: "UAV (MSP)".into(), count: 0, ok: false }],
            );
        }
    }
}
