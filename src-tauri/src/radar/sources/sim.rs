// Radar — dev-only synthetic source.
//
// Emits a handful of moving contacts across ALL three systems (a single source may report vehicles
// of different `system`s; the aggregator routes by `vehicle.system`). Exists only to exercise the
// pipeline / store / panel end-to-end before real sources land. Compiled only in debug builds via
// the call site (`#[cfg(debug_assertions)]` in the manager).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::radar::now_ms;
use crate::radar::source::{RadarSource, SourceHandle, SourceUpdate};
use crate::radar::vehicle::{AltRef, TrackedVehicle, VehicleSource, VehicleSystem};

pub struct SimSource {
    center: (f64, f64),
}

impl SimSource {
    pub fn new(center: (f64, f64)) -> Self {
        Self { center }
    }
}

impl RadarSource for SimSource {
    fn system(&self) -> VehicleSystem {
        VehicleSystem::Adsb // nominal; the batch mixes systems
    }
    fn source(&self) -> VehicleSource {
        VehicleSource::Sim
    }
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let (clat, clon) = self.center;
        thread::spawn(move || {
            let mut t = 0.0_f64;
            while !stop_worker.load(Ordering::Relaxed) {
                let vehicles = build(clat, clon, t, now_ms());
                if tx.send(SourceUpdate { source: VehicleSource::Sim, vehicles }).is_err() {
                    break;
                }
                t += 1.0;
                thread::sleep(Duration::from_millis(1000));
            }
        });
        SourceHandle::new(move || stop.store(true, Ordering::Relaxed))
    }
}

/// Offset a lat/lon by metres north/east.
fn offset(lat: f64, lon: f64, north_m: f64, east_m: f64) -> (f64, f64) {
    let dlat = north_m / 111_320.0;
    let dlon = east_m / (111_320.0 * lat.to_radians().cos().abs().max(0.01));
    (lat + dlat, lon + dlon)
}

/// One contact orbiting `center` at radius `r` metres, angular rate `w` rad/s.
#[allow(clippy::too_many_arguments)]
fn orbiting(
    id: &str,
    system: VehicleSystem,
    callsign: &str,
    clat: f64,
    clon: f64,
    r: f64,
    w: f64,
    t: f64,
    alt_m: f64,
    alt_ref: AltRef,
    gs: f64,
    now: i64,
) -> TrackedVehicle {
    let a = w * t;
    let (lat, lon) = offset(clat, clon, r * a.cos(), r * a.sin());
    // Tangential heading (deg, 0 = N): velocity direction is the orbit angle ± 90°.
    let heading = (a.to_degrees() + if w >= 0.0 { 90.0 } else { -90.0 }).rem_euclid(360.0);
    let mut v = TrackedVehicle::new(id, system, VehicleSource::Sim, lat, lon, now);
    v.callsign = Some(callsign.to_string());
    v.alt_m = Some(alt_m);
    v.alt_ref = alt_ref;
    v.heading_deg = Some(heading);
    v.ground_speed_ms = Some(gs);
    v
}

fn build(clat: f64, clon: f64, t: f64, now: i64) -> Vec<TrackedVehicle> {
    vec![
        orbiting("3C6DA1", VehicleSystem::Adsb, "DLH2AB", clat, clon, 9000.0, 0.05, t, 3000.0, AltRef::BaroMsl, 210.0, now),
        orbiting("4CA2BC", VehicleSystem::Adsb, "RYR41X", clat, clon, 5000.0, -0.08, t, 1500.0, AltRef::BaroMsl, 180.0, now),
        orbiting("FF-PEER1", VehicleSystem::FormationFlight, "Peer-1", clat, clon, 350.0, 0.3, t, 20.0, AltRef::Relative, 8.0, now),
        orbiting("TX-2", VehicleSystem::Radio, "Wing-2", clat, clon, 1200.0, 0.15, t, 200.0, AltRef::GeoMsl, 15.0, now),
    ]
}
