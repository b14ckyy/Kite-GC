// Radar — ADS-B receiver over serial MAVLink (Phase 2).
//
// A dedicated hardware ADS-B receiver (e.g. ADSBee, PicoADSB) connected over Serial USB streams
// MAVLink `ADSB_VEHICLE` (#246) frames. This source opens the port, parses frames (reusing the
// project's MavParser), maps #246 → TrackedVehicle, and pushes batches to the aggregator (merged
// with the online ADS-B feeds by ICAO). Runs on a dedicated thread; reconnects on serial error.
// TCP/WiFi transport (same MAVLink decode) comes later. See docs/active/RADAR_TRACKING_CORE.md §7.1.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mavlink::ardupilotmega::MavMessage;
use mavlink::Message;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::mavlink_proto::parser::MavParser;
use crate::radar::now_ms;
use crate::radar::source::{RadarSource, SourceHandle, SourceUpdate};
use crate::radar::vehicle::{AltRef, TrackedVehicle, VehicleSource, VehicleSystem};
use crate::radar::ADSB_STATUS_EVENT;
use crate::transport::serial::SerialConnection;
use crate::transport::{ByteTransport, TransportError};

/// Push the accumulated set + a status tick at this cadence.
const EMIT_INTERVAL: Duration = Duration::from_millis(1000);
/// Drop a contact from this source's working set if it hasn't been heard for this long.
const RECEIVER_TTL_MS: i64 = 60_000;
/// Backoff before reopening the port after a serial error.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Per-source status (shared `radar-adsb-status` event, keyed by name).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdsbProviderStatus {
    name: String,
    count: usize,
    ok: bool,
}

pub struct AdsbMavlinkSource {
    name: String,
    port: String,
    baud: u32,
    app: AppHandle,
}

impl AdsbMavlinkSource {
    pub fn new(name: String, port: String, baud: u32, app: AppHandle) -> Self {
        Self { name, port, baud, app }
    }

    fn emit_status(&self, count: usize, ok: bool) {
        let _ = self.app.emit(
            ADSB_STATUS_EVENT,
            &[AdsbProviderStatus { name: self.name.clone(), count, ok }],
        );
    }
}

impl RadarSource for AdsbMavlinkSource {
    fn system(&self) -> VehicleSystem {
        VehicleSystem::Adsb
    }
    fn source(&self) -> VehicleSource {
        VehicleSource::AdsbReceiver
    }
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();

        thread::spawn(move || {
            while !stop_worker.load(Ordering::Relaxed) {
                let mut conn = match SerialConnection::open(&self.port, self.baud) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[radar][adsb-mav] {} open {} failed: {}", self.name, self.port, e);
                        self.emit_status(0, false);
                        if wait_or_stop(&stop_worker, RECONNECT_DELAY) {
                            break;
                        }
                        continue;
                    }
                };
                // Raise DTR/RTS — many USB-serial ADS-B receivers only stream once the host does.
                if let Err(e) = conn.set_control_signals(true, true) {
                    eprintln!("[radar][adsb-mav] {} DTR/RTS failed (continuing): {}", self.name, e);
                }
                eprintln!("[radar][adsb-mav] {} reading {} @ {}", self.name, self.port, self.baud);

                let mut parser = MavParser::new();
                let mut vehicles: HashMap<String, TrackedVehicle> = HashMap::new();
                let mut buf = [0u8; 512];
                let mut last_emit = Instant::now();
                // Diagnostics (kept on; ADS-B receivers vary wildly in framing/baud).
                let mut dbg_bytes = 0usize;
                let mut dbg_frames = 0usize;
                let mut dbg_ids: BTreeSet<u32> = BTreeSet::new();

                loop {
                    if stop_worker.load(Ordering::Relaxed) {
                        return;
                    }
                    match conn.read_bytes(&mut buf) {
                        Ok(0) => {}
                        Ok(n) => {
                            dbg_bytes += n;
                            for &b in &buf[..n] {
                                if let Some(frame) = parser.push(b) {
                                    dbg_frames += 1;
                                    dbg_ids.insert(frame.message.message_id());
                                    if let MavMessage::ADSB_VEHICLE(av) = frame.message {
                                        if let Some(v) = map_adsb_vehicle(&av, now_ms()) {
                                            vehicles.insert(v.id.clone(), v);
                                        }
                                    }
                                }
                            }
                        }
                        Err(TransportError::Timeout) => {}
                        Err(e) => {
                            eprintln!("[radar][adsb-mav] {} serial error: {} — reconnecting", self.name, e);
                            self.emit_status(0, false);
                            break; // reopen
                        }
                    }

                    if last_emit.elapsed() >= EMIT_INTERVAL {
                        let now = now_ms();
                        vehicles.retain(|_, v| now - v.last_seen_ms <= RECEIVER_TTL_MS);
                        let list: Vec<TrackedVehicle> = vehicles.values().cloned().collect();
                        let count = list.len();
                        if tx.send(SourceUpdate { source: VehicleSource::AdsbReceiver, vehicles: list }).is_err() {
                            return; // aggregator gone
                        }
                        self.emit_status(count, true);
                        eprintln!(
                            "[radar][adsb-mav] {} bytes/s={} frames/s={} msgIds={:?} tracked={}",
                            self.name, dbg_bytes, dbg_frames, dbg_ids, count
                        );
                        dbg_bytes = 0;
                        dbg_frames = 0;
                        dbg_ids.clear();
                        last_emit = Instant::now();
                    }
                }

                if wait_or_stop(&stop_worker, RECONNECT_DELAY) {
                    break;
                }
            }
        });

        SourceHandle::new(move || stop.store(true, Ordering::Relaxed))
    }
}

/// Interruptible wait. Returns true if a stop was requested during the wait.
fn wait_or_stop(stop: &Arc<AtomicBool>, dur: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < dur {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(step);
        waited += step;
    }
    false
}

/// Map a MAVLink ADSB_VEHICLE to a TrackedVehicle. Skips entries without a usable position.
fn map_adsb_vehicle(av: &mavlink::ardupilotmega::ADSB_VEHICLE_DATA, now: i64) -> Option<TrackedVehicle> {
    let lat = av.lat as f64 / 1e7;
    let lon = av.lon as f64 / 1e7;
    if lat == 0.0 && lon == 0.0 {
        return None;
    }
    let id = format!("{:06X}", av.ICAO_address);
    let mut v = TrackedVehicle::new(id, VehicleSystem::Adsb, VehicleSource::AdsbReceiver, lat, lon, now);

    let callsign = av.callsign.to_str().unwrap_or("").trim().to_string();
    v.callsign = (!callsign.is_empty()).then_some(callsign);
    v.alt_m = Some(av.altitude as f64 / 1000.0); // mm → m
    v.alt_ref = AltRef::GeoMsl;
    v.heading_deg = Some(av.heading as f64 / 100.0); // cdeg → deg
    v.ground_speed_ms = Some(av.hor_velocity as f64 / 100.0); // cm/s → m/s
    v.vertical_speed_ms = Some(av.ver_velocity as f64 / 100.0); // cm/s → m/s
    v.squawk = Some(av.squawk);
    v.category = emitter_category(&av.emitter_type);
    Some(v)
}

/// Map MAVLink `ADSB_EMITTER_TYPE` to the same ADS-B category code the online feeds use (A1…C7), so
/// the frontend's type abbreviation works uniformly. Matches on the Debug name to stay robust across
/// `mavlink` crate versions; order matters (specific substrings first).
fn emitter_category(emitter: &mavlink::ardupilotmega::AdsbEmitterType) -> Option<String> {
    let n = format!("{emitter:?}").to_uppercase();
    let code = if n.contains("ROTOCRAFT") || n.contains("ROTORCRAFT") {
        "A7"
    } else if n.contains("GLIDER") {
        "B1"
    } else if n.contains("LIGHTER") {
        "B2"
    } else if n.contains("PARACHUTE") {
        "B3"
    } else if n.contains("ULTRA") {
        "B4"
    } else if n.contains("UAV") {
        "B6"
    } else if n.contains("SPACE") {
        "B7"
    } else if n.contains("HIGH_VORTEX") {
        "A4"
    } else if n.contains("HEAVY") {
        "A5"
    } else if n.contains("HIGHLY") {
        "A6"
    } else if n.contains("SMALL") {
        "A2"
    } else if n.contains("LARGE") {
        "A3"
    } else if n.contains("LIGHT") {
        "A1"
    } else if n.contains("POINT_OBSTACLE") {
        "C3"
    } else if n.contains("SURFACE") {
        "C1"
    } else {
        return None;
    };
    Some(code.to_string())
}
