// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle / link commands — the multi-vehicle surface next to `connection::{connect, disconnect}`:
// list the open links and pick which vehicle the singleton UI (widgets, control panel, untargeted
// commands) follows. See docs/02-design/features/multi-vehicle.design.md §3.1.

use tauri::{AppHandle, Emitter, State};

use crate::commands::connection::ActiveVehicleChanged;
use crate::flightlog::group::GroupDebugState;
use crate::state::AppState;
use crate::state::ActiveProtocol;
use crate::vehicle_registry::{LinkSummary, VehicleId, VehicleInfo};

/// Every open link with its protocol, transport and handshake info.
#[tauri::command]
pub fn list_links(state: State<'_, AppState>) -> Result<Vec<LinkSummary>, String> {
    let reg = state.links.lock().map_err(|e| e.to_string())?;
    Ok(reg.summaries())
}

/// The active vehicle's key (`"L1:S1"`), or `None` when nothing is connected.
#[tauri::command]
pub fn get_active_vehicle(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let reg = state.links.lock().map_err(|e| e.to_string())?;
    Ok(reg.active().map(|v| v.to_key()))
}

/// Make `vehicle_id` the active vehicle. The link must be open; for MAVLink the sysid may be one the
/// handler discovered after the handshake (those never enter the registry). Emits
/// `active-vehicle-changed` so every frontend consumer re-targets at once.
#[tauri::command]
pub fn set_active_vehicle(vehicle_id: String, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let vid = VehicleId::parse(&vehicle_id).ok_or_else(|| format!("Invalid vehicle id '{vehicle_id}'"))?;
    {
        let mut reg = state.links.lock().map_err(|e| e.to_string())?;
        if reg.active() == Some(&vid) {
            return Ok(());
        }
        reg.set_active(vid.clone())?;
    }
    state.retarget_rc(Some(&vid)); // RC stream follows the active vehicle — disengaged, re-engage explicitly
    log::info!("Active vehicle → {}", vid);
    let _ = app_handle.emit("active-vehicle-changed", ActiveVehicleChanged { vehicle_id: Some(vid.to_key()) });
    Ok(())
}

/// Debug Monitor runtime toggle of the multi-vehicle feature gate (normally the `--fleet` / `KITE_FLEET`
/// launch flag, `fleet_mode`). Refused in a release build — `--debug` included: a tester must not reach a
/// half-finished feature. Off: no secondary-vehicle discovery from now on and the group coordinator never
/// forms a group flight; vehicles already discovered stay, and a group already running records on to its
/// normal end — it is never cut mid-flight.
#[tauri::command]
pub fn set_fleet_enabled(on: bool, state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("The fleet features are switched by the --fleet launch flag".into());
    }
    if crate::fleet_mode::enabled() != on {
        crate::fleet_mode::set(on);
        log::warn!("Fleet features {} (Debug Monitor)", if on { "enabled" } else { "disabled" });
    }
    // Off while a secondary vehicle is active: every untargeted command would now be refused — fall back
    // to its link's primary.
    if !on {
        let back = {
            let mut reg = state.links.lock().map_err(|e| e.to_string())?;
            let primary = reg.active_entry().map(|e| e.primary.clone());
            match primary {
                Some(p) if reg.active() != Some(&p) => {
                    reg.set_active(p.clone())?;
                    Some(p)
                }
                _ => None,
            }
        };
        if let Some(p) = back {
            state.retarget_rc(Some(&p));
            log::info!("Active vehicle → {} (fleet features off)", p);
            let _ = app_handle.emit("active-vehicle-changed", ActiveVehicleChanged { vehicle_id: Some(p.to_key()) });
        }
    }
    Ok(())
}

/// Debug Monitor ("Fleet (gated)" tab): the group-flight coordinator's live state — registered
/// recorders, the running group with its members, the ended groups awaiting their store prompt.
/// Read-only.
#[tauri::command]
pub fn debug_group_state(state: State<'_, AppState>) -> Result<GroupDebugState, String> {
    Ok(state.groups.debug_state())
}

/// Re-announce every known vehicle (`vehicle-discovered`), so a frontend that (re)loaded after the
/// live announcements can rebuild its vehicle list. The primary always comes from the registry entry
/// (its identity reflects the MSP tunnel probe); MAVLink handlers add the secondaries they discovered
/// on a shared link.
#[tauri::command]
pub fn announce_vehicles(state: State<'_, AppState>, app_handle: AppHandle) -> Result<(), String> {
    let reg = state.links.lock().map_err(|e| e.to_string())?;
    for entry in reg.iter() {
        let _ = app_handle.emit("vehicle-discovered", VehicleInfo::primary_of(entry, 0));
        if let ActiveProtocol::Mavlink(h) = &entry.protocol {
            h.announce_vehicles();
        }
    }
    Ok(())
}
