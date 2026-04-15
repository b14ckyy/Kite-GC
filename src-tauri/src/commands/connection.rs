// Connection Commands — serial port listing, connect, disconnect

use tauri::State;

use crate::msp::{
    FcInfo, FeatureSet, InavVersion, MSP_API_VERSION, MSP_BOARD_INFO, MSP_FC_VARIANT,
    MSP_FC_VERSION,
};
use crate::msp::features::is_version_supported;
use crate::state::AppState;
use crate::transport::serial::SerialConnection;
use crate::transport::PortInfo;

/// List available serial ports
#[tauri::command]
pub fn list_serial_ports() -> Vec<PortInfo> {
    crate::transport::serial::list_ports()
}

/// Connect to a flight controller on the given serial port.
/// Performs the MSP handshake (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO)
/// and returns the FC info on success.
#[tauri::command]
pub async fn connect(
    port: String,
    baud_rate: u32,
    state: State<'_, AppState>,
) -> Result<FcInfo, String> {
    // Check if already connected
    {
        let conn = state.connection.lock().map_err(|e| e.to_string())?;
        if conn.is_some() {
            return Err("Already connected. Disconnect first.".into());
        }
    }

    // Open serial port
    let mut serial = SerialConnection::open(&port, baud_rate)?;

    // ── MSP Handshake ──────────────────────────────────────────────
    let mut fc_info = FcInfo::default();

    // 1) MSP_API_VERSION → [mspProtocol, apiVersionMajor, apiVersionMinor]
    let resp = serial.msp_request(MSP_API_VERSION, &[])?;
    if resp.payload.len() >= 3 {
        fc_info.msp_protocol = resp.payload[0];
        fc_info.api_version = format!("{}.{}", resp.payload[1], resp.payload[2]);
    }

    // 2) MSP_FC_VARIANT → 4-byte identifier string (e.g. "INAV")
    let resp = serial.msp_request(MSP_FC_VARIANT, &[])?;
    fc_info.fc_variant = String::from_utf8_lossy(&resp.payload).trim().to_string();

    // 3) MSP_FC_VERSION → [major, minor, patch]
    let resp = serial.msp_request(MSP_FC_VERSION, &[])?;
    if resp.payload.len() >= 3 {
        fc_info.fc_version = format!(
            "{}.{}.{}",
            resp.payload[0], resp.payload[1], resp.payload[2]
        );
    }

    // 4) MSP_BOARD_INFO → board identifier (4 bytes) + hw revision (u16 LE)
    let resp = serial.msp_request(MSP_BOARD_INFO, &[])?;
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
            "Unsupported firmware variant: '{}'. INAV GCS only supports INAV.",
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

    log::info!(
        "Connected to {} {} v{} on {} (board: {}, API: {})",
        fc_info.fc_variant,
        fc_info.fc_version,
        fc_info.api_version,
        port,
        fc_info.board_id,
        fc_info.api_version
    );

    // Store connection and FC info
    {
        let mut conn = state.connection.lock().map_err(|e| e.to_string())?;
        *conn = Some(serial);
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
    let mut conn = state.connection.lock().map_err(|e| e.to_string())?;
    if conn.is_none() {
        return Err("Not connected".into());
    }

    // Take the connection out and drop it (closes the port)
    let _ = conn.take();

    // Clear FC info
    let mut info = state.fc_info.lock().map_err(|e| e.to_string())?;
    *info = None;

    log::info!("Disconnected");
    Ok(())
}
