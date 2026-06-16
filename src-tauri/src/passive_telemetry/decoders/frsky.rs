// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FrSky S.Port decoder — turns raw S.Port frames (forwarded over BLE from an EdgeTX/ETHOS radio) into
// the unified telemetry events the frontend already consumes (telemetry-attitude / -gps / -altitude /
// -analog / -airspeed). Validated against INAV 9.x; see docs/active/RADIO_TELEMETRY.md for the appID map
// and the value scalings (extracted from INAV telemetry/smartport.c).
//
// Frame (after 0x7D unstuffing, 0x7E-delimited): <physID> 0x10 <appID:2 LE> <value:4 LE> <crc>.
// physID 0x00 = flight controller, 0x98 = receiver.

use tauri::{AppHandle, Emitter};

use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData,
};

// ── FrSky S.Port appIDs (INAV 9.x; standard FrSky IDs stable across 7/8/9) ────
const ID_ALTITUDE: u16 = 0x0100;
const ID_VARIO: u16 = 0x0110;
const ID_CURRENT: u16 = 0x0200;
const ID_VFAS: u16 = 0x0210;
const ID_FUEL: u16 = 0x0600; // mAh / % (config-dependent)
const ID_PITCH: u16 = 0x0430;
const ID_ROLL: u16 = 0x0440;
const ID_FPV: u16 = 0x0450; // GPS ground course (COG)
const ID_LATLONG: u16 = 0x0800;
const ID_GPS_ALT: u16 = 0x0820;
const ID_SPEED: u16 = 0x0830; // GPS ground speed
const ID_HEADING: u16 = 0x0840; // FC yaw / heading
const ID_GNSS: u16 = 0x0480; // INAV >=8.0 packed GNSS state (sats in low 2 digits)
const ID_LEGACY_GNSS: u16 = 0x0410; // INAV <=7.x
const ID_ASPD: u16 = 0x0A00;
const ID_RSSI: u16 = 0xF101;

/// Decoded telemetry state, accumulated from S.Port frames.
#[derive(Default)]
struct State {
    roll: f64,
    pitch: f64,
    yaw: i16,
    seen_attitude: bool,

    lat: f64,
    lon: f64,
    gps_alt: f64,
    ground_speed: f64,
    course: f64,
    num_sat: u8,
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
}

pub struct FrskyDecoder {
    /// Bytes accumulated for the current frame (between 0x7E delimiters).
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
                // Guard against runaway accumulation if framing is lost.
                if self.acc.len() > 32 {
                    self.acc.clear();
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
                s.yaw = ((value as f64 / 100.0).round() as i32).rem_euclid(360) as i16;
                s.seen_attitude = true;
            }
            ID_SPEED => { s.ground_speed = value as f64 / 1944.0; s.seen_gps = true; }
            ID_GPS_ALT => { s.gps_alt = ivalue as f64 / 100.0; s.seen_gps = true; }
            ID_LATLONG => {
                let raw = value & 0x3FFF_FFFF;
                let mut deg = raw as f64 / 600_000.0;
                if value & 0x4000_0000 != 0 {
                    deg = -deg;
                }
                if value & 0x8000_0000 != 0 {
                    s.lon = deg;
                } else {
                    s.lat = deg;
                }
                if s.lat != 0.0 || s.lon != 0.0 {
                    s.have_fix = true;
                }
                s.seen_gps = true;
            }
            ID_GNSS | ID_LEGACY_GNSS => {
                s.num_sat = (value % 100) as u8;
                s.seen_gps = true;
            }
            ID_ASPD => { s.airspeed = value as f64 * 0.514444; s.seen_airspeed = true; }
            ID_RSSI => { s.rssi = value as u16; s.seen_analog = true; }
            _ => {}
        }
    }

    /// Emit the accumulated state as the unified telemetry events. Each event is only sent once its
    /// relevant fields have been seen, so widgets aren't fed placeholder zeros.
    pub fn emit(&self, app: &AppHandle) {
        let s = &self.state;
        if s.seen_attitude {
            let _ = app.emit("telemetry-attitude", AttitudeData {
                roll: s.roll, pitch: s.pitch, yaw: s.yaw,
            });
        }
        if s.seen_gps && s.have_fix {
            let _ = app.emit("telemetry-gps", GpsData {
                fix_type: if s.num_sat >= 4 { 3 } else { 2 },
                num_sat: s.num_sat,
                lat: s.lat,
                lon: s.lon,
                alt_msl: s.gps_alt,
                ground_speed: s.ground_speed,
                course: s.course,
            });
        }
        if s.seen_altitude {
            let _ = app.emit("telemetry-altitude", AltitudeData {
                altitude: s.baro_alt, vario: s.vario,
            });
        }
        if s.seen_analog {
            let _ = app.emit("telemetry-analog", AnalogData {
                voltage: s.voltage,
                mah_drawn: s.mah_drawn,
                rssi: s.rssi,
                current: s.current,
                power: s.voltage * s.current,
                battery_percentage: 0,
                cell_count: 0,
            });
        }
        if s.seen_airspeed {
            let _ = app.emit("telemetry-airspeed", AirspeedData { airspeed: s.airspeed });
        }
    }
}
