// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// OpenAIP provider — fetches each enabled layer for a region (pos + dist), paginates, and maps the
// verified v2 response shape into the normalized aero models. Defensive parsing (every field optional).

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use super::{
    airport_type_name, alt_label, datum_name, icao_class_name, obstacle_type_name, airspace_type_name,
    to_meters, AeroData, AltLimit, Airspace, PointFeature,
};

const BASE: &str = "https://api.core.openaip.net/api";
const PAGE_LIMIT: i64 = 1000;
const MAX_PAGES: i64 = 30; // safety cap per layer

pub async fn fetch(api_key: &str, lat: f64, lon: f64, radius_km: f64, layers: &[String]) -> Result<AeroData, String> {
    let client = reqwest::Client::builder()
        .user_agent("Kite-GC/0.5 aero")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    // Airspaces are few → fetch the wide region; obstacles/airports/RC are dense → short range only.
    let airspace_dist = (radius_km * 1000.0).round() as i64;
    let point_dist = (radius_km.min(super::POINT_RADIUS_KM) * 1000.0).round() as i64;
    let mut data = AeroData::default();
    // Each layer is independent — one failing/empty layer must not blank the others.
    for layer in layers {
        match layer.as_str() {
            "airspaces" => match fetch_layer(&client, api_key, "airspaces", lat, lon, airspace_dist, parse_airspace).await {
                Ok(v) => data.airspaces = v,
                Err(e) => eprintln!("[aero][openaip] airspaces: {e}"),
            },
            "obstacles" => match fetch_layer(&client, api_key, "obstacles", lat, lon, point_dist, |v| parse_point(v, "obstacle")).await {
                Ok(v) => data.obstacles = v,
                Err(e) => eprintln!("[aero][openaip] obstacles: {e}"),
            },
            "airports" => match fetch_layer(&client, api_key, "airports", lat, lon, point_dist, |v| parse_point(v, "airport")).await {
                Ok(v) => data.airports = v,
                Err(e) => eprintln!("[aero][openaip] airports: {e}"),
            },
            "rc" | "rcAirfields" => match fetch_layer(&client, api_key, "rc-airfields", lat, lon, point_dist, |v| parse_point(v, "rc")).await {
                Ok(v) => data.rc_airfields = v,
                Err(e) => eprintln!("[aero][openaip] rc-airfields: {e}"),
            },
            _ => {}
        }
    }
    Ok(data)
}

async fn fetch_layer<T>(
    client: &reqwest::Client,
    api_key: &str,
    endpoint: &str,
    lat: f64,
    lon: f64,
    dist_m: i64,
    parse: impl Fn(&Value) -> Option<T>,
) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    let mut page = 1i64;
    loop {
        let url = format!(
            "{BASE}/{endpoint}?apiKey={api_key}&pos={lat:.5},{lon:.5}&dist={dist_m}&page={page}&limit={PAGE_LIMIT}"
        );
        let resp = client.get(&url).send().await.map_err(|e| format!("{endpoint} fetch failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{endpoint} HTTP {}", resp.status()));
        }
        let root: Value = resp.json().await.map_err(|e| format!("{endpoint} parse: {e}"))?;
        if let Some(items) = root.get("items").and_then(Value::as_array) {
            for it in items {
                if let Some(p) = parse(it) {
                    out.push(p);
                }
            }
        }
        let total_pages = root.get("totalPages").and_then(Value::as_i64).unwrap_or(1);
        if page >= total_pages || page >= MAX_PAGES {
            break;
        }
        page += 1;
    }
    Ok(out)
}

fn limit_from(v: &Value, key: &str) -> AltLimit {
    let o = v.get(key);
    let value = o.and_then(|x| x.get("value")).and_then(Value::as_f64).unwrap_or(0.0);
    let unit = o.and_then(|x| x.get("unit")).and_then(Value::as_i64).unwrap_or(1);
    let datum = o.and_then(|x| x.get("referenceDatum")).and_then(Value::as_i64).unwrap_or(1);
    AltLimit { value_m: to_meters(value, unit), datum: datum_name(datum), label: alt_label(value, unit, datum) }
}

fn parse_airspace(v: &Value) -> Option<Airspace> {
    let id = v.get("_id").and_then(Value::as_str)?.to_string();
    let type_id = v.get("type").and_then(Value::as_u64).unwrap_or(0) as u16;
    let icao_class = v.get("icaoClass").and_then(Value::as_i64).unwrap_or(8) as i32;
    let outlines = polygon_rings(v.get("geometry")?);
    if outlines.is_empty() {
        return None;
    }
    Some(Airspace {
        id,
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        type_id,
        type_name: airspace_type_name(type_id),
        icao_class,
        icao_class_name: icao_class_name(icao_class),
        lower: limit_from(v, "lowerLimit"),
        upper: limit_from(v, "upperLimit"),
        outlines,
    })
}

/// Outer ring(s) of a GeoJSON Polygon / MultiPolygon as `[lon, lat]` lists (holes ignored).
fn polygon_rings(geom: &Value) -> Vec<Vec<[f64; 2]>> {
    let coords = geom.get("coordinates");
    let mut rings = Vec::new();
    match geom.get("type").and_then(Value::as_str).unwrap_or("") {
        "Polygon" => {
            if let Some(outer) = coords.and_then(Value::as_array).and_then(|a| a.first()) {
                if let Some(r) = ring(outer) {
                    rings.push(r);
                }
            }
        }
        "MultiPolygon" => {
            if let Some(polys) = coords.and_then(Value::as_array) {
                for poly in polys {
                    if let Some(outer) = poly.as_array().and_then(|a| a.first()) {
                        if let Some(r) = ring(outer) {
                            rings.push(r);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    rings
}

fn ring(v: &Value) -> Option<Vec<[f64; 2]>> {
    let arr = v.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for p in arr {
        let pa = p.as_array()?;
        out.push([pa.first()?.as_f64()?, pa.get(1)?.as_f64()?]);
    }
    if out.len() >= 3 {
        Some(out)
    } else {
        None
    }
}

fn elev_meters(v: &Value, key: &str) -> Option<f64> {
    let o = v.get(key)?;
    let value = o.get("value")?.as_f64()?;
    Some(to_meters(value, o.get("unit").and_then(Value::as_i64).unwrap_or(0)))
}

fn parse_point(v: &Value, kind: &'static str) -> Option<PointFeature> {
    let id = v.get("_id").and_then(Value::as_str)?.to_string();
    let geom = v.get("geometry")?;
    if geom.get("type").and_then(Value::as_str) != Some("Point") {
        return None;
    }
    let c = geom.get("coordinates").and_then(Value::as_array)?;
    let lon = c.first()?.as_f64()?;
    let lat = c.get(1)?.as_f64()?;
    let type_raw = v.get("type").and_then(Value::as_u64).map(|t| t as u16);
    let type_id = type_raw.unwrap_or(0);
    let subtype = match kind {
        "obstacle" => obstacle_type_name(type_id),
        "airport" => airport_type_name(type_id),
        _ => "",
    };
    let mut extra: HashMap<String, String> = HashMap::new();
    if kind == "rc" {
        if let Some(a) = v.get("permittedAltitude").and_then(Value::as_f64) {
            extra.insert("permittedAltitude".into(), format!("{}", a as i64));
        }
        if let Some(o) = v.get("operator").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            extra.insert("operator".into(), o.to_string());
        }
        for k in ["electric", "combustion", "turbine"] {
            if v.get(k).and_then(Value::as_bool) == Some(true) {
                extra.insert(k.into(), "1".into());
            }
        }
    }
    Some(PointFeature {
        id,
        kind,
        type_id: if kind == "rc" { None } else { type_raw },
        subtype,
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        lat,
        lon,
        elevation_m: elev_meters(v, "elevation"),
        height_m: elev_meters(v, "height"),
        extra,
    })
}
