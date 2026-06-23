// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// ArduPilot / PX4 geofence (MAVLink `MAV_MISSION_TYPE_FENCE`). Third "Airspace Manager" safety
// subsystem — the MAVLink counterpart to the INAV geozone editor. Fence geometry = inclusion/exclusion
// × polygon/circle (+ an optional return point) exchanged over the normal mission microprotocol;
// enforcement (enable/action/altitude/radius) is global params (ArduPilot FENCE_* / PX4 GF_*).
// See docs/active/GEOFENCE.md.

use serde::{Deserialize, Serialize};
use tauri::State;

use ::mavlink::ardupilotmega::MavMissionType;

use crate::mavlink_proto::{self, control, params_rt, mission::ArduWaypoint};
use crate::state::{ActiveProtocol, AppState};

// MAV_CMD fence item commands.
const CMD_RETURN_POINT: u16 = 5000;
const CMD_POLY_INCL: u16 = 5001;
const CMD_POLY_EXCL: u16 = 5002;
const CMD_CIRCLE_INCL: u16 = 5003;
const CMD_CIRCLE_EXCL: u16 = 5004;

const KIND_INCLUSION: u8 = 0;
const KIND_EXCLUSION: u8 = 1;
const SHAPE_POLYGON: u8 = 0;
const SHAPE_CIRCLE: u8 = 1;

/// Curated core fence params (read best-effort; only those the FC reports are returned). ArduPilot uses
/// the FENCE_* set, PX4 the GF_* set — we ask for both and keep whatever exists.
const FENCE_PARAM_NAMES: &[&str] = &[
    "FENCE_ENABLE", "FENCE_ACTION", "FENCE_ALT_MAX", "FENCE_ALT_MIN", "FENCE_RADIUS", "FENCE_MARGIN",
    "GF_ACTION", "GF_MAX_HOR_DIST", "GF_MAX_VER_DIST",
];

/// One fence vertex (lat/lon in degrees × 1e7).
#[derive(Serialize, Deserialize, Clone)]
pub struct FenceVertex {
    pub lat: i32,
    pub lon: i32,
}

/// One fence region. `kind` 0 = inclusion, 1 = exclusion. `shape` 0 = polygon, 1 = circle. A circle
/// carries `radius_cm` + a single centre vertex; a polygon carries its corners.
#[derive(Serialize, Deserialize, Clone)]
pub struct FenceZone {
    pub kind: u8,
    pub shape: u8,
    pub radius_cm: Option<u32>,
    pub vertices: Vec<FenceVertex>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FenceParam {
    pub name: String,
    pub value: f32,
}

/// Full geofence snapshot for the frontend.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FenceConfig {
    pub zones: Vec<FenceZone>,
    pub return_point: Option<FenceVertex>,
    pub params: Vec<FenceParam>,
    /// True when a MAVLink (ArduPilot/PX4) FC is connected — drives the UI's visibility.
    pub has_fence: bool,
}

/// Resolve the MAVLink command sender + sysid (fences are MAVLink-only).
fn mav_handle(state: &State<'_, AppState>) -> Result<Option<(std::sync::mpsc::Sender<crate::mavlink_proto::handler::MavlinkCommand>, u8)>, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    match proto.as_ref() {
        Some(ActiveProtocol::Mavlink(h)) => Ok(Some((h.cmd_tx_clone(), h.fc_sysid))),
        _ => Ok(None), // MSP / passive / disconnected → no fence
    }
}

/// Read the fence geometry + core params from the FC. Returns `has_fence=false` (empty) when not on a
/// MAVLink link, so the frontend can always call it on connect.
#[tauri::command(async)]
pub fn fence_read_all(state: State<'_, AppState>) -> Result<FenceConfig, String> {
    let Some((cmd_tx, fc_sysid)) = mav_handle(&state)? else {
        return Ok(FenceConfig::default());
    };
    let items = mavlink_proto::mission::download(&cmd_tx, fc_sysid, false, MavMissionType::MAV_MISSION_TYPE_FENCE)?;
    let (zones, return_point) = decode_fence(&items);
    let pmap = params_rt::read_params(&cmd_tx, fc_sysid, FENCE_PARAM_NAMES);
    let params = FENCE_PARAM_NAMES.iter()
        .filter_map(|n| pmap.get(*n).map(|v| FenceParam { name: (*n).to_string(), value: *v }))
        .collect();
    eprintln!("[FENCE] read {} zone(s), return_point={}, {} params", zones.len(), return_point.is_some(), pmap.len());
    Ok(FenceConfig { zones, return_point, params, has_fence: true })
}

/// "Save to FC": upload the fence geometry (or clear it when empty), then write the provided params.
#[tauri::command(async)]
pub fn fence_write_all(config: FenceConfig, state: State<'_, AppState>) -> Result<(), String> {
    let Some((cmd_tx, fc_sysid)) = mav_handle(&state)? else {
        return Err("FC is not running MAVLink".into());
    };
    let items = encode_fence(&config);
    if items.is_empty() {
        mavlink_proto::mission::clear(&cmd_tx, fc_sysid, MavMissionType::MAV_MISSION_TYPE_FENCE)?;
    } else {
        mavlink_proto::mission::upload(&cmd_tx, fc_sysid, &items, false, MavMissionType::MAV_MISSION_TYPE_FENCE)?;
    }
    for p in &config.params {
        control::set_param(&cmd_tx, fc_sysid, &p.name, p.value)?;
    }
    eprintln!("[FENCE] saved {} zone(s) + {} params to FC", config.zones.len(), config.params.len());
    Ok(())
}

/// Group raw fence MISSION items into zones. Polygon vertices arrive as consecutive items of the same
/// command, each carrying the polygon's total vertex count in `param1`; a circle is one item
/// (`param1` = radius m); command 5000 is the return point.
fn decode_fence(items: &[ArduWaypoint]) -> (Vec<FenceZone>, Option<FenceVertex>) {
    let mut zones: Vec<FenceZone> = Vec::new();
    let mut ret: Option<FenceVertex> = None;
    let mut cur: Option<FenceZone> = None;
    let mut cur_expected = 0usize;

    let flush = |cur: &mut Option<FenceZone>, zones: &mut Vec<FenceZone>| {
        if let Some(z) = cur.take() {
            if z.vertices.len() >= 3 { zones.push(z); }
        }
    };

    for it in items {
        match it.command {
            CMD_POLY_INCL | CMD_POLY_EXCL => {
                let kind = if it.command == CMD_POLY_INCL { KIND_INCLUSION } else { KIND_EXCLUSION };
                let count = (it.param1.round() as i64).max(1) as usize;
                let start_new = !matches!(&cur, Some(z) if z.kind == kind && z.vertices.len() < cur_expected);
                if start_new {
                    flush(&mut cur, &mut zones);
                    cur = Some(FenceZone { kind, shape: SHAPE_POLYGON, radius_cm: None, vertices: Vec::new() });
                    cur_expected = count;
                }
                if let Some(z) = cur.as_mut() {
                    z.vertices.push(FenceVertex { lat: it.lat, lon: it.lon });
                    if z.vertices.len() >= cur_expected {
                        flush(&mut cur, &mut zones);
                    }
                }
            }
            CMD_CIRCLE_INCL | CMD_CIRCLE_EXCL => {
                flush(&mut cur, &mut zones);
                let kind = if it.command == CMD_CIRCLE_INCL { KIND_INCLUSION } else { KIND_EXCLUSION };
                zones.push(FenceZone {
                    kind,
                    shape: SHAPE_CIRCLE,
                    radius_cm: Some((it.param1 * 100.0).round().max(0.0) as u32),
                    vertices: vec![FenceVertex { lat: it.lat, lon: it.lon }],
                });
            }
            CMD_RETURN_POINT => ret = Some(FenceVertex { lat: it.lat, lon: it.lon }),
            _ => {}
        }
    }
    flush(&mut cur, &mut zones);
    (zones, ret)
}

/// Flatten the fence zones back into MISSION items (polygons → N vertex items with the count in param1,
/// circles → one item with the radius in param1), then the return point last.
fn encode_fence(config: &FenceConfig) -> Vec<ArduWaypoint> {
    let mut items: Vec<ArduWaypoint> = Vec::new();
    let item = |command: u16, param1: f32, lat: i32, lon: i32| ArduWaypoint {
        command, frame: 0, param1, param2: 0.0, param3: 0.0, param4: 0.0, lat, lon, alt: 0.0, autocontinue: true,
    };
    for z in &config.zones {
        if z.shape == SHAPE_CIRCLE {
            let Some(c) = z.vertices.first() else { continue };
            let r_m = z.radius_cm.unwrap_or(0) as f32 / 100.0;
            let cmd = if z.kind == KIND_INCLUSION { CMD_CIRCLE_INCL } else { CMD_CIRCLE_EXCL };
            items.push(item(cmd, r_m, c.lat, c.lon));
        } else {
            let n = z.vertices.len();
            if n < 3 { continue; }
            let cmd = if z.kind == KIND_INCLUSION { CMD_POLY_INCL } else { CMD_POLY_EXCL };
            for v in &z.vertices {
                items.push(item(cmd, n as f32, v.lat, v.lon));
            }
        }
    }
    if let Some(rp) = &config.return_point {
        items.push(item(CMD_RETURN_POINT, 0.0, rp.lat, rp.lon));
    }
    items
}
