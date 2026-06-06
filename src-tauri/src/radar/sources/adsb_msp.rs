// Radar — ADS-B from the connected UAV via MSP (Phase 4).
//
// The INAV FC (8.0+) exposes its onboard ADS-B receiver's vehicle list via `MSP2_ADSB_VEHICLE_LIST`
// (0x2090). This is a *scheduler-fed* source: the MSP scheduler (which owns the link) polls the
// message and calls `decode()` here; the result is pushed into the radar aggregator (merged by ICAO
// like the online + serial ADS-B sources). See docs/active/RADAR_TRACKING_CORE.md §6, §7.1.
//
// Payload layout (from INAV fc_msp.c):
//   u8  max_vehicles · u8 callsign_len(=9) · u32 vehiclesMessagesTotal · u32 heartbeatMessagesTotal
//   then max_vehicles × {
//     callsign[callsign_len] · u32 icao · i32 lat(degE7) · i32 lon(degE7) · i32 alt
//     · u16 heading(deg) · u8 tslc · u8 emitterType · u8 ttl
//   }

use crate::radar::vehicle::{AltRef, TrackedVehicle, VehicleSource, VehicleSystem};

/// INAV ADS-B altitude is centimetres (FC internal convention) → metres.
const ADSB_ALT_CM_TO_M: f64 = 0.01;

/// Decode an `MSP2_ADSB_VEHICLE_LIST` payload into tracked vehicles (empty slots skipped).
pub fn decode(payload: &[u8], now_ms: i64) -> Vec<TrackedVehicle> {
    let mut r = Reader::new(payload);
    let max_vehicles = match r.u8() {
        Some(v) => v as usize,
        None => return Vec::new(),
    };
    let callsign_len = r.u8().unwrap_or(9) as usize;
    let _vehicles_total = r.u32();
    let _heartbeats_total = r.u32();

    let mut out = Vec::new();
    for _ in 0..max_vehicles {
        let callsign_bytes = match r.bytes(callsign_len) {
            Some(b) => b,
            None => break,
        };
        let (icao, lat_e7, lon_e7, alt, heading, _tslc, emitter, ttl) =
            match (r.u32(), r.i32(), r.i32(), r.i32(), r.u16(), r.u8(), r.u8(), r.u8()) {
                (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(g), Some(h)) => {
                    (a, b, c, d, e, f, g, h)
                }
                _ => break,
            };

        // Empty / expired slot.
        if ttl == 0 || icao == 0 || (lat_e7 == 0 && lon_e7 == 0) {
            continue;
        }

        let id = format!("{icao:06X}");
        let lat = lat_e7 as f64 / 1e7;
        let lon = lon_e7 as f64 / 1e7;
        let mut v = TrackedVehicle::new(id, VehicleSystem::Adsb, VehicleSource::AdsbMsp, lat, lon, now_ms);

        let callsign: String = callsign_bytes
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect::<String>()
            .trim()
            .to_string();
        v.callsign = (!callsign.is_empty()).then_some(callsign);
        v.alt_m = Some(alt as f64 * ADSB_ALT_CM_TO_M);
        v.alt_ref = AltRef::GeoMsl;
        v.heading_deg = Some(heading as f64); // already degrees
        v.category = emitter_category(emitter);
        out.push(v);
    }
    out
}

/// MAVLink/INAV ADS-B emitter type (numeric) → ADS-B category code (A1…C7), matching the online feeds.
fn emitter_category(emitter: u8) -> Option<String> {
    let code = match emitter {
        1 => "A1",  // light
        2 => "A2",  // small
        3 => "A3",  // large
        4 => "A4",  // high vortex large
        5 => "A5",  // heavy
        6 => "A6",  // highly manoeuvrable
        7 => "A7",  // rotorcraft
        9 => "B1",  // glider
        10 => "B2", // lighter-than-air
        11 => "B3", // parachutist
        12 => "B4", // ultralight
        14 => "B6", // UAV
        15 => "B7", // space
        17 => "C1", // emergency surface
        18 => "C2", // service surface
        19 => "C3", // point obstacle
        _ => return None,
    };
    Some(code.to_string())
}

/// Minimal little-endian byte reader for the MSP payload.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.bytes(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}
