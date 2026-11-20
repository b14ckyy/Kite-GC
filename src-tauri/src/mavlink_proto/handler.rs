// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink handler: the dedicated thread owning the ByteTransport (push telemetry, GCS heartbeats, commands).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{MavAutopilot, MavMessage, RC_CHANNELS_OVERRIDE_DATA, MANUAL_CONTROL_DATA};
use ::mavlink::{MavHeader, Message};
use crate::vehicle_registry::emitter::VehicleEmitter;
use crate::vehicle_registry::VehicleInfo;

use crate::flightlog::recorder::{FlightRecorder, FlightRecorderHandle, SessionSlotsHandle};
use crate::flightlog::types::FlightLogSettings;
use crate::msp::FcInfo;
use crate::scheduler::rc_tx::{self, RcTxHandle};
use crate::scheduler::telemetry::{
    AttitudeData, GpsData, AltitudeData, AnalogData, StatusData,
    SensorStatusData, AirspeedData, WindData, Misc2Data, BatteryInstanceData, GpsReceiverData,
};
use crate::transport::ByteTransport;

use super::codec::{self, MavSequence};
use super::parser::{MavParser, Parsed};

/// GCS heartbeat interval (1 Hz as per MAVLink spec)
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// A secondary vehicle (discovered on a shared link) that stays silent this long is dropped from the
/// vehicle list (`vehicle-lost`); it comes back on its next HEARTBEAT. The primary never times out here —
/// its silence is the link's problem and handled by the existing disconnect paths.
const VEHICLE_TIMEOUT: Duration = Duration::from_secs(10);
const VEHICLE_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
/// A vehicle discovered on a shared link is asked for its AUTOPILOT_VERSION (FC hardware id, firmware
/// version) like the handshake asks the primary — at most this often, this many times, from the handler
/// loop (never blocking it). An autopilot that does not answer keeps no hardware id.
const IDENTITY_REQUEST_INTERVAL: Duration = Duration::from_millis(1500);
const MAX_IDENTITY_REQUESTS: u8 = 4;

/// Read timeout while RC override is streaming. The loop is read-driven, so the idle read timeout (50 ms)
/// would otherwise quantize the RC send rate (a 25 Hz request landing on a 50 ms grid only managed ~15
/// Hz). 10 ms granularity lets even 25 Hz stream accurately; restored to the idle value on disengage.
const RC_READ_TIMEOUT_FAST: Duration = Duration::from_millis(10);
/// Idle read timeout (matches the transports' construction default) — restored when RC isn't streaming.
const RC_READ_TIMEOUT_IDLE: Duration = Duration::from_millis(50);

/// Commands sent to the handler thread via channel
// SendMessage carries a MavMessage; boxing would ripple through all send sites in the handler
#[allow(clippy::large_enum_variant)]
pub enum MavlinkCommand {
    /// Stop the handler and return the transport
    Stop,
    /// Send a MAVLink message to the FC
    SendMessage {
        msg: MavMessage,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// Register a channel to receive MISSION_* messages from `sysid` during an active operation.
    /// Only that vehicle's frames are forwarded — on a shared link the others' MISSION_CURRENT etc.
    /// keep flowing to telemetry dispatch.
    RegisterMissionReceiver { sysid: u8, tx: mpsc::Sender<MavMessage> },
    /// Unregister the mission receiver — telemetry dispatch resumes for all messages
    UnregisterMissionReceiver,
    /// Register a channel to receive COMMAND_ACK during an active command operation
    /// (mode switch / arm / takeoff / reposition / …). See `control.rs`.
    RegisterCommandReceiver { sysid: u8, tx: mpsc::Sender<MavMessage> },
    /// Unregister the command-ack receiver
    UnregisterCommandReceiver,
    /// Register a channel to receive PARAM_VALUE during an active param read/write (see `params_rt.rs`)
    RegisterParamReceiver { sysid: u8, tx: mpsc::Sender<MavMessage> },
    /// Unregister the param receiver
    UnregisterParamReceiver,
    /// Re-emit `vehicle-discovered` for every vehicle this link currently knows (primary included) —
    /// the frontend asks after a page reload, when it has lost the announcements it saw live.
    AnnounceVehicles,
    /// Send a hand-encoded message payload (fire-and-forget) — the handler frames it with its own
    /// sequence number and records/counts it like every TX. Used for TUNNEL (#385), which the typed
    /// crate cannot encode with INAV's payload type 0x8001 (see `tunnel.rs`).
    SendRaw {
        msg_id: u32,
        crc_extra: u8,
        payload: Vec<u8>,
    },
    /// Register the MSP-tunnel receiver: payloads of INAV MSP TUNNEL frames addressed to Kite are
    /// forwarded here (see `transport::tunnel::TunnelTransport`).
    RegisterTunnelReceiver(mpsc::Sender<Vec<u8>>),
    /// Unregister the MSP-tunnel receiver
    UnregisterTunnelReceiver,
    /// Start reading the handshake vehicle's own mode list (AVAILABLE_MODES, see `modes.rs`). Sent by
    /// the connect path once the MSP-tunnel probe has ruled out INAV, which reports an ArduPilot identity
    /// in its HEARTBEAT. Ignored for a non-ArduPilot vehicle.
    StartModeList,
    /// Send the handshake vehicle's connect requests (`configure_primary`). The connect path defers them
    /// behind the MSP-tunnel probe: on a slow uplink they would queue in front of it for seconds.
    ConfigurePrimary,
}

/// Handle for interacting with the running MAVLink handler
pub struct MavlinkHandle {
    cmd_tx: mpsc::Sender<MavlinkCommand>,
    thread: Option<thread::JoinHandle<Option<Box<dyn ByteTransport>>>>,
    /// FC variant string from the handshake ("ArduPlane"/"ArduCopter"/"PX4"/…). Drives firmware-
    /// specific mission handling — notably the home-slot convention (ArduPilot reserves mission item 0
    /// for home, PX4 does not).
    pub fc_variant: String,
    /// MSP scheduler (tunnel mode) running over the MAVLink TUNNEL when the FC is INAV ≥ 10.0 and
    /// answered the connect-time probe; `None` on a plain MAVLink link. Stopped before the handler.
    pub msp: Option<crate::scheduler::SchedulerHandle>,
    /// Set once the MSP-over-MAVLink probe identified this FC as INAV (`probe_msp_tunnel`). The handler
    /// decodes the few INAV-specific MAVLink conventions with it (MISSION_CURRENT numbering) — not with
    /// the heartbeat variant, which INAV reports as Generic / ArduPilot.
    pub inav_tunnel: Arc<AtomicBool>,
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

    /// Ask the handler to re-emit `vehicle-discovered` for every vehicle it knows.
    pub fn announce_vehicles(&self) {
        let _ = self.cmd_tx.send(MavlinkCommand::AnnounceVehicles);
    }

    /// Start reading the handshake vehicle's mode list (ArduPilot only — see `MavlinkCommand::StartModeList`).
    pub fn start_mode_list(&self) {
        let _ = self.cmd_tx.send(MavlinkCommand::StartModeList);
    }

    /// Stop the handler and return the transport for cleanup. A tunnel-mode MSP scheduler is stopped
    /// first — its `TunnelTransport` unregisters from the still-running handler on drop.
    pub fn stop(mut self) -> Option<Box<dyn ByteTransport>> {
        if let Some(msp) = self.msp.take() {
            let _ = msp.stop();
            log::info!("MSP tunnel scheduler stopped");
        }
        let _ = self.cmd_tx.send(MavlinkCommand::Stop);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .flatten()
    }
}

/// The GCS-requested stream shaping the connect path applied to the handshake vehicle — replayed for
/// every vehicle discovered later on the same link so they stream the same set at the same rates.
#[derive(Clone, Debug)]
pub struct StreamRateConfig {
    /// "Full MAVLink telemetry": reset our managed messages to the FC's SRn defaults instead of shaping.
    pub full_telemetry: bool,
    pub attitude_hz: f64,
    pub position_hz: f64,
    pub airspeed_enabled: bool,
    pub wind_enabled: bool,
}

/// How to record the flights of vehicles discovered on a shared link (they have no connect-time
/// recorder of their own). DB recording only — the raw `.tlog` is per link and already holds every
/// vehicle's frames. `None` = flight logging is off.
#[derive(Clone)]
pub struct SecondaryRecording {
    pub settings: FlightLogSettings,
    pub portable: bool,
    /// The app-wide per-vehicle recording slots — a secondary's pending and live files are protected
    /// like the primary's.
    pub slots: SessionSlotsHandle,
}

/// Per-vehicle decode state on one link. The primary (handshake) vehicle gets one at start; further
/// vehicles are created on their first autopilot HEARTBEAT (multi-vehicle on a shared link — see
/// docs/02-design/features/multi-vehicle.design.md §4).
struct VehicleCtx {
    compid: u8,
    fc_variant: String,
    platform_type: u8,
    mav_type: u8,
    /// Emits with this vehicle's `vehicleId` stamped on every payload.
    emitter: VehicleEmitter,
    analog: AnalogState,
    batteries: std::collections::BTreeMap<u8, BatteryInstanceData>,
    fused: FusedPos,
    quadplane_seen: bool,
    last_seen: Instant,
    /// Unattended flight recorder (secondary vehicles only; the primary's is the link's recorder).
    recorder: Option<FlightRecorderHandle>,
    /// This vehicle's own identity from its AUTOPILOT_VERSION (secondaries; the primary's comes from the
    /// handshake): FC hardware id and firmware version. `identity_done` once it answered or the request
    /// budget is spent.
    fc_uid: Option<String>,
    fc_version: Option<String>,
    identity_requests: u8,
    last_identity_request: Option<Instant>,
    identity_done: bool,
    /// Reader of this vehicle's own mode list (AVAILABLE_MODES) — ArduPilot vehicles only; `None` for
    /// PX4 / INAV / anything else, and for the primary until the connect path sends `StartModeList`.
    modes: Option<super::modes::ModeEnumerator>,
}

impl VehicleCtx {
    fn new(compid: u8, fc_variant: String, platform_type: u8, mav_type: u8, emitter: VehicleEmitter) -> Self {
        Self {
            compid,
            fc_variant,
            platform_type,
            mav_type,
            emitter,
            analog: AnalogState::default(),
            batteries: std::collections::BTreeMap::new(),
            fused: FusedPos::default(),
            quadplane_seen: false,
            last_seen: Instant::now(),
            recorder: None,
            fc_uid: None,
            fc_version: None,
            identity_requests: 0,
            last_identity_request: None,
            identity_done: false,
            modes: None,
        }
    }

    /// Take this vehicle's AUTOPILOT_VERSION. Returns whether its identity changed (then the caller
    /// re-announces it); the recorder gets it for the running session and every later flight.
    fn apply_identity(&mut self, v: super::handshake::AutopilotVersion) -> bool {
        self.identity_done = true;
        let fc_version = Some(v.fc_version);
        if self.fc_uid == v.fc_uid && self.fc_version == fc_version {
            return false;
        }
        self.fc_uid = v.fc_uid;
        self.fc_version = fc_version;
        if let Some(rec) = &self.recorder {
            if let Ok(mut r) = rec.lock() {
                r.set_fc_identity(self.fc_uid.clone(), self.fc_version.clone());
            }
        }
        true
    }
}

impl VehicleCtx {
    fn info(&self, sysid: u8, primary: bool) -> VehicleInfo {
        VehicleInfo {
            vehicle_id: self.emitter.key().to_string(),
            link_id: self.emitter.link_id(),
            sysid,
            compid: self.compid,
            fc_variant: self.fc_variant.clone(),
            platform_type: self.platform_type,
            mav_type: self.mav_type,
            protocol: "MAVLink".to_string(),
            primary,
            fc_uid: self.fc_uid.clone(),
            fc_version: self.fc_version.clone(),
        }
    }
}

/// The unattended recorder of a vehicle discovered on a shared link: flights under "<variant> #<sysid>",
/// auto-committed until member mode; registered in app-state under its vehicle key.
fn secondary_recorder(cfg: &SecondaryRecording, sysid: u8, fc_variant: &str, platform_type: u8, mav_type: u8, emitter: &VehicleEmitter) -> Option<FlightRecorderHandle> {
    if !cfg.settings.enabled || !cfg.settings.db_enabled {
        return None;
    }
    let settings = FlightLogSettings { raw_enabled: false, raw_always: false, ..cfg.settings.clone() };
    let fc_info = FcInfo {
        fc_variant: fc_variant.to_string(),
        platform_type,
        mav_type,
        craft_name: format!("{} #{}", fc_variant, sysid),
        ..FcInfo::default()
    };
    let none_sink: crate::flightlog::msp_raw_logger::MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
    match FlightRecorder::new(settings, fc_info, "MAVLink", cfg.portable, emitter.clone(), cfg.slots.clone(), none_sink) {
        Ok(mut rec) => {
            rec.set_auto_commit(true);
            log::info!("Flight recorder (unattended) initialized for sysid={}", sysid);
            let handle = std::sync::Arc::new(std::sync::Mutex::new(rec));
            register_recorder(emitter, Some(&handle));
            Some(handle)
        }
        Err(e) => {
            log::error!("Failed to initialize recorder for sysid={}: {}", sysid, e);
            None
        }
    }
}

/// Publish (or drop) a secondary vehicle's recorder in app-state (`AppState::recorders`), so the
/// command layer reaches it when that vehicle is the active one. A new recorder is also registered with
/// the group-flight coordinator (it unregisters itself on teardown).
fn register_recorder(emitter: &VehicleEmitter, recorder: Option<&FlightRecorderHandle>) {
    use tauri::Manager;
    if let Some(state) = emitter.app().try_state::<crate::state::AppState>() {
        state.set_recorder(emitter.key(), recorder);
        if let Some(r) = recorder {
            state.groups.register(r);
        }
    }
}

/// Teardown for every secondary vehicle's recorder (link stop / transport loss).
fn shutdown_secondaries(vehicles: &mut std::collections::BTreeMap<u8, VehicleCtx>) {
    for ctx in vehicles.values_mut() {
        if let Some(rec) = ctx.recorder.take() {
            if let Ok(mut r) = rec.lock() { r.shutdown(); }
        }
    }
}

/// Give a newly discovered vehicle the same treatment the connect path gave the handshake vehicle:
/// stream shaping (or the full-telemetry reset) plus the one-shot EKF-type / home / Q_ENABLE requests.
fn configure_new_vehicle(transport: &mut dyn ByteTransport, sysid: u8, fc_variant: &str, rates: &StreamRateConfig) {
    let is_px4 = fc_variant == "PX4";
    if rates.full_telemetry {
        super::streamrates::reset_stream_rates(transport, sysid, is_px4);
    } else {
        super::streamrates::apply_stream_rates(
            transport, sysid, rates.attitude_hz, rates.position_hz, rates.airspeed_enabled, rates.wind_enabled, is_px4,
        );
    }
    super::params::request_ekf_type(transport, sysid);
    super::params::request_home_position(transport, sysid);
    if fc_variant.starts_with("Ardu") {
        super::params::request_quadplane_flag(transport, sysid);
    }
}

/// The handshake vehicle's connect requests: `configure_new_vehicle` plus the logging-backend parameter
/// behind the vehicle library's "blackbox available" flag (ArduPilot LOG_BACKEND_TYPE, PX4 SDLOG_MODE).
pub fn configure_primary(transport: &mut dyn ByteTransport, sysid: u8, fc_variant: &str, rates: &StreamRateConfig) {
    configure_new_vehicle(transport, sysid, fc_variant, rates);
    if fc_variant.starts_with("Ardu") {
        super::params::request_param(transport, sysid, "LOG_BACKEND_TYPE");
    } else if fc_variant.contains("PX4") {
        super::params::request_param(transport, sysid, "SDLOG_MODE");
    }
}

/// Start the MAVLink handler on a dedicated thread
pub fn start(
    transport: Box<dyn ByteTransport>,
    fc_sysid: u8,
    fc_compid: u8,
    fc_variant: String,
    fc_identity: (u8, u8), // (platform_type, mav_type) of the handshake vehicle
    app_handle: VehicleEmitter,
    recorder: Option<FlightRecorderHandle>,
    rc_tx: RcTxHandle,
    rates: StreamRateConfig,
    secondary_recording: Option<SecondaryRecording>,
) -> MavlinkHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<MavlinkCommand>();

    let handle_variant = fc_variant.clone();
    let inav_tunnel = Arc::new(AtomicBool::new(false));
    let loop_inav_tunnel = inav_tunnel.clone();
    let thread = thread::spawn(move || {
        handler_loop(transport, fc_sysid, fc_compid, fc_variant, fc_identity, app_handle, cmd_rx, recorder, rc_tx, rates, secondary_recording, loop_inav_tunnel)
    });

    MavlinkHandle {
        cmd_tx,
        thread: Some(thread),
        fc_variant: handle_variant,
        msp: None,
        inav_tunnel,
    }
}

/// Main handler loop — runs until Stop command received
#[allow(clippy::too_many_arguments)] // thread entry point; args carry the loop's full context
fn handler_loop(
    mut transport: Box<dyn ByteTransport>,
    fc_sysid: u8,
    fc_compid: u8,
    fc_variant: String,
    fc_identity: (u8, u8),
    app_handle: VehicleEmitter,
    cmd_rx: mpsc::Receiver<MavlinkCommand>,
    recorder: Option<FlightRecorderHandle>,
    rc_tx: RcTxHandle,
    rates: StreamRateConfig,
    secondary_recording: Option<SecondaryRecording>,
    inav_tunnel: Arc<AtomicBool>,
) -> Option<Box<dyn ByteTransport>> {
    let mut parser = MavParser::new();
    let mut seq = MavSequence::new();
    let mut buf = [0u8; 1024];
    let mut last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL; // Send immediately
    // Stall watchdog: the transport may stay open while the vehicle goes quiet — nothing is torn down, the
    // status bar gets `telemetry-fc-link` (as from MSP) and again when frames resume. MSP's threshold.
    const STALL_WARN_AFTER: Duration = Duration::from_secs(3);
    // A silence at least this long may have been an FC reboot → its mode list is read again on recovery.
    const MODE_REREAD_AFTER_SILENCE: Duration = Duration::from_secs(5);
    let mut last_fc_rx = Instant::now();
    let mut stall_warned = false;
    let mut read_errors = crate::transport::ReadErrorThrottle::new();
    let mut msg_count: u64 = 0;
    let mut debug_tracker = super::debug::MavlinkDebugTracker::new();
    // Always-on link-rate meter (release too) — feeds the Relay panel's live RX/TX readout.
    let mut link_stats = crate::link_stats::LinkStats::new();

    // Every vehicle on this link, by system id. The handshake vehicle is the primary; more appear on
    // their first autopilot HEARTBEAT (a telemetry radio / UDP port shared by several aircraft). Each
    // carries its own accumulated analog / battery / position state and its own stamped emitter.
    let mut vehicles: std::collections::BTreeMap<u8, VehicleCtx> = std::collections::BTreeMap::new();
    let mut primary_ctx = VehicleCtx::new(fc_compid, fc_variant.clone(), fc_identity.0, fc_identity.1, app_handle.clone());
    primary_ctx.identity_done = true; // the handshake asked it already
    vehicles.insert(fc_sysid, primary_ctx);
    let mut last_sweep = Instant::now();
    // Decoded telemetry feeds the PRIMARY vehicle's recorder (it also owns the raw .tlog); secondaries get
    // their own unattended recorder, so their arm/disarm never drives the primary's End-Flight dialog.

    // Active mission operation receiver — when set, that vehicle's MISSION_* messages are forwarded
    // here instead of being dispatched as telemetry events.
    let mut mission_fwd: Option<(u8, mpsc::Sender<MavMessage>)> = None;

    // Active command operation receiver — when set, that vehicle's COMMAND_ACK is forwarded here so
    // the blocking command helper (control.rs) can match the ACK to the command it sent.
    let mut cmd_fwd: Option<(u8, mpsc::Sender<MavMessage>)> = None;
    let mut param_fwd: Option<(u8, mpsc::Sender<MavMessage>)> = None;
    // MSP-over-MAVLink tunnel receiver (INAV 10.0+): payloads of TUNNEL frames addressed to Kite.
    let mut tunnel_fwd: Option<mpsc::Sender<Vec<u8>>> = None;
    // When the receiver was released — a reply after that came too late for whoever asked.
    let mut tunnel_released: Option<Instant> = None;
    let mut tunnel_late_warned = false;

    // QuadPlane detection: it reports MAV_TYPE_FIXED_WING, so only Q_ENABLE tells. The pre-handler request
    // can be lost on a noisy link — re-request a few times; bounded, so a plain plane stops soon.
    let is_ardupilot = fc_variant.starts_with("Ardu");
    // PX4 takes RC over MANUAL_CONTROL (#69); everything else (ArduPilot) over RC_CHANNELS_OVERRIDE (#70).
    let is_px4 = fc_variant == "PX4";
    let mut param_probes: u8 = 0;
    let mut last_param_probe = Instant::now();
    const MAX_PARAM_PROBES: u8 = 6;
    const PARAM_PROBE_INTERVAL: Duration = Duration::from_millis(1200);

    let gcs_header = codec::gcs_header();

    // RC override streaming (ArduPilot, docs/active/MAVLINK_RC_CONTROL.md §3) while engaged and the frontend
    // heartbeat is fresh; no release frame on disengage (see below). PX4 (MANUAL_CONTROL) not handled yet.
    let mut rc_override_last = Instant::now();
    // Tracks whether the read timeout is currently in fast (streaming) mode, so we only re-set it on a
    // transition rather than every loop.
    let mut rc_fast_read = false;

    log::info!("MAVLink handler started (FC sysid={} compid={})", fc_sysid, fc_compid);

    loop {
        // 1. Check for commands (non-blocking)
        match cmd_rx.try_recv() {
            Ok(MavlinkCommand::Stop) => {
                log::info!("MAVLink handler stopping (processed {} messages)", msg_count);
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                shutdown_secondaries(&mut vehicles);
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
            Ok(MavlinkCommand::RegisterMissionReceiver { sysid, tx }) => {
                log::debug!("MAVLink mission receiver registered (sysid={})", sysid);
                mission_fwd = Some((sysid, tx));
                continue;
            }
            Ok(MavlinkCommand::UnregisterMissionReceiver) => {
                log::debug!("MAVLink mission receiver unregistered");
                mission_fwd = None;
                continue;
            }
            Ok(MavlinkCommand::RegisterCommandReceiver { sysid, tx }) => {
                log::debug!("MAVLink command receiver registered (sysid={})", sysid);
                cmd_fwd = Some((sysid, tx));
                continue;
            }
            Ok(MavlinkCommand::UnregisterCommandReceiver) => {
                log::debug!("MAVLink command receiver unregistered");
                cmd_fwd = None;
                continue;
            }
            Ok(MavlinkCommand::RegisterParamReceiver { sysid, tx }) => {
                log::debug!("MAVLink param receiver registered (sysid={})", sysid);
                param_fwd = Some((sysid, tx));
                continue;
            }
            Ok(MavlinkCommand::UnregisterParamReceiver) => {
                log::debug!("MAVLink param receiver unregistered");
                param_fwd = None;
                continue;
            }
            Ok(MavlinkCommand::AnnounceVehicles) => {
                // Secondaries only: the primary is announced from the registry entry (`announce_vehicles`
                // command), whose identity reflects the MSP tunnel probe — this ctx still holds the
                // pre-tunnel HEARTBEAT identity and would overwrite it.
                let mut n = 0;
                for (sysid, ctx) in vehicles.iter().filter(|(s, _)| **s != fc_sysid) {
                    let _ = ctx.emitter.emit("vehicle-discovered", ctx.info(*sysid, false));
                    n += 1;
                }
                log::debug!("MAVLink: re-announced {} secondary vehicle(s)", n);
                // The FC mode lists read so far (primary included) — a reloaded frontend lost them too.
                for ctx in vehicles.values() {
                    if let Some(list) = ctx.modes.as_ref().and_then(|e| e.last_complete()) {
                        let _ = ctx.emitter.emit("available-modes", super::modes::AvailableModesEvent { modes: list });
                    }
                }
                continue;
            }
            Ok(MavlinkCommand::StartModeList) => {
                if let Some(ctx) = vehicles.get_mut(&fc_sysid) {
                    if ctx.fc_variant.starts_with("Ardu") && ctx.modes.is_none() {
                        ctx.modes = Some(super::modes::ModeEnumerator::new(fc_sysid, Instant::now()));
                    }
                }
                continue;
            }
            Ok(MavlinkCommand::SendRaw { msg_id, crc_extra, payload }) => {
                let frame = codec::serialize_raw_v2(&gcs_header, msg_id, crc_extra, &payload, &mut seq);
                debug_tracker.on_tx(msg_id, frame.len());
                link_stats.on_tx(frame.len());
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
                }
                if let Err(e) = transport.write_bytes(&frame) {
                    log::warn!("MAVLink raw send failed (msg_id {}): {}", msg_id, e);
                }
                continue;
            }
            Ok(MavlinkCommand::RegisterTunnelReceiver(tx)) => {
                log::debug!("MAVLink tunnel receiver registered");
                tunnel_fwd = Some(tx);
                tunnel_released = None;
                continue;
            }
            Ok(MavlinkCommand::UnregisterTunnelReceiver) => {
                log::debug!("MAVLink tunnel receiver unregistered");
                tunnel_fwd = None;
                tunnel_released = Some(Instant::now());
                continue;
            }
            Ok(MavlinkCommand::ConfigurePrimary) => {
                if let Some(ctx) = vehicles.get(&fc_sysid) {
                    configure_primary(&mut *transport, fc_sysid, &ctx.fc_variant, &rates);
                }
                continue;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("MAVLink handler command channel disconnected");
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                shutdown_secondaries(&mut vehicles);
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
                    // Multi-vehicle: the stream addresses the ACTIVE vehicle. A target on another link
                    // means this handler must stay silent; no target = the handshake vehicle (legacy).
                    let (tgt_sysid, tgt_px4, ours) = match s.mav_target {
                        Some(t) => (t.sysid, t.px4, t.link == app_handle.link_id()),
                        None => (fc_sysid, is_px4, true),
                    };
                    let has_payload = if tgt_px4 { s.mav_manual.is_some() } else { !s.mav_override_us.is_empty() };
                    let alive = ours && s.enabled && now.duration_since(s.last_update) < rc_tx::RC_DEADMAN && has_payload;
                    let msg = if !alive {
                        None
                    } else if tgt_px4 {
                        s.mav_manual.map(|m| manual_control_msg(tgt_sysid, &m))
                    } else {
                        Some(rc_override_msg(tgt_sysid, &rc_tx::override_channels(&s.mav_override_us)))
                    };
                    (alive, msg, s.raw_interval)
                }
                Err(_) => (false, None, rc_tx::RC_RAW_DEFAULT_INTERVAL),
            };

            // Shorter read timeout while streaming, so the 50 ms idle read does not cap the send cadence. Logged
            // on each transition — "did the RC stream start / why did it stop" is the first question.
            if alive != rc_fast_read {
                transport.set_read_timeout(if alive { RC_READ_TIMEOUT_FAST } else { RC_READ_TIMEOUT_IDLE });
                rc_fast_read = alive;
                if alive {
                    let (sysid, px4) = rc_tx.lock().ok().and_then(|s| s.mav_target.map(|t| (t.sysid, t.px4))).unwrap_or((fc_sysid, is_px4));
                    log::info!("RC stream started → sysid={} ({})", sysid, if px4 { "MANUAL_CONTROL" } else { "RC_CHANNELS_OVERRIDE" });
                } else {
                    let why = match rc_tx.lock() {
                        Ok(s) => {
                            if !s.enabled { "disengaged" }
                            else if s.mav_target.is_some_and(|t| t.link != app_handle.link_id()) { "target on another link" }
                            else if now.duration_since(s.last_update) >= rc_tx::RC_DEADMAN { "deadman (no frontend frame within 500 ms)" }
                            else { "no payload yet" }
                        }
                        Err(_) => "state poisoned",
                    };
                    log::info!("RC stream stopped ({})", why);
                }
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
        let primary_quadplane_seen = vehicles.get(&fc_sysid).is_some_and(|c| c.quadplane_seen);
        if is_ardupilot && !primary_quadplane_seen && param_probes < MAX_PARAM_PROBES
            && last_param_probe.elapsed() >= PARAM_PROBE_INTERVAL
        {
            super::params::request_quadplane_flag(&mut *transport, fc_sysid);
            param_probes += 1;
            last_param_probe = Instant::now();
        }

        // 2c. Ask vehicles discovered on this link for their own identity (FC hardware id, firmware
        //     version) — the same AUTOPILOT_VERSION request the handshake sends the primary.
        let now = Instant::now();
        for (sysid, ctx) in vehicles.iter_mut() {
            if *sysid == fc_sysid || ctx.identity_done
                || ctx.last_identity_request.is_some_and(|t| now.duration_since(t) < IDENTITY_REQUEST_INTERVAL)
            {
                continue;
            }
            if ctx.identity_requests >= MAX_IDENTITY_REQUESTS {
                ctx.identity_done = true;
                log::info!("MAVLink: sysid={} did not answer AUTOPILOT_VERSION — no FC hardware id", sysid);
                continue;
            }
            ctx.identity_requests += 1;
            ctx.last_identity_request = Some(now);
            let msg = super::handshake::autopilot_version_request(*sysid, ctx.compid);
            let frame = codec::serialize_v2(&gcs_header, &msg, &mut seq);
            debug_tracker.on_tx(msg.message_id(), frame.len());
            link_stats.on_tx(frame.len());
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
            }
            if let Err(e) = transport.write_bytes(&frame) {
                log::warn!("Failed to request AUTOPILOT_VERSION from sysid={}: {}", sysid, e);
            }
        }

        // 2d. Read each ArduPilot vehicle's own mode list (AVAILABLE_MODES), one index per request and
        //     at most one in flight per vehicle — see modes.rs. Paused while a mission transfer runs on
        //     this link (its request/response exchange must not compete with extra traffic).
        if mission_fwd.is_none() {
            let now = Instant::now();
            for (sysid, ctx) in vehicles.iter_mut() {
                // Component 0 would be a broadcast (forwarded to every peripheral) — never send that.
                if ctx.compid == 0 { continue; }
                let Some(index) = ctx.modes.as_mut().and_then(|e| e.poll(now)) else { continue };
                let msg = super::modes::request_msg(*sysid, ctx.compid, index);
                send_mav_frame(&mut *transport, &gcs_header, &mut seq, &msg,
                    &mut debug_tracker, &mut link_stats, &recorder);
            }
        }

        // 3. Read incoming bytes and parse MAVLink frames
        match transport.read_bytes(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                for parsed in parser.parse_all(&buf[..n]) {
                    let frame = match parsed {
                        Parsed::Msg(frame) => frame,
                        // TUNNEL (#385), decoded raw by the parser (INAV MSP tunnel, payload type 0x8001).
                        // Same bookkeeping as every FC frame — stall watchdog, debug/link counters, tlog —
                        // then forwarded to the MSP tunnel transport when it is addressed to us.
                        Parsed::Tunnel { header, raw_bytes, tunnel } => {
                            if header.system_id != fc_sysid { continue; }
                            msg_count += 1;
                            if header.component_id == fc_compid {
                                last_fc_rx = Instant::now();
                                if stall_warned {
                                    stall_warned = false;
                                    log::warn!("Link recovered — MAVLink frames from the FC resumed");
                                    let _ = app_handle.emit("telemetry-fc-link", FcLinkAlive { alive: true });
                                }
                            }
                            debug_tracker.on_rx(super::tunnel::TUNNEL_MSG_ID, raw_bytes.len());
                            link_stats.on_rx(raw_bytes.len());
                            if let Some(ref rec) = recorder {
                                if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&raw_bytes); }
                            }
                            let for_us = tunnel.payload_type == super::tunnel::PAYLOAD_TYPE_INAV_MSP
                                && header.component_id == fc_compid
                                && ((tunnel.target_system == codec::GCS_SYSTEM_ID
                                    && tunnel.target_component == codec::GCS_COMPONENT_ID)
                                    || (tunnel.target_system == 0 && tunnel.target_component == 0));
                            if !for_us {
                                log::debug!(
                                    "MAVLink TUNNEL ignored (type 0x{:04X}, from {}/{}, to {}/{})",
                                    tunnel.payload_type, header.system_id, header.component_id,
                                    tunnel.target_system, tunnel.target_component,
                                );
                            } else if let Some(ref tx) = tunnel_fwd {
                                if tx.send(tunnel.payload).is_err() {
                                    tunnel_fwd = None; // tunnel transport dropped
                                }
                            } else if let (Some(released), false) = (tunnel_released, tunnel_late_warned) {
                                // Once per link at the default level: the usual case is a reply that
                                // outlived the connect probe, which a tester's log must show.
                                tunnel_late_warned = true;
                                log::warn!(
                                    "MAVLink TUNNEL (MSP) reply arrived {:.1} s after the tunnel was closed — dropped (the link answers slower than Kite waited)",
                                    released.elapsed().as_secs_f64()
                                );
                            } else {
                                log::debug!("MAVLink TUNNEL (MSP) with no tunnel receiver — dropped");
                            }
                            continue;
                        }
                    };
                    // MSP-over-MAVLink TUNNEL frames (above) are the primary's only; every other frame is
                    // demuxed per system id below (multi-vehicle on a shared link).
                    let sysid = frame.header.system_id;
                    // Other GCSs on the same link (Mission Planner, QGC, …) are never vehicles.
                    if sysid == codec::GCS_SYSTEM_ID { continue; }

                    // Multi-vehicle on one link: an unknown sysid becomes a vehicle on its first
                    // autopilot HEARTBEAT (same acceptance rule as the handshake); any other frame from
                    // an unknown sysid is ignored until then.
                    // GATE(fleet): without the `--fleet` launch flag every frame of another system id is
                    // dropped, as before multi-vehicle — no secondary vehicle, recorder, marker or tagged
                    // STATUSTEXT. Vehicles discovered while it was on stay until they time out.
                    if !vehicles.contains_key(&sysid) {
                        if !crate::fleet_mode::enabled() { continue; }
                        let MavMessage::HEARTBEAT(ref hb) = frame.message else { continue };
                        let is_autopilot = hb.autopilot != MavAutopilot::MAV_AUTOPILOT_INVALID
                            || frame.header.component_id == 1;
                        if !is_autopilot { continue; }
                        let (variant, platform_type) = super::vehicle::identify(hb.autopilot, hb.mavtype);
                        let mut ctx = VehicleCtx::new(frame.header.component_id, variant, platform_type, hb.mavtype as u8, app_handle.for_sysid(sysid));
                        if let Some(cfg) = &secondary_recording {
                            ctx.recorder = secondary_recorder(cfg, sysid, &ctx.fc_variant, platform_type, hb.mavtype as u8, &ctx.emitter);
                        }
                        log::info!(
                            "MAVLink: discovered vehicle sysid={} compid={} ({}) on L{}",
                            sysid, frame.header.component_id, ctx.fc_variant, app_handle.link_id(),
                        );
                        // Its own mode list, after the start delay (the configure burst below goes first).
                        if ctx.fc_variant.starts_with("Ardu") {
                            ctx.modes = Some(super::modes::ModeEnumerator::new(sysid, Instant::now()));
                        }
                        configure_new_vehicle(&mut *transport, sysid, &ctx.fc_variant, &rates);
                        let _ = ctx.emitter.emit("vehicle-discovered", ctx.info(sysid, false));
                        vehicles.insert(sysid, ctx);
                    }
                    let ctx = vehicles.get_mut(&sysid).expect("vehicle registered above");
                    ctx.last_seen = Instant::now();
                    let primary = sysid == fc_sysid;

                    msg_count += 1;
                    // Only the autopilot component of the PRIMARY vehicle feeds the stall watchdog: peripherals on the FC's
                    // sysid (SIYI air unit, gimbal, CAN nodes) keep heartbeating while the FC may be gone, and a second
                    // vehicle must not mask the primary's silence.
                    if primary && frame.header.component_id == fc_compid {
                        let silent_for = last_fc_rx.elapsed();
                        last_fc_rx = Instant::now();
                        if stall_warned {
                            stall_warned = false;
                            log::warn!("Link recovered — MAVLink frames from the FC resumed");
                            let _ = app_handle.emit("telemetry-fc-link", FcLinkAlive { alive: true });
                            // The FC may have rebooted with another mode set and no MONITOR says so —
                            // re-read, but only after a silence long enough for a reboot, not on every
                            // recovery of a link flapping around the stall threshold.
                            if silent_for >= MODE_REREAD_AFTER_SILENCE
                                && ctx.modes.as_mut().is_some_and(|e| e.restart(Instant::now()))
                            {
                                log::info!("AVAILABLE_MODES: sysid={} heard again after a stall — reading the mode list again", sysid);
                            }
                        }
                    }
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
                        if let Some((target, ref tx)) = mission_fwd {
                            if target == sysid {
                                if tx.send(frame.message).is_err() {
                                    // Receiver dropped — operation likely timed out
                                    mission_fwd = None;
                                }
                                continue;
                            }
                        }
                    }

                    // Forward COMMAND_ACK to any active command operation so the helper can match it
                    // to the command it sent (mode/arm/takeoff/reposition/…). Otherwise it falls
                    // through to dispatch (which logs it at trace level).
                    if matches!(frame.message, MavMessage::COMMAND_ACK(_)) {
                        if let Some((target, ref tx)) = cmd_fwd {
                            if target == sysid {
                                if tx.send(frame.message).is_err() {
                                    cmd_fwd = None;
                                }
                                continue;
                            }
                        }
                    }

                    // Forward PARAM_VALUE to an active param read/write (fence params). Otherwise it
                    // falls through to dispatch (the handshake EKF-type reader).
                    if matches!(frame.message, MavMessage::PARAM_VALUE(_)) {
                        if let Some((target, ref tx)) = param_fwd {
                            if target == sysid {
                                if tx.send(frame.message).is_err() {
                                    param_fwd = None;
                                }
                                continue;
                            }
                        }
                    }

                    // The vehicle's own mode list (modes.rs): accepted only from its autopilot component —
                    // the same messages from a peripheral (gimbal, camera, companion, radio) are ignored.
                    // Never dispatched as telemetry.
                    match frame.message {
                        MavMessage::AVAILABLE_MODES(ref m) => {
                            if super::modes::from_autopilot(&frame.header, sysid, ctx.compid) {
                                if let Some(list) = ctx.modes.as_mut().and_then(|e| e.on_mode(m)) {
                                    let _ = ctx.emitter.emit("available-modes", super::modes::AvailableModesEvent { modes: &list });
                                }
                            } else {
                                log::debug!("AVAILABLE_MODES from {}/{} ignored (not the autopilot)", sysid, frame.header.component_id);
                            }
                            continue;
                        }
                        MavMessage::AVAILABLE_MODES_MONITOR(ref m) => {
                            if super::modes::from_autopilot(&frame.header, sysid, ctx.compid) {
                                if let Some(e) = ctx.modes.as_mut() { e.on_monitor(m.seq); }
                            }
                            continue;
                        }
                        _ => {}
                    }

                    // Only the autopilot component drives mode + armed: peripherals on the FC's sysid (gimbal, companion,
                    // CAN nodes) send HEARTBEATs with custom_mode 0/0xFFFF and no armed bit, which flapped mode and armed.
                    // Their frames still go to the tlog, and non-HEARTBEAT messages from them are unaffected.
                    if matches!(frame.message, MavMessage::HEARTBEAT(_)) && frame.header.component_id != ctx.compid {
                        continue;
                    }

                    // A secondary's own identity (the primary's came from the handshake) — from its
                    // autopilot component only, not a peripheral on the same system id.
                    if !primary && frame.header.component_id == ctx.compid {
                        if let MavMessage::AUTOPILOT_VERSION(ref ver) = frame.message {
                            let v = super::handshake::parse_autopilot_version(ver);
                            log::info!(
                                "MAVLink: sysid={} AUTOPILOT_VERSION v{} — FC id {}",
                                sysid, v.fc_version, v.fc_uid.as_deref().unwrap_or("none"),
                            );
                            if ctx.apply_identity(v) {
                                let _ = ctx.emitter.emit("vehicle-discovered", ctx.info(sysid, false));
                            }
                        }
                    }

                    dispatch_message(
                        &frame.header, &frame.message, &ctx.fc_variant, &ctx.emitter,
                        &mut ctx.analog, &mut ctx.batteries, &mut ctx.fused, &mut ctx.quadplane_seen,
                        if primary { &recorder } else { &ctx.recorder },
                        primary,
                        // The INAV tunnel belongs to the handshake (primary) vehicle only.
                        primary && inav_tunnel.load(Ordering::Relaxed),
                    );
                }
            }
            Err(crate::transport::TransportError::Timeout) => {}
            Err(crate::transport::TransportError::Disconnected) => {
                // Real transport loss: the same `connection-lost` as the MSP scheduler (the frontend tears down), and
                // `shutdown_lost` offers the recording-recovery prompt (ADR-042).
                log::warn!("MAVLink transport disconnected — tearing down");
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown_lost(); }
                }
                shutdown_secondaries(&mut vehicles);
                // One event only (the fork also emitted `mavlink-disconnected`): stamped with this
                // link's `linkId` by the VehicleEmitter, the +page.svelte listener closes just that
                // link when others remain.
                let _ = app_handle.emit("connection-lost", ());
                return Some(transport);
            }
            Err(e) => {
                // Not a known link loss, so no teardown — but it may repeat on every pass.
                read_errors.on_error("MAVLink read error", &e);
            }
        }

        // 3b. Stall watchdog: warn once at the default level when the FC has been silent for a while
        //     although the transport is still open, and flip the status bar to "not alive". No teardown.
        if !stall_warned && last_fc_rx.elapsed() >= STALL_WARN_AFTER {
            stall_warned = true;
            log::warn!(
                "Link stalled — no MAVLink frame from the FC for {:.0}s (transport still open)",
                last_fc_rx.elapsed().as_secs_f32()
            );
            let _ = app_handle.emit("telemetry-fc-link", FcLinkAlive { alive: false });
        }

        // 3c. Drop secondary vehicles that fell silent (they re-register on their next HEARTBEAT).
        if last_sweep.elapsed() >= VEHICLE_SWEEP_INTERVAL {
            last_sweep = Instant::now();
            let stale: Vec<u8> = vehicles.iter()
                .filter(|(s, c)| **s != fc_sysid && c.last_seen.elapsed() > VEHICLE_TIMEOUT)
                .map(|(s, _)| *s)
                .collect();
            for s in stale {
                if let Some(mut c) = vehicles.remove(&s) {
                    if let Some(rec) = c.recorder.take() {
                        if let Ok(mut r) = rec.lock() { r.shutdown_lost(); }
                        register_recorder(&c.emitter, None);
                    }
                    log::info!("MAVLink: vehicle sysid={} silent for {}s — dropped", s, VEHICLE_TIMEOUT.as_secs());
                    let _ = c.emitter.emit("vehicle-lost", serde_json::json!({
                        "vehicleId": c.emitter.key(),
                        "linkId": c.emitter.link_id(),
                        "reason": "timeout",
                    }));
                }
            }
        }

        // 4. Emit debug stats to the Debug Monitor (throttled internally; no-op in release)
        debug_tracker.maybe_emit(&app_handle);
        // Always-on link-rate emit (Relay panel) — throttled internally, compiled in release too.
        link_stats.maybe_emit(&app_handle);
    }
}

/// Build an `RC_CHANNELS_OVERRIDE` message (ArduPilot RC injection). `ch` is the 18-channel positional
/// wire array already carrying the per-band ignore sentinels (see rc_tx::override_channels). Also reused
/// for the deliberate-disengage release frame (rc_tx::release_channels), hence `pub(crate)`.
pub(crate) fn rc_override_msg(fc_sysid: u8, ch: &[u16; rc_tx::MAV_OVERRIDE_CHANNELS]) -> MavMessage {
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

/// Serialize + send one handler-originated message (streamed RC override / manual control, the mode-list
/// requests), with the same send-path bookkeeping as the command channel (debug stats, link meter, tlog
/// record).
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
        log::warn!("Failed to send MAVLink frame (msg_id {}): {}", msg.message_id(), e);
    }
}

/// MISSION_CURRENT → the displayed 1-based WP number (0 = none). ArduPilot/PX4: `seq` as is (home slot 0
/// dropped). INAV over the MSP tunnel: `seq = active WP − 1`, and `mission_mode == 1` for a running mission.
fn active_wp_from_mission_current(seq: u16, mission_mode: u8, inav: bool) -> u16 {
    if !inav {
        seq
    } else if mission_mode == 1 {
        seq.saturating_add(1)
    } else {
        0
    }
}

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

/// Payload of `telemetry-fc-link` — the status bar's FC liveness. Same shape the MSP scheduler and the
/// passive-telemetry handler emit (`false` while the FC is silent on an open transport, `true` once
/// frames resume). Kept module-local like the other event payloads here.
#[derive(Clone, serde::Serialize)]
struct FcLinkAlive {
    alive: bool,
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

/// The FC's active navigation target (`guided-target`) from POSITION_TARGET_GLOBAL_INT. The GUIDED target
/// only in guided mode — in AUTO/RTL it carries the mission WP / home, so the frontend gates on the mode.
/// `alt` is AMSL (frame GLOBAL).
#[derive(serde::Serialize)]
struct GuidedTargetEvent {
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

/// One receiver as GPS_RAW_INT / GPS2_RAW last reported it. Speed and course are kept only for a link
/// that never sends GLOBAL_POSITION_INT.
#[derive(Clone, Copy)]
struct RawGps {
    data: GpsReceiverData,
    ground_speed: f64,
    course: f64,
    at: std::time::Instant,
}

/// A receiver whose message stopped coming counts as lost; its stream is requested at 1 Hz.
const GPS_RECEIVER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Splits the overlapping ArduPilot GPS messages into single owners: GLOBAL_POSITION_INT (fused) owns the
/// position — emitting a receiver's raw lat/lon too zig-zags the track — and the receivers own fix type
/// and sats, which GLOBAL_POSITION_INT lacks (else the UI flashes 0 sats).
#[derive(Default, Clone, Copy)]
struct FusedPos {
    /// Index = receiver id. A port is listed once it reported more than "no receiver" and then stays.
    receivers: [Option<RawGps>; 2],
    valid: bool,
    lat: f64,
    lon: f64,
    alt_msl: f64,
    ground_speed: f64,
    course: f64,
    fix_type: u8,
    num_sat: u8,
    /// A receiver reported (GPS_RAW_INT / GPS2_RAW): its fix type stands from then on, "no fix" included.
    raw_seen: bool,
}

fn unified_fix(raw: ::mavlink::ardupilotmega::GpsFixType) -> u8 {
    use ::mavlink::ardupilotmega::GpsFixType::*;
    match raw {
        GPS_FIX_TYPE_NO_GPS | GPS_FIX_TYPE_NO_FIX => 0,
        GPS_FIX_TYPE_2D_FIX => 1,
        GPS_FIX_TYPE_3D_FIX => 2,
        GPS_FIX_TYPE_DGPS | GPS_FIX_TYPE_RTK_FLOAT | GPS_FIX_TYPE_RTK_FIXED | GPS_FIX_TYPE_STATIC
        | GPS_FIX_TYPE_PPP => 3,
    }
}

/// `yaw` of GPS_RAW_INT / GPS2_RAW in cdeg: 0 = not provided, u16::MAX = configured but not available,
/// 36000 = north.
fn gps_yaw_deg(raw: u16) -> Option<f64> {
    (1..=36000).contains(&raw).then(|| (raw % 36000) as f64 / 100.0)
}

/// One GPS_RAW_INT (id 0) or GPS2_RAW (id 1). The FC does not report which receiver it navigates on, so
/// fix type, sat count and HDOP of `telemetry-gps` come from the better one: higher fix, then more sats.
fn on_gps_receiver(rx: RawGps, fused: &mut FusedPos, app_handle: &VehicleEmitter, recorder: &Option<FlightRecorderHandle>) {
    if let Some(slot) = fused.receivers.get_mut(rx.data.id as usize) {
        if slot.is_some() || rx.data.fix_raw > 0 {
            *slot = Some(rx);
        }
    }
    // A receiver that went silent would otherwise keep winning with its last fix.
    for r in fused.receivers.iter_mut().flatten() {
        if rx.at.duration_since(r.at) > GPS_RECEIVER_TIMEOUT {
            r.data.fix_raw = 0;
            r.data.fix_type = 0;
            r.data.num_sat = 0;
            r.data.hdop = None;
        }
    }
    // STATIC (7) and PPP (8) sit above RTK fixed (6) in the enum but are not the better fix.
    let rank = |raw: u8| if raw > 6 { 4 } else { raw };
    let best = fused
        .receivers
        .iter()
        .flatten()
        .max_by_key(|r| (rank(r.data.fix_raw), r.data.num_sat, std::cmp::Reverse(r.data.id)))
        .copied()
        .unwrap_or(rx);
    fused.fix_type = best.data.fix_type;
    fused.num_sat = best.data.num_sat;
    fused.raw_seen = true;

    let list: Vec<GpsReceiverData> = fused.receivers.iter().flatten().map(|r| r.data).collect();
    if !list.is_empty() {
        let _ = app_handle.emit("telemetry-gps-receivers", &list);
    }

    // Position from the fused GLOBAL_POSITION_INT; until the first one arrives from ONE receiver, the
    // first that has a fix — two raw streams taken in turn sawtooth the track.
    let data = if fused.valid {
        GpsData {
            fix_type: fused.fix_type,
            num_sat: fused.num_sat,
            lat: fused.lat,
            lon: fused.lon,
            alt_msl: fused.alt_msl,
            ground_speed: fused.ground_speed,
            course: fused.course,
        }
    } else {
        let src = fused.receivers.iter().flatten().find(|r| r.data.fix_type > 0).copied().unwrap_or(rx);
        GpsData {
            fix_type: fused.fix_type,
            num_sat: fused.num_sat,
            lat: src.data.lat,
            lon: src.data.lon,
            alt_msl: src.data.alt_msl,
            ground_speed: src.ground_speed,
            course: src.course,
        }
    };
    let _ = app_handle.emit("telemetry-gps", &data);

    // Same stats event INAV uses (MSP_GPSSTATISTICS), so the HDOP readout is one path for both protocols.
    if let Some(hdop) = best.data.hdop {
        let _ = app_handle.emit("telemetry-gps-stats", serde_json::json!({ "hdop": hdop }));
    }

    // A port that never had a receiver (GPS2_RAW with "no GPS") is not listed and not recorded.
    let listed = fused.receivers.get(rx.data.id as usize).is_some_and(|r| r.is_some());
    if let Some(ref rec) = recorder {
        if let Ok(mut r) = rec.lock() {
            // `on_gps` first: the first fix can be what starts the session.
            r.on_gps(&data);
            if listed {
                r.on_gps_receiver(&rx.data);
            }
        }
    }
}

/// Dispatch a MAVLink message to the same Tauri events as the MSP scheduler. Only the `primary` vehicle
/// feeds the process-global OS link status — secondaries would flap it.
#[allow(clippy::too_many_arguments)] // per-vehicle decode context, split out of VehicleCtx for borrowck
fn dispatch_message(header: &MavHeader, message: &MavMessage, fc_variant: &str, app_handle: &VehicleEmitter, analog: &mut AnalogState, batteries: &mut std::collections::BTreeMap<u8, BatteryInstanceData>, fused: &mut FusedPos, quadplane_seen: &mut bool, recorder: &Option<FlightRecorderHandle>, primary: bool, inav_tunnel: bool) {
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
            if primary { crate::link_status::on_status(&data); }
            if primary { crate::link_status::on_flightmode(&fm); }

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_status(&data);
                    r.on_flightmode(&fm);
                }
            }
            // MAV_STATE: ArduPlane / ArduCopter report ACTIVE only while flying, PX4 while armed.
            let _ = app_handle.emit("vehicle-flight-state", serde_json::json!({ "system_status": hb.system_status as u8 }));
        }

        // ── EXTENDED_SYS_STATE → vehicle-flight-state (MAV_LANDED_STATE) ─
        MavMessage::EXTENDED_SYS_STATE(st) => {
            let _ = app_handle.emit("vehicle-flight-state", serde_json::json!({ "landed_state": st.landed_state as u8 }));
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
            if primary { crate::link_status::on_home(data.lat, data.lon); }
            let _ = app_handle.emit("home-position", &data);
        }

        // ── POSITION_TARGET_GLOBAL_INT → guided-target ──────────────
        // Requested at 1 Hz (SET_MESSAGE_INTERVAL) — keeps the map's Guided marker on the FC's
        // actual loiter/goto point, including targets set by another GCS. See GuidedTargetEvent.
        MavMessage::POSITION_TARGET_GLOBAL_INT(pt) => {
            let data = GuidedTargetEvent {
                lat: pt.lat_int as f64 / 1e7,
                lon: pt.lon_int as f64 / 1e7,
                alt: pt.alt as f64, // m AMSL
            };
            let _ = app_handle.emit("guided-target", &data);
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_nav_target(crate::flightlog::nav_target(pt.lat_int, pt.lon_int, pt.alt));
                }
            }
        }

        // ── MISSION_CURRENT → telemetry-nav-status (active waypoint) ─
        // The FC's current mission item sequence, mapped to the displayed 1-based WP number (see
        // `active_wp_from_mission_current`). Reuses the unified nav-status event (same shape MSP emits)
        // so the widget + map highlight work identically.
        MavMessage::MISSION_CURRENT(mc) => {
            let _ = app_handle.emit("telemetry-nav-status", serde_json::json!({
                "active_wp_number": active_wp_from_mission_current(mc.seq, mc.mission_mode, inav_tunnel),
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
            // fix_type / num_sat belong to the receiver messages — leave them.
            fused.valid = true;
            fused.lat = gpi.lat as f64 / 1e7;
            fused.lon = gpi.lon as f64 / 1e7;
            fused.alt_msl = gpi.alt as f64 / 1000.0; // mm → m
            fused.ground_speed = ((gpi.vx as f64).powi(2) + (gpi.vy as f64).powi(2)).sqrt() / 100.0; // cm/s → m/s
            // COG from the fused velocity (vx North, vy East) — `gpi.hdg` is the HEADING, which comes from ATTITUDE.
            // Below walking pace COG is velocity noise: hold the previous value.
            if fused.ground_speed > 0.5 {
                fused.course = (gpi.vy as f64).atan2(gpi.vx as f64).to_degrees().rem_euclid(360.0);
            }

            let gps = GpsData {
                // ArduPilot sends GLOBAL_POSITION_INT without a fix too (position 0/0), so only a real
                // position on a link that never carried a receiver message is taken as a 3D fix.
                fix_type: if fused.raw_seen || (gpi.lat == 0 && gpi.lon == 0) { fused.fix_type } else { 2 },
                num_sat: fused.num_sat,
                lat: fused.lat,
                lon: fused.lon,
                alt_msl: fused.alt_msl,
                ground_speed: fused.ground_speed,
                course: fused.course,
            };
            let _ = app_handle.emit("telemetry-gps", &gps);
            if primary { crate::link_status::on_gps(&gps); }

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

        // ── GPS_RAW_INT / GPS2_RAW → telemetry-gps (fix, sats, hdop) + telemetry-gps-receivers ──
        // `eph` is HDOP × 100 (u16::MAX = unknown). Sensor health stays with SYS_STATUS.
        MavMessage::GPS_RAW_INT(gps) => {
            on_gps_receiver(RawGps {
                data: GpsReceiverData {
                    id: 0,
                    fix_type: unified_fix(gps.fix_type),
                    fix_raw: gps.fix_type as u8,
                    num_sat: gps.satellites_visible,
                    hdop: (gps.eph != u16::MAX).then(|| gps.eph as f64 / 100.0),
                    lat: gps.lat as f64 / 1e7,
                    lon: gps.lon as f64 / 1e7,
                    alt_msl: gps.alt as f64 / 1000.0, // mm → m
                    vdop: (gps.epv != u16::MAX).then(|| gps.epv as f64 / 100.0),
                    h_acc_m: (gps.h_acc > 0).then(|| gps.h_acc as f64 / 1000.0),
                    v_acc_m: (gps.v_acc > 0).then(|| gps.v_acc as f64 / 1000.0),
                    speed_ms: (gps.vel != u16::MAX).then(|| gps.vel as f64 / 100.0),
                    course_deg: (gps.cog != u16::MAX).then(|| gps.cog as f64 / 100.0),
                    yaw_deg: gps_yaw_deg(gps.yaw),
                },
                ground_speed: gps.vel as f64 / 100.0, // cm/s → m/s
                course: gps.cog as f64 / 100.0,       // cdeg → deg
                at: std::time::Instant::now(),
            }, fused, app_handle, recorder);
        }
        MavMessage::GPS2_RAW(gps) => {
            on_gps_receiver(RawGps {
                data: GpsReceiverData {
                    id: 1,
                    fix_type: unified_fix(gps.fix_type),
                    fix_raw: gps.fix_type as u8,
                    num_sat: gps.satellites_visible,
                    hdop: (gps.eph != u16::MAX).then(|| gps.eph as f64 / 100.0),
                    lat: gps.lat as f64 / 1e7,
                    lon: gps.lon as f64 / 1e7,
                    alt_msl: gps.alt as f64 / 1000.0,
                    vdop: (gps.epv != u16::MAX).then(|| gps.epv as f64 / 100.0),
                    h_acc_m: (gps.h_acc > 0).then(|| gps.h_acc as f64 / 1000.0),
                    v_acc_m: (gps.v_acc > 0).then(|| gps.v_acc as f64 / 1000.0),
                    speed_ms: (gps.vel != u16::MAX).then(|| gps.vel as f64 / 100.0),
                    course_deg: (gps.cog != u16::MAX).then(|| gps.cog as f64 / 100.0),
                    yaw_deg: gps_yaw_deg(gps.yaw),
                },
                ground_speed: gps.vel as f64 / 100.0,
                course: gps.cog as f64 / 100.0,
                at: std::time::Instant::now(),
            }, fused, app_handle, recorder);
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
            if primary { crate::link_status::on_analog(&analog.to_analog_data()); }
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
            // Pre-arm readiness from the PREARM_CHECK bit (enabled = checks active, health = all passed), as QGC/MP
            // do: 0 unknown, 1 ready, 2 blocked. Authoritative at 1 Hz; "PreArm: …" STATUSTEXT only adds detail.
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
                // The FC's own "RC input valid" flag. ArduPilot sets RC_RECEIVER health for any live RC
                // source, incl. a MAVLink-RC override (SIYI) with no serial RX — a far more robust
                // "transmitter present" signal than the receiver RSSI (which such links leave at 255).
                rc_receiver: s3(Sns::MAV_SYS_STATUS_SENSOR_RC_RECEIVER),
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
        // `speed` is horizontal m/s. PX4 sends the same estimate as WIND_COV below, and INAV has its
        // own MSP2_INAV_WIND from 10.0 on.
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

        // ── WIND_COV → telemetry-wind ──────────────────────────────────
        // PX4's wind estimate (it has no WIND, just as ArduPilot has no WIND_COV). `wind_x`/`wind_y`
        // are the north/east components of the wind *velocity* — the direction the air moves TOWARD —
        // so the bearing is turned around into the FROM convention `WindData` carries. Either field
        // reads NaN while the estimate is unknown.
        MavMessage::WIND_COV(w) => {
            let (north, east) = (w.wind_x as f64, w.wind_y as f64);
            if north.is_finite() && east.is_finite() {
                let wind = WindData {
                    direction_from_deg: (east.atan2(north).to_degrees() + 180.0).rem_euclid(360.0),
                    speed_ms: north.hypot(east),
                };
                let _ = app_handle.emit("telemetry-wind", &wind);

                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.on_wind(&wind); }
                }
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
            if primary { crate::link_status::on_analog(&analog.to_analog_data()); }
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

        // ── BATTERY_STATUS → per-instance battery (multi-battery) + primary into analog ───
        // ArduPilot/PX4 send one BATTERY_STATUS per configured monitor, keyed by `id`. Total pack
        // voltage is the sum of the non-0xFFFF entries in `voltages[]` (mV); current is cA, consumed
        // is mAh, remaining is %, temperature is centi-°C (INT16_MAX = invalid).
        MavMessage::BATTERY_STATUS(bat) => {
            let voltage = bat
                .voltages
                .iter()
                .filter(|&&v| v != u16::MAX)
                .map(|&v| v as f64)
                .sum::<f64>()
                / 1000.0;
            let current = if bat.current_battery >= 0 { bat.current_battery as f64 / 100.0 } else { 0.0 };
            let mah = if bat.current_consumed >= 0 { bat.current_consumed as u32 } else { 0 };
            let pct = if bat.battery_remaining >= 0 { bat.battery_remaining as u8 } else { 0 };
            let cell_count = if voltage > 0.5 { ((voltage / 3.7).round() as u8).max(1) } else { 0 };
            let temperature = if bat.temperature != i16::MAX { Some(bat.temperature as f64 / 100.0) } else { None };

            batteries.insert(bat.id, BatteryInstanceData {
                id: bat.id, voltage, current, mah_drawn: mah, percentage: pct, cell_count, temperature,
            });
            let list: Vec<BatteryInstanceData> = batteries.values().cloned().collect();
            let _ = app_handle.emit("telemetry-batteries", &list);

            // Primary monitor (id 0) also feeds the single-battery analog path (top bar / widgets).
            if bat.id == 0 {
                analog.mah_drawn = mah;
                if bat.current_battery >= 0 {
                    analog.current = current;
                    analog.power = analog.voltage * analog.current;
                }
                if bat.battery_remaining >= 0 {
                    analog.battery_percentage = pct;
                }
                let _ = app_handle.emit("telemetry-analog", &analog.to_analog_data());
            if primary { crate::link_status::on_analog(&analog.to_analog_data()); }
            }

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_batteries(&list);
                    if bat.id == 0 { r.on_analog(&analog.to_analog_data()); }
                }
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

        // ── PARAM_VALUE → the one-shot answers of connect (see params.rs) ───
        // AHRS_EKF_TYPE: 2 = EKF2, 3 = EKF3; anything else shows as a generic "EKF".
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
                // A QuadPlane reports MAV_TYPE_FIXED_WING: the recorder stores it as VTOL.
                if quadplane {
                    if let Some(ref rec) = recorder {
                        if let Ok(mut r) = rec.lock() { r.set_quadplane(); }
                    }
                    // The link's stored FcInfo follows (primary only, once registered) while it still says Airplane.
                    if primary {
                        if let Ok(mut reg) = tauri::Manager::state::<crate::state::AppState>(app_handle.app()).links.lock() {
                            if let Some(entry) = reg.get_mut(app_handle.link_id()) {
                                if entry.fc_info.platform_type == super::vehicle::platform::AIRPLANE {
                                    entry.fc_info.platform_type = super::vehicle::platform::VTOL;
                                }
                            }
                        }
                    }
                }
                // After the raise: the frontend answers this event with a platform type of its own.
                let _ = app_handle.emit("telemetry-vehicle", serde_json::json!({
                    "quadplane": quadplane,
                }));
            } else if name == "LOG_BACKEND_TYPE" || name == "SDLOG_MODE" {
                // Logging backend (requested in connect_mavlink): ArduPilot bitmask 0 = none, PX4 mode
                // -1 = disabled. Lands in the stored FcInfo (the UAV Info panel's "Save to vehicle
                // library" reads `blackbox`) and reaches the frontend via telemetry-vehicle.
                let value = pv.param_value as i32;
                let blackbox = if name == "SDLOG_MODE" { value >= 0 } else { value != 0 };
                eprintln!("[MAVLINK-PARAM] {} = {} (blackbox={})", name, value, blackbox);
                // Only the link's primary vehicle owns the link's FcInfo (multi-vehicle).
                if primary {
                    if let Ok(mut reg) = tauri::Manager::state::<crate::state::AppState>(app_handle.app()).links.lock() {
                        if let Some(entry) = reg.get_mut(app_handle.link_id()) { entry.fc_info.blackbox = Some(blackbox); }
                    }
                }
                let _ = app_handle.emit("telemetry-vehicle", serde_json::json!({
                    "blackbox": blackbox,
                }));
            }
        }

        _ => {
            // Log unhandled messages at trace level
            log::trace!("MAVLink msg_id={} from sysid={}", message.message_id(), header.system_id);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::{active_wp_from_mission_current, gps_yaw_deg};

    #[test]
    fn gps_yaw_sentinels() {
        assert_eq!(gps_yaw_deg(0), None, "not provided");
        assert_eq!(gps_yaw_deg(u16::MAX), None, "configured but not available");
        assert_eq!(gps_yaw_deg(36001), None);
        assert_eq!(gps_yaw_deg(1), Some(0.01));
        assert_eq!(gps_yaw_deg(27150), Some(271.5));
        assert_eq!(gps_yaw_deg(36000), Some(0.0), "north");
    }

    #[test]
    fn mission_current_numbering_per_autopilot() {
        // ArduPilot/PX4: seq is already the displayed number (home slot 0 dropped).
        assert_eq!(active_wp_from_mission_current(3, 0, false), 3);
        // INAV tunnel: 0-based seq, only while mission_mode == 1.
        assert_eq!(active_wp_from_mission_current(0, 1, true), 1);
        assert_eq!(active_wp_from_mission_current(4, 1, true), 5);
        assert_eq!(active_wp_from_mission_current(4, 0, true), 0);
        assert_eq!(active_wp_from_mission_current(4, 2, true), 0);
    }
}
