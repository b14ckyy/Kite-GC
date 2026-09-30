// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Connection Commands — serial port listing, connect, disconnect, BLE scanning

use tauri::{AppHandle, Emitter, State};

use crate::flightlog::msp_raw_logger::MspRawSink;
use crate::flightlog::recorder::FlightRecorder;
use crate::flightlog::types::{FlightLogSettings, InavStats};
use crate::mavlink_proto;
use crate::msp::{
    FcInfo, InavVersion, MspTransport, MSP_API_VERSION, MSP_EEPROM_WRITE, MSP_SET_NAME,
};
use crate::scheduler;
use crate::scheduler::{SchedulerHandle, SchedulerMode, TelemetryConfig};
use crate::state::{ActiveProtocol, AppState};
use crate::transport::{ByteTransport, Transport, TransportType};
use crate::vehicle_registry::emitter::VehicleEmitter;
use crate::vehicle_registry::{LinkEntry, LinkId, VehicleId, VehicleInfo, ERR_NOT_CONNECTED};
use crate::transport::PortInfo;
use crate::transport::serial::SerialConnection;
use crate::transport::tcp::TcpTransport;
use crate::transport::tunnel::{self as msp_tunnel, TunnelTransport};
use crate::transport::udp::UdpTransport;
// `transport::ble` resolves per platform behind one name (btleplug on desktop, CoreBluetooth on iOS).
use crate::transport::ble::{self as ble_backend, BleDeviceInfo};

/// How long `disconnect` lets a running MSP transaction finish before it stops the scheduler.
const DISCONNECT_TXN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Home position pushed to the frontend (event `home-position`). Same shape/name regardless of
/// protocol so MAVLink (HOME_POSITION) can emit it identically later.
#[derive(serde::Serialize, Clone)]
struct HomeEvent {
    lat: f64,
    lon: f64,
    alt: f64,
}

/// What `connect` hands back: the registry ids of the new link / its primary vehicle plus the
/// handshake info (the pre-multi-vehicle return value).
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResult {
    pub link_id: LinkId,
    pub vehicle_id: String,
    pub fc_info: FcInfo,
}

/// A protocol path's result: the started handler and what it learned about the vehicle. `connect`
/// turns this into a registry entry.
struct Opened {
    protocol: ActiveProtocol,
    protocol_name: &'static str,
    fc_info: FcInfo,
    /// MAVLink system id of the handshake vehicle; 0 for MSP / passive (one vehicle per link).
    sysid: u8,
    /// MAVLink autopilot component id; 0 for MSP / passive.
    compid: u8,
}

/// `link-closed` event payload.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LinkClosed {
    link_id: LinkId,
    reason: &'static str,
}

/// `vehicle-lost` event payload.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VehicleLost {
    vehicle_id: String,
    link_id: LinkId,
    reason: &'static str,
}

/// `active-vehicle-changed` event payload.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActiveVehicleChanged {
    pub vehicle_id: Option<String>,
}

/// List available serial ports. On iOS this is always empty — `transport::serial` resolves to the
/// stand-in there (no serial access exists), and the UI hides serial on mobile anyway.
#[tauri::command]
pub fn list_serial_ports() -> Vec<PortInfo> {
    crate::transport::serial::list_ports()
}

/// Scan for BLE devices matching known serial profiles
#[tauri::command]
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    ble_backend::scan_ble_devices().await
}

/// Start a live BLE scan session. Discovered/updated devices are emitted as `ble-device` events
/// for the frontend to populate in real time. Restarts any previous session.
#[tauri::command]
pub async fn ble_scan_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    {
        // Replace any existing sender — dropping the old one ends the previous session.
        let mut guard = state.ble_scan_stop.lock().map_err(|e| e.to_string())?;
        *guard = Some(tx);
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = ble_backend::run_scan_session(app, rx).await {
            log::warn!("BLE scan session ended: {}", e);
        }
    });
    Ok(())
}

/// Stop the live BLE scan session (if any).
#[tauri::command]
pub fn ble_scan_stop(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.ble_scan_stop.lock().map_err(|e| e.to_string())?;
    *guard = None; // drop the sender → the session's stop future resolves
    Ok(())
}

/// Write a craft name to the connected INAV FC (MSP_SET_NAME) and persist it (MSP_EEPROM_WRITE),
/// then update the cached `fc_info`. INAV/MSP only; rejected for non-MSP / disconnected links. Used
/// post-flight to push a newly chosen craft name to the FC so future flights auto-link to a vehicle.
#[tauri::command(async)]
pub fn inav_set_craft_name(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let trimmed = name.trim();
    // INAV stores the craft name in a fixed 16-byte buffer; clamp to the conventional limit.
    if trimmed.len() > 16 {
        return Err("Craft name too long (max 16 characters)".into());
    }
    crate::state::with_msp(&state, |handle| {
        handle.msp_request(MSP_SET_NAME, trimmed.as_bytes())?;
        handle.msp_request(MSP_EEPROM_WRITE, &[])?;
        Ok(())
    })?;
    // Keep the cached craft name in sync so the UI reflects it without a reconnect (the active
    // vehicle's link — the one `with_msp` just wrote to).
    if let Ok(mut reg) = state.links.lock() {
        if let Some(link) = reg.active().map(|v| v.link) {
            if let Some(entry) = reg.get_mut(link) {
                entry.fc_info.craft_name = trimmed.to_string();
            }
        }
    }
    Ok(())
}

/// Read the INAV lifetime flight statistics from the FC `stats` settings (MSP2_COMMON_SETTING by
/// name). `enabled` mirrors the `stats` toggle; the totals are only meaningful when it is on. Used
/// to offer the FC's lifetime totals as a vehicle baseline. INAV/MSP only.
#[tauri::command(async)]
pub fn inav_read_stats(state: State<'_, AppState>) -> Result<InavStats, String> {
    use crate::commands::fc_settings::try_read_uint_setting;
    crate::state::with_msp(&state, |handle| {
        // A failed read (e.g. an MSP-over-MAVLink tunnel timeout) is an error, never a silent 0 that
        // would be adopted as the vehicle baseline; an absent setting (empty reply) still reads as 0.
        let read = |name: &str| -> Result<u64, String> { Ok(try_read_uint_setting(handle, name)?.unwrap_or(0)) };
        let enabled = read("stats")? != 0;
        let mut stats = InavStats { enabled, ..Default::default() };
        if enabled {
            stats.flight_count = read("stats_flight_count")? as i64;
            stats.total_time_s = read("stats_total_time")? as i64;
            stats.total_dist_m = read("stats_total_dist")? as i64;
            stats.total_energy = read("stats_total_energy")? as i64;
        }
        Ok(stats)
    })
}

/// Connect to a flight controller on the given transport and protocol.
/// MSP: Performs handshake + starts telemetry scheduler.
/// MAVLink: Waits for HEARTBEAT + starts handler thread.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub async fn connect(
    transport_type: TransportType,
    // Protocol selection ("msp" or "mavlink", defaults to "msp")
    protocol: Option<String>,
    // Serial params
    port: Option<String>,
    baud_rate: Option<u32>,
    // TCP/UDP params
    host: Option<String>,
    tcp_port: Option<u16>,
    // BLE params
    ble_device_id: Option<String>,
    // Telemetry config
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    wind_enabled: Option<bool>,
    // MAVLink: when true, request no stream rates — FC streams per its own SRn_* params (ADR-043)
    mavlink_full_telemetry: Option<bool>,
    // Flight log config
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<ConnectResult, String> {
    // Multi-vehicle: every connect opens an additional link. Reserve its id now (the handlers stamp
    // their events with it) and, on the first link only, start RC injection from a clean slate — with
    // another link already up, resetting would yank the other vehicle's engaged override.
    let link_id = {
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        if reg.is_empty() {
            if let Ok(mut rc) = state.rc_tx.lock() {
                *rc = crate::scheduler::rc_tx::RcTxState::default();
            }
        }
        reg.reserve_id()
    };

    let proto = protocol.as_deref().unwrap_or("msp");

    log::info!(
        "Connect requested: protocol={} transport={:?} port={:?} baud={:?} host={:?} tcp_port={:?} ble={:?}",
        proto, transport_type, port, baud_rate, host, tcp_port, ble_device_id,
    );

    // Open byte-level transport based on type. Every variant is handled on every platform — the
    // platform differences live behind `transport::serial` / `transport::ble` (on iOS a serial open
    // fails with a clear runtime error; BLE is CoreBluetooth there). TCP/UDP are platform-independent.
    // The OS-facing link status names the transport ("MSP over Serial"); the protocol paths
    // below only know their protocol.
    crate::link_status::set_transport(&transport_type.to_string());
    let byte_transport: Box<dyn ByteTransport> = match transport_type {
        TransportType::Serial => {
            let port_name = port.ok_or("Serial port name required")?;
            let baud = baud_rate.unwrap_or(115200);
            Box::new(SerialConnection::open(&port_name, baud)?)
        }
        TransportType::Tcp => {
            let h = host.ok_or("TCP host required")?;
            let p = tcp_port.ok_or("TCP port required")?;
            Box::new(TcpTransport::connect(&h, p)?)
        }
        TransportType::Udp => {
            let h = host.ok_or("UDP host required")?;
            let p = tcp_port.ok_or("UDP port required")?;
            Box::new(UdpTransport::connect(&h, p)?)
        }
        TransportType::Ble => {
            let dev_id = ble_device_id.ok_or("BLE device ID required")?;
            if proto == "telemetry" {
                // Passive mode: no known profile required — auto-discover + subscribe to all
                // Notify/Indicate characteristics and dump the GATT table to the Debug Monitor.
                Box::new(ble_backend::connect_ble_listen(&dev_id, app_handle.clone()).await?)
            } else {
                Box::new(ble_backend::connect_ble(&dev_id).await?)
            }
        }
    };

    let transport_desc = byte_transport.description().to_string();
    let transport_note = byte_transport.diagnostic_note();
    log::info!("Transport opened, protocol={} link=L{} ({})", proto, link_id, transport_desc);

    // One link per endpoint. A second socket talking to the same UDP/TCP peer (or the same serial
    // port / BLE device) does not give a second view of the vehicles — the peer answers whichever
    // socket spoke last, so both links starve each other (PX4 SITL: 0 bytes on the newcomer, dropped
    // vehicles on the first). Every vehicle behind one endpoint is already discovered on one link.
    {
        let reg = state.links.lock().map_err(|e| e.to_string())?;
        let duplicate = reg.iter().find(|e| e.transport == transport_desc).map(|e| e.id);
        drop(reg);
        if let Some(existing) = duplicate {
            log::warn!("Connect refused: {} is already open as L{}", transport_desc, existing);
            return Err(format!(
                "Already connected to {} as link L{}. Every vehicle on that endpoint is discovered there — select it from the Links list instead of opening a second link.",
                transport_desc, existing
            ));
        }
    }

    let result = match proto {
        "mavlink" => {
            // ── MAVLink Path ─────────────────────────────────────────────
            connect_mavlink(
                link_id,
                byte_transport,
                attitude_rate_hz,
                position_rate_hz,
                airspeed_enabled,
                wind_enabled,
                mavlink_full_telemetry,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                flight_log_raw,
                flight_log_raw_always,
                state.clone(),
                app_handle.clone(),
            )
        }
        "telemetry" => {
            // ── Passive Telemetry Path (listen-only, auto-detect) ────────
            connect_passive_telemetry(
                link_id,
                byte_transport,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                state.clone(),
                app_handle.clone(),
            )
        }
        _ => {
            // ── MSP Path ────────────────────────────────────────────────
            connect_msp(
                link_id,
                byte_transport,
                attitude_rate_hz,
                position_rate_hz,
                airspeed_enabled,
                wind_enabled,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                flight_log_raw,
                flight_log_raw_always,
                state.clone(),
                app_handle.clone(),
            )
        }
    };

    // Central success/failure log — a failed connect otherwise only surfaces in the UI toast and
    // leaves no trace in the diagnostics log (the original PX4 report had nothing to go on).
    let opened = match result {
        Ok(o) => {
            log::info!(
                "Connection established: {} {} (platform={}) as L{}:S{}",
                o.fc_info.fc_variant, o.fc_info.fc_version, o.fc_info.platform_type, link_id, o.sysid,
            );
            o
        }
        Err(e) => {
            log::error!("Connection failed (protocol={}): {}", proto, e);
            // A transport-level explanation (busy local UDP port → the vehicles' pushes go elsewhere)
            // turns a bare "no HEARTBEAT" into something the user can act on.
            return Err(match transport_note {
                Some(note) => format!("{e}\n\n{note}"),
                None => e,
            });
        }
    };

    // Register the link. The first link's primary vehicle becomes the active one.
    let primary = VehicleId::new(link_id, opened.sysid);
    let entry = LinkEntry {
        id: link_id,
        protocol: opened.protocol,
        protocol_name: opened.protocol_name,
        transport: transport_desc,
        fc_info: opened.fc_info.clone(),
        primary: primary.clone(),
    };
    let discovered = VehicleInfo::primary_of(&entry, opened.compid);
    let activated = {
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        reg.insert(entry)
    };
    crate::link_presence::link_up(&opened.fc_info, opened.protocol_name);
    let _ = app_handle.emit("vehicle-discovered", &discovered);
    if activated {
        state.retarget_rc(Some(&primary));
        let _ = app_handle.emit("active-vehicle-changed", ActiveVehicleChanged { vehicle_id: Some(primary.to_key()) });
    }

    Ok(ConnectResult { link_id, vehicle_id: primary.to_key(), fc_info: opened.fc_info })
}

/// MSP connection path: handshake → scheduler
#[allow(clippy::too_many_arguments)] // mirrors the connect() command's parameter set
fn connect_msp(
    link_id: LinkId,
    byte_transport: Box<dyn ByteTransport>,
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    wind_enabled: Option<bool>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Opened, String> {
    // Shared MSP raw-serial log sink (ADR-049): the transport writes into it, the recorder owns its
    // lifecycle. Created up front so both share the same slot.
    let msp_raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));

    // In CONTINUOUS raw mode, open the raw logger NOW — before the handshake — so the handshake's
    // identity frames (MSP_NAME / MSP_FC_VARIANT / …) are captured in the log and the offline parser
    // can recover the vehicle info. The recorder later adopts this same logger (ADR-049). Per-flight
    // mode opens on arm instead, so it intentionally has no handshake.
    if flight_log_enabled.unwrap_or(false)
        && flight_log_raw.unwrap_or(false)
        && flight_log_raw_always.unwrap_or(false)
    {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);
        let raw_dir = crate::flightlog::db::resolve_raw_log_dir(
            flight_log_raw_path.as_deref().unwrap_or(""),
            portable,
        );
        match crate::flightlog::msp_raw_logger::MspRawLogger::new(&raw_dir, 0, &chrono::Utc::now()) {
            Ok(logger) => {
                if let Ok(mut g) = msp_raw_sink.lock() {
                    *g = Some(logger);
                }
                log::info!("Continuous MSP raw log opened pre-handshake");
            }
            Err(e) => log::warn!("Failed to open pre-handshake MSP raw log: {}", e),
        }
    }

    // Wrap in MSP protocol layer (adds MSP v2 framing + response parser)
    let mut transport = MspTransport::new(byte_transport, msp_raw_sink.clone());
    // No tunnel on this link — clear the Tunnel tab's counters of an earlier MAVLink session.
    msp_tunnel::stats::reset();

    // One vehicle per MSP link → sysid 0. Everything this link emits carries "L{id}:S0".
    let emitter = VehicleEmitter::new(app_handle.clone(), VehicleId::new(link_id, 0));

    // ── MSP Handshake ──────────────────────────────────────────────
    // Identity, version gate, feature set, craft info and home (shared with the MSP-over-MAVLink
    // tunnel — see msp/handshake.rs). One attempt per request on a direct MSP link.
    let handshake = crate::msp::handshake::run(&mut transport, 1)?;
    let fc_info = handshake.fc_info;
    if let Some(fix) = handshake.home {
        let home = HomeEvent { lat: fix.lat, lon: fix.lon, alt: fix.alt };
        crate::link_status::on_home(home.lat, home.lon);
        let _ = emitter.emit("home-position", home);
    }
    let link_stats_supported = fc_info.features.as_ref().is_some_and(|f| f.link_stats);
    let wind_supported = fc_info.features.as_ref().is_some_and(|f| f.wind_estimate);

    // ── Start telemetry scheduler ────────────────────────────────────────
    let config = TelemetryConfig {
        attitude_rate_hz: attitude_rate_hz.unwrap_or(5.0),
        position_rate_hz: position_rate_hz.unwrap_or(2.0),
        airspeed_enabled: airspeed_enabled.unwrap_or(false),
        // RC link stats poll (MSP2_INAV_GET_LINK_STATS) — INAV 9.1+ only.
        link_stats_enabled: link_stats_supported,
        // Wind poll (MSP2_INAV_WIND) — opt-in AND INAV 10.0+ only.
        wind_enabled: wind_enabled.unwrap_or(false) && wind_supported,
    };

    // ── Flight recorder setup ────────────────────────────────────────────
    let flight_log_settings = FlightLogSettings {
        enabled: flight_log_enabled.unwrap_or(false),
        db_enabled: flight_log_db_enabled.unwrap_or(false),
        db_path: flight_log_path.unwrap_or_default(),
        raw_log_path: flight_log_raw_path.unwrap_or_default(),
        raw_enabled: flight_log_raw.unwrap_or(false),
        raw_always: flight_log_raw_always.unwrap_or(false),
    };

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);

        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "MSP", portable, emitter.clone(), state.sessions.clone(), msp_raw_sink.clone()) {
            Ok(mut rec) => {
                rec.start_continuous_log();
                let handle = std::sync::Arc::new(std::sync::Mutex::new(rec));
                log::info!("Flight recorder initialized");
                Some(handle)
            }
            Err(e) => {
                log::error!("Failed to initialize flight recorder: {}", e);
                None
            }
        }
    } else {
        None
    };

    store_recorder(&state, emitter.key(), &recorder_handle);
    let handle = scheduler::start(
        Box::new(transport),
        config,
        emitter,
        recorder_handle,
        state.radar_ingest.clone(),
        state.radar_msp_enabled.clone(),
        state.rc_tx.clone(),
        SchedulerMode::Telemetry,
    );

    Ok(Opened { protocol: ActiveProtocol::Msp(handle), protocol_name: "MSP", fc_info, sysid: 0, compid: 0 })
}

/// MAVLink connection path: handshake → handler
#[allow(clippy::too_many_arguments)]
fn connect_mavlink(
    link_id: LinkId,
    mut byte_transport: Box<dyn ByteTransport>,
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    wind_enabled: Option<bool>,
    mavlink_full_telemetry: Option<bool>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Opened, String> {
    // MAVLink handshake: wait for FC HEARTBEAT, send GCS HEARTBEAT back
    let (mut fc_info, fc_sysid, fc_compid, uid2_valid) = mavlink_proto::perform_handshake(&mut *byte_transport)?;

    log::info!(
        "MAVLink connected: {} (sysid={}) via {}",
        fc_info.fc_variant,
        fc_sysid,
        byte_transport.description(),
    );

    // Configure telemetry stream rates (ADR-043) — mirrors the MSP poll-rate knobs. Skipped when
    // "Full MAVLink Telemetry" is on, so the FC streams purely per its own SRn_* params (.tlog gets
    // everything). Applied here, before the handler thread starts, while we still own the transport.
    // Which of the two wind messages this FC speaks — WIND on ArduPilot, WIND_COV on PX4.
    let is_px4 = fc_info.fc_variant == "PX4";
    if mavlink_full_telemetry.unwrap_or(false) {
        // SET_MESSAGE_INTERVAL is sticky on the FC until reboot, so a prior reduced session would
        // otherwise keep the link narrow. Reset our managed messages to the FC's SRn defaults.
        mavlink_proto::streamrates::reset_stream_rates(&mut *byte_transport, fc_sysid, is_px4);
    } else {
        mavlink_proto::streamrates::apply_stream_rates(
            &mut *byte_transport,
            fc_sysid,
            attitude_rate_hz.unwrap_or(5.0),
            position_rate_hz.unwrap_or(2.0),
            airspeed_enabled.unwrap_or(false),
            wind_enabled.unwrap_or(false),
            is_px4,
        );
    }

    // Ask the FC which EKF core is active (AHRS_EKF_TYPE) for the header EKF indicator. One-shot,
    // fire-and-forget — the PARAM_VALUE reply is decoded by the handler thread once it starts.
    mavlink_proto::params::request_ekf_type(&mut *byte_transport, fc_sysid);

    // One-shot HOME_POSITION request — recovers the real FC home on a mid-flight connect (MAVLink
    // counterpart of the MSP_WP(0) read below in the MSP path). Unconditional: in Full-telemetry
    // mode this is the ONLY source (no interval push), in reduced mode it beats the first 0.2 Hz tick.
    mavlink_proto::params::request_home_position(&mut *byte_transport, fc_sysid);

    // ArduPilot: read Q_ENABLE to detect a QuadPlane (which reports MAV_TYPE_FIXED_WING, so the mission
    // vehicle class can't be told from the HEARTBEAT alone). Copter/Rover/Sub lack the param → no reply.
    if fc_info.fc_variant.starts_with("Ardu") {
        mavlink_proto::params::request_quadplane_flag(&mut *byte_transport, fc_sysid);
    }

    // Logging backend for the vehicle library's "blackbox available" flag: ArduPilot LOG_BACKEND_TYPE
    // (bitmask, 0 = none), PX4 SDLOG_MODE (-1 = disabled). The reply lands in the handler, which
    // updates the stored FC info and emits `telemetry-vehicle { blackbox }`.
    if fc_info.fc_variant.starts_with("Ardu") {
        mavlink_proto::params::request_param(&mut *byte_transport, fc_sysid, "LOG_BACKEND_TYPE");
    } else if fc_info.fc_variant.contains("PX4") {
        mavlink_proto::params::request_param(&mut *byte_transport, fc_sysid, "SDLOG_MODE");
    }

    // ── Flight recorder setup ────────────────────────────────────────────
    let flight_log_settings = FlightLogSettings {
        enabled: flight_log_enabled.unwrap_or(false),
        db_enabled: flight_log_db_enabled.unwrap_or(false),
        db_path: flight_log_path.unwrap_or_default(),
        raw_log_path: flight_log_raw_path.unwrap_or_default(),
        raw_enabled: flight_log_raw.unwrap_or(false),
        raw_always: flight_log_raw_always.unwrap_or(false),
    };

    // Events of this link's handshake vehicle carry "L{link}:S{sysid}" (the recorder and the tunnel
    // scheduler below share the emitter — they talk to the same vehicle).
    let emitter = VehicleEmitter::new(app_handle.clone(), VehicleId::new(link_id, fc_sysid));

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);

        // MAVLink records via .tlog; the MSP raw sink is unused here (kept empty).
        let msp_raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
        match FlightRecorder::new(flight_log_settings.clone(), fc_info.clone(), "MAVLink", portable, emitter.clone(), state.sessions.clone(), msp_raw_sink) {
            Ok(mut rec) => {
                rec.start_continuous_log();
                let handle = std::sync::Arc::new(std::sync::Mutex::new(rec));
                log::info!("Flight recorder initialized (MAVLink)");
                Some(handle)
            }
            Err(e) => {
                log::error!("Failed to initialize flight recorder: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Start the MAVLink handler thread. Its events carry "L{link}:S{sysid}" of the handshake vehicle;
    // vehicles it discovers later on the same link get their own sysid stamped. The stream shaping
    // applied above is handed along so those vehicles are configured identically.
    let rates = mavlink_proto::handler::StreamRateConfig {
        full_telemetry: mavlink_full_telemetry.unwrap_or(false),
        attitude_hz: attitude_rate_hz.unwrap_or(5.0),
        position_hz: position_rate_hz.unwrap_or(2.0),
        airspeed_enabled: airspeed_enabled.unwrap_or(false),
        wind_enabled: wind_enabled.unwrap_or(false),
    };
    store_recorder(&state, emitter.key(), &recorder_handle);
    // Vehicles discovered later on this link record unattended (DB only) with the same settings.
    let secondary_recording = if flight_log_settings.enabled && flight_log_settings.db_enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);
        Some(mavlink_proto::handler::SecondaryRecording {
            settings: flight_log_settings.clone(),
            portable,
            slots: state.sessions.clone(),
        })
    } else {
        None
    };
    let link_desc = byte_transport.description();
    let probe_recorder = recorder_handle.clone();
    let mut handle = mavlink_proto::handler::start(byte_transport, fc_sysid, fc_compid, fc_info.fc_variant.clone(), (fc_info.platform_type, fc_info.mav_type), emitter.clone(), recorder_handle, state.rc_tx.clone(), rates, secondary_recording);

    // INAV MSP over MAVLink (D2): INAV's AUTOPILOT_VERSION carries an all-zero uid2 (and a fake
    // ArduPilot 4.7.0 version), so the MAVLink identity can't tell INAV apart — only the tunnel can.
    // uid2 all-zero (or no AUTOPILOT_VERSION at all) → probe the TUNNEL inline (≤ 2 s); a real uid2 →
    // ArduPilot / PX4, no probe, no delay.
    msp_tunnel::stats::reset();
    if !uid2_valid {
        match probe_msp_tunnel(&handle, fc_sysid, fc_info.mav_type, &link_desc, &state, &emitter) {
            TunnelProbe::Up(sched, inav_info) => {
                handle.msp = Some(sched);
                // INAV confirmed → the handler switches to INAV's MAVLink conventions (MISSION_CURRENT).
                handle.inav_tunnel.store(true, std::sync::atomic::Ordering::Relaxed);
                // Two identities on a tunnel link, on purpose:
                //  • `handle.fc_variant` keeps the HEARTBEAT variant ("Generic" / "ArduPlane" / …). The
                //    handler decodes telemetry with it — INAV emulates ArduPilot flight modes in the
                //    HEARTBEAT `custom_mode`, so the MAVLink mode tables are the right ones.
                //  • the link entry's `fc_info` (returned to the frontend below) is the INAV identity from the MSP
                //    handshake, with `features.msp_tunnel = true`. The frontend keys its INAV surface on
                //    that flag (`hasMsp` / `isArduPilotLink` in stores/connection.ts), and every INAV
                //    command reaches this scheduler through `state::with_msp`.
                fc_info = inav_info;
                // The recorder was created with the heartbeat identity before the probe — upgrade it so
                // the flights of this link are stored as the INAV craft (name, version, FC id).
                if let Some(ref rec) = probe_recorder {
                    if let Ok(mut r) = rec.lock() {
                        r.set_fc_info(fc_info.clone());
                    }
                }
            }
            TunnelProbe::NoTunnel => {}
            TunnelProbe::LinkLost => {
                // The handler already emitted `connection-lost` (before the link was registered, so the
                // frontend's disconnect found nothing) — tear down and fail the connect instead of
                // storing a dead "connected" link.
                // That `connection-lost` carries this (still unregistered) link's `linkId`; the
                // +page.svelte listener ignores a linkId it does not know, so other links stay up.
                let _ = handle.stop();
                // The primary's and any secondary discovered during the probe.
                state.drop_link_recorders(link_id);
                msp_tunnel::stats::emit_now(&app_handle);
                return Err("Link lost during MSP tunnel probe".into());
            }
        }
    }
    msp_tunnel::stats::emit_now(&app_handle);

    Ok(Opened { protocol: ActiveProtocol::Mavlink(handle), protocol_name: "MAVLink", fc_info, sysid: fc_sysid, compid: fc_compid })
}

/// Outcome of the connect-time MSP-over-MAVLink probe.
// `Up` carries the running scheduler + FC info; the enum is built once per connect.
#[allow(clippy::large_enum_variant)]
enum TunnelProbe {
    /// Tunnel up: the tunnel-mode scheduler and the INAV `FcInfo`.
    Up(SchedulerHandle, FcInfo),
    /// No (usable) tunnel — stay plain MAVLink.
    NoTunnel,
    /// The MAVLink link itself died during the probe / handshake.
    LinkLost,
}

/// Probe for INAV's MSP-over-MAVLink tunnel on a running MAVLink handler (D2) and, on success, run the
/// INAV MSP handshake through it and start the MSP scheduler in tunnel mode. `Up` carries the scheduler
/// and the INAV `FcInfo` (`features.msp_tunnel = true`, `adsb_msp = false`, `mav_type` kept from the
/// HEARTBEAT). Every failure is logged at warn; the tunnel receiver is unregistered on drop.
fn probe_msp_tunnel(
    handle: &mavlink_proto::MavlinkHandle,
    fc_sysid: u8,
    mav_type: u8,
    link_desc: &str,
    state: &State<'_, AppState>,
    app_handle: &VehicleEmitter,
) -> TunnelProbe {
    use std::time::{Duration, Instant};

    /// Probe attempts: send at t = 0 / 500 / 1000 ms, wait for the first reply until t = 2 s.
    const PROBE_WAITS_MS: [u64; 3] = [500, 500, 1000];
    /// After the reply: swallow late duplicate replies of the earlier attempts before the handshake.
    const PROBE_DRAIN: Duration = Duration::from_millis(100);

    log::warn!(
        "MSP tunnel probe: AUTOPILOT_VERSION uid2 is zero or missing — probing INAV MSP over MAVLink (FC sysid {}, up to 2 s)",
        fc_sysid
    );
    let tunnel = match TunnelTransport::open(handle.cmd_tx_clone(), fc_sysid, link_desc) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("MSP tunnel probe: {}", e);
            return TunnelProbe::LinkLost;
        }
    };
    // The tunnel traffic is already in the .tlog (every TUNNEL frame is recorded by the handler), and
    // the MAVLink recorder never opens an MSP raw log — so no .rawmsp capture here (empty sink).
    let raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut msp = MspTransport::new(Box::new(tunnel), raw_sink);

    let started = Instant::now();
    let mut answered: Option<(usize, Duration)> = None;
    for (i, wait_ms) in PROBE_WAITS_MS.iter().enumerate() {
        let sent = Instant::now();
        match msp.msp_request_timeout(MSP_API_VERSION, &[], *wait_ms) {
            Ok(_) => {
                answered = Some((i + 1, sent.elapsed()));
                break;
            }
            Err(e) if msp.is_connection_lost() => {
                log::warn!("MSP tunnel probe: MAVLink link lost during the probe ({})", e);
                return TunnelProbe::LinkLost;
            }
            Err(_) => {}
        }
    }
    let Some((attempt, rtt)) = answered else {
        log::warn!("MSP tunnel probe: no reply after 2 s (3 attempts) — plain MAVLink");
        msp_tunnel::stats::set_probe("no_reply", None, format!("{} ms, 3 attempts", started.elapsed().as_millis()));
        return TunnelProbe::NoTunnel;
    };
    let rtt_ms = rtt.as_millis() as u64;
    log::warn!(
        "MSP tunnel probe: MSP_API_VERSION answered in {} ms (attempt {}/3, {} ms since probe start)",
        rtt_ms,
        attempt,
        started.elapsed().as_millis()
    );

    // Late duplicates (a slow reply to an earlier attempt) are unsolicited to the handshake — drain them.
    let drain_until = Instant::now() + PROBE_DRAIN;
    while Instant::now() < drain_until {
        if msp.poll_incoming().is_err() {
            break;
        }
    }

    // INAV handshake through the tunnel — two tries per request (a lost chunk means no reply at all).
    let handshake = match crate::msp::handshake::run(&mut msp, 2) {
        Ok(h) => h,
        Err(e) if msp.is_connection_lost() => {
            log::warn!("MSP tunnel: MAVLink link lost during the INAV handshake ({})", e);
            return TunnelProbe::LinkLost;
        }
        Err(e) => {
            log::warn!("MSP tunnel: the tunnel answered but the INAV handshake rejected it ({}) — plain MAVLink", e);
            msp_tunnel::stats::set_probe("rejected", Some(rtt_ms), e);
            return TunnelProbe::NoTunnel;
        }
    };
    let mut fc_info = handshake.fc_info;
    let tunnel_min = InavVersion::new(10, 0, 0);
    let version_ok = InavVersion::parse(&fc_info.fc_version).is_some_and(|v| v.is_at_least(tunnel_min));
    if fc_info.fc_variant != "INAV" || !version_ok {
        log::warn!(
            "MSP tunnel: {} {} is not INAV >= 10.0.0 — plain MAVLink",
            fc_info.fc_variant,
            fc_info.fc_version
        );
        msp_tunnel::stats::set_probe(
            "rejected",
            Some(rtt_ms),
            format!("{} {}", fc_info.fc_variant, fc_info.fc_version),
        );
        return TunnelProbe::NoTunnel;
    }
    if let Some(f) = fc_info.features.as_mut() {
        f.msp_tunnel = true;
        // The tunnel scheduler never polls MSP2_ADSB_VEHICLE_LIST, so FC-side ADS-B is not available on
        // a tunnel link today (FC-relayed MAVLink ADSB_VEHICLE ingestion is a separate open feature, see
        // Dev-Docs QUICK_NOTES) — the "ADS-B from FC (MSP)" source must not be offered.
        f.adsb_msp = false;
        // Likewise the MSP RC stream (MSP_SET_RAW_RC / MSP2_INAV_SET_AUX_RC): the tunnel scheduler never
        // runs it (one request in flight, on-demand only), so this LINK has neither MSP-RC nor AUX-RC.
        // The RC tab is hidden on a tunnel link until Stage 3 — MAVLink-RX mode, see Dev-Docs
        // active/MSP_OVER_MAVLINK.md.
        f.msp_rc = false;
        f.aux_rc = false;
    }
    fc_info.mav_type = mav_type;
    log::warn!(
        "MSP tunnel: INAV {} handshake OK (board {}, API {}, craft '{}')",
        fc_info.fc_version,
        fc_info.board_id,
        fc_info.api_version,
        fc_info.craft_name
    );
    msp_tunnel::stats::set_probe(
        "ok",
        Some(rtt_ms),
        format!("attempt {}/3 — INAV {}", attempt, fc_info.fc_version),
    );

    // Tunnel-mode scheduler: no polling, no recorder, no RC stream / radar, one request in flight.
    let sched = scheduler::start(
        Box::new(msp),
        TelemetryConfig::default(),
        app_handle.clone(),
        None,
        state.radar_ingest.clone(),
        state.radar_msp_enabled.clone(),
        state.rc_tx.clone(),
        SchedulerMode::Tunnel,
    );
    log::warn!("MSP tunnel: MSP scheduler started in tunnel mode (on-demand only, 1 request in flight)");
    TunnelProbe::Up(sched, fc_info)
}

/// Reply of the dev-only tunnel request (Debug Monitor → Tunnel tab).
#[derive(serde::Serialize)]
pub struct TunnelMspReply {
    /// Reply payload as spaced hex
    reply_hex: String,
    /// Reply payload length in bytes
    bytes: usize,
    /// TUNNEL chunks received for this request (as counted by the tunnel transport)
    chunks: u32,
    rtt_ms: u64,
}

/// Dev tool (Debug Monitor → Tunnel): send one MSP request with a hex payload through the active MSP
/// scheduler (the MSP-over-MAVLink tunnel, or a direct MSP link) and return the raw reply. The
/// hardware test for the tunnel's multi-chunk replies: `0x2048` must come back as 640 payload bytes.
/// Only available while debug mode is on (debug build or `--debug`).
#[tauri::command(async)]
pub fn debug_tunnel_msp_request(
    code: u16,
    payload_hex: String,
    state: State<'_, AppState>,
) -> Result<TunnelMspReply, String> {
    if !crate::debug_mode::enabled() {
        return Err("Debug mode is off".into());
    }
    let payload = parse_hex_payload(&payload_hex)?;

    let started = std::time::Instant::now();
    let reply = crate::state::with_msp(&state, |h| h.msp_request(code, &payload))?;
    let rtt_ms = started.elapsed().as_millis() as u64;
    Ok(TunnelMspReply {
        reply_hex: reply.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "),
        bytes: reply.len(),
        chunks: msp_tunnel::stats::last_request_chunks(),
        rtt_ms,
    })
}

/// Dev tool (Debug Monitor → Tunnel): the current tunnel stats, so a tab opened after connect shows the
/// probe result without waiting for the next `debug-tunnel-stats` event (none come on a plain link).
#[tauri::command]
pub fn debug_tunnel_stats_snapshot() -> Result<msp_tunnel::stats::TunnelStatsSnapshot, String> {
    if !crate::debug_mode::enabled() {
        return Err("Debug mode is off".into());
    }
    Ok(msp_tunnel::stats::snapshot())
}

/// Parse the dev form's payload: hex digits, optionally `0x`-prefixed, separators (whitespace, `,`,
/// `:`) ignored. Parsed byte-wise, so non-ASCII input is an error instead of a slice panic.
fn parse_hex_payload(text: &str) -> Result<Vec<u8>, String> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != ':')
        .collect();
    let compact = compact.strip_prefix("0x").or_else(|| compact.strip_prefix("0X")).unwrap_or(&compact);
    if !compact.is_ascii() {
        return Err("Payload is not valid hex".into());
    }
    if !compact.len().is_multiple_of(2) {
        return Err("Payload hex must have an even number of digits".into());
    }
    compact
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).ok().and_then(|s| u8::from_str_radix(s, 16).ok()))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| "Payload is not valid hex".to_string())
}

/// Passive telemetry path: no handshake — start the listen-only handler immediately.
/// The wire protocol is auto-detected by the handler; nothing is ever transmitted. When flight logging
/// is enabled, a recorder is attached and fed the decoded telemetry (arm/disarm derived from the FC's
/// flight-mode field — e.g. FrSky MODES).
fn connect_passive_telemetry(
    link_id: LinkId,
    byte_transport: Box<dyn ByteTransport>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Opened, String> {
    log::info!(
        "Passive telemetry connect (listen-only) via {}",
        byte_transport.description()
    );

    // Synthesize a minimal FcInfo so the frontend enters the connected state. Passive telemetry carries
    // no FC identity (no handshake) — leave firmware empty (shown as "N/A") and use the Generic platform
    // (255) so the map shows the generic arrow rather than defaulting to a multirotor.
    let fc_info = FcInfo {
        platform_type: 255, // PLATFORM_GENERIC
        ..FcInfo::default()
    };

    // Flight recorder (no raw byte log on this path — FrSky has no MSP raw stream).
    let flight_log_settings = FlightLogSettings {
        enabled: flight_log_enabled.unwrap_or(false),
        db_enabled: flight_log_db_enabled.unwrap_or(false),
        db_path: flight_log_path.unwrap_or_default(),
        raw_log_path: flight_log_raw_path.unwrap_or_default(),
        raw_enabled: false,
        raw_always: false,
    };

    // One vehicle per passive link → sysid 0.
    let emitter = VehicleEmitter::new(app_handle, VehicleId::new(link_id, 0));

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);
        let msp_raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "Telemetry", portable, emitter.clone(), state.sessions.clone(), msp_raw_sink) {
            Ok(mut rec) => {
                rec.start_continuous_log();
                log::info!("Flight recorder initialized (passive telemetry)");
                Some(std::sync::Arc::new(std::sync::Mutex::new(rec)))
            }
            Err(e) => {
                log::error!("Failed to initialize flight recorder: {}", e);
                None
            }
        }
    } else {
        None
    };

    store_recorder(&state, emitter.key(), &recorder_handle);
    let handle = crate::passive_telemetry::start(byte_transport, emitter, recorder_handle);

    Ok(Opened { protocol: ActiveProtocol::PassiveTelemetry(handle), protocol_name: "Telemetry", fc_info, sysid: 0, compid: 0 })
}

/// Disconnect one link (`link_id`) or, with `None`, every link (the pre-multi-vehicle behaviour the
/// single connect/disconnect toggle still relies on). Stops the handler(s), drops the transport(s) and
/// moves the active vehicle to a remaining link if its own went away.
#[tauri::command]
pub async fn disconnect(link_id: Option<LinkId>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // A running MSP write transaction (safehome / geozone / mission save) gets up to 5 s to finish, so
    // the scheduler is not stopped between its SET requests (half-written RAM config). Read
    // transactions simply end on the scheduler stop. Only the link(s) about to close are waited for.
    let msps: Vec<crate::scheduler::MspRequester> = {
        let reg = state.links.lock().map_err(|e| e.to_string())?;
        reg.iter()
            .filter(|e| link_id.is_none_or(|id| e.id == id))
            .filter_map(|e| e.protocol.msp_requester())
            .collect()
    };
    for msp in msps {
        let idle = tauri::async_runtime::spawn_blocking(move || msp.wait_idle(DISCONNECT_TXN_WAIT))
            .await
            .unwrap_or(true);
        if !idle {
            log::warn!("Disconnect: an MSP transaction was still running after 5 s — stopping anyway");
        }
    }

    // Take the entries out under the lock, stop them after releasing it (stop() joins the thread).
    let (entries, active_change) = {
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        if reg.is_empty() {
            return Err(ERR_NOT_CONNECTED.into());
        }
        match link_id {
            None => (reg.drain(), Some(None)),
            Some(id) => {
                let (entry, change) = reg.remove(id).ok_or_else(|| format!("Unknown link L{id}"))?;
                (vec![entry], change)
            }
        }
    };

    for entry in entries {
        let vehicle_key = entry.primary.to_key();
        match entry.protocol {
            ActiveProtocol::Msp(handle) => {
                let _transport = handle.stop(); // transport dropped here
                log::info!("MSP scheduler stopped (L{})", entry.id);
            }
            ActiveProtocol::Mavlink(handle) => {
                let _transport = handle.stop(); // transport dropped here
                log::info!("MAVLink handler stopped (L{})", entry.id);
            }
            ActiveProtocol::PassiveTelemetry(handle) => {
                let _transport = handle.stop(); // transport dropped here
                log::info!("Passive telemetry handler stopped (L{})", entry.id);
            }
        }
        state.drop_link_recorders(entry.id);
        let _ = app_handle.emit("vehicle-lost", VehicleLost { vehicle_id: vehicle_key, link_id: entry.id, reason: "user" });
        let _ = app_handle.emit("link-closed", LinkClosed { link_id: entry.id, reason: "user" });
    }

    if let Some(new_active) = active_change {
        state.retarget_rc(new_active.as_ref());
        let _ = app_handle.emit("active-vehicle-changed", ActiveVehicleChanged { vehicle_id: new_active.map(|v| v.to_key()) });
    }

    let none_left = state.links.lock().map(|r| r.is_empty()).unwrap_or(true);
    if none_left {
        if let Ok(mut recs) = state.recorders.lock() {
            recs.clear();
        }
        crate::link_presence::link_down();
        log::info!("Disconnected");
    } else {
        log::info!("Link closed; other links remain");
    }
    Ok(())
}

/// Publish (or drop) a link primary's recorder handle for the command layer, under its vehicle key
/// (dropped again on disconnect).
fn store_recorder(state: &State<'_, AppState>, key: &str, rec: &Option<crate::flightlog::recorder::FlightRecorderHandle>) {
    state.set_recorder(key, rec.as_ref());
}

/// Override the platform type of the connected vehicle for this session (UAV Info panel dropdown).
/// Updates the stored FC info and the recorder, so the flight being recorded — and any flight started
/// later on this link — is saved with the chosen type. RAM only; nothing is persisted.
#[tauri::command]
pub fn set_platform_type(platform_type: u8, state: State<'_, AppState>) -> Result<(), String> {
    let active_key = {
        // Multi-vehicle: the link's FcInfo describes its primary vehicle — only that one's override
        // lands there; a selected secondary leaves it alone.
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        let active = reg.active().cloned().ok_or_else(|| crate::vehicle_registry::ERR_NOT_CONNECTED.to_string())?;
        match reg.get_mut(active.link) {
            Some(entry) => {
                if entry.primary == active {
                    entry.fc_info.platform_type = platform_type;
                }
            }
            None => return Err(crate::vehicle_registry::ERR_NOT_CONNECTED.to_string()),
        }
        active.to_key()
    };
    // The ACTIVE vehicle's recorder — not whichever link connected last.
    let recorder = state.recorders.lock().ok().and_then(|m| m.get(&active_key).cloned());
    if let Some(rec) = recorder {
        if let Ok(mut r) = rec.lock() {
            r.set_platform_type(platform_type);
        }
    }
    log::info!("Platform type override: {}", platform_type);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_hex_payload;

    #[test]
    fn hex_payload_parses_with_separators() {
        assert_eq!(parse_hex_payload("0x01 02:0a,FF").unwrap(), vec![0x01, 0x02, 0x0A, 0xFF]);
        assert_eq!(parse_hex_payload("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hex_payload_rejects_non_ascii_without_panicking() {
        // en-dash (3 UTF-8 bytes) used to panic with "byte index is not a char boundary"
        assert!(parse_hex_payload("0\u{2013}02").is_err());
        assert!(parse_hex_payload("0–02").is_err());
        assert!(parse_hex_payload("123").is_err());
        assert!(parse_hex_payload("zz").is_err());
    }
}
