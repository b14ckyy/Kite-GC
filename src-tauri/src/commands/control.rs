// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle-Control Commands — Tauri command handlers for direct GCS control of a MAVLink vehicle
// (ArduPilot + PX4): flight-mode switch, arm/disarm, takeoff/land/RTL, Guided reposition, speed,
// and mission start/pause/set-current. Each fires a MAVLink command and waits for its COMMAND_ACK.
//
// These are MAVLink-only (INAV guided steering is a later phase). The mode `custom_mode` encoding is
// firmware-specific and computed frontend-side (ArduPilot: flat mode number in `main`, `sub`=0; PX4:
// packed `main`/`sub`) — the backend just forwards the params. See docs/active/VEHICLE_CONTROL.md.

use std::sync::mpsc;
use tauri::State;

use ::mavlink::ardupilotmega::{MavCmd, MavFrame};

use crate::mavlink_proto::control;
use crate::mavlink_proto::handler::MavlinkCommand;
use crate::state::{ActiveProtocol, AppState};

/// ArduPilot's "force" magic for COMMAND_ARM_DISARM param2 — bypasses pre-arm checks.
const ARM_FORCE_MAGIC: f32 = 21196.0;

/// Resolve the active MAVLink handle to (command channel, FC system id), or an error if the active
/// protocol is not MAVLink. Holds the protocol mutex only briefly; the command exchange runs after.
fn mav_handle(state: &State<'_, AppState>) -> Result<(mpsc::Sender<MavlinkCommand>, u8), String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    match proto.as_ref() {
        Some(ActiveProtocol::Mavlink(h)) => Ok((h.cmd_tx_clone(), h.fc_sysid)),
        Some(_) => Err("FC is not running MAVLink".into()),
        None => Err("Not connected".into()),
    }
}

/// Set the flight mode via `MAV_CMD_DO_SET_MODE`. `main`/`sub` are the firmware-specific custom-mode
/// parts (ArduPilot: `main` = flat mode number, `sub` = 0; PX4: packed main/sub mode). param1 is the
/// base mode with `MAV_MODE_FLAG_CUSTOM_MODE_ENABLED` (bit 0) set.
#[tauri::command(async)]
pub fn mav_set_mode(main: u32, sub: u32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_SET_MODE,
        [1.0, main as f32, sub as f32, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Arm or disarm via `MAV_CMD_COMPONENT_ARM_DISARM`. `force` bypasses pre-arm checks (use with care).
#[tauri::command(async)]
pub fn mav_arm(arm: bool, force: bool, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_COMPONENT_ARM_DISARM,
        [if arm { 1.0 } else { 0.0 }, if force { ARM_FORCE_MAGIC } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Take off to `altitude` (m, relative to home) via `MAV_CMD_NAV_TAKEOFF`.
#[tauri::command(async)]
pub fn mav_takeoff(altitude: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param7 = altitude; lat/lon (param5/6) = 0 → take off in place.
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_NAV_TAKEOFF,
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, altitude],
    )
}

/// Land in place via `MAV_CMD_NAV_LAND`.
#[tauri::command(async)]
pub fn mav_land(state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(&cmd_tx, fc_sysid, MavCmd::MAV_CMD_NAV_LAND, [0.0; 7])
}

/// Return to launch via `MAV_CMD_NAV_RETURN_TO_LAUNCH`.
#[tauri::command(async)]
pub fn mav_rtl(state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(&cmd_tx, fc_sysid, MavCmd::MAV_CMD_NAV_RETURN_TO_LAUNCH, [0.0; 7])
}

/// Guided "fly here" via `MAV_CMD_DO_REPOSITION` (COMMAND_INT for lat/lon precision). `lat`/`lon` are
/// degrees × 1e7; `alt` is metres relative to home. Optional `ground_speed` (m/s; default if None),
/// `yaw` (deg; keep current if None — multirotor only), `loiter_radius` (m; fixed-wing only).
#[tauri::command(async)]
pub fn mav_reposition(
    lat: i32,
    lon: i32,
    alt: f32,
    ground_speed: Option<f32>,
    yaw: Option<f32>,
    loiter_radius: Option<f32>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = ground speed (-1 = default). param2 = bitmask (unused = 0). param3 = loiter radius
    // (0 = default). param4 = yaw heading (NaN = keep current).
    let params = [
        ground_speed.unwrap_or(-1.0),
        0.0,
        loiter_radius.unwrap_or(0.0),
        yaw.unwrap_or(f32::NAN),
    ];
    control::send_command_int(
        &cmd_tx,
        fc_sysid,
        MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT,
        MavCmd::MAV_CMD_DO_REPOSITION,
        params,
        lat,
        lon,
        alt,
    )
}

/// Change target speed via `MAV_CMD_DO_CHANGE_SPEED`. `speed_type`: 0 = airspeed, 1 = groundspeed.
#[tauri::command(async)]
pub fn mav_change_speed(speed_type: u8, speed: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = speed type, param2 = speed (m/s), param3 = throttle (-1 = no change).
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_CHANGE_SPEED,
        [speed_type as f32, speed, -1.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Start the loaded mission via `MAV_CMD_MISSION_START`.
#[tauri::command(async)]
pub fn mav_mission_start(state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(&cmd_tx, fc_sysid, MavCmd::MAV_CMD_MISSION_START, [0.0; 7])
}

/// Pause (`pause = true`) or resume the mission via `MAV_CMD_DO_PAUSE_CONTINUE`.
#[tauri::command(async)]
pub fn mav_mission_pause(pause: bool, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = 0 → pause (hold), 1 → continue.
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_PAUSE_CONTINUE,
        [if pause { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Jump the mission to item `seq` via `MAV_CMD_DO_SET_MISSION_CURRENT`.
#[tauri::command(async)]
pub fn mav_mission_set_current(seq: u16, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_SET_MISSION_CURRENT,
        [seq as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Set the home position to the vehicle's current location via `MAV_CMD_DO_SET_HOME` (param1 = 1).
#[tauri::command(async)]
pub fn mav_set_home_here(state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_SET_HOME,
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Abort a landing / go around via `MAV_CMD_DO_GO_AROUND` (param1 = climb altitude, 0 = default).
#[tauri::command(async)]
pub fn mav_abort_landing(altitude: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_GO_AROUND,
        [altitude, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Command a VTOL transition via `MAV_CMD_DO_VTOL_TRANSITION` (PX4). `to_fw` = true → forward/
/// fixed-wing flight (MAV_VTOL_STATE_FW = 4), false → hover/multicopter (MAV_VTOL_STATE_MC = 3).
/// (ArduPlane transitions by mode switch instead — handled in the controller.)
#[tauri::command(async)]
pub fn mav_vtol_transition(to_fw: bool, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_DO_VTOL_TRANSITION,
        [if to_fw { 4.0 } else { 3.0 }, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}

/// Set a single FC parameter (e.g. the fixed-wing loiter radius `WP_LOITER_RAD`). Fire-and-forget.
#[tauri::command(async)]
pub fn mav_set_param(name: String, value: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    control::set_param(&cmd_tx, fc_sysid, &name, value)
}

/// Set the Guided target course/heading for a fixed-wing via `MAV_CMD_GUIDED_CHANGE_HEADING` — the
/// plane flies this bearing continuously (not an orbit). `heading` is degrees (0–359). We command
/// course-over-ground (HEADING_TYPE 0), which is the direction the aircraft actually tracks.
///
/// ArduPlane only handles this command as a COMMAND_INT (in `handle_command_int_guided_slew_commands`)
/// — sent as COMMAND_LONG it is ack'd but not executed. So we must use COMMAND_INT here.
#[tauri::command(async)]
pub fn mav_guided_change_heading(heading: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = HEADING_TYPE (0 = course-over-ground), param2 = heading deg, param3 = rate (0 = default).
    // No position payload → x/y/z = 0, frame irrelevant (GLOBAL).
    control::send_command_int(
        &cmd_tx,
        fc_sysid,
        MavFrame::MAV_FRAME_GLOBAL,
        MavCmd::MAV_CMD_GUIDED_CHANGE_HEADING,
        [0.0, heading, 0.0, 0.0],
        0,
        0,
        0.0,
    )
}

/// Clear an active fixed-wing Guided heading override (HEADING_TYPE_DEFAULT → GUIDED_HEADING_NONE),
/// so the plane resumes waypoint navigation. ArduPlane's heading slew is sticky and is NOT cleared by
/// DO_REPOSITION in current firmware — only by a mode change or this explicit reset. See
/// docs/active/VEHICLE_CONTROL.md.
#[tauri::command(async)]
pub fn mav_guided_clear_heading(state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = 2 (HEADING_TYPE_DEFAULT) → clears the heading controller; other params ignored.
    control::send_command_int(
        &cmd_tx,
        fc_sysid,
        MavFrame::MAV_FRAME_GLOBAL,
        MavCmd::MAV_CMD_GUIDED_CHANGE_HEADING,
        [2.0, 0.0, 0.0, 0.0],
        0,
        0,
        0.0,
    )
}

/// Point the nose to an absolute heading for a multirotor via `MAV_CMD_CONDITION_YAW` (Guided).
/// `heading` is degrees (0–359).
#[tauri::command(async)]
pub fn mav_condition_yaw(heading: f32, state: State<'_, AppState>) -> Result<(), String> {
    let (cmd_tx, fc_sysid) = mav_handle(&state)?;
    // param1 = target angle deg, param2 = yaw rate (0 = default), param3 = direction (0 = shortest), param4 = absolute (0).
    control::send_command_long(
        &cmd_tx,
        fc_sysid,
        MavCmd::MAV_CMD_CONDITION_YAW,
        [heading, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
}
