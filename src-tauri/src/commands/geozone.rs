// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV geozone config (read). See docs/active/GEOZONES.md.
//
// `geozone_read_all` reads every geozone slot the FC can hold (id 0..62) plus each used zone's
// vertices, for the map overlay + the Airspace Manager panel list. Geozones are an INAV ≥8.0 feature
// (gated by `FeatureSet.geozones`); on older firmware / non-INAV links we return an empty,
// `has_geozones=false` config. Writing/editing (batch SET + EEPROM) is Phase 2 and not implemented yet.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::msp::{
    MSP2_INAV_GEOZONE, MSP2_INAV_GEOZONE_VERTEX, MSP2_INAV_SET_GEOZONE, MSP2_INAV_SET_GEOZONE_VERTEX,
    MSP_API_VERSION, MSP_EEPROM_WRITE, MSP_SET_REBOOT,
};
use crate::scheduler::{MspRequester, ProbeOutcome};
use crate::state::{with_msp_blocking, AppState};

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

/// Result of "Save to FC".
#[derive(Serialize)]
pub struct GeozoneWriteResult {
    /// The FC restarted after the save (geozones only take effect after a reboot). Always `true` on a
    /// direct MSP link — the reboot drops the link and the reconnect reloads. Over MSP over MAVLink the
    /// link survives, so the restart is confirmed by watching the FC go silent and answer again.
    pub reboot_confirmed: bool,
}

/// Read all geozones + their vertices. Returns an empty `has_geozones=false` config when the firmware
/// lacks the feature (<8.0), so callers can always invoke it on INAV connect. Direct MSP or the
/// MSP-over-MAVLink tunnel, on a blocking worker (`with_msp_blocking`: 60+ round trips).
#[tauri::command]
pub async fn geozone_read_all(app: AppHandle) -> Result<GeozoneConfig, String> {
    with_msp_blocking(&app, |app, handle| read_all(&app.state::<AppState>(), handle)).await
}

fn read_all(state: &AppState, handle: &MspRequester) -> Result<GeozoneConfig, String> {
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
/// Runs on a blocking worker (`with_msp_blocking`).
#[tauri::command]
pub async fn geozone_write_all(config: GeozoneConfig, app: AppHandle) -> Result<GeozoneWriteResult, String> {
    with_msp_blocking(&app, move |_, handle| write_all(&config, handle)).await
}

fn write_all(config: &GeozoneConfig, handle: &MspRequester) -> Result<GeozoneWriteResult, String> {
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
    // so the EEPROM write alone doesn't take effect. INAV sends the reply BEFORE it restarts.
    //  • Direct MSP: the reboot drops the link (USB/serial), the frontend reconnects and the handshake
    //    re-reads the zones — a missing/late reply is fine, nothing to confirm.
    //  • MSP over MAVLink: the MAVLink link usually survives the restart, so nothing reconnects —
    //    confirm the restart by watching the FC go silent and answer again (`confirm_tunnel_reboot`).
    let started = Instant::now();
    let ack = handle.msp_request(MSP_SET_REBOOT, &[]).is_ok();
    eprintln!("[GEOZONE] reboot requested to apply geozones (ack={ack})");
    if !handle.is_tunnel() {
        return Ok(GeozoneWriteResult { reboot_confirmed: true });
    }
    let reboot_confirmed = confirm_tunnel_reboot(handle, started, ack);
    if reboot_confirmed {
        log::info!("Geozones: FC restart confirmed over MSP over MAVLink ({} ms)", started.elapsed().as_millis());
    } else {
        log::warn!("Geozones: saved, but the FC restart was not confirmed over MSP over MAVLink — power-cycle to apply");
    }
    Ok(GeozoneWriteResult { reboot_confirmed })
}

/// Reboot confirmation over a link that survives the FC restart: probe interval / reply deadline.
const REBOOT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// A silence at least this long, followed by an answer, is a restart (shorter = a lost reply).
const REBOOT_SILENT_MIN: Duration = Duration::from_secs(1);
/// No silence started by then → the FC did not reboot.
const REBOOT_GO_SILENT_WITHIN: Duration = Duration::from_secs(3);
/// Give up waiting for the FC to answer again.
const REBOOT_CONFIRM_MAX: Duration = Duration::from_secs(10);

/// Decides "did the FC reboot" from probe samples: silent for ≥ `REBOOT_SILENT_MIN`, then answers again.
/// Sample times are offsets from the reboot request — a silent probe counts from when it was sent, an
/// answer from when it arrived.
#[derive(Default)]
struct RebootWatch {
    silent_since: Option<Duration>,
}

impl RebootWatch {
    /// Feed one probe sample; `Some(verdict)` once decided.
    fn sample(&mut self, t: Duration, answered: bool) -> Option<bool> {
        if answered {
            match self.silent_since {
                Some(since) if t.saturating_sub(since) >= REBOOT_SILENT_MIN => return Some(true),
                _ => self.silent_since = None, // a short gap = a lost reply, not a restart
            }
        } else if self.silent_since.is_none() {
            self.silent_since = Some(t);
        }
        if self.silent_since.is_none() && t >= REBOOT_GO_SILENT_WITHIN {
            return Some(false); // kept answering — no restart
        }
        if t >= REBOOT_CONFIRM_MAX {
            return Some(false); // silent, but never came back in time
        }
        None
    }
}

/// Poll `MSP_API_VERSION` after a reboot request on an MSP-over-MAVLink link and confirm the restart
/// (`RebootWatch`). `ack` = the reboot request itself was answered; if it was not, the FC counts as
/// silent from the request on. The scheduler going away (link lost with the restart) counts as a
/// restart, like on a direct link.
fn confirm_tunnel_reboot(handle: &MspRequester, started: Instant, ack: bool) -> bool {
    let mut watch = RebootWatch::default();
    if !ack {
        watch.sample(Duration::ZERO, false);
    }
    loop {
        let sent = started.elapsed();
        let (t, answered) = match handle.probe(MSP_API_VERSION, REBOOT_PROBE_TIMEOUT) {
            ProbeOutcome::Answered => (started.elapsed(), true),
            ProbeOutcome::Silent => {
                // A probe that failed at once (e.g. a write error) still paces the loop.
                let spent = started.elapsed() - sent;
                if spent < REBOOT_PROBE_TIMEOUT {
                    std::thread::sleep(REBOOT_PROBE_TIMEOUT - spent);
                }
                (sent, false)
            }
            ProbeOutcome::SchedulerGone => {
                log::info!("Geozones: MSP link gone after the reboot request — treating as restarted");
                return true;
            }
        };
        if let Some(verdict) = watch.sample(t, answered) {
            return verdict;
        }
        if started.elapsed() >= REBOOT_CONFIRM_MAX + REBOOT_PROBE_TIMEOUT {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn reboot_confirmed_after_silence_then_answer() {
        let mut w = RebootWatch::default();
        assert_eq!(w.sample(ms(0), false), None);
        assert_eq!(w.sample(ms(700), false), None);
        assert_eq!(w.sample(ms(1400), false), None);
        assert_eq!(w.sample(ms(2150), true), Some(true));
    }

    #[test]
    fn short_gap_is_a_lost_reply_not_a_reboot() {
        let mut w = RebootWatch::default();
        assert_eq!(w.sample(ms(0), false), None);
        assert_eq!(w.sample(ms(750), true), None); // 750 ms gap < 1 s
        assert_eq!(w.sample(ms(1500), true), None);
        assert_eq!(w.sample(ms(3100), true), Some(false)); // never silent long enough by 3 s
    }

    #[test]
    fn never_silent_means_no_reboot() {
        let mut w = RebootWatch::default();
        assert_eq!(w.sample(ms(20), true), None);
        assert_eq!(w.sample(ms(3000), true), Some(false));
    }

    #[test]
    fn silent_but_never_back_is_unconfirmed() {
        let mut w = RebootWatch::default();
        assert_eq!(w.sample(ms(0), false), None);
        assert_eq!(w.sample(ms(5000), false), None);
        assert_eq!(w.sample(ms(10_000), false), Some(false));
    }

    #[test]
    fn late_silence_start_keeps_waiting_past_3s() {
        let mut w = RebootWatch::default();
        assert_eq!(w.sample(ms(100), true), None);
        assert_eq!(w.sample(ms(2900), false), None);
        assert_eq!(w.sample(ms(3500), false), None);
        assert_eq!(w.sample(ms(4200), true), Some(true));
    }
}
