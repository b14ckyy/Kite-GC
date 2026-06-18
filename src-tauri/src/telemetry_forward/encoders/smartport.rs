// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! FrSky S.Port encoder — the inverse of `passive_telemetry::decoders::frsky`. Emits a set of S.Port
//! sensor frames per pacer tick. Each frame is `0x7E <physID=0x00 (FC)> 0x10 <appID:2 LE> <value:4 LE>
//! <crc>` with 0x7D byte-stuffing; CRC is the FrSky S.Port checksum over type+appID+value. Scalings are
//! the exact inverse of the decoder (validated against INAV 9.x).

use super::super::cache::TelemetryCache;
use super::Encoder;

// ── FrSky S.Port appIDs (mirror decoders/frsky.rs) ────────────────────────────
const ID_ALTITUDE: u16 = 0x0100;
const ID_VARIO: u16 = 0x0110;
const ID_CURRENT: u16 = 0x0200;
const ID_VFAS: u16 = 0x0210;
const ID_FUEL: u16 = 0x0600;
const ID_PITCH: u16 = 0x0430;
const ID_ROLL: u16 = 0x0440;
const ID_FPV: u16 = 0x0450; // COG
const ID_LATLONG: u16 = 0x0800;
const ID_GPS_ALT: u16 = 0x0820;
const ID_SPEED: u16 = 0x0830;
const ID_HEADING: u16 = 0x0840; // FC yaw
const ID_MODES: u16 = 0x0470;
const ID_GNSS: u16 = 0x0480;
const ID_ASPD: u16 = 0x0A00;
const ID_RSSI: u16 = 0xF101;

// Unified flight-mode bits (mirror scheduler::telemetry::box_id_to_flight_mode_bit output).
const FM_ANGLE: u32 = 1 << 0;
const FM_HORIZON: u32 = 1 << 1;
const FM_HEADING: u32 = 1 << 2;
const FM_NAV_ALTHOLD: u32 = 1 << 3;
const FM_NAV_RTH: u32 = 1 << 4;
const FM_NAV_POSHOLD: u32 = 1 << 5;
const FM_MANUAL: u32 = 1 << 8;
const FM_FAILSAFE: u32 = 1 << 9;
const FM_AUTO_TUNE: u32 = 1 << 10;
const FM_NAV_WP: u32 = 1 << 11;
const FM_NAV_COURSE_HOLD: u32 = 1 << 12;
const FM_FLAPERON: u32 = 1 << 13;
const FM_NAV_FW_AUTOLAND: u32 = 1 << 18;
const ARMED_FLAG: u32 = 0x04;

#[derive(Default)]
pub struct SmartportEncoder;

impl SmartportEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl Encoder for SmartportEncoder {
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8> {
        let mut out = Vec::with_capacity(160);

        if let Some(a) = cache.attitude.as_ref() {
            frame(ID_PITCH, ((a.pitch * 10.0).round() as i32) as u32, &mut out);
            frame(ID_ROLL, ((a.roll * 10.0).round() as i32) as u32, &mut out);
            frame(ID_HEADING, (a.yaw * 100.0).round() as u32, &mut out);
        }
        if let Some(al) = cache.altitude.as_ref() {
            frame(ID_ALTITUDE, ((al.altitude * 100.0).round() as i32) as u32, &mut out);
            frame(ID_VARIO, ((al.vario * 100.0).round() as i32) as u32, &mut out);
        }
        if let Some(an) = cache.analog.as_ref() {
            frame(ID_VFAS, (an.voltage * 100.0).round() as u32, &mut out);
            frame(ID_CURRENT, (an.current * 10.0).round() as u32, &mut out);
            frame(ID_FUEL, an.mah_drawn, &mut out);
            frame(ID_RSSI, an.rssi as u32, &mut out);
        }
        if let Some(g) = cache.gps.as_ref() {
            frame(ID_LATLONG, latlong_value(g.lat, false), &mut out);
            frame(ID_LATLONG, latlong_value(g.lon, true), &mut out);
            frame(ID_GPS_ALT, ((g.alt_msl * 100.0).round() as i32) as u32, &mut out);
            frame(ID_SPEED, (g.ground_speed * 1944.0).round() as u32, &mut out); // m/s → knots*1000
            frame(ID_FPV, (g.course * 10.0).round() as u32, &mut out);
            let is3d = g.fix_type >= 3;
            frame(ID_GNSS, g.num_sat as u32 + if is3d { 1000 } else { 0 }, &mut out);
        }
        if let Some(asp) = cache.airspeed.as_ref() {
            frame(ID_ASPD, (asp.airspeed / 0.514444).round() as u32, &mut out); // m/s → knots
        }
        if let Some(s) = cache.status.as_ref() {
            frame(ID_MODES, encode_modes(s.flight_mode_flags, s.arming_flags & ARMED_FLAG != 0), &mut out);
        }
        out
    }
}

/// FrSky S.Port checksum over the bytes after the physID (type + appID + value).
fn sport_crc(bytes: &[u8]) -> u8 {
    let mut crc: u16 = 0;
    for &b in bytes {
        crc += b as u16;
        crc += crc >> 8;
        crc &= 0xFF;
    }
    (0xFF - crc) as u8
}

fn push_stuffed(out: &mut Vec<u8>, b: u8) {
    if b == 0x7E || b == 0x7D {
        out.push(0x7D);
        out.push(b ^ 0x20);
    } else {
        out.push(b);
    }
}

/// Append a `0x7E`-delimited, byte-stuffed S.Port sensor frame (physID 0x00 = flight controller).
fn frame(appid: u16, value: u32, out: &mut Vec<u8>) {
    let raw = [
        0x00u8, // physID = FC
        0x10,   // sensor data frame
        (appid & 0xFF) as u8,
        (appid >> 8) as u8,
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
        ((value >> 24) & 0xFF) as u8,
    ];
    let crc = sport_crc(&raw[1..]);
    out.push(0x7E);
    for &b in raw.iter().chain(std::iter::once(&crc)) {
        push_stuffed(out, b);
    }
}

/// Encode a coordinate into the FrSky 0x0800 packed format (raw = |deg|·600000, bit30 = negative,
/// bit31 = longitude).
fn latlong_value(deg: f64, is_lon: bool) -> u32 {
    let mut v = ((deg.abs() * 600_000.0).round() as u32) & 0x3FFF_FFFF;
    if deg < 0.0 {
        v |= 0x4000_0000;
    }
    if is_lon {
        v |= 0x8000_0000;
    }
    v
}

/// Inverse of `decoders::frsky::decode_modes`: pack the unified flight-mode flags + armed into INAV's
/// decimal-column format (`frskyGetFlightMode`).
fn encode_modes(flags: u32, armed: bool) -> u32 {
    let mut tens = 0u32;
    if flags & FM_ANGLE != 0 { tens |= 1; }
    if flags & FM_HORIZON != 0 { tens |= 2; }
    if flags & FM_MANUAL != 0 { tens |= 4; }
    let mut huns = 0u32;
    if flags & FM_HEADING != 0 { huns |= 1; }
    if flags & FM_NAV_ALTHOLD != 0 { huns |= 2; }
    if flags & FM_NAV_POSHOLD != 0 { huns |= 4; }
    let mut thous = 0u32;
    if flags & FM_NAV_RTH != 0 { thous |= 1; }
    if flags & FM_NAV_WP != 0 { thous |= 2; }
    if flags & FM_NAV_COURSE_HOLD != 0 { thous |= 8; }
    let mut tenk = 0u32;
    if flags & FM_FLAPERON != 0 { tenk |= 1; }
    if flags & FM_AUTO_TUNE != 0 { tenk |= 2; }
    if flags & FM_FAILSAFE != 0 { tenk |= 4; }
    let mut hundk = 0u32;
    if flags & FM_NAV_FW_AUTOLAND != 0 { hundk |= 1; }

    let units = if armed { 4 } else { 0 };
    units + tens * 10 + huns * 100 + thous * 1000 + tenk * 10_000 + hundk * 100_000
}
