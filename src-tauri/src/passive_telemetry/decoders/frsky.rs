// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FrSky S.Port decoder — turns raw S.Port frames (forwarded over BLE from an EdgeTX/ETHOS radio) into
// the unified telemetry events the frontend already consumes (telemetry-attitude / -gps / -altitude /
// -analog / -airspeed / -status / -flightmode) and feeds the flight recorder. Validated against INAV
// 9.x; see docs/active/RADIO_TELEMETRY.md for the appID map + value scalings (from INAV
// telemetry/smartport.c).
//
// Frame (after 0x7D unstuffing, 0x7E-delimited): <physID> 0x10 <appID:2 LE> <value:4 LE> <crc>.
// physID 0x00 = flight controller, 0x98 = receiver.

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::flightmode::{classify_inav, FlightModeState};
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, StatusData,
};

// ── FrSky S.Port appIDs (INAV; standard FrSky IDs stable across 7/8/9) ────────
const ID_ALTITUDE: u16 = 0x0100;
const ID_VARIO: u16 = 0x0110;
const ID_CURRENT: u16 = 0x0200;
const ID_VFAS: u16 = 0x0210;
const ID_FUEL: u16 = 0x0600;
const ID_PITCH: u16 = 0x0430;
const ID_ROLL: u16 = 0x0440;
const ID_FPV: u16 = 0x0450; // GPS ground course (COG)
const ID_LATLONG: u16 = 0x0800;
const ID_GPS_ALT: u16 = 0x0820;
const ID_SPEED: u16 = 0x0830;
const ID_HEADING: u16 = 0x0840; // FC yaw / heading
const ID_MODES: u16 = 0x0470; // INAV >=8.0 flight modes (decimal-packed)
const ID_LEGACY_MODES: u16 = 0x0400; // INAV <=7.x (T1)
const ID_GNSS: u16 = 0x0480; // INAV >=8.0 packed GNSS state
const ID_LEGACY_GNSS: u16 = 0x0410; // INAV <=7.x (T2)
const ID_ASPD: u16 = 0x0A00;
const ID_RSSI: u16 = 0xF101;

// Normalized INAV flight-mode bits — must match the layout in `flightmode::classify_inav`.
const F_ANGLE: u32 = 1 << 0;
const F_HORIZON: u32 = 1 << 1;
const F_HEADING: u32 = 1 << 2;
const F_NAV_ALTHOLD: u32 = 1 << 3;
const F_NAV_RTH: u32 = 1 << 4;
const F_NAV_POSHOLD: u32 = 1 << 5;
const F_HEADFREE: u32 = 1 << 6;
const F_MANUAL: u32 = 1 << 8;
const F_FAILSAFE: u32 = 1 << 9;
const F_AUTO_TUNE: u32 = 1 << 10;
const F_NAV_WP: u32 = 1 << 11;
const F_NAV_COURSE_HOLD: u32 = 1 << 12;
const F_FLAPERON: u32 = 1 << 13;
const F_NAV_FW_AUTOLAND: u32 = 1 << 18;

/// INAV arming_flags bit 2 = ARMED (what the recorder + frontend look for).
const ARMED_FLAG: u32 = 0x04;

/// Decode INAV's decimal-column-packed flight-mode value (`frskyGetFlightMode`) into `(armed,
/// normalized-flags)`. Mirrors smartport.c exactly.
fn decode_modes(value: u32) -> (bool, u32) {
    let armed = (value % 10) & 4 != 0;
    let tens = (value / 10) % 10;
    let huns = (value / 100) % 10;
    let thous = (value / 1000) % 10;
    let tenk = (value / 10_000) % 10;
    let hundk = (value / 100_000) % 10;
    let mil = (value / 1_000_000) % 10;

    let mut f = 0u32;
    if tens & 1 != 0 { f |= F_ANGLE; }
    if tens & 2 != 0 { f |= F_HORIZON; }
    if tens & 4 != 0 { f |= F_MANUAL; }
    if huns & 1 != 0 { f |= F_HEADING; }
    if huns & 2 != 0 { f |= F_NAV_ALTHOLD; }
    if huns & 4 != 0 { f |= F_NAV_POSHOLD; }
    if thous & 1 != 0 { f |= F_NAV_RTH; }
    if thous & 8 != 0 { f |= F_NAV_COURSE_HOLD; }
    else if thous & 2 != 0 { f |= F_NAV_WP; }
    else if thous & 4 != 0 { f |= F_HEADFREE; }
    if tenk & 1 != 0 { f |= F_FLAPERON; }
    if tenk & 4 != 0 { f |= F_FAILSAFE; }
    else if tenk & 2 != 0 { f |= F_AUTO_TUNE; }
    if hundk & 1 != 0 { f |= F_NAV_FW_AUTOLAND; }
    if hundk & 8 != 0 { f |= F_NAV_POSHOLD; } // POSHOLD (airplane)
    if mil & 1 != 0 { f |= F_NAV_RTH; } // WP-mission RTH
    (armed, f)
}

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
    fix_type: u8,
    seen_gnss: bool,
    seen_gps: bool,
    have_fix: bool,

    baro_alt: f64,
    vario: f64,
    seen_altitude: bool,

    voltage: f64,
    current: f64,
    mah_drawn: u32,
    rssi: u16,
    seen_analog: bool,

    airspeed: f64,
    seen_airspeed: bool,

    armed: bool,
    mode_flags: u32,
    seen_status: bool,
}

pub struct FrskyDecoder {
    acc: Vec<u8>,
    state: State,
}

impl FrskyDecoder {
    pub fn new() -> Self {
        Self { acc: Vec::with_capacity(16), state: State::default() }
    }

    /// Feed a freshly-read chunk; extract + apply complete S.Port frames.
    pub fn push_bytes(&mut self, data: &[u8]) {
        for &b in data {
            if b == 0x7E {
                if !self.acc.is_empty() {
                    let frame = std::mem::take(&mut self.acc);
                    self.process(&frame);
                }
            } else {
                self.acc.push(b);
                if self.acc.len() > 32 {
                    self.acc.clear(); // framing lost — resync on next 0x7E
                }
            }
        }
    }

    fn process(&mut self, raw: &[u8]) {
        // Unstuff 0x7D xx → xx XOR 0x20.
        let mut f = Vec::with_capacity(9);
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == 0x7D && i + 1 < raw.len() {
                f.push(raw[i + 1] ^ 0x20);
                i += 2;
            } else {
                f.push(raw[i]);
                i += 1;
            }
        }
        if f.len() != 9 || f[1] != 0x10 {
            return; // not a data frame
        }
        let appid = (f[2] as u16) | ((f[3] as u16) << 8);
        let value = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
        self.apply(appid, value);
    }

    fn apply(&mut self, appid: u16, value: u32) {
        let s = &mut self.state;
        let ivalue = value as i32;
        match appid {
            ID_ALTITUDE => { s.baro_alt = ivalue as f64 / 100.0; s.seen_altitude = true; }
            ID_VARIO => { s.vario = ivalue as f64 / 100.0; s.seen_altitude = true; }
            ID_CURRENT => { s.current = value as f64 / 10.0; s.seen_analog = true; }
            ID_VFAS => { s.voltage = value as f64 / 100.0; s.seen_analog = true; }
            ID_FUEL => { s.mah_drawn = value; s.seen_analog = true; }
            ID_PITCH => { s.pitch = ivalue as f64 / 10.0; s.seen_attitude = true; }
            ID_ROLL => { s.roll = ivalue as f64 / 10.0; s.seen_attitude = true; }
            ID_FPV => { s.course = (ivalue as f64 / 10.0).rem_euclid(360.0); s.seen_gps = true; }
            ID_HEADING => {
                s.yaw = (value as f64 / 100.0).rem_euclid(360.0); // FrSky HEADING is deg*100
                s.seen_attitude = true;
            }
            ID_SPEED => { s.ground_speed = value as f64 / 1944.0; s.seen_gps = true; }
            ID_GPS_ALT => { s.gps_alt = ivalue as f64 / 100.0; s.seen_gps = true; }
            ID_LATLONG => {
                let raw = value & 0x3FFF_FFFF;
                let mut deg = raw as f64 / 600_000.0;
                if value & 0x4000_0000 != 0 { deg = -deg; }
                if value & 0x8000_0000 != 0 { s.lon = deg; } else { s.lat = deg; }
                if s.lat != 0.0 || s.lon != 0.0 { s.have_fix = true; }
                s.seen_gps = true;
            }
            ID_GNSS | ID_LEGACY_GNSS => {
                // sats in the low two decimal digits; GPS_FIX in the thousands column.
                s.num_sat = (value % 100) as u8;
                let thous = (value / 1000) % 10;
                s.fix_type = if thous & 1 != 0 { 3 } else { 0 };
                s.seen_gnss = true;
                s.seen_gps = true;
            }
            ID_MODES | ID_LEGACY_MODES => {
                let (armed, flags) = decode_modes(value);
                s.armed = armed;
                s.mode_flags = flags;
                s.seen_status = true;
            }
            ID_ASPD => { s.airspeed = value as f64 * 0.514444; s.seen_airspeed = true; }
            ID_RSSI => { s.rssi = value as u16; s.seen_analog = true; }
            _ => {}
        }
    }

    /// Emit the accumulated state as the unified telemetry events and feed the flight recorder. Each
    /// event is only sent once its relevant fields have been seen, so widgets/recorder aren't fed
    /// placeholder zeros.
    pub fn publish(&self, app: &AppHandle, recorder: Option<&FlightRecorderHandle>) {
        let s = &self.state;

        // Status first: drives the recorder's arm/disarm edge + the ARMED indicator.
        if s.seen_status {
            let status = StatusData {
                arming_flags: if s.armed { ARMED_FLAG } else { 0 },
                flight_mode_flags: s.mode_flags,
                cpu_load: 0,
                sensor_status: 0,
            };
            let fm: FlightModeState = classify_inav(s.mode_flags);
            let _ = app.emit("telemetry-status", &status);
            let _ = app.emit("telemetry-flightmode", &fm);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_status(&status);
                    r.on_flightmode(&fm);
                }
            }
        }

        if s.seen_attitude {
            let att = AttitudeData { roll: s.roll, pitch: s.pitch, yaw: s.yaw };
            let _ = app.emit("telemetry-attitude", &att);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_attitude(&att); }
            }
        }

        if s.seen_gps && s.have_fix {
            let fix = if s.seen_gnss { s.fix_type } else if s.num_sat >= 4 { 3 } else { 2 };
            let gps = GpsData {
                fix_type: fix,
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

        if s.seen_altitude {
            let alt = AltitudeData { altitude: s.baro_alt, vario: s.vario };
            let _ = app.emit("telemetry-altitude", &alt);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_altitude(&alt); }
            }
        }

        if s.seen_analog {
            let analog = AnalogData {
                voltage: s.voltage,
                mah_drawn: s.mah_drawn,
                rssi: s.rssi,
                current: s.current,
                power: s.voltage * s.current,
                battery_percentage: 0,
                cell_count: 0,
            };
            let _ = app.emit("telemetry-analog", &analog);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_analog(&analog); }
            }
        }

        if s.seen_airspeed {
            let aspd = AirspeedData { airspeed: s.airspeed };
            let _ = app.emit("telemetry-airspeed", &aspd);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_airspeed(&aspd); }
            }
        }
    }
}
