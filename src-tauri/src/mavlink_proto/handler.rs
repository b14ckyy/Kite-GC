// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Handler
// Dedicated thread that owns the ByteTransport and handles MAVLink communication.
// Unlike MSP (poll-based), MAVLink is push-based: the FC streams telemetry,
// the GCS sends heartbeats and occasional commands.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::MavMessage;
use ::mavlink::{MavHeader, Message};
use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::scheduler::telemetry::{
    AttitudeData, GpsData, AltitudeData, AnalogData, StatusData,
    SensorStatusData, AirspeedData,
};
use crate::transport::ByteTransport;

use super::codec::{self, MavSequence};
use super::parser::MavParser;

/// GCS heartbeat interval (1 Hz as per MAVLink spec)
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

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
}

/// Handle for interacting with the running MAVLink handler
pub struct MavlinkHandle {
    cmd_tx: mpsc::Sender<MavlinkCommand>,
    thread: Option<thread::JoinHandle<Option<Box<dyn ByteTransport>>>>,
    /// System ID of the connected FC (from handshake)
    pub fc_sysid: u8,
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
) -> MavlinkHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<MavlinkCommand>();

    let thread = thread::spawn(move || {
        handler_loop(transport, fc_sysid, fc_variant, app_handle, cmd_rx, recorder)
    });

    MavlinkHandle {
        cmd_tx,
        thread: Some(thread),
        fc_sysid,
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
) -> Option<Box<dyn ByteTransport>> {
    let mut parser = MavParser::new();
    let mut seq = MavSequence::new();
    let mut buf = [0u8; 1024];
    let mut last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL; // Send immediately
    let mut msg_count: u64 = 0;
    let mut debug_tracker = super::debug::MavlinkDebugTracker::new();

    // Accumulated analog state — MAVLink splits battery data across multiple messages
    let mut analog = AnalogState::default();
    let mut fused = FusedPos::default();

    // Active mission operation receiver — when set, MISSION_* messages are forwarded
    // here instead of being dispatched as telemetry events.
    let mut mission_fwd: Option<mpsc::Sender<MavMessage>> = None;

    let gcs_header = codec::gcs_header();

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
            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.write_raw_mavlink_frame(&frame); }
            }
            if let Err(e) = transport.write_bytes(&frame) {
                log::warn!("Failed to send GCS heartbeat: {}", e);
            }
            last_heartbeat = Instant::now();
        }

        // 3. Read incoming bytes and parse MAVLink frames
        match transport.read_bytes(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                for frame in parser.parse_bytes(&buf[..n]) {
                    if frame.header.system_id != fc_sysid { continue; }

                    msg_count += 1;
                    debug_tracker.on_rx(frame.message.message_id(), frame.raw_bytes.len());

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

                    dispatch_message(&frame.header, &frame.message, &fc_variant, &app_handle, &mut analog, &mut fused, &recorder);
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
fn dispatch_message(header: &MavHeader, message: &MavMessage, fc_variant: &str, app_handle: &AppHandle, analog: &mut AnalogState, fused: &mut FusedPos, recorder: &Option<FlightRecorderHandle>) {
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
            let sensor_data = SensorStatusData {
                gyro: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_GYRO),
                acc: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_ACCEL),
                mag: s3(Sns::MAV_SYS_STATUS_SENSOR_3D_MAG),
                baro: s3(Sns::MAV_SYS_STATUS_SENSOR_ABSOLUTE_PRESSURE),
                gps: s3(Sns::MAV_SYS_STATUS_SENSOR_GPS),
                rangefinder: s3(Sns::MAV_SYS_STATUS_SENSOR_LASER_POSITION),
                pitot: s3(Sns::MAV_SYS_STATUS_SENSOR_DIFFERENTIAL_PRESSURE),
                opflow: s3(Sns::MAV_SYS_STATUS_SENSOR_OPTICAL_FLOW),
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

            if let Some(ref rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_airspeed(&airspeed); }
            }
        }

        // ── RC_CHANNELS → RSSI (merged into analog state) ──────────
        MavMessage::RC_CHANNELS(rc) => {
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
            }
        }

        _ => {
            // Log unhandled messages at trace level
            log::trace!("MAVLink msg_id={} from sysid={}", message.message_id(), header.system_id);
        }
    }
}

