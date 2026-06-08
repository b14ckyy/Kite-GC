// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — normalized foreign-vehicle model
//
// Heterogeneous by design: each system (ADS-B / FormationFlight / radio telemetry) carries
// different native fields, so this is an OPTIONAL-FIELD SUPERSET — a source fills only what it
// has, the rest stay `None`. No lossy common schema. Native fields without a slot here go into
// `extra`. See docs/active/RADAR_TRACKING_CORE.md §4.

use std::collections::BTreeMap;

use serde::Serialize;

/// Top-level grouping — the three separate, never-merged lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VehicleSystem {
    Adsb,
    FormationFlight,
    Radio,
}

/// Concrete feed within a system (a vehicle's `sources` accumulates these after per-system merge).
/// Variants beyond `Sim`/`AdsbOnline` are scaffolding for later phases (see §9).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleSource {
    AdsbOnline,
    AdsbReceiver,
    AdsbMsp,
    FormationFlight,
    RadioCrsf,
    RadioMavlink,
    RadioFrsky,
    /// Dev-only synthetic source (debug builds) — drives the pipeline before real sources exist.
    Sim,
}

/// What the altitude is referenced to (systems differ; ADS-B is baro/geo MSL, radar is often relative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AltRef {
    BaroMsl,
    GeoMsl,
    Relative,
    Unknown,
}

/// One tracked foreign vehicle. Distance/bearing/relative-altitude are NOT here — they are derived
/// in the frontend from the user location, to keep the backend location-agnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedVehicle {
    /// Stable id within the system: ICAO hex (ADS-B) | peer id (FormationFlight) | sysid (radio).
    pub id: String,
    pub system: VehicleSystem,
    /// Which feeds currently report this id (after per-system merge by `id`).
    pub sources: Vec<VehicleSource>,
    pub callsign: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: Option<f64>,
    pub alt_ref: AltRef,
    pub heading_deg: Option<f64>,
    pub ground_speed_ms: Option<f64>,
    pub vertical_speed_ms: Option<f64>,
    /// Aircraft/emitter category or type label, when known (ADS-B emitter type, etc.).
    pub category: Option<String>,
    /// RSSI/SNR where available (FormationFlight LoRa, radio links).
    pub signal: Option<f64>,
    /// ADS-B transponder squawk, when known.
    pub squawk: Option<u16>,
    /// Unix milliseconds of the last update — drives TTL expiry and the UI "age".
    pub last_seen_ms: i64,
    /// True when this update carried a usable position.
    pub valid_pos: bool,
    /// Native fields with no dedicated slot above (per-system extras).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl TrackedVehicle {
    /// Minimal constructor for a positioned contact; optional fields default to `None`.
    pub fn new(
        id: impl Into<String>,
        system: VehicleSystem,
        source: VehicleSource,
        lat: f64,
        lon: f64,
        last_seen_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            system,
            sources: vec![source],
            callsign: None,
            lat,
            lon,
            alt_m: None,
            alt_ref: AltRef::Unknown,
            heading_deg: None,
            ground_speed_ms: None,
            vertical_speed_ms: None,
            category: None,
            signal: None,
            squawk: None,
            last_seen_ms,
            valid_pos: true,
            extra: BTreeMap::new(),
        }
    }
}
