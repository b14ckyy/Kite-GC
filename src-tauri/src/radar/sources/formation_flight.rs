// Radar — FormationFlight (INAV-Radar / ESP32) over serial MSP (Phase F1).
//
// An ESP32 INAV-Radar module (LoRa / ESP-NOW) talks MSP to a flight controller: it polls the FC for its
// own telemetry, broadcasts it to the formation, and relays received peers back into the FC via
// MSP2_COMMON_SET_RADAR_POS. Kite joins as a GROUND NODE by emulating an INAV FC over this serial link:
//   - We answer the module's MSP requests as an armed "INAV" FC at the GCS location (so the module runs
//     as a full TX node — sidestepping the historical "GCS" listen-only bug, which gated on the host type).
//   - We parse the peers it pushes (MSP2_COMMON_SET_RADAR_POS) into TrackedVehicles (system=FormationFlight).
// Runs on a dedicated thread, fully isolated from the main scheduler/FC link. See
// docs/active/RADAR_FORMATION_FLIGHT.md.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::msp::codec::MspCodec;
use crate::msp::types::{
    MspDirection, MspMessage, MspVersion, MSP_ANALOG, MSP_API_VERSION, MSP_ATTITUDE, MSP_FC_VARIANT,
    MSP_FC_VERSION, MSP_NAME, MSP_RAW_GPS,
};
use crate::radar::now_ms;
use crate::radar::source::{RadarSource, SourceHandle, SourceUpdate};
use crate::radar::vehicle::{AltRef, TrackedVehicle, VehicleSource, VehicleSystem};
use crate::transport::serial::SerialConnection;
use crate::transport::{ByteTransport, TransportError};

/// INAV MSP2 radar message: the module pushes each peer's position with this command.
const MSP2_COMMON_SET_RADAR_POS: u16 = 0x100B;

/// Max craft-name bytes we report via MSP_NAME. The FormationFlight module stores it in a `char name[16]`
/// and uses it as a C-string; a name that fills all 16 bytes (no room for the NUL) overflows it and
/// crashes the firmware. Cap at 15 + ASCII-only to stay safe regardless of the module's missing bounds check.
const MSP_NAME_MAX: usize = 15;

/// Push the accumulated peer set at this cadence.
const EMIT_INTERVAL: Duration = Duration::from_millis(1000);
/// No update for this long ⇒ we consider the peer LOST (shown red).
const LOST_AFTER_MS: i64 = 12_000;
/// Keep a lost peer this long (at its last position, red) so the pilot still sees where it dropped.
const LOST_RETAIN_MS: i64 = 300_000; // 5 min
/// Backoff before reopening the port after a serial error.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// Resync guard: if the accumulator grows past this without yielding a frame, drop the leading byte.
const MAX_ACC: usize = 1024;

/// Live GCS node position (lat, lon, alt_m) we advertise as the "FC". Shared with the manager so it can
/// follow the resolved GCS location without restarting the source. `None` ⇒ we report no GPS fix.
pub type NodePos = Arc<Mutex<Option<(f64, f64, f64)>>>;
/// Live craft name we report via MSP_NAME. Shared so a name change never restarts the source (which would
/// reopen the serial port and reset the ESP32).
pub type NodeName = Arc<Mutex<String>>;

/// One tracked formation peer + when we last heard it (for the timeout-based "lost" state).
struct PeerEntry {
    vehicle: TrackedVehicle,
    last_heard: i64,
}

/// A parsed SET_RADAR_POS peer (armed/disarmed already baked into `vehicle.extra["ffState"]`).
struct ParsedPeer {
    id: u8,
    vehicle: TrackedVehicle,
}

pub struct FormationFlightSource {
    name: String,
    port: String,
    baud: u32,
    node_name: NodeName,
    node_pos: NodePos,
    #[allow(dead_code)]
    app: AppHandle,
}

impl FormationFlightSource {
    pub fn new(
        name: String,
        port: String,
        baud: u32,
        node_name: NodeName,
        node_pos: NodePos,
        app: AppHandle,
    ) -> Self {
        Self { name, port, baud, node_name, node_pos, app }
    }
}

impl RadarSource for FormationFlightSource {
    fn system(&self) -> VehicleSystem {
        VehicleSystem::FormationFlight
    }
    fn source(&self) -> VehicleSource {
        VehicleSource::FormationFlight
    }
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();

        thread::spawn(move || {
            while !stop_worker.load(Ordering::Relaxed) {
                let baud = if self.baud > 0 { self.baud } else { 115200 };
                let mut conn = match SerialConnection::open(&self.port, baud) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[radar][ff] {} open {} failed: {}", self.name, self.port, e);
                        if wait_or_stop(&stop_worker, RECONNECT_DELAY) {
                            break;
                        }
                        continue;
                    }
                };
                // Do NOT touch DTR/RTS here: toggling them resets the ESP32 (the USB auto-reset line). We
                // also keep this port open across radar reconfigures (the manager only restarts the FF
                // source when the port/baud actually change) so we don't reset the module mid-session.
                eprintln!("[radar][ff] {} MSP slave on {} @ {}", self.name, self.port, baud);

                let mut acc: Vec<u8> = Vec::with_capacity(512);
                let mut buf = [0u8; 512];
                let mut peers: HashMap<u8, PeerEntry> = HashMap::new();
                let mut last_emit = Instant::now();
                let mut dbg_req: HashMap<u16, u32> = HashMap::new();
                let mut dbg_pos = 0u32;

                loop {
                    if stop_worker.load(Ordering::Relaxed) {
                        return;
                    }
                    match conn.read_bytes(&mut buf) {
                        Ok(0) => {}
                        Ok(n) => {
                            acc.extend_from_slice(&buf[..n]);
                            while let Some(msg) = next_frame(&mut acc) {
                                if msg.direction != MspDirection::Request {
                                    continue; // we are the slave; ignore stray responses
                                }
                                // Peer push → parse into a contact (still ACK it — MSP requires a reply
                                // unless the master sets a no-reply flag, which this module does not).
                                if msg.code == MSP2_COMMON_SET_RADAR_POS {
                                    dbg_pos += 1;
                                    if let Some(p) = parse_radar_pos(&msg.payload, now_ms()) {
                                        peers.insert(p.id, PeerEntry { vehicle: p.vehicle, last_heard: now_ms() });
                                    }
                                } else {
                                    *dbg_req.entry(msg.code).or_insert(0) += 1;
                                }
                                // Reply (FC emulation for reads; empty ACK for sets/unknowns).
                                let node = self.node_pos.lock().ok().and_then(|g| *g);
                                let name = self.node_name.lock().map(|n| n.clone()).unwrap_or_default();
                                let resp = build_response(&msg, node, &name);
                                if let Err(e) = conn.write_bytes(&resp) {
                                    eprintln!("[radar][ff] {} write error: {} — reconnecting", self.name, e);
                                    break;
                                }
                            }
                            if acc.len() > MAX_ACC {
                                acc.drain(..1); // corrupt stream guard
                            }
                        }
                        Err(TransportError::Timeout) => {}
                        Err(e) => {
                            eprintln!("[radar][ff] {} serial error: {} — reconnecting", self.name, e);
                            break; // reopen
                        }
                    }

                    if last_emit.elapsed() >= EMIT_INTERVAL {
                        let now = now_ms();
                        // Drop peers not heard for the full retention window; keep the rest. Those past the
                        // LOST_AFTER threshold are shown red ("lost") at their last position — "lost" is a
                        // TIMEOUT we determine, not the firmware's state byte (which only gives armed/disarmed).
                        peers.retain(|_, e| now - e.last_heard <= LOST_RETAIN_MS);
                        let list: Vec<TrackedVehicle> = peers
                            .values()
                            .map(|e| {
                                let mut v = e.vehicle.clone();
                                if now - e.last_heard > LOST_AFTER_MS {
                                    v.extra.insert("ffState".to_string(), "lost".to_string());
                                    v.last_seen_ms = now; // refresh so the aggregator's short TTL keeps it
                                }
                                v
                            })
                            .collect();
                        let count = list.len();
                        if tx
                            .send(SourceUpdate { source: VehicleSource::FormationFlight, vehicles: list })
                            .is_err()
                        {
                            return; // aggregator gone
                        }
                        eprintln!(
                            "[radar][ff] {} peers={} radarPos/s={} reqs={:?}",
                            self.name, count, dbg_pos, dbg_req
                        );
                        dbg_pos = 0;
                        dbg_req.clear();
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

/// Pull the next complete MSP frame out of `acc`, draining its bytes. Resyncs past junk / a corrupt
/// frame; returns `None` when only an incomplete frame remains (wait for more bytes).
fn next_frame(acc: &mut Vec<u8>) -> Option<MspMessage> {
    loop {
        if acc.is_empty() {
            return None;
        }
        if acc[0] != b'$' {
            match acc.iter().position(|&b| b == b'$') {
                Some(pos) => {
                    acc.drain(..pos);
                    continue;
                }
                None => {
                    acc.clear();
                    return None;
                }
            }
        }
        if let Some((msg, consumed)) = MspCodec::decode(acc) {
            acc.drain(..consumed);
            return Some(msg);
        }
        // decode failed: a full-but-corrupt frame, or just incomplete. If the declared length is present
        // in the buffer, the checksum was bad → skip this '$' and resync; otherwise wait for more bytes.
        match declared_len(acc) {
            Some(total) if acc.len() >= total => {
                acc.drain(..1);
                continue;
            }
            _ => return None,
        }
    }
}

/// Total byte length of the frame whose header is at the front of `buf`, if the header is fully present.
fn declared_len(buf: &[u8]) -> Option<usize> {
    if buf.len() < 3 || buf[0] != b'$' {
        return None;
    }
    match buf[1] {
        b'M' => (buf.len() >= 4).then(|| 6 + buf[3] as usize),
        b'X' => (buf.len() >= 8).then(|| 9 + ((buf[6] as usize) | ((buf[7] as usize) << 8))),
        _ => None,
    }
}

/// Build the MSP response for an incoming request: FC emulation for the reads the module needs, an empty
/// ACK (same code) for sets/unknowns so the module's request loop keeps flowing.
fn build_response(msg: &MspMessage, node: Option<(f64, f64, f64)>, node_name: &str) -> Vec<u8> {
    let payload: Vec<u8> = match msg.code {
        MSP_API_VERSION => vec![0, 2, 5],                 // protocol, api major, api minor
        MSP_FC_VARIANT => b"INAV".to_vec(),               // host type — makes the module a full TX node
        MSP_FC_VERSION => vec![8, 0, 0],                  // major, minor, patch
        // Clamp to a NUL-safe length + printable ASCII — an over-long name overflows the module's
        // `char name[16]` and crashes it (the FF firmware lacks a bounds check).
        MSP_NAME => node_name
            .bytes()
            .filter(|b| b.is_ascii_graphic() || *b == b' ')
            .take(MSP_NAME_MAX)
            .collect(),
        MSP_RAW_GPS => raw_gps_payload(node),
        MSP_ATTITUDE => vec![0u8; 6],                     // roll/pitch/yaw decideg — level
        MSP_ANALOG => analog_payload(),
        _ => Vec::new(),                                  // empty ACK (incl. SET_RADAR_POS / ITD)
    };
    match msg.version {
        MspVersion::V1 => MspCodec::encode_v1_response(msg.code, &payload),
        MspVersion::V2 => MspCodec::encode_v2_response(msg.code, &payload),
    }
}

/// INAV `MSP_RAW_GPS` (106): fixType, numSat, lat/lon (deg·1e7), alt (m), groundSpeed (cm/s),
/// groundCourse (decideg), hdop. We advertise the GCS node position with a synthetic 3D fix.
fn raw_gps_payload(node: Option<(f64, f64, f64)>) -> Vec<u8> {
    let mut p = Vec::with_capacity(18);
    match node {
        Some((lat, lon, alt)) => {
            p.push(3); // fixType 3D
            p.push(12); // numSat
            p.extend_from_slice(&((lat * 1e7) as i32).to_le_bytes());
            p.extend_from_slice(&((lon * 1e7) as i32).to_le_bytes());
            p.extend_from_slice(&(alt.clamp(0.0, 65535.0) as u16).to_le_bytes());
            p.extend_from_slice(&0u16.to_le_bytes()); // groundSpeed cm/s (stationary)
            p.extend_from_slice(&0u16.to_le_bytes()); // groundCourse decideg
            p.extend_from_slice(&100u16.to_le_bytes()); // hdop
        }
        None => {
            p.push(0); // no fix
            p.push(0);
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&9999u16.to_le_bytes());
        }
    }
    p
}

/// INAV legacy `MSP_ANALOG` (110): vbat (0.1 V), mAh drawn, rssi, amperage (0.01 A). Dummy but valid.
fn analog_payload() -> Vec<u8> {
    let mut p = Vec::with_capacity(7);
    p.push(120); // 12.0 V
    p.extend_from_slice(&0u16.to_le_bytes()); // mAh
    p.extend_from_slice(&0u16.to_le_bytes()); // rssi
    p.extend_from_slice(&0i16.to_le_bytes()); // amperage
    p
}

/// Peer slot id → display letter (A..Z, matching the FF OSD; `id` itself beyond 25).
fn peer_letter(id: u8) -> String {
    if id < 26 {
        ((b'A' + id) as char).to_string()
    } else {
        id.to_string()
    }
}

/// Parse `MSP2_COMMON_SET_RADAR_POS` (19 bytes). `state` is 0 disarmed / 1 armed / 2 lost — only a truly
/// empty slot (null position) is skipped. The peer's letter (A..) goes into the callsign and the
/// armed/disarmed/lost state into `extra["ffState"]` for the frontend's colour.
fn parse_radar_pos(payload: &[u8], now: i64) -> Option<ParsedPeer> {
    if payload.len() < 19 {
        return None;
    }
    let id = payload[0];
    let state = payload[1];
    let lat = i32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]) as f64 / 1e7;
    let lon = i32::from_le_bytes([payload[6], payload[7], payload[8], payload[9]]) as f64 / 1e7;
    let alt_cm = i32::from_le_bytes([payload[10], payload[11], payload[12], payload[13]]);
    let heading = u16::from_le_bytes([payload[14], payload[15]]);
    let speed_cms = u16::from_le_bytes([payload[16], payload[17]]);
    let lq = payload[18];

    if lat == 0.0 && lon == 0.0 {
        return None; // empty slot (no peer in this slot)
    }

    let mut v = TrackedVehicle::new(
        format!("ff-{id}"),
        VehicleSystem::FormationFlight,
        VehicleSource::FormationFlight,
        lat,
        lon,
        now,
    );
    v.callsign = Some(peer_letter(id)); // A.. (matches the FF OSD); no real name over MSP
    v.alt_m = Some(alt_cm as f64 / 100.0);
    v.alt_ref = AltRef::GeoMsl;
    v.heading_deg = Some(heading as f64);
    v.ground_speed_ms = Some(speed_cms as f64 / 100.0);
    v.signal = Some(lq as f64); // link quality 0..4
    // The firmware state byte only distinguishes armed (1) from disarmed (anything else). "lost" is a
    // timeout we apply ourselves on emit, so it's never set here.
    let st = if state == 1 { "armed" } else { "disarmed" };
    v.extra.insert("ffState".to_string(), st.to_string());
    Some(ParsedPeer { id, vehicle: v })
}
