// Connection Commands — serial port listing, connect, disconnect, BLE scanning

use tauri::{AppHandle, State};

use crate::flightlog::recorder::FlightRecorder;
use crate::flightlog::types::FlightLogSettings;
use crate::mavlink_proto;
use crate::msp::{
    FcInfo, FeatureSet, InavVersion, MspTransport, MSP_API_VERSION, MSP_BOARD_INFO, MSP_FC_VARIANT,
    MSP_FC_VERSION, MSP_NAME, MSPV2_INAV_MIXER,
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
    // Flight log config
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
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

    let use_mavlink = protocol.as_deref() == Some("mavlink");

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
            Box::new(crate::transport::ble::connect_ble(&dev_id).await?)
        }
    };

    log::info!("Transport opened, protocol={}", if use_mavlink { "MAVLink" } else { "MSP" });

    let fc_info = if use_mavlink {
        // ── MAVLink Path ─────────────────────────────────────────────
        connect_mavlink(
            byte_transport,
            flight_log_enabled,
            flight_log_db_enabled,
            flight_log_path,
            flight_log_raw,
            flight_log_raw_always,
            state,
            app_handle,
        )?
    } else {
        // ── MSP Path ────────────────────────────────────────────────
        connect_msp(
            byte_transport,
            attitude_rate_hz,
            position_rate_hz,
            airspeed_enabled,
            flight_log_enabled,
            flight_log_db_enabled,
            flight_log_path,
            flight_log_raw,
            flight_log_raw_always,
            state,
            app_handle,
        )?
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
    flight_log_raw: Option<bool>,
    flight_log_raw_always: Option<bool>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<FcInfo, String> {
    // Wrap in MSP protocol layer (adds MSP v2 framing + response parser)
    let mut transport = MspTransport::new(byte_transport);

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
        raw_enabled: flight_log_raw.unwrap_or(false),
        raw_always: flight_log_raw_always.unwrap_or(false),
    };

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);

        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "MSP", portable, app_handle.clone()) {
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
fn connect_mavlink(
    mut byte_transport: Box<dyn ByteTransport>,
    flight_log_enabled: Option<bool>,
    flight_log_db_enabled: Option<bool>,
    flight_log_path: Option<String>,
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

    // ── Flight recorder setup ────────────────────────────────────────────
    let flight_log_settings = FlightLogSettings {
        enabled: flight_log_enabled.unwrap_or(false),
        db_enabled: flight_log_db_enabled.unwrap_or(false),
        db_path: flight_log_path.unwrap_or_default(),
        raw_enabled: flight_log_raw.unwrap_or(false),
        raw_always: flight_log_raw_always.unwrap_or(false),
    };

    let recorder_handle = if flight_log_settings.enabled {
        let portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
            .unwrap_or(false);

        match FlightRecorder::new(flight_log_settings, fc_info.clone(), "MAVLink", portable, app_handle.clone()) {
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
    let handle = mavlink_proto::handler::start(byte_transport, fc_sysid, app_handle, recorder_handle);

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
        None => {}
    }

    // Clear FC info
    let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
    *info = None;

    log::info!("Disconnected");
    Ok(())
}
