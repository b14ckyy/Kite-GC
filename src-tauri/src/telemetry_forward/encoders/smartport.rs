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
const FM_HEADFREE: u32 = 1 << 6;
const FM_MANUAL: u32 = 1 << 8;
const FM_FAILSAFE: u32 = 1 << 9;
const FM_AUTO_TUNE: u32 = 1 << 10;
const FM_NAV_WP: u32 = 1 << 11;
const FM_NAV_COURSE_HOLD: u32 = 1 << 12;
const FM_FLAPERON: u32 = 1 << 13;
const FM_TURTLE: u32 = 1 << 15;
const FM_ANGLEHOLD: u32 = 1 << 17;
const FM_NAV_FW_AUTOLAND: u32 = 1 << 18;
const ARMED_FLAG: u32 = 0x04;
/// INAV armingFlag_e: ARMING_DISABLED_* reasons live in bits 6..30 (mirrors `helpers/arming.ts`).
const ARMING_DISABLED_MASK: u32 = !0x3F;

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
            frame(ID_MODES, encode_modes(s.flight_mode_flags, s.arming_flags), &mut out);
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

/// Inverse of `decoders::frsky::decode_modes`: pack the unified flight-mode flags + arming state into
/// INAV's decimal-column format — `frskyGetFlightMode()` (`telemetry/smartport.c`) verbatim, including
/// the else-chains that keep every column a single decimal digit (our old independent ORs could push the
/// thousands column past 9 and corrupt the neighbours). Not derivable from the unified flags: the
/// POSHOLD-airplane split (hundreds vs the 800000 "LOTR" column — we always use the hundreds column,
/// which every decoder reads as plain POSHOLD) and WP-mission RTH (millions column — we always use the
/// thousands column).
fn encode_modes(flags: u32, arming_flags: u32) -> u32 {
    let mut v: u32 = 0;
    // ones column: readiness + armed
    if arming_flags & ARMING_DISABLED_MASK == 0 { v += 1; } else { v += 2; }
    if arming_flags & ARMED_FLAG != 0 { v += 4; }
    // tens column
    if flags & FM_ANGLE != 0 { v += 10; }
    if flags & FM_HORIZON != 0 { v += 20; }
    if flags & FM_MANUAL != 0 { v += 40; }
    // hundreds column
    if flags & FM_HEADING != 0 { v += 100; }
    if flags & FM_NAV_ALTHOLD != 0 { v += 200; }
    if flags & FM_NAV_POSHOLD != 0 { v += 400; }
    // thousands column
    if flags & FM_NAV_RTH != 0 { v += 1000; }
    if flags & FM_NAV_COURSE_HOLD != 0 {
        v += 8000;
    } else if flags & FM_NAV_WP != 0 {
        v += 2000;
    } else if flags & FM_HEADFREE != 0 {
        v += 4000;
    }
    // ten-thousands column
    if flags & FM_FLAPERON != 0 { v += 10_000; }
    if flags & FM_FAILSAFE != 0 {
        v += 40_000;
    } else if flags & FM_AUTO_TUNE != 0 {
        v += 20_000;
    }
    // hundred-thousands column
    if flags & FM_NAV_FW_AUTOLAND != 0 { v += 100_000; }
    if flags & FM_TURTLE != 0 { v += 200_000; }
    // millions column
    if flags & FM_ANGLEHOLD != 0 { v += 2_000_000; }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passive_telemetry::decoders::frsky::decode_modes;

    /// Ready-to-arm + armed → ones column 5, like INAV.
    const ARMED: u32 = ARMED_FLAG;

    /// Every decimal column must stay a single digit even with every encodable flag set at once —
    /// the INAV else-chains guarantee it (the old independent ORs could carry into the neighbour
    /// column: RTH+WP+CRUZ made the thousands column 11).
    #[test]
    fn digits_never_overflow() {
        let all = FM_ANGLE | FM_HORIZON | FM_HEADING | FM_NAV_ALTHOLD | FM_NAV_RTH | FM_NAV_POSHOLD
            | FM_HEADFREE | FM_MANUAL | FM_FAILSAFE | FM_AUTO_TUNE | FM_NAV_WP | FM_NAV_COURSE_HOLD
            | FM_FLAPERON | FM_TURTLE | FM_ANGLEHOLD | FM_NAV_FW_AUTOLAND;
        // ones=5 tens=7 huns=7 thous=9 (RTH+course; WP/headfree suppressed) tenk=5 (flaperon+failsafe;
        // autotune suppressed) hundk=3 mil=2
        assert_eq!(encode_modes(all, ARMED), 2_359_775);
    }

    /// encode → decode must reproduce the flags exactly (cases chosen within INAV's lossless set).
    #[test]
    fn roundtrip_through_frsky_decoder() {
        let cases: &[u32] = &[
            FM_NAV_RTH | FM_NAV_FW_AUTOLAND | FM_ANGLE | FM_NAV_ALTHOLD, // RTH + autoland
            FM_NAV_POSHOLD | FM_NAV_ALTHOLD | FM_ANGLE,
            FM_NAV_COURSE_HOLD | FM_NAV_ALTHOLD | FM_ANGLE,              // cruise
            FM_HEADFREE | FM_ANGLE,
            FM_ANGLEHOLD | FM_NAV_ALTHOLD,
            FM_TURTLE,
            FM_MANUAL,
            0,
        ];
        for &flags in cases {
            let (armed, disable_bits, decoded) = decode_modes(encode_modes(flags, ARMED));
            assert!(armed, "flags=0x{flags:X}");
            assert_eq!(disable_bits, 0, "flags=0x{flags:X}");
            assert_eq!(decoded, flags, "flags=0x{flags:X}");
        }
    }

    /// The ones column carries readiness: 1 = ready, 2 = arming blocked (any ARMING_DISABLED_* bit).
    #[test]
    fn arming_state_columns() {
        let (armed, disable, _) = decode_modes(encode_modes(0, 0));
        assert!(!armed);
        assert_eq!(disable, 0);
        let (armed, disable, _) = decode_modes(encode_modes(0, 1 << 12)); // compass not calibrated
        assert!(!armed);
        assert_ne!(disable, 0);
    }
}
