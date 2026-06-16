// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Connection Commands — serial port listing, connect, disconnect, BLE scanning

use tauri::{AppHandle, Emitter, State};

use crate::flightlog::msp_raw_logger::MspRawSink;
use crate::flightlog::recorder::FlightRecorder;
use crate::flightlog::types::FlightLogSettings;
use crate::mavlink_proto;
use crate::msp::{
    FcInfo, FeatureSet, InavVersion, MspTransport, MSP_API_VERSION, MSP_BOARD_INFO, MSP_FC_VARIANT,
    MSP_FC_VERSION, MSP_NAME, MSP_WP, MSPV2_INAV_MIXER,
};
use crate::msp::features::is_version_supported;
use crate::scheduler;
use crate::scheduler::TelemetryConfig;
use crate::state::{ActiveProtocol, AppState};
use crate::transport::{ByteTransport, PortInfo, Transport, TransportType};
use crate::transport::serial::SerialConnection;
use crate::transport::tcp::TcpTransport;
use crate::transport::udp::UdpTransport;
use crate::transport::ble::BleDeviceInfo;

/// Home position pushed to the frontend (event `home-position`). Same shape/name regardless of
/// protocol so MAVLink (HOME_POSITION) can emit it identically later.
#[derive(serde::Serialize, Clone)]
struct HomeEvent {
    lat: f64,
    lon: f64,
    alt: f64,
}

/// List available serial ports
#[tauri::command]
pub fn list_serial_ports() -> Vec<PortInfo> {
    crate::transport::serial::list_ports()
}

/// Scan for BLE devices matching known serial profiles
#[tauri::command]
pub async fn scan_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
    crate::transport::ble::scan_ble_devices().await
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
        if let Err(e) = crate::transport::ble::run_scan_session(app, rx).await {
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

/// Connect to a flight controller on the given transport and protocol.
/// MSP: Performs handshake + starts telemetry scheduler.
/// MAVLink: Waits for HEARTBEAT + starts handler thread.
#[tauri::command]
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
) -> Result<FcInfo, String> {
    // Check if already connected
    {
        let proto = state.protocol.lock().map_err(|e| e.to_string())?;
        if proto.is_some() {
            return Err("Already connected. Disconnect first.".into());
        }
    }

    let proto = protocol.as_deref().unwrap_or("msp");

    // Open byte-level transport based on type
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
                Box::new(crate::transport::ble::connect_ble_listen(&dev_id, app_handle.clone()).await?)
            } else {
                Box::new(crate::transport::ble::connect_ble(&dev_id).await?)
            }
        }
    };

    log::info!("Transport opened, protocol={}", proto);

    let fc_info = match proto {
        "mavlink" => {
            // ── MAVLink Path ─────────────────────────────────────────────
            connect_mavlink(
                byte_transport,
                attitude_rate_hz,
                position_rate_hz,
                airspeed_enabled,
                mavlink_full_telemetry,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                flight_log_raw,
                flight_log_raw_always,
                state,
                app_handle,
            )?
        }
        "telemetry" => {
            // ── Passive Telemetry Path (listen-only, auto-detect) ────────
            connect_passive_telemetry(
                byte_transport,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                state,
                app_handle,
            )?
        }
        _ => {
            // ── MSP Path ────────────────────────────────────────────────
            connect_msp(
                byte_transport,
                attitude_rate_hz,
                position_rate_hz,
                airspeed_enabled,
                flight_log_enabled,
                flight_log_db_enabled,
                flight_log_path,
                flight_log_raw_path,
                flight_log_raw,
                flight_log_raw_always,
                state,
                app_handle,
            )?
        }
    };

    Ok(fc_info)
}

/// MSP connection path: handshake → scheduler
fn connect_msp(
    byte_transport: Box<dyn ByteTransport>,
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<FcInfo, String> {
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

    // ── MSP Handshake ──────────────────────────────────────────────
    let mut fc_info = FcInfo::default();

    // 1) MSP_API_VERSION → [mspProtocol, apiVersionMajor, apiVersionMinor]
    let resp = transport.msp_request(MSP_API_VERSION, &[])?;
    if resp.payload.len() >= 3 {
        fc_info.msp_protocol = resp.payload[0];
        fc_info.api_version = format!("{}.{}", resp.payload[1], resp.payload[2]);
    }

    // 2) MSP_FC_VARIANT → 4-byte identifier string (e.g. "INAV")
    let resp = transport.msp_request(MSP_FC_VARIANT, &[])?;
    fc_info.fc_variant = String::from_utf8_lossy(&resp.payload).trim().to_string();

    // 3) MSP_FC_VERSION → [major, minor, patch]
    let resp = transport.msp_request(MSP_FC_VERSION, &[])?;
    if resp.payload.len() >= 3 {
        fc_info.fc_version = format!(
            "{}.{}.{}",
            resp.payload[0], resp.payload[1], resp.payload[2]
        );
    }

    // 4) MSP_BOARD_INFO → board identifier (4 bytes) + hw revision (u16 LE)
    let resp = transport.msp_request(MSP_BOARD_INFO, &[])?;
    if resp.payload.len() >= 4 {
        fc_info.board_id = String::from_utf8_lossy(&resp.payload[..4])
            .trim()
            .to_string();
    }
    if resp.payload.len() >= 6 {
        fc_info.hardware_revision =
            (resp.payload[4] as u16) | ((resp.payload[5] as u16) << 8);
    }

    // ── Version check & feature detection ────────────────────────────
    if fc_info.fc_variant != "INAV" {
        return Err(format!(
            "Unsupported firmware variant: '{}'. Only INAV is currently supported.",
            fc_info.fc_variant
        ));
    }

    let version = InavVersion::parse(&fc_info.fc_version).ok_or_else(|| {
        format!("Cannot parse firmware version: '{}'", fc_info.fc_version)
    })?;

    if !is_version_supported(version) {
        return Err(format!(
            "INAV {} is not supported. Minimum required version is 7.0.0.",
            version
        ));
    }

    let feature_set = FeatureSet::for_version(version);
    log::info!(
        "Feature gates for INAV {}: autoland={}, geozones={}, msp_rc={}, aux_rc={}",
        version,
        feature_set.autoland_config,
        feature_set.geozones,
        feature_set.msp_rc,
        feature_set.aux_rc
    );
    fc_info.features = Some(feature_set);

    // 5) MSP2_INAV_MIXER → platform type and mixer preset
    match transport.msp_request(MSPV2_INAV_MIXER, &[]) {
        Ok(resp) => {
            if resp.payload.len() >= 7 {
                fc_info.platform_type = resp.payload[3];
                fc_info.mixer_preset =
                    (resp.payload[5] as i16) | ((resp.payload[6] as i16) << 8);
            }
        }
        Err(e) => {
            log::warn!("Failed to query mixer config: {}", e);
        }
    }

    // 6) MSP_NAME → craft name configured in the FC
    match transport.msp_request(MSP_NAME, &[]) {
        Ok(resp) => {
            fc_info.craft_name = String::from_utf8_lossy(&resp.payload).trim().to_string();
        }
        Err(e) => {
            log::warn!("Failed to query craft name: {}", e);
        }
    }

    // 7) Home position — MSP_WP #0 is INAV's RTH home (GPS_home, lat/lon in deg·1e7). One-shot at
    //    connect so a mid-flight connect / app restart recovers Home; the live arm-transition path
    //    only sets it when we actually witness the arm. Raw-parse the 21-byte WP payload (the home
    //    WP's action byte isn't a normal nav action, so we don't go through decode_wp). lat==lon==0
    //    means no home is set yet (on the ground, pre-arm) → skip; arm will set it live.
    match transport.msp_request(MSP_WP, &[0]) {
        Ok(resp) if resp.payload.len() >= 14 => {
            let p = &resp.payload;
            let lat_e7 = i32::from_le_bytes([p[2], p[3], p[4], p[5]]);
            let lon_e7 = i32::from_le_bytes([p[6], p[7], p[8], p[9]]);
            let alt_cm = i32::from_le_bytes([p[10], p[11], p[12], p[13]]);
            if lat_e7 != 0 || lon_e7 != 0 {
                let home = HomeEvent {
                    lat: lat_e7 as f64 / 1e7,
                    lon: lon_e7 as f64 / 1e7,
                    alt: alt_cm as f64 / 100.0,
                };
                log::info!("Home from FC (MSP_WP 0): {:.7}, {:.7}", home.lat, home.lon);
                let _ = app_handle.emit("home-position", home);
            } else {
                log::info!("MSP_WP(0): no home set on FC yet");
            }
        }
        Ok(_) => log::warn!("MSP_WP(0) home response too short"),
        Err(e) => log::warn!("Failed to query home (MSP_WP 0): {}", e),
    }

    let transport_desc = transport.description();
    log::info!(
        "Connected to {} {} v{} via {} (board: {}, API: {}, platform: {})",
        fc_info.fc_variant,
        fc_info.fc_version,
        fc_info.api_version,
        transport_desc,
        fc_info.board_id,
        fc_info.api_version,
        fc_info.platform_type,
    );

    // ── Start telemetry scheduler ────────────────────────────────────────
    let config = TelemetryConfig {
        attitude_rate_hz: attitude_rate_hz.unwrap_or(5.0),
        position_rate_hz: position_rate_hz.unwrap_or(2.0),
        airspeed_enabled: airspeed_enabled.unwrap_or(false),
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

        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "MSP", portable, app_handle.clone(), state.pending_session.clone(), state.resume_pending.clone(), msp_raw_sink.clone()) {
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

    let handle = scheduler::start(
        Box::new(transport),
        config,
        app_handle,
        recorder_handle,
        state.radar_ingest.clone(),
        state.radar_msp_enabled.clone(),
    );

    // Store MSP scheduler handle and FC info
    {
        let mut proto = state.protocol.lock().map_err(|e| e.to_string())?;
        *proto = Some(ActiveProtocol::Msp(handle));
    }
    {
        let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
        *info = Some(fc_info.clone());
    }

    Ok(fc_info)
}

/// MAVLink connection path: handshake → handler
#[allow(clippy::too_many_arguments)]
fn connect_mavlink(
    mut byte_transport: Box<dyn ByteTransport>,
    attitude_rate_hz: Option<f64>,
    position_rate_hz: Option<f64>,
    airspeed_enabled: Option<bool>,
    mavlink_full_telemetry: Option<bool>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<FcInfo, String> {
    // MAVLink handshake: wait for FC HEARTBEAT, send GCS HEARTBEAT back
    let (fc_info, fc_sysid) = mavlink_proto::perform_handshake(&mut *byte_transport)?;

    log::info!(
        "MAVLink connected: {} (sysid={}) via {}",
        fc_info.fc_variant,
        fc_sysid,
        byte_transport.description(),
    );

    // Configure telemetry stream rates (ADR-043) — mirrors the MSP poll-rate knobs. Skipped when
    // "Full MAVLink Telemetry" is on, so the FC streams purely per its own SRn_* params (.tlog gets
    // everything). Applied here, before the handler thread starts, while we still own the transport.
    if mavlink_full_telemetry.unwrap_or(false) {
        // SET_MESSAGE_INTERVAL is sticky on the FC until reboot, so a prior reduced session would
        // otherwise keep the link narrow. Reset our managed messages to the FC's SRn defaults.
        mavlink_proto::streamrates::reset_stream_rates(&mut *byte_transport, fc_sysid);
    } else {
        mavlink_proto::streamrates::apply_stream_rates(
            &mut *byte_transport,
            fc_sysid,
            attitude_rate_hz.unwrap_or(5.0),
            position_rate_hz.unwrap_or(2.0),
            airspeed_enabled.unwrap_or(false),
        );
    }

    // Ask the FC which EKF core is active (AHRS_EKF_TYPE) for the header EKF indicator. One-shot,
    // fire-and-forget — the PARAM_VALUE reply is decoded by the handler thread once it starts.
    mavlink_proto::params::request_ekf_type(&mut *byte_transport, fc_sysid);

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

        // MAVLink records via .tlog; the MSP raw sink is unused here (kept empty).
        let msp_raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "MAVLink", portable, app_handle.clone(), state.pending_session.clone(), state.resume_pending.clone(), msp_raw_sink) {
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

    // Start the MAVLink handler thread
    let handle = mavlink_proto::handler::start(byte_transport, fc_sysid, fc_info.fc_variant.clone(), app_handle, recorder_handle);

    // Store MAVLink handle and FC info
    {
        let mut proto = state.protocol.lock().map_err(|e| e.to_string())?;
        *proto = Some(ActiveProtocol::Mavlink(handle));
    }
    {
        let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
        *info = Some(fc_info.clone());
    }

    Ok(fc_info)
}

/// Passive telemetry path: no handshake — start the listen-only handler immediately.
/// The wire protocol is auto-detected by the handler; nothing is ever transmitted. When flight logging
/// is enabled, a recorder is attached and fed the decoded telemetry (arm/disarm derived from the FC's
/// flight-mode field — e.g. FrSky MODES).
fn connect_passive_telemetry(
    byte_transport: Box<dyn ByteTransport>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
    flight_log_raw_path: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<FcInfo, String> {
    log::info!(
        "Passive telemetry connect (listen-only) via {}",
        byte_transport.description()
    );

    // Synthesize a minimal FcInfo so the frontend enters the connected state. Passive telemetry carries
    // no FC identity (no handshake) — leave firmware empty (shown as "N/A") and use the Generic platform
    // (255) so the map shows the generic arrow rather than defaulting to a multirotor.
    let mut fc_info = FcInfo::default();
    fc_info.platform_type = 255; // PLATFORM_GENERIC

    // Flight recorder (no raw byte log on this path — FrSky has no MSP raw stream).
    let flight_log_settings = FlightLogSettings {
        enabled: flight_log_enabled.unwrap_or(false),
        db_enabled: flight_log_db_enabled.unwrap_or(false),
        db_path: flight_log_path.unwrap_or_default(),
        raw_log_path: flight_log_raw_path.unwrap_or_default(),
        raw_enabled: false,
        raw_always: false,
    };

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);
        let msp_raw_sink: MspRawSink = std::sync::Arc::new(std::sync::Mutex::new(None));
        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "Telemetry", portable, app_handle.clone(), state.pending_session.clone(), state.resume_pending.clone(), msp_raw_sink) {
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

    let handle = crate::passive_telemetry::start(byte_transport, app_handle, recorder_handle);

    {
        let mut proto = state.protocol.lock().map_err(|e| e.to_string())?;
        *proto = Some(ActiveProtocol::PassiveTelemetry(handle));
    }
    {
        let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
        *info = Some(fc_info.clone());
    }

    Ok(fc_info)
}

/// Disconnect from the flight controller
#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    let mut proto = state.protocol.lock().map_err(|e| e.to_string())?;
    if proto.is_none() {
        return Err("Not connected".into());
    }

    // Stop the active protocol handler
    match proto.take() {
        Some(ActiveProtocol::Msp(handle)) => {
            let _transport = handle.stop(); // transport dropped here
            log::info!("MSP scheduler stopped");
        }
        Some(ActiveProtocol::Mavlink(handle)) => {
            let _transport = handle.stop(); // transport dropped here
            log::info!("MAVLink handler stopped");
        }
        Some(ActiveProtocol::PassiveTelemetry(handle)) => {
            let _transport = handle.stop(); // transport dropped here
            log::info!("Passive telemetry handler stopped");
        }
        None => {}
    }

    // Clear FC info
    let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
    *info = None;

    log::info!("Disconnected");
    Ok(())
}
