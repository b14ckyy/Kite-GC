// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV geozone config (read). See docs/active/GEOZONES.md.
//
// `geozone_read_all` reads every geozone slot the FC can hold (id 0..62) plus each used zone's
// vertices, for the map overlay + the Airspace Manager panel list. Geozones are an INAV ≥8.0 feature
// (gated by `FeatureSet.geozones`); on older firmware / non-INAV links we return an empty,
// `has_geozones=false` config. Writing/editing (batch SET + EEPROM) is Phase 2 and not implemented yet.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::msp::{
    MSP2_INAV_GEOZONE, MSP2_INAV_GEOZONE_VERTEX, MSP2_INAV_SET_GEOZONE, MSP2_INAV_SET_GEOZONE_VERTEX,
    MSP_EEPROM_WRITE, MSP_SET_REBOOT,
};
use crate::scheduler::SchedulerHandle;
use crate::state::{ActiveProtocol, AppState};

/// Geozone slots the FC config can hold (`MAX_GEOZONES_IN_CONFIG`; ids 0..62).
const MAX_GEOZONES: u8 = 63;

const GEOZONE_SHAPE_CIRCULAR: u8 = 0;

/// One geozone vertex (lat/lon in degrees × 1e7).
#[derive(Serialize, Deserialize, Clone)]
pub struct GeoZoneVertex {
    pub lat: i32,
    pub lon: i32,
}

/// One geozone. `zone_type` 0 = exclusive (NFZ), 1 = inclusive (FZ). `shape` 0 = circular, 1 = polygon.
/// `fence_action` 0 = none, 1 = avoid, 2 = pos-hold, 3 = RTH. Altitudes in cm: `min_alt_cm` 0 = ground,
/// `max_alt_cm` 0 = no upper limit. For a circular zone `radius_cm` is set and `vertices` holds the
/// single centre point; for a polygon `radius_cm` is None and `vertices` holds all corners.
#[derive(Serialize, Deserialize, Clone)]
pub struct GeoZone {
    pub id: u8,
    pub zone_type: u8,
    pub shape: u8,
    pub min_alt_cm: i32,
    pub max_alt_cm: i32,
    pub is_sealevel_ref: bool,
    pub fence_action: u8,
    pub radius_cm: Option<u32>,
    pub vertices: Vec<GeoZoneVertex>,
}

/// Full geozone snapshot for the frontend.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct GeozoneConfig {
    pub zones: Vec<GeoZone>,
    /// True when the geozone feature is available (INAV ≥8.0) — drives the UI's visibility.
    pub has_geozones: bool,
}

/// Resolve the MSP scheduler handle, erroring for non-MSP / disconnected links.
fn msp_handle(proto: &Option<ActiveProtocol>) -> Result<&SchedulerHandle, String> {
    match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => Ok(h),
        Some(_) => Err("FC is not running MSP (INAV)".into()),
        None => Err("Not connected".into()),
    }
}

/// Read all geozones + their vertices. Returns an empty `has_geozones=false` config when the firmware
/// lacks the feature (<8.0), so callers can always invoke it on INAV connect.
#[tauri::command(async)]
pub fn geozone_read_all(state: State<'_, AppState>) -> Result<GeozoneConfig, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = msp_handle(&proto)?;

    let has_geozones = {
        let info = state.fc_info.lock().map_err(|e| e.to_string())?;
        info.as_ref()
            .and_then(|fc| fc.features.as_ref())
            .map(|f| f.geozones)
            .unwrap_or(false)
    };
    if !has_geozones {
        return Ok(GeozoneConfig { zones: Vec::new(), has_geozones: false });
    }

    let mut zones = Vec::new();
    for id in 0..MAX_GEOZONES {
        // Header resp = [id, type, shape, minAlt(4), maxAlt(4), isSealevelRef, fenceAction, vertexCount] = 14 bytes.
        let r = handle.msp_request(MSP2_INAV_GEOZONE, &[id])?;
        if r.len() < 14 {
            continue;
        }
        let shape = r[2];
        let vertex_count = r[13];
        if vertex_count == 0 {
            continue; // unused slot
        }

        let mut vertices = Vec::with_capacity(vertex_count as usize);
        let mut radius_cm: Option<u32> = None;
        if shape == GEOZONE_SHAPE_CIRCULAR {
            // Circle: a single vertex (centre); the radius is appended (resp = 14 bytes).
            let v = handle.msp_request(MSP2_INAV_GEOZONE_VERTEX, &[id, 0])?;
            if v.len() >= 10 {
                vertices.push(GeoZoneVertex {
                    lat: i32::from_le_bytes([v[2], v[3], v[4], v[5]]),
                    lon: i32::from_le_bytes([v[6], v[7], v[8], v[9]]),
                });
            }
            if v.len() >= 14 {
                radius_cm = Some(u32::from_le_bytes([v[10], v[11], v[12], v[13]]));
            }
        } else {
            // Polygon: vertexCount corners, each resp = [zoneId, vertexId, lat(4), lon(4)] = 10 bytes.
            for vi in 0..vertex_count {
                let v = handle.msp_request(MSP2_INAV_GEOZONE_VERTEX, &[id, vi])?;
                if v.len() >= 10 {
                    vertices.push(GeoZoneVertex {
                        lat: i32::from_le_bytes([v[2], v[3], v[4], v[5]]),
                        lon: i32::from_le_bytes([v[6], v[7], v[8], v[9]]),
                    });
                }
            }
        }

        zones.push(GeoZone {
            id: r[0],
            zone_type: r[1],
            shape,
            min_alt_cm: i32::from_le_bytes([r[3], r[4], r[5], r[6]]),
            max_alt_cm: i32::from_le_bytes([r[7], r[8], r[9], r[10]]),
            is_sealevel_ref: r[11] != 0,
            fence_action: r[12],
            radius_cm,
            vertices,
        });
    }

    eprintln!("[GEOZONE] read {} active zone(s)", zones.len());
    Ok(GeozoneConfig { zones, has_geozones: true })
}

/// "Save to FC": write the whole geozone config as a batch, then a single EEPROM write to persist.
/// Every slot id 0..62 is written — active zones with their data, all other slots cleared
/// (`vertexCount = 0`) so removed zones don't linger. The zone header is written BEFORE its vertices
/// (the FC's vertex handler branches on the stored shape to read the circle radius); polygon vertices
/// go out in ascending order; a circle writes its single centre vertex with the radius appended.
#[tauri::command(async)]
pub fn geozone_write_all(config: GeozoneConfig, state: State<'_, AppState>) -> Result<(), String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = msp_handle(&proto)?;

    for id in 0..MAX_GEOZONES {
        let zone = config.zones.iter().find(|z| z.id == id);
        match zone {
            Some(z) => {
                let circular = z.shape == GEOZONE_SHAPE_CIRCULAR;
                let vertex_count: u8 = if circular { 1 } else { z.vertices.len() as u8 };
                // Header [id, type, shape, minAlt(4), maxAlt(4), isSealevelRef, fenceAction, vertexCount].
                let mut h = Vec::with_capacity(14);
                h.push(id);
                h.push(z.zone_type);
                h.push(z.shape);
                h.extend_from_slice(&z.min_alt_cm.to_le_bytes());
                h.extend_from_slice(&z.max_alt_cm.to_le_bytes());
                h.push(z.is_sealevel_ref as u8);
                h.push(z.fence_action);
                h.push(vertex_count);
                handle.msp_request(MSP2_INAV_SET_GEOZONE, &h)?;

                if circular {
                    // [id, 0, lat(4), lon(4), radius(4)] — FC stores the radius as the hidden vertex 1.
                    let c = z.vertices.first().ok_or("circular geozone has no centre vertex")?;
                    let mut p = Vec::with_capacity(14);
                    p.push(id);
                    p.push(0);
                    p.extend_from_slice(&c.lat.to_le_bytes());
                    p.extend_from_slice(&c.lon.to_le_bytes());
                    p.extend_from_slice(&z.radius_cm.unwrap_or(0).to_le_bytes());
                    handle.msp_request(MSP2_INAV_SET_GEOZONE_VERTEX, &p)?;
                } else {
                    for (vi, v) in z.vertices.iter().enumerate() {
                        let mut p = Vec::with_capacity(10);
                        p.push(id);
                        p.push(vi as u8);
                        p.extend_from_slice(&v.lat.to_le_bytes());
                        p.extend_from_slice(&v.lon.to_le_bytes());
                        handle.msp_request(MSP2_INAV_SET_GEOZONE_VERTEX, &p)?;
                    }
                }
            }
            None => {
                // Clear the slot: header with vertexCount 0 marks it unused.
                let mut h = Vec::with_capacity(14);
                h.push(id);
                h.extend_from_slice(&[0u8; 11]); // type, shape, minAlt(4), maxAlt(4), isSealevelRef
                h.push(0); // fenceAction
                h.push(0); // vertexCount
                handle.msp_request(MSP2_INAV_SET_GEOZONE, &h)?;
            }
        }
    }

    handle.msp_request(MSP_EEPROM_WRITE, &[])?;
    eprintln!("[GEOZONE] saved {} active zone(s) to FC (EEPROM written)", config.zones.len());

    // Geozones MUST be applied via a reboot: INAV recomputes the internal zone structures only at boot,
    // so the EEPROM write alone doesn't take effect. INAV ACKs the reboot before restarting; the link
    // then drops (the frontend reconnects + re-reads on handshake), so a missing/late reply is fine.
    let _ = handle.msp_request(MSP_SET_REBOOT, &[]);
    eprintln!("[GEOZONE] reboot requested to apply geozones");
    Ok(())
}
