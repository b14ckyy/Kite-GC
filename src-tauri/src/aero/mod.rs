// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Aeronautical data subsystem ("Airspace Manager"): fetch OpenAIP airspaces / obstacles / airports /
// RC-airfields for a region, normalize them into shared models, and cache the last region in RAM.
// On-demand (no background threads) — the frontend calls `aero_fetch` as the reference moves.
// Enum tables + field shapes verified against the live OpenAIP v2 schemas.
// See docs/active/AIRSPACE_MANAGER.md + ADR-038.

pub mod openaip;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Short range (km) for the dense point layers (obstacles / airports / RC) — fetched only near the
/// reference, unlike airspaces (few → fetched for the wide region).
pub const POINT_RADIUS_KM: f64 = 50.0;
/// Re-fetch once the reference moves more than this from the cached centre (keeps the short-range
/// point layers fresh; airspaces are cheap to re-pull).
const REFETCH_MOVE_KM: f64 = 30.0;

/// An altitude limit normalized to metres, with a datum tag + a human label for display.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AltLimit {
    pub value_m: f64,
    /// "gnd" | "msl" | "std"
    pub datum: &'static str,
    /// Original OpenAIP reading for display ("FL90", "1600 ft MSL", "0 GND").
    pub label: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Airspace {
    pub id: String,
    pub name: String,
    pub type_id: u16,
    pub type_name: &'static str,
    pub icao_class: i32,
    pub icao_class_name: &'static str,
    pub lower: AltLimit,
    pub upper: AltLimit,
    /// One or more outer rings (Polygon → 1, MultiPolygon → n), each a list of `[lon, lat]`.
    pub outlines: Vec<Vec<[f64; 2]>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PointFeature {
    pub id: String,
    /// "obstacle" | "airport" | "rc"
    pub kind: &'static str,
    /// Raw OpenAIP `type` id (obstacle/airport); None for RC airfields. Used for zoom-density filtering.
    pub type_id: Option<u16>,
    /// Mapped type name (e.g. "Wind Turbine", "Glider Site"); empty when n/a (RC airfields have no type).
    pub subtype: &'static str,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Base elevation MSL (m), if known.
    pub elevation_m: Option<f64>,
    /// AGL height (m) — obstacles → 3D column height.
    pub height_m: Option<f64>,
    /// Misc display fields (operator, permittedAltitude, propulsion, …).
    pub extra: HashMap<String, String>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AeroData {
    pub airspaces: Vec<Airspace>,
    pub obstacles: Vec<PointFeature>,
    pub airports: Vec<PointFeature>,
    pub rc_airfields: Vec<PointFeature>,
}

/// RAM cache of the last fetched region (aero data is static → long-lived; refetched only when the
/// requested region no longer fits inside the cached one, or the provider/key/layers change).
pub struct AeroCache {
    pub provider: String,
    pub key: String,
    pub layers: Vec<String>,
    pub center: (f64, f64),
    pub data: AeroData,
    pub fetched_at_ms: u128,
}

impl AeroCache {
    /// True when this cache can satisfy the request: same provider/key, all requested layers present,
    /// and the reference hasn't moved far enough to risk stale short-range point data.
    pub fn covers(&self, provider: &str, key: &str, layers: &[String], lat: f64, lon: f64) -> bool {
        if self.provider != provider || self.key != key {
            return false;
        }
        if !layers.iter().all(|l| self.layers.contains(l)) {
            return false;
        }
        haversine_km(self.center.0, self.center.1, lat, lon) <= REFETCH_MOVE_KM
    }
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

// ── OpenAIP enum tables (verified against the live schemas) ──────────────────
pub fn airspace_type_name(t: u16) -> &'static str {
    match t {
        0 => "Other", 1 => "Restricted", 2 => "Danger", 3 => "Prohibited", 4 => "CTR", 5 => "TMZ",
        6 => "RMZ", 7 => "TMA", 8 => "TRA", 9 => "TSA", 10 => "FIR", 11 => "UIR", 12 => "ADIZ",
        13 => "ATZ", 14 => "MATZ", 15 => "Airway", 16 => "MTR", 17 => "Alert Area", 18 => "Warning Area",
        19 => "Protected Area", 20 => "HTZ", 21 => "Gliding Sector", 22 => "TRP", 23 => "TIZ", 24 => "TIA",
        25 => "MTA", 26 => "CTA", 27 => "ACC Sector", 28 => "Aerial Sporting/Recreational",
        29 => "Low-Alt Overflight Restriction", 30 => "MRT", 31 => "TFR", 32 => "VFR Sector",
        33 => "FIS Sector", 34 => "LTA", 35 => "UTA", 36 => "MCTR", _ => "Airspace",
    }
}
pub fn icao_class_name(c: i32) -> &'static str {
    match c { 0 => "A", 1 => "B", 2 => "C", 3 => "D", 4 => "E", 5 => "F", 6 => "G", 8 => "Unclassified/SUA", _ => "?" }
}
pub fn obstacle_type_name(t: u16) -> &'static str {
    match t { 0 => "Obstacle", 1 => "Chimney", 2 => "Building", 3 => "Wind Turbine", 4 => "Tower", _ => "Obstacle" }
}
pub fn airport_type_name(t: u16) -> &'static str {
    match t {
        0 => "Airport", 1 => "Glider Site", 2 => "Airfield", 3 => "International Airport",
        4 => "Heliport (mil)", 5 => "Military Aerodrome", 6 => "Ultralight Site", 7 => "Heliport",
        8 => "Aerodrome (closed)", 9 => "Airfield IFR", 10 => "Water Airfield", 11 => "Landing Strip",
        12 => "Agricultural Strip", 13 => "Altiport", _ => "Airport",
    }
}

/// OpenAIP `{value, unit}` → metres. unit: 0 = m, 1 = ft, 6 = FL (hundreds of ft).
pub fn to_meters(value: f64, unit: i64) -> f64 {
    match unit { 0 => value, 1 => value * 0.3048, 6 => value * 100.0 * 0.3048, _ => value }
}
pub fn datum_name(d: i64) -> &'static str {
    match d { 0 => "gnd", 1 => "msl", 2 => "std", _ => "msl" }
}
pub fn alt_label(value: f64, unit: i64, datum: i64) -> String {
    let d = match datum { 0 => "GND", 1 => "MSL", _ => "" };
    let v = value as i64;
    match unit {
        6 => format!("FL{v}"),
        1 => format!("{v} ft {d}").trim().to_string(),
        0 => format!("{v} m {d}").trim().to_string(),
        _ => format!("{v} {d}").trim().to_string(),
    }
}
