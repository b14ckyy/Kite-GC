// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// CRSF (Crossfire / ELRS) decoder — passive telemetry.
//
// A streaming, self-resynchronizing CRSF frame parser. CRSF is not delimiter-framed, so we scan for a
// plausible `<sync> <len>` header, require the full frame to be present, and validate the trailing
// CRC8/DVB-S2 over `type..payload` (same poly as MSP v2). Only CRC-valid frames are decoded; on a CRC
// miss we advance one byte and resync.
//
// Each valid frame is decoded per the INAV crsf.c scalings (see docs/active/RADIO_TELEMETRY.md) into an
// accumulated state and:
//   - E2: appended as a human-readable line to a `radiotelem_<ts>.crsf.txt` dump (offline validation),
//   - E4: published as the unified telemetry events (same names/payloads as MSP/MAVLink/FrSky) and fed
//     to the flight recorder via `publish()`.
//
// Validation status (2026-06-17): framing/CRC and the ATTITUDE scaling are bench-confirmed. GPS/battery/
// airspeed/baro numeric scalings are source-derived (bench capture had no fix/battery/baro). The flight-
// mode (0x21) string→mode mapping is source-derived and UNVALIDATED — the bench capture carried no 0x21
// frames; confirm on a real armed INAV+CRSF flight.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::flightmode::FlightModeState;
use crate::msp::codec::MspCodec;
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, LinkStatsData, StatusData,
};

use super::ap_passthrough::ApPassthroughDecoder;

/// CRSF frame types carrying ArduPilot passthrough packets (AP_CUSTOM_TELEM, new + legacy) → routed to
/// the shared AP-passthrough sub-decoder (level-2 sub-detection).
const FT_AP_CUSTOM_TELEM: u8 = 0x80;
const FT_AP_CUSTOM_TELEM_LEGACY: u8 = 0x7F;

/// INAV arming_flags bit 2 = ARMED (what the recorder + frontend look for).
const ARMED_FLAG: u32 = 0x04;

// ── INAV CRSF telemetry frame types ───────────────────────────────────────────
const FT_GPS: u8 = 0x02;
const FT_VARIO: u8 = 0x07;
const FT_BATTERY: u8 = 0x08;
const FT_BARO_ALT: u8 = 0x09;
const FT_AIRSPEED: u8 = 0x0A;
const FT_ATTITUDE: u8 = 0x1E;
const FT_FLIGHT_MODE: u8 = 0x21;
const FT_LINK_STATS: u8 = 0x14;

/// Decoded view of one CRSF frame (E2: used for the analysis dump; E4 will reuse the scalings).
enum Decoded {
    Gps { lat: f64, lon: f64, gs_ms: f64, course: f64, alt_m: i32, sats: u8 },
    Vario { vspeed_ms: f64 },
    Battery { volts: f64, amps: f64, mah: u32, pct: u8 },
    BaroAlt { alt_m: f64, raw: u16 },
    Airspeed { aspd_ms: f64 },
    Attitude { pitch: f64, roll: f64, yaw: f64 },
    FlightMode { text: String },
    LinkStats { rssi_dbm: i16, lq: u8, snr: i8 },
    Other,
}

/// Decode a single CRSF frame payload (frame type + payload, CRC already validated by the caller).
/// Scalings mirror INAV `telemetry/crsf.c` — see docs/active/RADIO_TELEMETRY.md.
fn decode_frame(ty: u8, p: &[u8]) -> Decoded {
    match ty {
        FT_GPS if p.len() >= 15 => {
            let lat = be_i32(&p[0..4]) as f64 / 1e7;
            let lon = be_i32(&p[4..8]) as f64 / 1e7;
            let gs_ms = be_u16(&p[8..10]) as f64 / 36.0; // frame is km/h * 10
            let course = be_u16(&p[10..12]) as f64 / 100.0; // frame is centidegrees
            let alt_m = be_u16(&p[12..14]) as i32 - 1000; // 1000 m offset
            let sats = p[14];
            Decoded::Gps { lat, lon, gs_ms, course, alt_m, sats }
        }
        FT_VARIO if p.len() >= 2 => Decoded::Vario { vspeed_ms: be_i16(&p[0..2]) as f64 / 100.0 },
        FT_BATTERY if p.len() >= 8 => {
            let volts = be_u16(&p[0..2]) as f64 / 10.0; // INAV: getBatteryVoltage()/10 (centi → deci-V)
            let amps = be_u16(&p[2..4]) as f64 / 10.0;
            let mah = ((p[4] as u32) << 16) | ((p[5] as u32) << 8) | p[6] as u32;
            let pct = p[7];
            Decoded::Battery { volts, amps, mah, pct }
        }
        FT_BARO_ALT if p.len() >= 2 => {
            let raw = be_u16(&p[0..2]);
            // Packed: high bit set ⇒ value in metres; else decimetres with a 10000 dm offset.
            let alt_m = if raw & 0x8000 != 0 {
                (raw & 0x7FFF) as f64
            } else {
                (raw as f64 - 10000.0) / 10.0
            };
            Decoded::BaroAlt { alt_m, raw }
        }
        FT_AIRSPEED if p.len() >= 2 => Decoded::Airspeed { aspd_ms: be_u16(&p[0..2]) as f64 / 36.0 },
        FT_ATTITUDE if p.len() >= 6 => {
            // pitch, roll, yaw — radians * 10000.
            let pitch = (be_i16(&p[0..2]) as f64 / 10000.0).to_degrees();
            let roll = (be_i16(&p[2..4]) as f64 / 10000.0).to_degrees();
            let yaw = (be_i16(&p[4..6]) as f64 / 10000.0).to_degrees().rem_euclid(360.0);
            Decoded::Attitude { pitch, roll, yaw }
        }
        FT_FLIGHT_MODE => {
            // Null-terminated ASCII string.
            let end = p.iter().position(|&b| b == 0).unwrap_or(p.len());
            Decoded::FlightMode { text: String::from_utf8_lossy(&p[..end]).into_owned() }
        }
        FT_LINK_STATS if p.len() >= 10 => {
            // LinkStatistics (RF link health). We surface the UPLINK (handset→model control link) —
            // what a pilot watches. RSSI is stored as a positive value = −dBm; the active antenna
            // (p[4]) selects which of the two uplink RSSI readings is live.
            let rssi_pos = if p[4] == 1 { p[1] } else { p[0] };
            Decoded::LinkStats { rssi_dbm: -(rssi_pos as i16), lq: p[2], snr: p[3] as i8 }
        }
        _ => Decoded::Other,
    }
}

fn be_u16(b: &[u8]) -> u16 {
    ((b[0] as u16) << 8) | b[1] as u16
}
fn be_i16(b: &[u8]) -> i16 {
    be_u16(b) as i16
}
fn be_i32(b: &[u8]) -> i32 {
    ((b[0] as i32) << 24) | ((b[1] as i32) << 16) | ((b[2] as i32) << 8) | b[3] as i32
}

fn frame_name(ty: u8) -> &'static str {
    match ty {
        FT_GPS => "GPS",
        FT_VARIO => "VARIO",
        FT_BATTERY => "BATTERY",
        FT_BARO_ALT => "BARO_ALT",
        FT_AIRSPEED => "AIRSPEED",
        FT_ATTITUDE => "ATTITUDE",
        FT_FLIGHT_MODE => "FLT_MODE",
        0x0C => "RPM",
        0x0D => "TEMP",
        0x14 => "LINK_STATS", // RF link health (RSSI/LQ/SNR) — from the radio/RX, not INAV
        0x16 => "RC_CHANNELS",
        0x1C => "LINK_RX",
        0x1D => "LINK_TX",
        0x29 => "DEVICE_INFO",
        0x3A => "RADIO_ID",
        0x7F | 0x80 => "AP_CUSTOM",
        0xF0 => "MSP_RESP",
        _ => "?",
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

/// Map an INAV CRSF flight-mode string (frame 0x21) to `(armed, FlightModeState)`. Unlike S.Port (a
/// decimal-packed bitmask), CRSF sends one dominant ASCII mode string. Disarmed sentinels are
/// "OK"/"WAIT"/"!ERR"; any other string means armed. Strings from INAV `telemetry/crsf.c`. An unknown
/// armed string is passed through lower-cased so the frontend registry can still surface it.
fn classify_crsf_mode(text: &str) -> (bool, FlightModeState) {
    let armed = !matches!(text, "OK" | "WAIT" | "!ERR" | "");
    let primary = match text {
        "ANGL" | "ANGH" | "AH" => "angle",
        "HOR" => "horizon",
        "HOLD" | "LOTR" => "poshold",
        "CRUZ" | "CRSH" => "cruise",
        "WP" => "mission",
        "RTH" | "WRTH" => "rth",
        "MANU" => "manual",
        "!FS!" => "failsafe",
        "ACRO" | "OK" | "WAIT" | "!ERR" => "acro",
        other => {
            return (armed, FlightModeState { primary: other.to_ascii_lowercase(), modifiers: Vec::new() });
        }
    };
    let mut modifiers = Vec::new();
    if matches!(text, "ANGH" | "AH") {
        modifiers.push("althold".to_string());
    }
    (armed, FlightModeState { primary: primary.to_string(), modifiers })
}

/// Accumulated decoded telemetry. Each event is only emitted once its fields have been seen, so the
/// widgets/recorder are never fed placeholder zeros (mirrors the FrSky decoder).
#[derive(Default)]
struct State {
    roll: f64,
    pitch: f64,
    yaw: f64,
    seen_attitude: bool,

    lat: f64,
    lon: f64,
    gps_alt: f64,
    ground_speed: f64,
    course: f64,
    num_sat: u8,
    have_fix: bool,
    seen_gps: bool,

    baro_alt: f64,
    vario: f64,
    seen_altitude: bool,

    voltage: f64,
    current: f64,
    mah_drawn: u32,
    battery_pct: u8,
    seen_analog: bool,

    airspeed: f64,
    seen_airspeed: bool,

    armed: bool,
    mode: FlightModeState,
    seen_status: bool,

    link_rssi_dbm: i16,
    link_lq: u8,
    link_snr: i8,
    seen_link: bool,

    // Per-type "a fresh frame arrived since the last publish" flags. Emission gates on these so we don't
    // re-publish unchanged cached state at the fixed handler tick — the output rate (and the relay) then
    // follows the FC's real data rate. Set in apply(), cleared in publish().
    fresh_attitude: bool,
    fresh_gps: bool,
    fresh_altitude: bool,
    fresh_analog: bool,
    fresh_airspeed: bool,
    fresh_status: bool,
    fresh_link: bool,
}

pub struct CrsfDecoder {
    acc: Vec<u8>,
    state: State,
    start: Instant,
    frames: u64,
    dump: Option<BufWriter<File>>,
    /// Lazily created when ArduPilot AP_CUSTOM_TELEM (0x80/0x7F) frames appear (level-2 sub-detection).
    ap: Option<ApPassthroughDecoder>,
    /// Last time an FC-origin frame was seen (anything but RF link-stats) — drives FC-link-alive. The
    /// TX/RX keeps sending link-stats (0x14/0x1C/0x1D) after the FC link drops, so those don't count.
    last_fc: Option<Instant>,
}

impl CrsfDecoder {
    /// Create a decoder. If `dump_path` is given, decoded frames are appended there for analysis.
    pub fn new(dump_path: Option<PathBuf>) -> Self {
        let dump = dump_path.and_then(|p| match File::create(&p) {
            Ok(f) => {
                log::info!("CRSF decoded dump → {}", p.to_string_lossy());
                Some(BufWriter::new(f))
            }
            Err(e) => {
                log::warn!("Failed to open CRSF dump {}: {}", p.to_string_lossy(), e);
                None
            }
        });
        Self {
            acc: Vec::with_capacity(128),
            state: State::default(),
            start: Instant::now(),
            frames: 0,
            dump,
            ap: None,
            last_fc: None,
        }
    }

    /// Feed a freshly-read chunk; extract complete CRC-valid CRSF frames, apply them to the accumulated
    /// state and (if a dump is open) append a decoded line.
    pub fn push_bytes(&mut self, data: &[u8]) {
        self.acc.extend_from_slice(data);

        let mut i = 0;
        while i + 1 < self.acc.len() {
            let sync = self.acc[i];
            let len = self.acc[i + 1] as usize;
            if !matches!(sync, 0xC8 | 0xEA | 0xEE) || !(2..=62).contains(&len) {
                i += 1;
                continue;
            }
            let crc_idx = i + 1 + len;
            if crc_idx >= self.acc.len() {
                break; // need more bytes for the full frame
            }
            let crc = MspCodec::crc8_dvb_s2(&self.acc[i + 2..crc_idx]); // over type..payload
            if crc != self.acc[crc_idx] {
                i += 1; // bad frame — resync one byte at a time
                continue;
            }
            let ty = self.acc[i + 2];
            // Copy the payload out before mutating self (state/dump writer).
            let payload = self.acc[i + 3..crc_idx].to_vec();
            self.frames += 1;
            // FC-origin frame (anything but RF link-stats / radio-id) → FC link alive.
            if !matches!(ty, 0x14 | 0x1C | 0x1D | 0x3A) {
                self.last_fc = Some(Instant::now());
            }
            if ty == FT_AP_CUSTOM_TELEM || ty == FT_AP_CUSTOM_TELEM_LEGACY {
                // ArduPilot source: route the embedded passthrough packets to the AP sub-decoder. The
                // native CRSF frames (GPS/battery/attitude/link-stats) continue to be decoded normally.
                self.ap.get_or_insert_with(ApPassthroughDecoder::new).apply_crsf_custom(&payload);
            }
            let decoded = decode_frame(ty, &payload);
            self.apply(&decoded);
            self.dump_frame(sync, ty, &payload, &decoded);
            i = crc_idx + 1; // consume the validated frame
        }

        self.acc.drain(0..i);
        if self.acc.len() > 256 {
            self.acc.clear(); // runaway garbage — drop and resync
        }
    }

    /// Merge one decoded frame into the accumulated state.
    fn apply(&mut self, d: &Decoded) {
        let s = &mut self.state;
        match d {
            Decoded::Gps { lat, lon, gs_ms, course, alt_m, sats } => {
                s.lat = *lat;
                s.lon = *lon;
                s.ground_speed = *gs_ms;
                s.course = *course;
                s.gps_alt = *alt_m as f64;
                s.num_sat = *sats;
                if *lat != 0.0 || *lon != 0.0 {
                    s.have_fix = true;
                }
                s.seen_gps = true;
                s.fresh_gps = true;
            }
            Decoded::Vario { vspeed_ms } => {
                s.vario = *vspeed_ms;
                s.seen_altitude = true;
                s.fresh_altitude = true;
            }
            Decoded::Battery { volts, amps, mah, pct } => {
                s.voltage = *volts;
                s.current = *amps;
                s.mah_drawn = *mah;
                s.battery_pct = *pct;
                s.seen_analog = true;
                s.fresh_analog = true;
            }
            Decoded::BaroAlt { alt_m, .. } => {
                s.baro_alt = *alt_m;
                s.seen_altitude = true;
                s.fresh_altitude = true;
            }
            Decoded::Airspeed { aspd_ms } => {
                s.airspeed = *aspd_ms;
                s.seen_airspeed = true;
                s.fresh_airspeed = true;
            }
            Decoded::Attitude { pitch, roll, yaw } => {
                s.pitch = *pitch;
                s.roll = *roll;
                s.yaw = *yaw;
                s.seen_attitude = true;
                s.fresh_attitude = true;
            }
            Decoded::FlightMode { text } => {
                let (armed, mode) = classify_crsf_mode(text);
                s.armed = armed;
                s.mode = mode;
                s.seen_status = true;
                s.fresh_status = true;
            }
            Decoded::LinkStats { rssi_dbm, lq, snr } => {
                s.link_rssi_dbm = *rssi_dbm;
                s.link_lq = *lq;
                s.link_snr = *snr;
                s.seen_link = true;
                s.fresh_link = true;
            }
            Decoded::Other => {}
        }
    }

    fn dump_frame(&mut self, sync: u8, ty: u8, payload: &[u8], decoded: &Decoded) {
        let Some(w) = self.dump.as_mut() else { return };
        let t_ms = self.start.elapsed().as_millis();
        let desc = match decoded {
            Decoded::Gps { lat, lon, gs_ms, course, alt_m, sats } => format!(
                "lat={:.7} lon={:.7} spd={:.2}m/s crs={:.2} alt={}m sats={}",
                lat, lon, gs_ms, course, alt_m, sats
            ),
            Decoded::Vario { vspeed_ms } => format!("vspeed={:.2}m/s", vspeed_ms),
            Decoded::Battery { volts, amps, mah, pct } => {
                format!("volt={:.1}V curr={:.1}A mah={} pct={}", volts, amps, mah, pct)
            }
            Decoded::BaroAlt { alt_m, raw } => format!("alt={:.1}m (raw={:#06x})", alt_m, raw),
            Decoded::Airspeed { aspd_ms } => format!("aspd={:.2}m/s", aspd_ms),
            Decoded::Attitude { pitch, roll, yaw } => {
                format!("pitch={:.1} roll={:.1} yaw={:.1}", pitch, roll, yaw)
            }
            Decoded::FlightMode { text } => format!("text={:?}", text),
            Decoded::LinkStats { rssi_dbm, lq, snr } => format!("rssi={}dBm lq={}% snr={}dB", rssi_dbm, lq, snr),
            Decoded::Other => String::from("(undecoded)"),
        };
        let line = format!(
            "[{:>8}ms] sync={:#04x} 0x{:02X} {:<11} raw=[{}] {}\n",
            t_ms,
            sync,
            ty,
            frame_name(ty),
            hex(payload),
            desc
        );
        let _ = w.write_all(line.as_bytes());
    }

    /// Emit the accumulated state as the unified telemetry events and feed the flight recorder. Each
    /// event is only sent once its relevant fields have been seen (mirrors the FrSky decoder).
    pub fn publish(&mut self, app: &AppHandle, recorder: Option<&FlightRecorderHandle>) {
        // ArduPilot passthrough (if present) owns status / flight mode / EKF — suppress the native CRSF
        // mode here so the real AP modes win (the native 0x21 string would be misread as an INAV mode).
        let ap_active = self.ap.is_some();
        // Only emit a type when a fresh frame updated it since the last publish (no fixed-tick re-publish
        // of cached state). Capture + clear the flags, then emit from the (now-immutably-borrowed) state.
        let (f_status, f_att, f_gps, f_alt, f_an, f_asp, f_link) = {
            let s = &self.state;
            (s.fresh_status, s.fresh_attitude, s.fresh_gps && s.have_fix, s.fresh_altitude, s.fresh_analog, s.fresh_airspeed, s.fresh_link)
        };
        self.state.fresh_status = false;
        self.state.fresh_attitude = false;
        self.state.fresh_gps = false;
        self.state.fresh_altitude = false;
        self.state.fresh_analog = false;
        self.state.fresh_airspeed = false;
        self.state.fresh_link = false;
        let s = &self.state;

        if f_status && !ap_active {
            let status = StatusData {
                arming_flags: if s.armed { ARMED_FLAG } else { 0 },
                flight_mode_flags: 0, // CRSF carries a single mode string, not the raw INAV bitmask
                cpu_load: 0,
                sensor_status: 0,
                msp_rc_override: false,
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
                fix_type: if s.num_sat >= 4 { 3 } else { 2 },
                num_sat: s.num_sat,
                lat: s.lat,
                lon: s.lon,
                alt_msl: s.gps_alt,
                ground_speed: s.ground_speed,
                course: s.course,
            };
            let _ = app.emit("telemetry-gps", &gps);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_gps(&gps); }
            }
        }

        if f_alt {
            let alt = AltitudeData { altitude: s.baro_alt, vario: s.vario };
            let _ = app.emit("telemetry-altitude", &alt);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_altitude(&alt); }
            }
        }

        if f_an {
            let analog = AnalogData {
                voltage: s.voltage,
                mah_drawn: s.mah_drawn,
                rssi: 0, // CRSF RSSI lives in LINK_STATS (0x14), not yet decoded
                current: s.current,
                power: s.voltage * s.current,
                battery_percentage: s.battery_pct,
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

        // RC link health (uplink RSSI dBm + LQ + SNR). Protocol-independent — emitted for both native
        // INAV-CRSF and AP-over-CRSF sources (the radio sends 0x14 regardless of the FC).
        if f_link {
            let ls = LinkStatsData {
                rssi_percent: Some(LinkStatsData::dbm_to_percent(s.link_rssi_dbm)),
                rssi_dbm: Some(s.link_rssi_dbm),
                lq: Some(s.link_lq),
                snr_db: Some(s.link_snr),
            };
            let _ = app.emit("telemetry-linkstats", &ls);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_linkstats(&ls); }
            }
        }

        // AP passthrough owns the AP-unique data (mode/armed/EKF/status-text/waypoint).
        if let Some(ap) = self.ap.as_mut() {
            ap.publish(app, recorder);
        }
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// True once ArduPilot-passthrough (AP_CUSTOM_TELEM) frames have appeared → a secondary protocol.
    pub fn ap_active(&self) -> bool {
        self.ap.is_some()
    }

    /// Age (ms) of the last FC-origin frame (excludes RF link-stats), or None if none seen yet.
    pub fn fc_age_ms(&self) -> Option<u128> {
        self.last_fc.map(|t| t.elapsed().as_millis())
    }

    pub fn flush(&mut self) {
        if let Some(w) = self.dump.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Drop for CrsfDecoder {
    fn drop(&mut self) {
        self.flush();
    }
}
