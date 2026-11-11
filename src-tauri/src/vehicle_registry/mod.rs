// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Vehicle / link registry — the multi-vehicle replacement for the single `protocol` slot.
//!
//! A **link** is one open transport running one protocol handler (MSP scheduler, MAVLink handler or the
//! passive listener). A **vehicle** is one flight controller reachable through a link; MAVLink links can
//! carry several (one per system id), MSP and passive links exactly one. Vehicles are addressed by a
//! string key `"L{link}:S{sysid}"` (`sysid` 0 for the non-MAVLink protocols, where MAVLink reserves 0
//! for broadcast) so the frontend never has to understand the structure.
//!
//! One vehicle is **active**: every command that does not name a target and every legacy singleton
//! consumer (link status, relay hub, …) works on it. Telemetry events carry their vehicle id via
//! [`emitter::VehicleEmitter`], so the frontend can filter or fan out per vehicle.
//!
//! See docs/02-design/features/multi-vehicle.design.md (fork Dev-Docs).

pub mod emitter;

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::msp::FcInfo;
use crate::state::ActiveProtocol;

/// Registry-assigned id of one open link (1-based, never reused within a process lifetime).
pub type LinkId = u16;

/// One flight controller reachable through a link.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VehicleId {
    pub link: LinkId,
    /// MAVLink system id; 0 for MSP / passive telemetry (one vehicle per link).
    pub sysid: u8,
}

impl VehicleId {
    pub fn new(link: LinkId, sysid: u8) -> Self {
        Self { link, sysid }
    }

    /// The wire key the frontend uses: `"L1:S1"`.
    pub fn to_key(&self) -> String {
        format!("L{}:S{}", self.link, self.sysid)
    }

    /// Parse a wire key back. Rejects anything that is not exactly `L<u16>:S<u8>`.
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix('L')?;
        let (link, sys) = rest.split_once(":S")?;
        Some(Self { link: link.parse().ok()?, sysid: sys.parse().ok()? })
    }
}

impl fmt::Display for VehicleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{}:S{}", self.link, self.sysid)
    }
}

/// One open link and the protocol handler driving it.
pub struct LinkEntry {
    pub id: LinkId,
    pub protocol: ActiveProtocol,
    /// "MSP" | "MAVLink" | "Telemetry" — the label the OS-facing link status and logs use.
    pub protocol_name: &'static str,
    /// Human-readable transport description ("Serial COM7 @ 57600").
    pub transport: String,
    /// Handshake result. For MAVLink this describes the primary (handshake) vehicle.
    pub fc_info: FcInfo,
    /// The vehicle the link was opened for (MAVLink: the handshake sysid; others: sysid 0).
    pub primary: VehicleId,
}

/// What `list_links` returns to the frontend.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSummary {
    pub link_id: LinkId,
    pub protocol: String,
    pub transport: String,
    pub fc_info: FcInfo,
    pub primary_vehicle_id: String,
}

/// Payload of the `vehicle-discovered` event (also returned inline by `connect`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VehicleInfo {
    pub vehicle_id: String,
    pub link_id: LinkId,
    pub sysid: u8,
    pub compid: u8,
    pub fc_variant: String,
    pub platform_type: u8,
    pub mav_type: u8,
    pub protocol: String,
    /// True for the vehicle the link was opened for (handshake target).
    pub primary: bool,
    /// The vehicle's own FC hardware id (`msp::hardware_uid`) and firmware version, when known: the
    /// primary's from its handshake, a vehicle discovered later on a shared link from its own
    /// AUTOPILOT_VERSION (re-announced when that arrives). `None` = not known (yet).
    pub fc_uid: Option<String>,
    pub fc_version: Option<String>,
}

impl VehicleInfo {
    /// Describe a link's primary vehicle from its handshake result.
    pub fn primary_of(entry: &LinkEntry, compid: u8) -> Self {
        Self {
            vehicle_id: entry.primary.to_key(),
            link_id: entry.id,
            sysid: entry.primary.sysid,
            compid,
            fc_variant: entry.fc_info.fc_variant.clone(),
            platform_type: entry.fc_info.platform_type,
            mav_type: entry.fc_info.mav_type,
            protocol: entry.protocol_name.to_string(),
            primary: true,
            fc_uid: entry.fc_info.fc_uid.clone(),
            fc_version: Some(entry.fc_info.fc_version.clone()).filter(|v| !v.is_empty()),
        }
    }
}

/// Error string every command reports when nothing is connected. Kept byte-identical to the
/// pre-registry code — the frontend surfaces it verbatim.
pub const ERR_NOT_CONNECTED: &str = "Not connected";

/// Error of a command aimed at a vehicle other than a link's primary while the multi-vehicle features
/// are off (`fleet_mode`).
pub const ERR_FLEET_OFF: &str = "Multi-vehicle features are off (fleet mode)";

/// All open links plus the active-vehicle selection.
pub struct LinkRegistry {
    links: BTreeMap<LinkId, LinkEntry>,
    active: Option<VehicleId>,
    next_id: LinkId,
}

impl Default for LinkRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkRegistry {
    pub fn new() -> Self {
        Self { links: BTreeMap::new(), active: None, next_id: 1 }
    }

    /// Reserve a link id for a connect that is about to run. Handler-side state (emitters) needs the
    /// id before the handshake completes, and two overlapping connects must never share one.
    pub fn reserve_id(&mut self) -> LinkId {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Register an open link under the id it reserved. When nothing was active, the new primary
    /// becomes the active vehicle. Returns whether the active selection changed.
    pub fn insert(&mut self, entry: LinkEntry) -> bool {
        debug_assert_eq!(entry.id, entry.primary.link, "primary vehicle must live on its own link");
        let activated = if self.active.is_none() {
            self.active = Some(entry.primary.clone());
            true
        } else {
            false
        };
        self.links.insert(entry.id, entry);
        activated
    }

    /// Remove a link. If the active vehicle lived on it, the active selection moves to the lowest
    /// remaining link's primary (or none). Returns `(entry, new active if it changed)`.
    pub fn remove(&mut self, id: LinkId) -> Option<(LinkEntry, Option<Option<VehicleId>>)> {
        let entry = self.links.remove(&id)?;
        let active_changed = match &self.active {
            Some(a) if a.link == id => {
                self.active = self.links.values().next().map(|e| e.primary.clone());
                Some(self.active.clone())
            }
            _ => None,
        };
        Some((entry, active_changed))
    }

    /// Drain every link (disconnect-all). Clears the active selection.
    pub fn drain(&mut self) -> Vec<LinkEntry> {
        self.active = None;
        std::mem::take(&mut self.links).into_values().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    #[allow(dead_code)] // Phase B (link list UI)
    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn get(&self, id: LinkId) -> Option<&LinkEntry> {
        self.links.get(&id)
    }

    pub fn get_mut(&mut self, id: LinkId) -> Option<&mut LinkEntry> {
        self.links.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &LinkEntry> {
        self.links.values()
    }

    pub fn active(&self) -> Option<&VehicleId> {
        self.active.as_ref()
    }

    /// Select the active vehicle. The link must exist; the sysid is not validated here (MAVLink
    /// vehicles discovered mid-session live in the handler thread, not the registry).
    pub fn set_active(&mut self, v: VehicleId) -> Result<(), String> {
        let Some(entry) = self.links.get(&v.link) else {
            return Err(format!("Unknown link L{}", v.link));
        };
        // GATE(fleet): only a link's primary vehicle without the multi-vehicle features.
        if !crate::fleet_mode::allows(v == entry.primary, crate::fleet_mode::enabled()) {
            return Err(ERR_FLEET_OFF.into());
        }
        self.active = Some(v);
        Ok(())
    }

    /// Resolve a command target: `None` → the active vehicle, `Some(key)` → that vehicle. Returns the
    /// link entry and the sysid to address. Errors use the legacy "Not connected" text when nothing
    /// is connected so existing UI toasts keep working.
    pub fn resolve(&self, target: Option<&str>) -> Result<(&LinkEntry, u8), String> {
        let vid = match target {
            Some(key) => VehicleId::parse(key).ok_or_else(|| format!("Invalid vehicle id '{key}'"))?,
            None => self.active.clone().ok_or(ERR_NOT_CONNECTED)?,
        };
        let entry = self.links.get(&vid.link).ok_or_else(|| {
            if self.links.is_empty() { ERR_NOT_CONNECTED.to_string() } else { format!("Vehicle {vid} is not connected") }
        })?;
        // GATE(fleet): without the multi-vehicle features a command reaches a link's primary vehicle only
        // (a sysid the handler never discovered must not be commanded).
        if !crate::fleet_mode::allows(vid == entry.primary, crate::fleet_mode::enabled()) {
            return Err(ERR_FLEET_OFF.into());
        }
        Ok((entry, vid.sysid))
    }

    /// The active link's protocol handler, if any. Mirrors the old `protocol.as_ref()` for the
    /// single-vehicle command paths (MSP mission/geozone/safehome/RC…).
    pub fn active_protocol(&self) -> Option<&ActiveProtocol> {
        self.active_entry().map(|e| &e.protocol)
    }

    pub fn active_entry(&self) -> Option<&LinkEntry> {
        self.active.as_ref().and_then(|a| self.links.get(&a.link))
    }

    /// The active link's handshake info (what `get_fc_info` returns).
    pub fn active_fc_info(&self) -> Option<&FcInfo> {
        self.active_entry().map(|e| &e.fc_info)
    }

    pub fn summaries(&self) -> Vec<LinkSummary> {
        self.links
            .values()
            .map(|e| LinkSummary {
                link_id: e.id,
                protocol: e.protocol_name.to_string(),
                transport: e.transport.clone(),
                fc_info: e.fc_info.clone(),
                primary_vehicle_id: e.primary.to_key(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vehicle_id_round_trips() {
        let v = VehicleId::new(3, 42);
        assert_eq!(v.to_key(), "L3:S42");
        assert_eq!(VehicleId::parse("L3:S42"), Some(v));
        assert_eq!(VehicleId::parse("L65535:S0"), Some(VehicleId::new(65535, 0)));
    }

    #[test]
    fn vehicle_id_rejects_garbage() {
        for s in ["", "L1", "L1:S", "1:1", "L:S1", "L1:S256", "L70000:S1", "l1:s1", "L1:S1:x"] {
            assert!(VehicleId::parse(s).is_none(), "{s:?} should not parse");
        }
    }

    #[test]
    fn resolve_without_links_is_not_connected() {
        let reg = LinkRegistry::new();
        assert_eq!(reg.resolve(None).err().as_deref(), Some(ERR_NOT_CONNECTED));
        assert_eq!(reg.resolve(Some("L1:S1")).err().as_deref(), Some(ERR_NOT_CONNECTED));
        assert!(reg.resolve(Some("bogus")).err().unwrap().starts_with("Invalid vehicle id"));
    }

    #[test]
    fn reserved_ids_are_monotonic_and_start_at_one() {
        let mut reg = LinkRegistry::new();
        assert_eq!(reg.reserve_id(), 1);
        assert_eq!(reg.reserve_id(), 2);
        reg.next_id = u16::MAX;
        assert_eq!(reg.reserve_id(), u16::MAX);
        assert_eq!(reg.reserve_id(), 1, "wraps past 0 (0 is never a valid link id)");
    }
}

/// Whether a stamped event payload (JSON text, see `emitter`) belongs to the active vehicle. Payloads
/// without a `vehicleId` (events that are not per-vehicle) pass. Used by the backend taps (relay hub,
/// Telemetry API) so their single caches follow the vehicle the operator selected instead of mixing
/// every aircraft on the link. Cheap: a substring scan, no JSON parse.
pub fn payload_is_active(app: &tauri::AppHandle, payload: &str) -> bool {
    use tauri::Manager;
    let Some(start) = payload.find("\"vehicleId\":\"") else { return true };
    let rest = &payload[start + 13..];
    let Some(end) = rest.find('"') else { return true };
    let vid = &rest[..end];
    let st = app.state::<crate::state::AppState>();
    let is_active = match st.links.lock() {
        Ok(reg) => reg.active().map(|a| a.to_key() == vid).unwrap_or(true),
        Err(_) => true,
    };
    is_active
}
