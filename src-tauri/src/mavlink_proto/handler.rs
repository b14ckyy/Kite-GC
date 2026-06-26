// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Handler
// Dedicated thread that owns the ByteTransport and handles MAVLink communication.
// Unlike MSP (poll-based), MAVLink is push-based: the FC streams telemetry,
// the GCS sends heartbeats and occasional commands.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{MavMessage, RC_CHANNELS_OVERRIDE_DATA, MANUAL_CONTROL_DATA};
use ::mavlink::{MavHeader, Message};
use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::scheduler::rc_tx::{self, RcTxHandle};
use crate::scheduler::telemetry::{
    AttitudeData, GpsData, AltitudeData, AnalogData, StatusData,
    SensorStatusData, AirspeedData, WindData, Misc2Data,
};
use crate::transport::ByteTransport;

use super::codec::{self, MavSequence};
use super::parser::MavParser;

/// GCS heartbeat interval (1 Hz as per MAVLink spec)
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// Read timeout while RC override is streaming. The loop is read-driven, so the idle read timeout (50 ms)
/// would otherwise quantize the RC send rate (a 25 Hz request landing on a 50 ms grid only managed ~15
/// Hz). 10 ms granularity lets even 25 Hz stream accurately; restored to the idle value on disengage.
const RC_READ_TIMEOUT_FAST: Duration = Duration::from_millis(10);
/// Idle read timeout (matches the transports' construction default) — restored when RC isn't streaming.
const RC_READ_TIMEOUT_IDLE: Duration = Duration::from_millis(50);

/// Commands sent to the handler thread via channel
pub enum MavlinkCommand {
    /// Stop the handler and return the transport
    Stop,
    /// Send a MAVLink message to the FC
    SendMessage {
        msg: MavMessage,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// Register a channel to receive MISSION_* messages during an active operation
    RegisterMissionReceiver(mpsc::Sender<MavMessage>),
    /// Unregister the mission receiver — telemetry dispatch resumes for all messages
    UnregisterMissionReceiver,
    /// Register a channel to receive COMMAND_ACK during an active command operation
    /// (mode switch / arm / takeoff / reposition / …). See `control.rs`.
    RegisterCommandReceiver(mpsc::Sender<MavMessage>),
    /// Unregister the command-ack receiver
    UnregisterCommandReceiver,
    /// Register a channel to receive PARAM_VALUE during an active param read/write (see `params_rt.rs`)
    RegisterParamReceiver(mpsc::Sender<MavMessage>),
    /// Unregister the param receiver
    UnregisterParamReceiver,
}

/// Handle for interacting with the running MAVLink handler
pub struct MavlinkHandle {
    cmd_tx: mpsc::Sender<MavlinkCommand>,
    thread: Option<thread::JoinHandle<Option<Box<dyn ByteTransport>>>>,
    /// System ID of the connected FC (from handshake)
    pub fc_sysid: u8,
    /// FC variant string from the handshake ("ArduPlane"/"ArduCopter"/"PX4"/…). Drives firmware-
    /// specific mission handling — notably the home-slot convention (ArduPilot reserves mission item 0
    /// for home, PX4 does not).
    pub fc_variant: String,
}

impl MavlinkHandle {
    /// Send a MAVLink message through the handler (blocks until sent)
    #[allow(dead_code)]
    pub fn send_message(&self, msg: MavMessage) -> Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(MavlinkCommand::SendMessage { msg, reply: reply_tx })
            .map_err(|_| "Handler thread gone".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Handler send timeout".to_string())?
    }

    /// Clone the command sender — callers can use this to drive mission operations
    /// without holding the AppState mutex for the duration of the operation.
    pub fn cmd_tx_clone(&self) -> mpsc::Sender<MavlinkCommand> {
        self.cmd_tx.clone()
    }

    /// Stop the handler and return the transport for cleanup
    pub fn stop(mut self) -> Option<Box<dyn ByteTransport>> {
        let _ = self.cmd_tx.send(MavlinkCommand::Stop);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .flatten()
    }
}

/// Start the MAVLink handler on a dedicated thread
pub fn start(
    transport: Box<dyn ByteTransport>,
    fc_sysid: u8,
    fc_variant: String,
    app_handle: AppHandle,
    recorder: Option<FlightRecorderHandle>,
    rc_tx: RcTxHandle,
) -> MavlinkHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<MavlinkCommand>();

    let handle_variant = fc_variant.clone();
    let thread = thread::spawn(move || {
        handler_loop(transport, fc_sysid, fc_variant, app_handle, cmd_rx, recorder, rc_tx)
    });

    MavlinkHandle {
        cmd_tx,
        thread: Some(thread),
        fc_sysid,
        fc_variant: handle_variant,
    }
}

/// Main handler loop — runs until Stop command received
fn handler_loop(
    mut transport: Box<dyn ByteTransport>,
    fc_sysid: u8,
    fc_variant: String,
    app_handle: AppHandle,
    cmd_rx: mpsc::Receiver<MavlinkCommand>,
    recorder: Option<FlightRecorderHandle>,
    rc_tx: RcTxHandle,
) -> Option<Box<dyn ByteTransport>> {
    let mut parser = MavParser::new();
    let mut seq = MavSequence::new();
    let mut buf = [0u8; 1024];
    let mut last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL; // Send immediately
    let mut msg_count: u64 = 0;
    let mut debug_tracker = super::debug::MavlinkDebugTracker::new();
    // Always-on link-rate meter (release too) — feeds the Relay panel's live RX/TX readout.
    let mut link_stats = crate::link_stats::LinkStats::new();

    // Accumulated analog state — MAVLink splits battery data across multiple messages
    let mut analog = AnalogState::default();
    let mut fused = FusedPos::default();

    // Active mission operation receiver — when set, MISSION_* messages are forwarded
    // here instead of being dispatched as telemetry events.
    let mut mission_fwd: Option<mpsc::Sender<MavMessage>> = None;

    // Active command operation receiver — when set, COMMAND_ACK is forwarded here so the
    // blocking command helper (control.rs) can match the ACK to the command it sent.
    let mut cmd_fwd: Option<mpsc::Sender<MavMessage>> = None;
    let mut param_fwd: Option<mpsc::Sender<MavMessage>> = None;

    // QuadPlane detection robustness: a QuadPlane reports MAV_TYPE_FIXED_WING, so the only reliable
    // signal is the Q_ENABLE parameter. The single pre-handler PARAM_REQUEST_READ can be lost on a
    // noisy link (then a QuadPlane is mistaken for a plain plane until reconnect), so we re-request it
    // a few times here until the FC answers. Bounded so a plain plane (no/zero Q_ENABLE) stops soon.
    let is_ardupilot = fc_variant.starts_with("Ardu");
    // PX4 takes RC over MANUAL_CONTROL (#69); everything else (ArduPilot) over RC_CHANNELS_OVERRIDE (#70).
    let is_px4 = fc_variant == "PX4";
    let mut quadplane_seen = false;
    let mut param_probes: u8 = 0;
    let mut last_param_probe = Instant::now();
    const MAX_PARAM_PROBES: u8 = 6;
    const PARAM_PROBE_INTERVAL: Duration = Duration::from_millis(1200);

    let gcs_header = codec::gcs_header();

    // RC override streaming (ArduPilot, docs/active/MAVLINK_RC_CONTROL.md §3). We stream
    // RC_CHANNELS_OVERRIDE from the shared RcTxState while engaged + the frontend heartbeat is fresh,
    // at the user-selected rate. `rc_override_last` paces it. On disengage we deliberately do NOT send a
    // release frame (see the streaming block). PX4 (MANUAL_CONTROL) is not handled here yet.
    let mut rc_override_last = Instant::now();
    // Tracks whether the read timeout is currently in fast (streaming) mode, so we only re-set it on a
    // transition rather than every loop.
    let mut rc_fast_read = false;

    log::info!("MAVLink handler started (FC sysid={})", fc_sysid);

    loop {
        // 1. Check for commands (non-blocking)
        match cmd_rx.try_recv() {
            Ok(MavlinkCommand::Stop) => {
                log::info!("MAVLink handler stopping (processed {} messages)", msg_count);
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                return Some(transport);
            }
            Ok(MavlinkCommand::SendMessage { msg, reply }) => {
                let frame = codec::serialize_v2(&gcs_header, &msg, &mut seq);
                debug_tracker.on_tx(msg.message_id(), frame.len());
                link_stats.on_tx(frame.len());
                // Record our outgoing frame to the tlog too (mission upload, commands, …) → a faithful
                // bidirectional .tlog, like Mission Planner / QGC.
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
                }
                let result = transport
                    .write_bytes(&frame)
                    .map_err(|e| format!("MAVLink send failed: {}", e));
                let _ = reply.send(result);
                continue;
            }
            Ok(MavlinkCommand::RegisterMissionReceiver(tx)) => {
                log::debug!("MAVLink mission receiver registered");
                mission_fwd = Some(tx);
                continue;
            }
            Ok(MavlinkCommand::UnregisterMissionReceiver) => {
                log::debug!("MAVLink mission receiver unregistered");
                mission_fwd = None;
                continue;
            }
            Ok(MavlinkCommand::RegisterCommandReceiver(tx)) => {
                log::debug!("MAVLink command receiver registered");
                cmd_fwd = Some(tx);
                continue;
            }
            Ok(MavlinkCommand::UnregisterCommandReceiver) => {
                log::debug!("MAVLink command receiver unregistered");
                cmd_fwd = None;
                continue;
            }
            Ok(MavlinkCommand::RegisterParamReceiver(tx)) => {
                log::debug!("MAVLink param receiver registered");
                param_fwd = Some(tx);
                continue;
            }
            Ok(MavlinkCommand::UnregisterParamReceiver) => {
                log::debug!("MAVLink param receiver unregistered");
                param_fwd = None;
                continue;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("MAVLink handler command channel disconnected");
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                return Some(transport);
            }
        }

        // 2. Send GCS heartbeat at 1 Hz
        if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
            let hb_msg = codec::gcs_heartbeat();
            let frame = codec::serialize_v2(&gcs_header, &hb_msg, &mut seq);
            debug_tracker.on_tx(hb_msg.message_id(), frame.len());
            link_stats.on_tx(frame.len());
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
            }
            if let Err(e) = transport.write_bytes(&frame) {
                log::warn!("Failed to send GCS heartbeat: {}", e);
            }
            last_heartbeat = Instant::now();
        }

        // 2a. RC streaming. ArduPilot → RC_CHANNELS_OVERRIDE (#70); PX4 → MANUAL_CONTROL (#69). Both stream
        // from the shared RcTxState while engaged + the frontend heartbeat is fresh, at the selected rate.
        {
            let now = Instant::now();
            let (alive, msg, interval) = match rc_tx.lock() {
                Ok(s) => {
                    let has_payload = if is_px4 { s.mav_manual.is_some() } else { !s.mav_override_us.is_empty() };
                    let alive = s.enabled && now.duration_since(s.last_update) < rc_tx::RC_DEADMAN && has_payload;
                    let msg = if !alive {
                        None
                    } else if is_px4 {
                        s.mav_manual.map(|m| manual_control_msg(fc_sysid, &m))
                    } else {
                        Some(rc_override_msg(fc_sysid, &rc_tx::override_channels(&s.mav_override_us)))
                    };
                    (alive, msg, s.raw_interval)
                }
                Err(_) => (false, None, rc_tx::RC_RAW_DEFAULT_INTERVAL),
            };

            // Shorten the read timeout while streaming so the send cadence isn't capped by the 50 ms idle
            // read; restore it on disengage. Only toggled on transition.
            if alive != rc_fast_read {
                transport.set_read_timeout(if alive { RC_READ_TIMEOUT_FAST } else { RC_READ_TIMEOUT_IDLE });
                rc_fast_read = alive;
            }

            if alive && now.duration_since(rc_override_last) >= interval {
                if let Some(m) = msg {
                    send_mav_frame(&mut *transport, &gcs_header, &mut seq, &m,
                        &mut debug_tracker, &mut link_stats, &recorder);
                    // Catch-up scheduling: advance by exactly one interval so loop jitter doesn't drag the
                    // long-term rate below target. If we've fallen more than one interval behind (engage,
                    // or a stall), resync to now instead of bursting to catch up.
                    rc_override_last += interval;
                    if now.duration_since(rc_override_last) > interval {
                        rc_override_last = now;
                    }
                }
            }
            // On disengage / deadman we intentionally send NO release frame. With the GCS as the sole RC
            // source (no physical RX) an explicit release fires ArduPilot's RC failsafe instantly; simply
            // stopping the stream lets the FC hold the last override for RC_OVERRIDE_TIME (~3 s, ArduPilot)
            // / COM_RC_LOSS_T (PX4) as a re-engage grace window before its own failsafe takes over.
        }

        // 2b. Re-probe Q_ENABLE until the FC answers (robust QuadPlane detection — see above).
        if is_ardupilot && !quadplane_seen && param_probes < MAX_PARAM_PROBES
            && last_param_probe.elapsed() >= PARAM_PROBE_INTERVAL
        {
            super::params::request_quadplane_flag(&mut *transport, fc_sysid);
            param_probes += 1;
            last_param_probe = Instant::now();
        }

        // 3. Read incoming bytes and parse MAVLink frames
        match transport.read_bytes(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                for frame in parser.parse_bytes(&buf[..n]) {
                    if frame.header.system_id != fc_sysid { continue; }

                    msg_count += 1;
                    debug_tracker.on_rx(frame.message.message_id(), frame.raw_bytes.len());
                    link_stats.on_rx(frame.raw_bytes.len());

                    // Record raw frame to tlog before any forwarding
                    if !frame.raw_bytes.is_empty() {
                        if let Some(ref rec) = recorder {
                            if let Ok(mut r) = rec.lock() {
                                r.write_raw_mavlink_frame(&frame.raw_bytes);
                            }
                        }
                    }

                    // Forward MISSION_* messages to any active mission operation.
                    // This keeps mission protocol messages out of the telemetry stream.
                    if is_mission_message(&frame.message) {
                        if let Some(ref tx) = mission_fwd {
                            if tx.send(frame.message).is_err() {
                                // Receiver dropped — operation likely timed out
                                mission_fwd = None;
                            }
                            continue;
                        }
                    }

                    // Forward COMMAND_ACK to any active command operation so the helper can match it
                    // to the command it sent (mode/arm/takeoff/reposition/…). Otherwise it falls
                    // through to dispatch (which logs it at trace level).
                    if matches!(frame.message, MavMessage::COMMAND_ACK(_)) {
                        if let Some(ref tx) = cmd_fwd {
                            if tx.send(frame.message).is_err() {
                                cmd_fwd = None;
                            }
                            continue;
                        }
                    }

                    // Forward PARAM_VALUE to an active param read/write (fence params). Otherwise it
                    // falls through to dispatch (the handshake EKF-type reader).
                    if matches!(frame.message, MavMessage::PARAM_VALUE(_)) {
                        if let Some(ref tx) = param_fwd {
                            if tx.send(frame.message).is_err() {
                                param_fwd = None;
                            }
                            continue;
                        }
                    }

                    dispatch_message(&frame.header, &frame.message, &fc_variant, &app_handle, &mut analog, &mut fused, &mut quadplane_seen, &recorder);
                }
            }
            Err(crate::transport::TransportError::Timeout) => {}
            Err(crate::transport::TransportError::Disconnected) => {
                log::warn!("MAVLink transport disconnected");
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                let _ = app_handle.emit("mavlink-disconnected", ());
                return Some(transport);
            }
            Err(e) => {
                log::warn!("MAVLink read error: {}", e);
            }
        }

        // 4. Emit debug stats to the Debug Monitor (throttled internally; no-op in release)
        debug_tracker.maybe_emit(&app_handle);
        // Always-on link-rate emit (Relay panel) — throttled internally, compiled in release too.
        link_stats.maybe_emit(&app_handle);
    }
}

/// Build an `RC_CHANNELS_OVERRIDE` message (ArduPilot RC injection). `ch` is the 18-channel positional
/// wire array already carrying the per-band ignore sentinels (see rc_tx::override_channels).
fn rc_override_msg(fc_sysid: u8, ch: &[u16; rc_tx::MAV_OVERRIDE_CHANNELS]) -> MavMessage {
    MavMessage::RC_CHANNELS_OVERRIDE(RC_CHANNELS_OVERRIDE_DATA {
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        chan1_raw: ch[0], chan2_raw: ch[1], chan3_raw: ch[2], chan4_raw: ch[3],
        chan5_raw: ch[4], chan6_raw: ch[5], chan7_raw: ch[6], chan8_raw: ch[7],
        chan9_raw: ch[8], chan10_raw: ch[9], chan11_raw: ch[10], chan12_raw: ch[11],
        chan13_raw: ch[12], chan14_raw: ch[13], chan15_raw: ch[14], chan16_raw: ch[15],
        chan17_raw: ch[16], chan18_raw: ch[17],
    })
}

/// Build a `MANUAL_CONTROL` message (PX4 RC injection). Axes already normalised to [-1000,1000];
/// `s`/`t` (pitch/roll-only extra-DOF axes) are unused. aux1–6 + enabled_extensions carry the optional
/// continuous extension axes.
fn manual_control_msg(fc_sysid: u8, m: &rc_tx::ManualControl) -> MavMessage {
    MavMessage::MANUAL_CONTROL(MANUAL_CONTROL_DATA {
        x: m.x, y: m.y, z: m.z, r: m.r,
        buttons: m.buttons,
        target: fc_sysid,
        buttons2: m.buttons2,
        enabled_extensions: m.ext,
        s: 0, t: 0,
        aux1: m.aux[0], aux2: m.aux[1], aux3: m.aux[2],
        aux4: m.aux[3], aux5: m.aux[4], aux6: m.aux[5],
    })
}

/// Serialize + send one streamed RC message (override / manual), with the same send-path bookkeeping as
/// the command channel (debug stats, link meter, tlog record).
fn send_mav_frame(
    transport: &mut dyn ByteTransport,
    header: &MavHeader,
    seq: &mut MavSequence,
    msg: &MavMessage,
    debug_tracker: &mut super::debug::MavlinkDebugTracker,
    link_stats: &mut crate::link_stats::LinkStats,
    recorder: &Option<FlightRecorderHandle>,
) {
    let frame = codec::serialize_v2(header, msg, seq);
    debug_tracker.on_tx(msg.message_id(), frame.len());
    link_stats.on_tx(frame.len());
    if let Some(ref rec) = recorder {
        if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
    }
    if let Err(e) = transport.write_bytes(&frame) {
        log::warn!("Failed to send streamed RC frame: {}", e);
    }
}

/// Returns true for MAVLink messages that belong to the mission microprotocol.
// We intentionally still recognise the deprecated non-`_INT` MISSION_REQUEST / MISSION_ITEM:
// older/legacy flight controllers may emit them, and routing them is harmless (we author with
// the `_INT` variants). Hence `#[allow(deprecated)]` rather than dropping the legacy arms.
#[allow(deprecated)]
fn is_mission_message(msg: &MavMessage) -> bool {
    matches!(msg,
        MavMessage::MISSION_REQUEST_LIST(_)
        | MavMessage::MISSION_COUNT(_)
        | MavMessage::MISSION_CLEAR_ALL(_)
        | MavMessage::MISSION_ACK(_)
        | MavMessage::MISSION_REQUEST_INT(_)
        | MavMessage::MISSION_REQUEST(_)
        | MavMessage::MISSION_ITEM_INT(_)
        | MavMessage::MISSION_ITEM(_)
    )
}

/// Authoritative FC home, emitted as the protocol-agnostic `home-position` event (same `{lat,lon,alt}`
/// shape the MSP path emits). MAVLink-adapter-local so we don't reach into the MSP / unified telemetry
/// pipeline — the frontend listener turns it into the locked green "H".
#[derive(serde::Serialize)]
struct HomeEvent {
    lat: f64,
    lon: f64,
    alt: f64,
}

/// Accumulated analog/battery state — MAVLink splits data across SYS_STATUS,
/// BATTERY_STATUS, and RC_CHANNELS. We merge and emit a complete AnalogData.
#[derive(Default)]
struct AnalogState {
    voltage: f64,
    current: f64,
    power: f64,
    mah_drawn: u32,
    battery_percentage: u8,
    rssi: u16,
    cell_count: u8,
}

impl AnalogState {
    fn to_analog_data(&self) -> AnalogData {
        AnalogData {
            voltage: self.voltage,
            current: self.current,
            power: self.power,
            mah_drawn: self.mah_drawn,
            battery_percentage: self.battery_percentage,
            rssi: self.rssi,
            cell_count: self.cell_count,
        }
    }
}

/// Shared GPS cache that splits the two overlapping ArduPilot GPS messages into single owners.
///
/// Position: GLOBAL_POSITION_INT carries the **fused** EKF solution; GPS_RAW_INT carries the raw
/// receiver position — noisier and lagging. Both carry lat/lon, so naively emitting both interleaves
/// two offset streams and the track zig-zags (sawtooth). GLOBAL_POSITION_INT owns position; GPS_RAW_INT
/// reuses the cached fused fix instead of its own raw lat/lon.
///
/// Fix type + sat count: only GPS_RAW_INT has them; GLOBAL_POSITION_INT does not. So GPS_RAW_INT owns
/// `fix_type`/`num_sat` and GLOBAL_POSITION_INT reuses the cached values — otherwise the two messages
/// would emit conflicting fix/sats (GLOBAL_POSITION_INT sending 0 sats) and the UI would flash.
#[derive(Default, Clone, Copy)]
struct FusedPos {
    valid: bool,
    lat: f64,
    lon: f64,
    alt_msl: f64,
    ground_speed: f64,
    course: f64,
    fix_type: u8,
    num_sat: u8,
}

/// Dispatch a received MAVLink message to the same Tauri events as the MSP scheduler.
/// This ensures widgets/store work identically regardless of protocol.
fn dispatch_message(header: &MavHeader, message: &MavMessage, fc_variant: &str, app_handle: &AppHandle, analog: &mut AnalogState, fused: &mut FusedPos, quadplane_seen: &mut bool, recorder: &Option<FlightRecorderHandle>) {
    match message {
        // ── HEARTBEAT → telemetry-status + telemetry-flightmode ─────
        MavMessage::HEARTBEAT(hb) => {
            let armed_bit = ::mavlink::ardupilotmega::MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED;
            let armed = hb.base_mode.bits() & armed_bit.bits() != 0;

            // Encode arming_flags compatible with recorder's ARMED_FLAG (bit 2 = 0x04)
            // Recorder checks: (arming_flags & 0x04) != 0 → armed
            let arming_flags: u32 = if armed { 0x04 } else { 0 };
            // ArduPilot encodes the flight mode as a single raw custom_mode value. We keep it in
            // StatusData as a forensic field, and classify it into the canonical model below.
            let flight_mode_flags = hb.custom_mode;

            let data = StatusData {
                arming_flags,
                flight_mode_flags,
                cpu_load: 0, // Not available from HEARTBEAT
                sensor_status: 0,
                msp_rc_override: false,
            };
            let _ = app_handle.emit("telemetry-status", &data);

            // Flight mode (canonical, protocol-agnostic). See docs/active/FLIGHT_MODE_UNIFIED.md.
            let fm = crate::flightmode::classify_mavlink(hb.custom_mode, fc_variant);
            let _ = app_handle.emit("telemetry-flightmode", &fm);

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_status(&data);
                    r.on_flightmode(&fm);
                }
            }
        }

        // ── HOME_POSITION → home-position ───────────────────────────
        // Authoritative FC home from the aircraft's memory (pushed at ~0.2 Hz + on home change), so
        // the map shows a consistent home instead of guessing from the GPS fix at arm.
        MavMessage::HOME_POSITION(home) => {
            let data = HomeEvent {
                lat: home.latitude as f64 / 1e7,
                lon: home.longitude as f64 / 1e7,
                alt: home.altitude as f64 / 1000.0, // mm AMSL → m
            };
            let _ = app_handle.emit("home-position", &data);
        }

        // ── MISSION_CURRENT → telemetry-nav-status (active waypoint) ─
        // The FC's current mission item sequence. Our displayed waypoints are seq 1..N (home slot 0 is
        // dropped), so seq maps directly to the displayed WP number. Reuses the unified nav-status event
        // (same shape MSP emits) so the widget + map highlight work identically.
        MavMessage::MISSION_CURRENT(mc) => {
            let _ = app_handle.emit("telemetry-nav-status", serde_json::json!({
                "active_wp_number": mc.seq,
                "nav_state": 0u8, // ArduPilot has no INAV nav_state; mission detection uses flight mode
            }));
        }

        // ── ATTITUDE → telemetry-attitude ───────────────────────────
        MavMessage::ATTITUDE(att) => {
            let data = AttitudeData {
                roll: att.roll.to_degrees() as f64,
                pitch: -(att.pitch.to_degrees() as f64), // MAVLink: up=positive → INAV: down=positive
                yaw: (att.yaw.to_degrees() as f64).rem_euclid(360.0),
            };
            let _ = app_handle.emit("telemetry-attitude", &data);
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_attitude(&data); }
            }
        }

        // ── GLOBAL_POSITION_INT → telemetry-gps + telemetry-altitude
        MavMessage::GLOBAL_POSITION_INT(gpi) => {
            // Update the shared cache's position (fix_type/num_sat are owned by GPS_RAW_INT — leave them).
            fused.valid = true;
            fused.lat = gpi.lat as f64 / 1e7;
            fused.lon = gpi.lon as f64 / 1e7;
            fused.alt_msl = gpi.alt as f64 / 1000.0; // mm → m
            fused.ground_speed = ((gpi.vx as f64).powi(2) + (gpi.vy as f64).powi(2)).sqrt() / 100.0; // cm/s → m/s
            // Course over ground from the fused horizontal velocity (vx=North, vy=East). NOTE: `gpi.hdg`
            // is the vehicle HEADING (yaw), not COG — heading is sourced from ATTITUDE.yaw, so don't
            // conflate it into `course` here. Below a walking pace COG is just atan2 of velocity noise,
            // so hold the previous value.
            if fused.ground_speed > 0.5 {
                fused.course = (gpi.vy as f64).atan2(gpi.vx as f64).to_degrees().rem_euclid(360.0);
            }

            let gps = GpsData {
                // Fix/sat come from GPS_RAW_INT (cached). Before the first GPS_RAW_INT, GLOBAL_POSITION_INT
                // already implies at least a 3D fix, so fall back to 2 rather than reporting "no fix".
                fix_type: if fused.fix_type > 0 { fused.fix_type } else { 2 },
                num_sat: fused.num_sat,
                lat: fused.lat,
                lon: fused.lon,
                alt_msl: fused.alt_msl,
                ground_speed: fused.ground_speed,
                course: fused.course,
            };
            let _ = app_handle.emit("telemetry-gps", &gps);

            let alt = AltitudeData {
                altitude: gpi.relative_alt as f64 / 1000.0, // mm → m (relative = baro-like)
                vario: gpi.vz as f64 / -100.0,              // cm/s NED down → m/s climb-positive
            };
            let _ = app_handle.emit("telemetry-altitude", &alt);

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_gps(&gps);
                    r.on_altitude(&alt);
                }
            }
        }

        // ── GPS_RAW_INT → telemetry-gps (fix, sats, hdop) ──────────
        MavMessage::GPS_RAW_INT(gps) => {
            let fix_type = match gps.fix_type {
                ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_NO_GPS
                | ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_NO_FIX => 0,
                ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_2D_FIX => 1,
                ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_3D_FIX => 2,
                ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_DGPS
                | ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_RTK_FLOAT
                | ::mavlink::ardupilotmega::GpsFixType::GPS_FIX_TYPE_RTK_FIXED => 3,
                _ => 0,
            };

            // GPS_RAW_INT owns fix type + sat count — cache them so GLOBAL_POSITION_INT reuses the same
            // values instead of emitting a conflicting fix/0-sats (which made the UI flash).
            fused.fix_type = fix_type;
            fused.num_sat = gps.satellites_visible;

            // Position comes from the fused GLOBAL_POSITION_INT solution (cached in `fused`) — emitting
            // GPS_RAW_INT's own raw lat/lon here would interleave two offset position streams and
            // sawtooth the track. Before the first GLOBAL_POSITION_INT (no fused fix yet) fall back to
            // the raw value.
            let data = if fused.valid {
                GpsData {
                    fix_type,
                    num_sat: gps.satellites_visible,
                    lat: fused.lat,
                    lon: fused.lon,
                    alt_msl: fused.alt_msl,
                    ground_speed: fused.ground_speed,
                    course: fused.course,
                }
            } else {
                GpsData {
                    fix_type,
                    num_sat: gps.satellites_visible,
                    lat: gps.lat as f64 / 1e7,
                    lon: gps.lon as f64 / 1e7,
                    alt_msl: gps.alt as f64 / 1000.0, // mm → m
                    ground_speed: gps.vel as f64 / 100.0, // cm/s → m/s
                    course: gps.cog as f64 / 100.0, // cdeg → deg
                }
            };
            let _ = app_handle.emit("telemetry-gps", &data);

            // HDOP — GPS_RAW_INT.eph is HDOP × 100 (u16::MAX = unknown). Reuse the same stats event INAV
            // uses (MSP_GPSSTATISTICS), so the frontend HDOP readout works identically for both protocols.
            if gps.eph != u16::MAX {
                let _ = app_handle.emit("telemetry-gps-stats", serde_json::json!({
                    "hdop": gps.eph as f64 / 100.0,
                }));
            }

            // Per-sensor hardware health is owned by SYS_STATUS (real present/health bitmasks); the
            // GPS fix nuance (amber on <3D) is layered on top frontend-side from this message's fix.

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_gps(&data); }
            }
        }

        // ── SYS_STATUS → telemetry-analog (battery) ────────────────
        MavMessage::SYS_STATUS(sys) => {
            // Estimate cell count from voltage (3.0–4.2V per cell)
            let voltage_v = sys.voltage_battery as f64 / 1000.0; // mV → V
            let cell_count = if voltage_v > 0.5 {
                ((voltage_v / 3.7).round() as u8).max(1)
            } else {
                0
            };

            analog.voltage = voltage_v;
            analog.current = sys.current_battery as f64 / 100.0; // cA → A
            analog.power = analog.voltage * analog.current;
            analog.cell_count = cell_count;
            if sys.battery_remaining >= 0 {
                analog.battery_percentage = sys.battery_remaining as u8;
            }

            let _ = app_handle.emit("telemetry-analog", &analog.to_analog_data());
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_analog(&analog.to_analog_data()); }
            }

            // Per-sensor hardware health from the standard MAVLink sensor bitmasks. We map to the
            // same 0=NONE / 1=OK / 3=UNHEALTHY model MSP_SENSOR_STATUS uses (3-state: "enabled" is
            // intentionally ignored — a not-present sensor is simply hidden in the header bar).
            use ::mavlink::ardupilotmega::MavSysStatusSensor as Sns;
            let present = sys.onboard_control_sensors_present;
            let health = sys.onboard_control_sensors_health;
            let s3 = |bit: Sns| -> u8 {
                if !present.contains(bit) { 0 } else if health.contains(bit) { 1 } else { 3 }
            };
            // Pre-arm readiness: ArduPilot/PX4 publish the prearm-check result via the PREARM_CHECK bit
            // (enabled = checks active, health = all passed) — the same signal QGC/MP use for "Ready to
            // Fly". 0=unknown (bit absent), 1=ready, 2=blocked. Authoritative + 1 Hz, unlike the periodic
            // "PreArm: …" STATUSTEXT (which we keep only for the human-readable tooltip detail).
            let enabled = sys.onboard_control_sensors_enabled;
            let prearm = {
                let bit = Sns::MAV_SYS_STATUS_PREARM_CHECK;
                if !enabled.contains(bit) { 0 } else if health.contains(bit) { 1 } else { 2 }
            };
            let sensor_data = SensorStatusData {
                gyro: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_GYRO),
                acc: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_ACCEL),
                mag: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_MAG),
                baro: s3(Sns::MAV_SYS_STATUS_SENSOR_ABSOLUTE_PRESSURE),
                gps: s3(Sns::MAV_SYS_STATUS_SENSOR_GPS),
                rangefinder: s3(Sns::MAV_SYS_STATUS_SENSOR_LASER_POSITION),
                pitot: s3(Sns::MAV_SYS_STATUS_SENSOR_DIFFERENTIAL_PRESSURE),
                opflow: s3(Sns::MAV_SYS_STATUS_SENSOR_OPTICAL_FLOW),
                prearm,
            };
            let _ = app_handle.emit("telemetry-sensor-status", &sensor_data);
        }

        // ── VFR_HUD → telemetry-airspeed ───────────────────────────
        // Altitude is intentionally NOT emitted here: VFR_HUD.alt is MSL, but the altitude widget +
        // recorder expect the relative (above-home) value. GLOBAL_POSITION_INT owns altitude
        // (relative_alt) and vario (vz); VFR_HUD only contributes airspeed.
        MavMessage::VFR_HUD(hud) => {
            let airspeed = AirspeedData {
                airspeed: hud.airspeed as f64, // m/s
            };
            let _ = app_handle.emit("telemetry-airspeed", &airspeed);

            // Throttle output (0–100%). MAVLink has no live uptime/flight-time in this message — those
            // stay 0 here (the frontend seeds its flight timer elsewhere); only throttle is meaningful.
            let misc2 = Misc2Data {
                throttle_pct: hud.throttle.min(100) as u8,
                auto_throttle: false,
                uptime_s: 0,
                flight_time_s: 0,
            };
            let _ = app_handle.emit("telemetry-misc2", &misc2);

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_airspeed(&airspeed); r.on_misc2(&misc2); }
            }
        }

        // ── WIND → telemetry-wind ──────────────────────────────────
        // ArduPilot's EKF wind estimate. `direction` is the bearing the wind blows FROM (deg);
        // `speed` is horizontal m/s. INAV has no live wind MSP message (blackbox/replay only).
        MavMessage::WIND(w) => {
            let wind = WindData {
                direction_from_deg: w.direction as f64,
                speed_ms: w.speed as f64,
            };
            let _ = app_handle.emit("telemetry-wind", &wind);

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_wind(&wind); }
            }
        }

        // ── RC_CHANNELS → RSSI (merged into analog state) + live channel µs (RC-control seed) ──
        MavMessage::RC_CHANNELS(rc) => {
            // Current RC channel µs, CH1..chancount — the RC-control engage seed reads the last value
            // (the MAVLink equivalent of MSP_RC; same `telemetry-rc-channels` event the seed listens to).
            let all = [
                rc.chan1_raw, rc.chan2_raw, rc.chan3_raw, rc.chan4_raw, rc.chan5_raw, rc.chan6_raw,
                rc.chan7_raw, rc.chan8_raw, rc.chan9_raw, rc.chan10_raw, rc.chan11_raw, rc.chan12_raw,
                rc.chan13_raw, rc.chan14_raw, rc.chan15_raw, rc.chan16_raw, rc.chan17_raw, rc.chan18_raw,
            ];
            let n = (rc.chancount as usize).clamp(1, all.len());
            let channels: Vec<u16> = all[..n].to_vec();
            let _ = app_handle.emit("telemetry-rc-channels", &channels);

            // rssi is 0–254 in RC_CHANNELS (255 = invalid), map to 0–1023 like INAV
            analog.rssi = (rc.rssi as u16) * 4;
            let _ = app_handle.emit("telemetry-analog", &analog.to_analog_data());
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_analog(&analog.to_analog_data()); }
            }
            // RC link: ArduPilot/PX4 expose only the receiver RSSI over MAVLink (no LQ/SNR in a
            // standard field). Surface it as a normalized RSSI-only link for the RC Link widget.
            if rc.rssi != 255 {
                let ls = crate::scheduler::telemetry::LinkStatsData {
                    rssi_percent: Some((rc.rssi as f32) / 254.0 * 100.0),
                    ..Default::default()
                };
                let _ = app_handle.emit("telemetry-linkstats", &ls);
            }
        }

        // ── BATTERY_STATUS → cumulative mAh (merged into analog) ───
        MavMessage::BATTERY_STATUS(bat) => {
            if bat.current_consumed >= 0 {
                analog.mah_drawn = bat.current_consumed as u32;
            }
            if bat.current_battery >= 0 {
                analog.current = bat.current_battery as f64 / 100.0;
                analog.power = analog.voltage * analog.current;
            }
            if bat.battery_remaining >= 0 {
                analog.battery_percentage = bat.battery_remaining as u8;
            }
            let _ = app_handle.emit("telemetry-analog", &analog.to_analog_data());
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_analog(&analog.to_analog_data()); }
            }
        }

        // ── STATUSTEXT → mavlink-statustext ─────────────────────────
        MavMessage::STATUSTEXT(st) => {
            let text: String = st.text.iter().filter(|&&c| c != 0).map(|&c| c as char).collect();
            log::info!(
                "FC STATUSTEXT [sev={:?}]: {}",
                st.severity,
                text
            );
            let _ = app_handle.emit("mavlink-statustext", serde_json::json!({
                "severity": st.severity as u8,
                "text": text,
            }));
        }

        // ── EKF_STATUS_REPORT → telemetry-ekf-status ────────────────
        // Estimator health for the header EKF indicator. Mission-Planner-style thresholds on the
        // worst normalised variance: green < 0.5, amber 0.5–0.8, red ≥ 0.8. Flags escalate: a GPS
        // glitch forces red; an uninitialised filter or no absolute horizontal position forces at
        // least amber (CONST_POS_MODE is a deliberate no-GPS mode, so it does not count as a fault).
        MavMessage::EKF_STATUS_REPORT(ekf) => {
            use ::mavlink::ardupilotmega::EkfStatusFlags as Ekf;
            let flags = ekf.flags;
            let max_var = ekf
                .velocity_variance
                .max(ekf.pos_horiz_variance)
                .max(ekf.pos_vert_variance)
                .max(ekf.compass_variance);
            let mut status: u8 = if max_var >= 0.8 { 3 } else if max_var >= 0.5 { 2 } else { 1 };
            if flags.contains(Ekf::EKF_GPS_GLITCHING) {
                status = status.max(3);
            }
            if flags.contains(Ekf::EKF_UNINITIALIZED)
                || (!flags.contains(Ekf::EKF_POS_HORIZ_ABS) && !flags.contains(Ekf::EKF_CONST_POS_MODE))
            {
                status = status.max(2);
            }
            let _ = app_handle.emit("telemetry-ekf-status", serde_json::json!({
                "status": status,
                "max_variance": max_var,
                "flags": flags.bits(),
            }));
        }

        // ── PARAM_VALUE → telemetry-ekf-type (AHRS_EKF_TYPE only) ───
        // Reply to the one-shot AHRS_EKF_TYPE read issued on connect (see params.rs). 2 = EKF2,
        // 3 = EKF3; anything else is shown as a generic "EKF" label frontend-side.
        MavMessage::PARAM_VALUE(pv) => {
            let end = pv.param_id.iter().position(|&c| c == 0).unwrap_or(pv.param_id.len());
            let name: String = pv.param_id[..end].iter().map(|&c| c as char).collect();
            if name == "AHRS_EKF_TYPE" {
                let ekf_type = pv.param_value as i32;
                eprintln!("[MAVLINK-PARAM] AHRS_EKF_TYPE = {}", ekf_type);
                let _ = app_handle.emit("telemetry-ekf-type", serde_json::json!({
                    "ekf_type": ekf_type,
                }));
            } else if name == "Q_ENABLE" {
                // Q_ENABLE = 1 → QuadPlane (the mission planner upgrades the vehicle class to quadplane;
                // a plain plane reports FIXED_WING and Q_ENABLE = 0 / no param). See params.rs.
                *quadplane_seen = true; // got the answer → the loop stops re-probing
                let quadplane = pv.param_value as i32 >= 1;
                eprintln!("[MAVLINK-PARAM] Q_ENABLE = {} (quadplane={})", pv.param_value, quadplane);
                let _ = app_handle.emit("telemetry-vehicle", serde_json::json!({
                    "quadplane": quadplane,
                }));
                // Annotate the recorded fc_variant so replay/logbook show "QuadPlane" (it reports
                // MAV_TYPE_FIXED_WING, so the recorded variant would otherwise read plain ArduPlane).
                if quadplane {
                    if let Some(ref rec) = recorder {
                        if let Ok(mut r) = rec.lock() { r.set_quadplane(); }
                    }
                }
            }
        }

        _ => {
            // Log unhandled messages at trace level
            log::trace!("MAVLink msg_id={} from sysid={}", message.message_id(), header.system_id);
        }
    }
}

