// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// ArduPilot / PX4 rally points (MAVLink `MAV_MISSION_TYPE_RALLY`). Companion to the geofence
// (commands/fence.rs) — the same mission microprotocol, a different mission_type. Rally points are RTL
// divert/return locations: on RTL the vehicle returns to the nearest rally point (within RALLY_LIMIT_KM)
// instead of home. Geometry is just points (lat/lon/alt); enforcement is the global RALLY_* params
// (ArduPilot only; PX4 has "safe points" via the same items but no RALLY_* params). See docs/active/GEOFENCE.md.

use serde::{Deserialize, Serialize};
use tauri::State;

use ::mavlink::ardupilotmega::MavMissionType;

use crate::mavlink_proto::{self, control, params_rt, mission::ArduWaypoint};
use crate::state::{ActiveProtocol, AppState};

// MAV_CMD for a rally point.
const CMD_RALLY_POINT: u16 = 5100;
// MAV_FRAME_GLOBAL_RELATIVE_ALT — rally altitude is relative to home (ArduPilot convention).
const FRAME_GLOBAL_REL_ALT: u8 = 3;

/// Curated rally params (read best-effort; only those the FC reports are returned). ArduPilot only.
const RALLY_PARAM_NAMES: &[&str] = &["RALLY_LIMIT_KM", "RALLY_INCL_HOME"];

/// One rally point: lat/lon in degrees × 1e7, altitude in cm (relative to home).
#[derive(Serialize, Deserialize, Clone)]
pub struct RallyPoint {
    pub lat: i32,
    pub lon: i32,
    pub alt_cm: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RallyParam {
    pub name: String,
    pub value: f32,
}

/// Full rally snapshot for the frontend.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct RallyConfig {
    pub points: Vec<RallyPoint>,
    pub params: Vec<RallyParam>,
    /// True when a MAVLink (ArduPilot/PX4) FC is connected — drives the UI's visibility.
    pub has_rally: bool,
}

/// Resolve the MAVLink command sender + sysid (rally is MAVLink-only).
fn mav_handle(state: &State<'_, AppState>) -> Result<Option<(std::sync::mpsc::Sender<crate::mavlink_proto::handler::MavlinkCommand>, u8)>, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    match proto.as_ref() {
        Some(ActiveProtocol::Mavlink(h)) => Ok(Some((h.cmd_tx_clone(), h.fc_sysid))),
        _ => Ok(None), // MSP / passive / disconnected → no rally
    }
}

/// Read the rally points + core params from the FC. Returns `has_rally=false` (empty) when not on a
/// MAVLink link, so the frontend can always call it on connect.
#[tauri::command(async)]
pub fn rally_read_all(state: State<'_, AppState>) -> Result<RallyConfig, String> {
    let Some((cmd_tx, fc_sysid)) = mav_handle(&state)? else {
        return Ok(RallyConfig::default());
    };
    let items = mavlink_proto::mission::download(&cmd_tx, fc_sysid, false, MavMissionType::MAV_MISSION_TYPE_RALLY, |_, _| {})?;
    let points = decode_rally(&items);
    let pmap = params_rt::read_params(&cmd_tx, fc_sysid, RALLY_PARAM_NAMES);
    let params = RALLY_PARAM_NAMES.iter()
        .filter_map(|n| pmap.get(*n).map(|v| RallyParam { name: (*n).to_string(), value: *v }))
        .collect();
    eprintln!("[RALLY] read {} point(s), {} params", points.len(), pmap.len());
    Ok(RallyConfig { points, params, has_rally: true })
}

/// "Save to FC": upload the rally points (or clear them when empty), then write the provided params.
#[tauri::command(async)]
pub fn rally_write_all(config: RallyConfig, state: State<'_, AppState>) -> Result<(), String> {
    let Some((cmd_tx, fc_sysid)) = mav_handle(&state)? else {
        return Err("FC is not running MAVLink".into());
    };
    let items = encode_rally(&config);
    if items.is_empty() {
        mavlink_proto::mission::clear(&cmd_tx, fc_sysid, MavMissionType::MAV_MISSION_TYPE_RALLY)?;
    } else {
        mavlink_proto::mission::upload(&cmd_tx, fc_sysid, &items, false, MavMissionType::MAV_MISSION_TYPE_RALLY)?;
    }
    for p in &config.params {
        control::set_param(&cmd_tx, fc_sysid, &p.name, p.value)?;
    }
    eprintln!("[RALLY] saved {} point(s) + {} params to FC", config.points.len(), config.params.len());
    Ok(())
}

/// Each rally MISSION item is one point (`MAV_CMD_NAV_RALLY_POINT`, x/y = lat/lon, z = alt m).
fn decode_rally(items: &[ArduWaypoint]) -> Vec<RallyPoint> {
    items.iter()
        .filter(|it| it.command == CMD_RALLY_POINT)
        .map(|it| RallyPoint { lat: it.lat, lon: it.lon, alt_cm: (it.alt * 100.0).round() as i32 })
        .collect()
}

/// Flatten the rally points back into MISSION items.
fn encode_rally(config: &RallyConfig) -> Vec<ArduWaypoint> {
    config.points.iter().map(|p| ArduWaypoint {
        command: CMD_RALLY_POINT, frame: FRAME_GLOBAL_REL_ALT,
        param1: 0.0, param2: 0.0, param3: 0.0, param4: 0.0,
        lat: p.lat, lon: p.lon, alt: p.alt_cm as f32 / 100.0, autocontinue: true,
    }).collect()
}
