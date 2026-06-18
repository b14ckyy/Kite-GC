// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// LTM (Lightweight TeleMetry) decoder — passive telemetry.
//
// LTM is a simple, fixed-layout serial telemetry protocol from INAV/Cleanflight. Each frame is
// `'$' 'T' <type> <payload...> <crc>` where the payload length is fixed per type and the CRC is the XOR
// of the payload bytes only (NOT the '$T' header or the type char). All multi-byte fields are
// little-endian. Source: INAV `telemetry/ltm.c` + `ltm.h` (see docs/active/RADIO_TELEMETRY.md).
//
// We decode A (attitude), G (GPS) and S (status) into the unified telemetry events + recorder; O (home),
// N (nav) and X (extra) are decoded into the analysis dump only. LTM carries no course-over-ground and
// no vario, so those stay 0.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::flightmode::FlightModeState;
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, StatusData,
};

/// INAV arming_flags bit 2 = ARMED (what the recorder + frontend look for).
const ARMED_FLAG: u32 = 0x04;

/// LTM carries no course-over-ground, so we synthesize it from successive GPS fixes. To avoid GPS-jitter
/// noise we only update the COG once the aircraft has both moved at least `MIN_COG_DIST_M` from the last
/// anchor and is going faster than `MIN_COG_SPEED_MS` (otherwise we hold the last COG / fall back to
/// heading). The anchor baseline keeps the bearing stable regardless of GPS frame rate.
const MIN_COG_SPEED_MS: f64 = 1.0;
const MIN_COG_DIST_M: f64 = 3.0;

/// LTM carries no vertical speed either, so we emulate it from the altitude derivative between G-frames.
/// The raw derivative is noisy, so we low-pass it with this EMA factor (per altitude sample).
const VARIO_SMOOTH: f64 = 0.3;

/// Great-circle distance (m) between two lat/lon points.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Initial bearing (forward azimuth, 0..360°) from point 1 to point 2.
fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dlon.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

/// Payload length (bytes, excluding the `$T<type>` header and the trailing CRC) for each LTM frame
/// type, or `None` for an unknown type. Shared with the detector so the LTM frame spec lives in one
/// place.
pub fn ltm_payload_len(ty: u8) -> Option<usize> {
    Some(match ty {
        b'A' => 6,  // attitude:  pitch/roll/yaw i16
        b'G' => 14, // gps:        lat i32, lon i32, gs u8, alt i32, sat/fix u8
        b'S' => 7,  // status:     vbat u16, mah u16, rssi u8, airspeed u8, statemode u8
        b'O' => 14, // origin/home lat i32, lon i32, alt i32, osd u8, fix u8
        b'N' => 6,  // nav:        6×u8
        b'X' => 6,  // extra:      hdop u16, hw u8, counter u8, disarm u8, reserved u8
        _ => return None,
    })
}

fn le_u16(b: &[u8]) -> u16 {
    (b[0] as u16) | ((b[1] as u16) << 8)
}
fn le_i16(b: &[u8]) -> i16 {
    le_u16(b) as i16
}
fn le_i32(b: &[u8]) -> i32 {
    (b[0] as i32) | ((b[1] as i32) << 8) | ((b[2] as i32) << 16) | ((b[3] as i32) << 24)
}

/// Map the LTM status-byte flight-mode number (`ltm_modes_e`) + failsafe bit to the canonical model.
/// Mode ordinals are from INAV `telemetry/ltm.h`. LTM collapses to a single dominant mode, so
/// modifier-like modes (alt/heading hold, autotune) are emitted as a best-fit primary + modifier.
fn classify_ltm_mode(mode: u8, failsafe: bool) -> FlightModeState {
    if failsafe {
        let p = if mode == 13 { "failsafe_rth" } else { "failsafe" };
        return FlightModeState { primary: p.to_string(), modifiers: Vec::new() };
    }
    let (primary, mods): (&str, &[&str]) = match mode {
        0 => ("manual", &[]),
        2 => ("angle", &[]),
        3 => ("horizon", &[]),
        8 => ("angle", &["althold"]),
        9 | 12 | 14 => ("poshold", &[]), // GPS hold / circle / follow-me
        10 => ("mission", &[]),
        11 => ("angle", &["headinghold"]),
        13 => ("rth", &[]),
        15 => ("rth", &["autoland"]),
        18 => ("cruise", &[]),
        20 => ("launch", &[]),
        21 => ("angle", &["autotune"]),
        // 1 rate, 4 acro, 5-7 stabilized, 16/17 flybywire, 19 unknown → acro/angle fallback
        1 | 4 | 5 | 6 | 7 => ("acro", &[]),
        16 | 17 => ("angle", &[]),
        _ => ("acro", &[]),
    };
    FlightModeState { primary: primary.to_string(), modifiers: mods.iter().map(|s| s.to_string()).collect() }
}

enum Decoded {
    Attitude { pitch: f64, roll: f64, yaw: f64 },
    Gps { lat: f64, lon: f64, gs_ms: f64, alt_m: f64, sats: u8, fix: u8 },
    Status { volts: f64, mah: u32, rssi: u16, airspeed_ms: f64, armed: bool, failsafe: bool, mode: u8 },
    Home { lat: f64, lon: f64, alt_m: f64, fix_home: u8 },
    Nav { raw: Vec<u8> },
    Extra { hdop: u16, hw_fail: u8, disarm_reason: u8 },
    Other,
}

fn decode_ltm(ty: u8, p: &[u8]) -> Decoded {
    match ty {
        b'A' if p.len() >= 6 => Decoded::Attitude {
            pitch: le_i16(&p[0..2]) as f64,
            roll: le_i16(&p[2..4]) as f64,
            yaw: (le_i16(&p[4..6]) as f64).rem_euclid(360.0),
        },
        b'G' if p.len() >= 14 => {
            let satfix = p[13];
            let raw_fix = satfix & 0x03; // 1=no fix, 2=2D, 3=3D
            Decoded::Gps {
                lat: le_i32(&p[0..4]) as f64 / 1e7,
                lon: le_i32(&p[4..8]) as f64 / 1e7,
                gs_ms: p[8] as f64,
                alt_m: le_i32(&p[9..13]) as f64 / 100.0, // cm → m
                sats: satfix >> 2,
                fix: raw_fix,
            }
        }
        b'S' if p.len() >= 7 => {
            let statemode = p[6];
            Decoded::Status {
                volts: le_u16(&p[0..2]) as f64 / 1000.0, // frame is mV (getBatteryVoltage()*10)
                mah: le_u16(&p[2..4]) as u32,
                rssi: (p[4] as u32 * 1023 / 254) as u16, // LTM 0..254 → MSP 0..1023 scale
                airspeed_ms: p[5] as f64,
                armed: statemode & 0x01 != 0,
                failsafe: statemode & 0x02 != 0,
                mode: statemode >> 2,
            }
        }
        b'O' if p.len() >= 14 => Decoded::Home {
            lat: le_i32(&p[0..4]) as f64 / 1e7,
            lon: le_i32(&p[4..8]) as f64 / 1e7,
            alt_m: le_i32(&p[8..12]) as f64 / 100.0,
            fix_home: p[13],
        },
        b'N' if p.len() >= 6 => Decoded::Nav { raw: p.to_vec() },
        b'X' if p.len() >= 6 => Decoded::Extra {
            hdop: le_u16(&p[0..2]),
            hw_fail: p[2],
            disarm_reason: p[4],
        },
        _ => Decoded::Other,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Accumulated decoded telemetry (mirrors the FrSky/CRSF decoders).
#[derive(Default)]
struct State {
    roll: f64,
    pitch: f64,
    yaw: f64,
    seen_attitude: bool,

    lat: f64,
    lon: f64,
    alt: f64,
    ground_speed: f64,
    num_sat: u8,
    have_fix: bool,
    fix_type: u8,
    seen_gps: bool,

    // Synthesized course-over-ground (LTM sends none — see MIN_COG_* above).
    course: f64,
    anchor_lat: f64,
    anchor_lon: f64,
    have_anchor: bool,
    have_course: bool,

    // Altitude (baro/estimated, present even without a GPS fix) + emulated vario.
    alt_seen: bool,
    vario: f64,
    prev_alt: f64,
    last_alt_ms: u128,
    have_alt_prev: bool,

    voltage: f64,
    mah_drawn: u32,
    rssi: u16,
    seen_analog: bool,

    airspeed: f64,
    seen_airspeed: bool,

    armed: bool,
    mode: FlightModeState,
    seen_status: bool,

    // Per-type "a fresh frame arrived since the last publish" flags — emission gates on these so cached
    // state isn't re-published at the fixed handler tick (output rate follows the real frame rate). Set in
    // apply(), cleared in publish().
    fresh_attitude: bool,
    fresh_gps: bool,
    fresh_altitude: bool,
    fresh_analog: bool,
    fresh_airspeed: bool,
    fresh_status: bool,
}

pub struct LtmDecoder {
    acc: Vec<u8>,
    state: State,
    start: Instant,
    frames: u64,
    dump: Option<BufWriter<File>>,
    /// Last time a valid LTM frame was seen (all LTM frames are FC-origin) — drives FC-link-alive.
    last_fc: Option<Instant>,
}

impl LtmDecoder {
    /// Create a decoder. If `dump_path` is given, decoded frames are appended there for analysis.
    pub fn new(dump_path: Option<PathBuf>) -> Self {
        let dump = dump_path.and_then(|p| match File::create(&p) {
            Ok(f) => {
                log::info!("LTM decoded dump → {}", p.to_string_lossy());
                Some(BufWriter::new(f))
            }
            Err(e) => {
                log::warn!("Failed to open LTM dump {}: {}", p.to_string_lossy(), e);
                None
            }
        });
        Self {
            acc: Vec::with_capacity(64),
            state: State::default(),
            start: Instant::now(),
            frames: 0,
            dump,
            last_fc: None,
        }
    }

    /// Age (ms) of the last valid LTM frame, or None if none seen yet.
    pub fn fc_age_ms(&self) -> Option<u128> {
        self.last_fc.map(|t| t.elapsed().as_millis())
    }

    /// Feed a freshly-read chunk; extract complete CRC-valid LTM frames, apply them to the accumulated
    /// state and (if a dump is open) append a decoded line.
    pub fn push_bytes(&mut self, data: &[u8]) {
        self.acc.extend_from_slice(data);

        let mut i = 0;
        while i + 2 < self.acc.len() {
            if self.acc[i] != b'$' || self.acc[i + 1] != b'T' {
                i += 1;
                continue;
            }
            let ty = self.acc[i + 2];
            let Some(plen) = ltm_payload_len(ty) else {
                i += 1;
                continue;
            };
            let crc_idx = i + 3 + plen;
            if crc_idx >= self.acc.len() {
                break; // need more bytes for the full frame
            }
            let crc = self.acc[i + 3..crc_idx].iter().fold(0u8, |c, &b| c ^ b);
            if crc != self.acc[crc_idx] {
                i += 1; // bad frame — resync one byte at a time
                continue;
            }
            let payload = self.acc[i + 3..crc_idx].to_vec();
            let decoded = decode_ltm(ty, &payload);
            self.frames += 1;
            self.last_fc = Some(Instant::now()); // all LTM frames are FC-origin
            self.apply(&decoded);
            self.dump_frame(ty, &payload, &decoded);
            i = crc_idx + 1; // consume the validated frame
        }

        self.acc.drain(0..i);
        if self.acc.len() > 256 {
            self.acc.clear(); // runaway garbage — drop and resync
        }
    }

    fn apply(&mut self, d: &Decoded) {
        let now_ms = self.start.elapsed().as_millis();
        let s = &mut self.state;
        match d {
            Decoded::Attitude { pitch, roll, yaw } => {
                s.pitch = *pitch;
                s.roll = *roll;
                s.yaw = *yaw;
                s.seen_attitude = true;
                s.fresh_attitude = true;
            }
            Decoded::Gps { lat, lon, gs_ms, alt_m, sats, fix } => {
                s.lat = *lat;
                s.lon = *lon;
                s.ground_speed = *gs_ms;
                s.alt = *alt_m;
                s.num_sat = *sats;
                s.fix_type = if *fix == 3 { 3 } else if *fix == 2 { 2 } else { 0 };
                s.have_fix = *fix >= 2;
                s.seen_gps = true;
                s.fresh_gps = true;

                // Altitude (baro/estimated) is present even without a GPS fix; emulate vertical speed
                // from its time derivative (LTM has no vario), low-passed to tame the noise.
                if s.have_alt_prev {
                    let dt = now_ms.saturating_sub(s.last_alt_ms) as f64 / 1000.0;
                    if dt > 0.05 && dt < 2.0 {
                        let raw = (*alt_m - s.prev_alt) / dt;
                        s.vario = s.vario * (1.0 - VARIO_SMOOTH) + raw * VARIO_SMOOTH;
                    }
                }
                s.prev_alt = *alt_m;
                s.last_alt_ms = now_ms;
                s.have_alt_prev = true;
                s.alt_seen = true;
                s.fresh_altitude = true;

                // Synthesize COG from successive fixes (LTM has none). Gate on speed + a minimum
                // baseline from the last anchor so it isn't GPS jitter; until we have a real COG, fall
                // back to heading so the direction markers track the nose instead of freezing.
                if s.have_fix {
                    if !s.have_anchor {
                        s.anchor_lat = *lat;
                        s.anchor_lon = *lon;
                        s.have_anchor = true;
                    }
                    let moved = haversine_m(s.anchor_lat, s.anchor_lon, *lat, *lon);
                    if *gs_ms >= MIN_COG_SPEED_MS && moved >= MIN_COG_DIST_M {
                        s.course = bearing_deg(s.anchor_lat, s.anchor_lon, *lat, *lon);
                        s.have_course = true;
                        s.anchor_lat = *lat;
                        s.anchor_lon = *lon;
                    } else if !s.have_course {
                        s.course = s.yaw; // heading fallback until the first reliable COG
                    }
                }
            }
            Decoded::Status { volts, mah, rssi, airspeed_ms, armed, failsafe, mode } => {
                s.voltage = *volts;
                s.mah_drawn = *mah;
                s.rssi = *rssi;
                s.seen_analog = true;
                s.fresh_analog = true;
                s.airspeed = *airspeed_ms;
                if *airspeed_ms > 0.0 {
                    s.seen_airspeed = true;
                    s.fresh_airspeed = true;
                }
                s.armed = *armed;
                s.mode = classify_ltm_mode(*mode, *failsafe);
                s.seen_status = true;
                s.fresh_status = true;
            }
            Decoded::Home { .. } | Decoded::Nav { .. } | Decoded::Extra { .. } | Decoded::Other => {}
        }
    }

    fn dump_frame(&mut self, ty: u8, payload: &[u8], decoded: &Decoded) {
        let cog = self.state.course; // synthesized COG / emulated vario (read before borrowing dump)
        let vario = self.state.vario;
        let Some(w) = self.dump.as_mut() else { return };
        let t_ms = self.start.elapsed().as_millis();
        let (name, desc) = match decoded {
            Decoded::Attitude { pitch, roll, yaw } => {
                ("ATTITUDE", format!("pitch={:.0} roll={:.0} yaw={:.0}", pitch, roll, yaw))
            }
            Decoded::Gps { lat, lon, gs_ms, alt_m, sats, fix } => (
                "GPS",
                format!("lat={:.7} lon={:.7} spd={:.0}m/s alt={:.1}m sats={} fix={} cog_est={:.1} vario_est={:.2}", lat, lon, gs_ms, alt_m, sats, fix, cog, vario),
            ),
            Decoded::Status { volts, mah, rssi, airspeed_ms, armed, failsafe, mode } => (
                "STATUS",
                format!("volt={:.2}V mah={} rssi={} aspd={:.0}m/s armed={} fs={} mode={}", volts, mah, rssi, airspeed_ms, armed, failsafe, mode),
            ),
            Decoded::Home { lat, lon, alt_m, fix_home } => {
                ("HOME", format!("lat={:.7} lon={:.7} alt={:.1}m fix={}", lat, lon, alt_m, fix_home))
            }
            Decoded::Nav { raw } => ("NAV", format!("raw=[{}]", hex(raw))),
            Decoded::Extra { hdop, hw_fail, disarm_reason } => {
                ("EXTRA", format!("hdop={} hw_fail={} disarm={}", hdop, hw_fail, disarm_reason))
            }
            Decoded::Other => ("?", String::from("(undecoded)")),
        };
        let line = format!(
            "[{:>8}ms] $T{} {:<9} raw=[{}] {}\n",
            t_ms,
            ty as char,
            name,
            hex(payload),
            desc
        );
        let _ = w.write_all(line.as_bytes());
    }

    /// Emit the accumulated state as the unified telemetry events and feed the flight recorder. Each
    /// event is only sent once its relevant fields have been seen (mirrors the FrSky/CRSF decoders).
    pub fn publish(&mut self, app: &AppHandle, recorder: Option<&FlightRecorderHandle>) {
        // Only emit a type when a fresh frame updated it since the last publish (no fixed-tick re-publish
        // of cached state). Capture + clear the flags, then emit from the immutably-borrowed state.
        let (f_status, f_att, f_gps, f_alt, f_an, f_asp) = {
            let s = &self.state;
            (s.fresh_status, s.fresh_attitude, s.fresh_gps && s.have_fix, s.fresh_altitude, s.fresh_analog, s.fresh_airspeed)
        };
        self.state.fresh_status = false;
        self.state.fresh_attitude = false;
        self.state.fresh_gps = false;
        self.state.fresh_altitude = false;
        self.state.fresh_analog = false;
        self.state.fresh_airspeed = false;
        let s = &self.state;

        if f_status {
            let status = StatusData {
                arming_flags: if s.armed { ARMED_FLAG } else { 0 },
                flight_mode_flags: 0, // LTM carries a single mode number, not the raw INAV bitmask
                cpu_load: 0,
                sensor_status: 0,
            };
            let _ = app.emit("telemetry-status", &status);
            let _ = app.emit("telemetry-flightmode", &s.mode);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_status(&status);
                    r.on_flightmode(&s.mode);
                }
            }
        }

        if f_att {
            let att = AttitudeData { roll: s.roll, pitch: s.pitch, yaw: s.yaw };
            let _ = app.emit("telemetry-attitude", &att);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_attitude(&att); }
            }
        }

        if f_gps {
            let gps = GpsData {
                fix_type: s.fix_type,
                num_sat: s.num_sat,
                lat: s.lat,
                lon: s.lon,
                alt_msl: s.alt,
                ground_speed: s.ground_speed,
                course: s.course, // synthesized from GPS fixes (LTM carries no COG)
            };
            let _ = app.emit("telemetry-gps", &gps);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_gps(&gps); }
            }
        }

        // Altitude (G-frame baro/estimated alt) + emulated vario — independent of GPS fix.
        if f_alt {
            let alt = AltitudeData { altitude: s.alt, vario: s.vario };
            let _ = app.emit("telemetry-altitude", &alt);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_altitude(&alt); }
            }
        }

        if f_an {
            let analog = AnalogData {
                voltage: s.voltage,
                mah_drawn: s.mah_drawn,
                rssi: s.rssi,
                current: 0.0, // LTM has no instantaneous current (only mAh drawn)
                power: 0.0,
                battery_percentage: 0,
                cell_count: 0,
            };
            let _ = app.emit("telemetry-analog", &analog);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_analog(&analog); }
            }
        }

        if f_asp {
            let aspd = AirspeedData { airspeed: s.airspeed };
            let _ = app.emit("telemetry-airspeed", &aspd);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_airspeed(&aspd); }
            }
        }
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn flush(&mut self) {
        if let Some(w) = self.dump.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Drop for LtmDecoder {
    fn drop(&mut self) {
        self.flush();
    }
}
