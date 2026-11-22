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

/// A protocol path's result: the started handler and what it learned about the vehicle.
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

#[tauri::command]
pub fn ble_scan_stop(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.ble_scan_stop.lock().map_err(|e| e.to_string())?;
    *guard = None; // drop the sender → the session's stop future resolves
    Ok(())
}

/// Write a craft name to the connected INAV FC (MSP_SET_NAME + EEPROM write) and update the cached
/// `fc_info`, so future flights auto-link to a vehicle. INAV/MSP only.
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

/// Read INAV's lifetime flight statistics (`stats` settings) as a vehicle baseline; the totals mean
/// something only when `enabled`. INAV/MSP only.
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

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command — args map to frontend invoke() params
pub async fn connect(
    transport_type: TransportType,
    // Protocol selection ("msp" or "mavlink", defaults to "msp")
    protocol: Option<String>,
    port: Option<String>,
    baud_rate: Option<u32>,
    host: Option<String>,
    tcp_port: Option<u16>,
    ble_device_id: Option<String>,
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    wind_enabled: Option<bool>,
    // MAVLink: when true, request no stream rates — FC streams per its own SRn_* params (ADR-043)
    mavlink_full_telemetry: Option<bool>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    trainer: State<'_, crate::hid::frsky_trainer::TrainerManager>,
    app_handle: AppHandle,
) -> Result<ConnectResult, String> {
    // Multi-vehicle: every connect opens an additional link. Reserve its id now (the handlers stamp
    // their events with it) and, on the first link only, start RC injection from a clean slate — with
    // another link already up, resetting would yank the other vehicle's engaged override.
    let (link_id, first_link) = {
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        let first = reg.is_empty();
        // GATE(fleet): a second link is a multi-vehicle feature.
        if !crate::fleet_mode::allows(first, crate::fleet_mode::enabled()) {
            return Err("Already connected — disconnect first (multiple links need fleet mode)".into());
        }
        if first {
            if let Ok(mut rc) = state.rc_tx.lock() {
                *rc = crate::scheduler::rc_tx::RcTxState::default();
            }
        }
        (reg.reserve_id(), first)
    };
    // Group recording off (the default): ONE recorded vehicle at a time — a link records only when no
    // recorder runs (the first link, or the recorded vehicle back over a new link). Any further link
    // records nothing, so a fleet session never writes group data into the flight database.
    let nobody_records = first_link || state.recorders.lock().map(|m| m.is_empty()).unwrap_or(true);
    let flight_log_enabled = if nobody_records || crate::fleet_mode::group_recording() {
        flight_log_enabled
    } else {
        log::warn!("L{}: flight recording off — another vehicle is being recorded (KITE_FLEET_RECORDING not set)", link_id);
        Some(false)
    };

    let proto = protocol.as_deref().unwrap_or("msp");

    log::info!(
        "Connect requested: protocol={} transport={:?} port={:?} baud={:?} host={:?} tcp_port={:?} ble={:?}",
        proto, transport_type, port, baud_rate, host, tcp_port, ble_device_id,
    );

    // Every transport type is handled on every platform; the differences live behind `transport::serial`
    // / `transport::ble` (on iOS a serial open fails with a clear runtime error).
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
            if trainer.holds(&dev_id) {
                return Err("This radio is remembered as a Bluetooth trainer input; RC control is looking for it. Close the RC panel or remove the radio there".into());
            }
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

    // One link per endpoint: a second socket to the same peer starves both links (the peer answers
    // whichever socket spoke last), and every vehicle behind one endpoint is already found on one link.
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

    // Central success/failure log — otherwise a failed connect leaves no trace in the diagnostics log.
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

    // The first link's primary vehicle becomes the active one.
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
    // This link records the raw stream continuously (the display then stays awake, link_presence):
    // what `FlightRecorder::start_continuous_log` goes by — `raw_always` alone, with the flight log
    // on. A passive telemetry link never records raw data.
    let raw_always = opened.protocol_name != "Telemetry"
        && flight_log_enabled.unwrap_or(false)
        && flight_log_raw_always.unwrap_or(false);
    crate::link_presence::link_up(&opened.fc_info, opened.protocol_name, raw_always);
    let _ = app_handle.emit("vehicle-discovered", &discovered);
    if activated {
        state.retarget_rc(Some(&primary));
        let _ = app_handle.emit("active-vehicle-changed", ActiveVehicleChanged { vehicle_id: Some(primary.to_key()) });
    }

    Ok(ConnectResult { link_id, vehicle_id: primary.to_key(), fc_info: opened.fc_info })
}

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

    // In CONTINUOUS raw mode open the raw logger before the handshake, so its identity frames are in the
    // log for the offline parser (the recorder adopts this logger later). Per-flight mode opens on arm.
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

    let mut transport = MspTransport::new(byte_transport, msp_raw_sink.clone());
    // No tunnel on this link — clear the Tunnel tab's counters of an earlier MAVLink session.
    msp_tunnel::stats::reset();

    // One vehicle per MSP link → sysid 0. Everything this link emits carries "L{id}:S0".
    let emitter = VehicleEmitter::new(app_handle.clone(), VehicleId::new(link_id, 0));

    // Identity, version gate, features, craft info and home (shared with the tunnel, msp/handshake.rs);
    // one attempt per request on a direct MSP link.
    let handshake = crate::msp::handshake::run(&mut transport, 1)?;
    let fc_info = handshake.fc_info;
    if let Some(fix) = handshake.home {
        let home = HomeEvent { lat: fix.lat, lon: fix.lon, alt: fix.alt };
        crate::link_status::on_home(home.lat, home.lon);
        let _ = emitter.emit("home-position", home);
    }
    let link_stats_supported = fc_info.features.as_ref().is_some_and(|f| f.link_stats);
    let wind_supported = fc_info.features.as_ref().is_some_and(|f| f.wind_estimate);

    let config = TelemetryConfig {
        attitude_rate_hz: attitude_rate_hz.unwrap_or(5.0),
        position_rate_hz: position_rate_hz.unwrap_or(2.0),
        airspeed_enabled: airspeed_enabled.unwrap_or(false),
        // RC link stats poll (MSP2_INAV_GET_LINK_STATS) — INAV 9.1+ only.
        link_stats_enabled: link_stats_supported,
        // Wind poll (MSP2_INAV_WIND) — opt-in AND INAV 10.0+ only.
        wind_enabled: wind_enabled.unwrap_or(false) && wind_supported,
    };

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
    let mavlink_proto::Handshake { mut fc_info, sysid: fc_sysid, compid: fc_compid, uid2_valid, version_rtt } =
        mavlink_proto::perform_handshake(&mut *byte_transport)?;

    log::info!(
        "MAVLink connected: {} (sysid={}) via {}",
        fc_info.fc_variant,
        fc_sysid,
        byte_transport.description(),
    );

    // Also handed to the handler: vehicles it discovers later on this link get the same shaping (ADR-043).
    let rates = mavlink_proto::handler::StreamRateConfig {
        full_telemetry: mavlink_full_telemetry.unwrap_or(false),
        attitude_hz: attitude_rate_hz.unwrap_or(5.0),
        position_hz: position_rate_hz.unwrap_or(2.0),
        airspeed_enabled: airspeed_enabled.unwrap_or(false),
        wind_enabled: wind_enabled.unwrap_or(false),
    };
    // A tunnel candidate gets them AFTER the probe: ~35 commands queue for seconds on a slow uplink
    // (ELRS), and the probe behind them would time out.
    if uid2_valid {
        mavlink_proto::handler::configure_primary(&mut *byte_transport, fc_sysid, &fc_info.fc_variant, &rates);
    }

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

    store_recorder(&state, emitter.key(), &recorder_handle);
    // Vehicles discovered later record unattended (DB only); with group recording off they get no
    // recorder — only the first vehicle's flights reach the logbook.
    let secondary_recording = if flight_log_settings.enabled && flight_log_settings.db_enabled && crate::fleet_mode::group_recording() {
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

    // INAV MSP over MAVLink: INAV's AUTOPILOT_VERSION has an all-zero uid2 (and a fake ArduPilot 4.7.0),
    // so only the tunnel tells it apart. All-zero / missing uid2 → probe the TUNNEL inline (2 s, longer
    // on a slow link); a real uid2 → ArduPilot / PX4, no probe.
    msp_tunnel::stats::reset();
    if !uid2_valid {
        match probe_msp_tunnel(&handle, fc_sysid, fc_info.mav_type, &link_desc, version_rtt, &state, &emitter) {
            TunnelProbe::Up(sched, inav_info) => {
                handle.msp = Some(sched);
                // INAV confirmed → the handler switches to INAV's MAVLink conventions (MISSION_CURRENT).
                handle.inav_tunnel.store(true, std::sync::atomic::Ordering::Relaxed);
                // Two identities on purpose: `handle.fc_variant` keeps the HEARTBEAT variant (INAV emulates ArduPilot
                // modes in `custom_mode`, so the handler decodes with those tables); `fc_info` is the INAV identity
                // with `features.msp_tunnel`, on which the frontend keys its INAV surface.
                fc_info = inav_info;
                // The recorder was created with the heartbeat identity before the probe — upgrade it so
                // the flights of this link are stored as the INAV craft (name, version, FC id).
                if let Some(ref rec) = probe_recorder {
                    if let Ok(mut r) = rec.lock() {
                        r.set_fc_info(fc_info.clone());
                    }
                }
            }
            // An `Up` probe has sent them itself, ahead of the tunnel scheduler.
            TunnelProbe::NoTunnel => {
                let _ = handle.cmd_tx_clone().send(mavlink_proto::handler::MavlinkCommand::ConfigurePrimary);
            }
            TunnelProbe::LinkLost => {
                // The handler already emitted `connection-lost` for this unregistered link (the frontend ignores an
                // unknown linkId) — tear down and fail the connect instead of storing a dead link.
                let _ = handle.stop();
                // The primary's and any secondary discovered during the probe.
                state.drop_link_recorders(link_id);
                msp_tunnel::stats::emit_now(&app_handle);
                return Err("Link lost during MSP tunnel probe".into());
            }
        }
    }
    msp_tunnel::stats::emit_now(&app_handle);

    // The FC's own mode list (AVAILABLE_MODES, ArduPilot 4.7+) — only now that the probe has ruled out
    // INAV, whose HEARTBEAT may carry an ArduPilot identity. The handler starts after a short delay.
    if !handle.inav_tunnel.load(std::sync::atomic::Ordering::Relaxed) && fc_info.fc_variant.starts_with("Ardu") {
        handle.start_mode_list();
    }

    Ok(Opened { protocol: ActiveProtocol::Mavlink(handle), protocol_name: "MAVLink", fc_info, sysid: fc_sysid, compid: fc_compid })
}

fn tunnel_probe_window(version_rtt: Option<std::time::Duration>) -> std::time::Duration {
    use std::time::Duration;
    version_rtt.map_or(Duration::from_secs(2), |r| (r * 3).clamp(Duration::from_secs(2), Duration::from_secs(10)))
}

fn tunnel_reply_timeout(probe_rtt: std::time::Duration) -> std::time::Duration {
    use std::time::Duration;
    (probe_rtt * 3).clamp(Duration::from_secs(2), Duration::from_secs(10))
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

/// Probe for INAV's MSP-over-MAVLink tunnel (D2) and, on success, run the INAV handshake through it
/// and start the scheduler in tunnel mode. Failures are logged at warn.
fn probe_msp_tunnel(
    handle: &mavlink_proto::MavlinkHandle,
    fc_sysid: u8,
    mav_type: u8,
    link_desc: &str,
    version_rtt: Option<std::time::Duration>,
    state: &State<'_, AppState>,
    app_handle: &VehicleEmitter,
) -> TunnelProbe {
    use std::time::{Duration, Instant};

    // Attempts go out at 0, 1/4 and 1/2 of the window.
    let window = tunnel_probe_window(version_rtt);
    let probe_waits = [window / 4, window / 4, window / 2];
    /// After the reply: swallow late duplicate replies of the earlier attempts before the handshake.
    const PROBE_DRAIN: Duration = Duration::from_millis(100);

    log::warn!(
        "MSP tunnel probe: AUTOPILOT_VERSION uid2 is zero or missing — probing INAV MSP over MAVLink (FC sysid {}, up to {} ms; AUTOPILOT_VERSION {})",
        fc_sysid,
        window.as_millis(),
        version_rtt.map_or("not answered".to_string(), |r| format!("answered in {} ms", r.as_millis()))
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
    // Round trip counted from the FIRST attempt: on a slow link the reply to it lands in a later wait.
    let mut answered: Option<(usize, Duration)> = None;
    for (i, wait) in probe_waits.iter().enumerate() {
        match msp.msp_request_timeout(MSP_API_VERSION, &[], wait.as_millis() as u64) {
            Ok(_) => {
                answered = Some((i + 1, started.elapsed()));
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
        log::warn!("MSP tunnel probe: no reply after {} ms (3 attempts) — plain MAVLink", window.as_millis());
        msp_tunnel::stats::set_probe(
            "no_reply",
            None,
            format!("{} ms, 3 attempts (window {} ms)", started.elapsed().as_millis(), window.as_millis()),
        );
        return TunnelProbe::NoTunnel;
    };
    let rtt_ms = rtt.as_millis() as u64;
    let reply_timeout = tunnel_reply_timeout(rtt);
    msp.set_response_timeout(reply_timeout);
    log::warn!(
        "MSP tunnel probe: MSP_API_VERSION answered {} ms after the first attempt (during attempt {}/3) — reply timeout {} ms",
        rtt_ms,
        attempt,
        reply_timeout.as_millis()
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
        // The tunnel scheduler never polls MSP2_ADSB_VEHICLE_LIST, so the "ADS-B from FC (MSP)" source must
        // not be offered on a tunnel link.
        f.adsb_msp = false;
        // Likewise no MSP RC stream on a tunnel link (one request in flight, on demand only); the RC tab is
        // hidden until Stage 3 (Dev-Docs active/MSP_OVER_MAVLINK.md).
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
        format!(
            "attempt {}/3 — INAV {} — window {} ms, reply timeout {} ms",
            attempt,
            fc_info.fc_version,
            window.as_millis(),
            reply_timeout.as_millis()
        ),
    );

    // The connect requests go out now, and one request behind them returns once the uplink has drained:
    // the scheduler's first reads would time out queued behind ~35 commands on a slow link.
    const DRAIN_MAX_MS: u64 = 10_000;
    let _ = handle.cmd_tx_clone().send(mavlink_proto::handler::MavlinkCommand::ConfigurePrimary);
    let drain = Instant::now();
    if let Err(e) = msp.msp_request_timeout(MSP_API_VERSION, &[], DRAIN_MAX_MS) {
        if msp.is_connection_lost() {
            log::warn!("MSP tunnel: MAVLink link lost after the INAV handshake ({})", e);
            return TunnelProbe::LinkLost;
        }
    }
    log::info!("MSP tunnel: connect requests sent, uplink clear after {} ms", drain.elapsed().as_millis());

    // Tunnel-mode scheduler: no polling, no recorder, no RC stream / radar, one request in flight.
    let sched = scheduler::start(
        Box::new(msp),
        TelemetryConfig::default(),
        app_handle.clone(),
        None,
        state.radar_ingest.clone(),
        state.radar_msp_enabled.clone(),
        state.rc_tx.clone(),
        SchedulerMode::Tunnel { reply_timeout },
    );
    log::warn!("MSP tunnel: MSP scheduler started in tunnel mode (on-demand only, 1 request in flight)");
    TunnelProbe::Up(sched, fc_info)
}

/// Reply of the dev-only tunnel request (Debug Monitor → Tunnel tab).
#[derive(serde::Serialize)]
pub struct TunnelMspReply {
    /// Reply payload as spaced hex
    reply_hex: String,
    bytes: usize,
    /// TUNNEL chunks received for this request (as counted by the tunnel transport)
    chunks: u32,
    rtt_ms: u64,
}

/// Dev tool (Debug Monitor → Tunnel): one MSP request with a hex payload through the active MSP
/// scheduler, raw reply back (`0x2048` must return 640 bytes). Debug mode only.
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

/// Passive telemetry path: listen-only, protocol auto-detected, nothing is ever sent. Arm/disarm for
/// the recorder comes from the decoded flight-mode field.
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

    // A minimal FcInfo so the frontend enters the connected state: no firmware (shown "N/A") and the
    // Generic platform, so the map shows the generic arrow rather than a multirotor.
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

/// Disconnect one link, or every link with `None` (the single connect toggle relies on that); the
/// active vehicle moves to a remaining link if its own went away.
#[tauri::command]
pub async fn disconnect(link_id: Option<LinkId>, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    // A running MSP write transaction gets up to 5 s to finish, so the scheduler never stops between its
    // SET requests (half-written RAM config).
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

/// Publish (or drop) a link primary's recorder for the command layer and register it with the group coordinator.
fn store_recorder(state: &State<'_, AppState>, key: &str, rec: &Option<crate::flightlog::recorder::FlightRecorderHandle>) {
    state.set_recorder(key, rec.as_ref());
    if let Some(r) = rec {
        state.groups.register(r);
    }
}

/// Override the connected vehicle's platform type for this session (RAM only): the stored FC info and
/// the recorder, so this and later flights on the link are saved with it.
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

/// The active vehicle's link turned out to carry MSL altitude (the frontend's check at the arm edge): the
/// recorder stores this and later flights on the link as MSL. The stream's own word comes from the handler.
#[tauri::command]
pub fn set_alt_reference_msl(state: State<'_, AppState>) -> Result<(), String> {
    let active_key = {
        let reg = state.links.lock().map_err(|e| e.to_string())?;
        reg.active().ok_or_else(|| crate::vehicle_registry::ERR_NOT_CONNECTED.to_string())?.to_key()
    };
    let recorder = state.recorders.lock().ok().and_then(|m| m.get(&active_key).cloned());
    if let Some(rec) = recorder {
        if let Ok(mut r) = rec.lock() {
            r.set_alt_relative(false);
        }
    }
    log::info!("Altitude reference: MSL by the check at arming");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_hex_payload, tunnel_probe_window, tunnel_reply_timeout};
    use std::time::Duration;

    #[test]
    fn tunnel_waits_follow_the_measured_round_trip() {
        let ms = Duration::from_millis;
        // A fast link and a missing AUTOPILOT_VERSION keep the 2 s floor.
        assert_eq!(tunnel_probe_window(None), ms(2000));
        assert_eq!(tunnel_probe_window(Some(ms(40))), ms(2000));
        assert_eq!(tunnel_reply_timeout(ms(15)), ms(2000));
        // A slow radio link: 2.4 s for AUTOPILOT_VERSION.
        assert_eq!(tunnel_probe_window(Some(ms(2400))), ms(7200));
        assert_eq!(tunnel_reply_timeout(ms(2500)), ms(7500));
        assert_eq!(tunnel_probe_window(Some(ms(2900))), ms(8700));
        assert_eq!(tunnel_reply_timeout(ms(9000)), ms(10000));
    }

    #[test]
    fn hex_payload_parses_with_separators() {
        assert_eq!(parse_hex_payload("0x01 02:0a,FF").unwrap(), vec![0x01, 0x02, 0x0A, 0xFF]);
        assert_eq!(parse_hex_payload("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hex_payload_rejects_non_ascii_without_panicking() {
        // en-dash (3 UTF-8 bytes): must not panic on a char boundary
        assert!(parse_hex_payload("0\u{2013}02").is_err());
        assert!(parse_hex_payload("0–02").is_err());
        assert!(parse_hex_payload("123").is_err());
        assert!(parse_hex_payload("zz").is_err());
    }
}
