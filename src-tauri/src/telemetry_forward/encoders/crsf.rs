// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! CRSF (Crossfire / ELRS) encoder — the inverse of `passive_telemetry::decoders::crsf`. Emits a set of
//! CRSF telemetry frames per pacer tick. Frame = `0xC8 <len> <type> <payload BE> <crc8-DVB-S2>`, where
//! `len = type + payload + crc`. CRSF is **big-endian**, and attitude is in **radians × 10000**.

use super::super::cache::TelemetryCache;
use super::Encoder;
use crate::msp::codec::MspCodec;

const SYNC: u8 = 0xC8;

const FT_GPS: u8 = 0x02;
const FT_BATTERY: u8 = 0x08;
const FT_BARO_ALT: u8 = 0x09;
const FT_ATTITUDE: u8 = 0x1E;
const FT_FLIGHT_MODE: u8 = 0x21;

// Unified flight-mode bits (mirror scheduler::telemetry::box_id_to_flight_mode_bit output).
const FM_ANGLE: u32 = 1 << 0;
const FM_HORIZON: u32 = 1 << 1;
const FM_NAV_ALTHOLD: u32 = 1 << 3;
const FM_NAV_RTH: u32 = 1 << 4;
const FM_NAV_POSHOLD: u32 = 1 << 5;
const FM_MANUAL: u32 = 1 << 8;
const FM_FAILSAFE: u32 = 1 << 9;
const FM_NAV_WP: u32 = 1 << 11;
const FM_NAV_COURSE_HOLD: u32 = 1 << 12;
const FM_TURTLE: u32 = 1 << 15;
const FM_ANGLEHOLD: u32 = 1 << 17;
const FM_NAV_FW_AUTOLAND: u32 = 1 << 18;
const ARMED_FLAG: u32 = 0x04;
/// INAV armingFlag_e: ARMING_DISABLED_* reasons live in bits 6..30 (mirrors `helpers/arming.ts`).
const ARMING_DISABLED_MASK: u32 = !0x3F;

#[derive(Default)]
pub struct CrsfEncoder;

impl CrsfEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl Encoder for CrsfEncoder {
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);

        if let Some(a) = cache.attitude.as_ref() {
            let mut p = Vec::with_capacity(6);
            p.extend_from_slice(&((a.pitch.to_radians() * 10000.0).round() as i16).to_be_bytes());
            p.extend_from_slice(&((a.roll.to_radians() * 10000.0).round() as i16).to_be_bytes());
            p.extend_from_slice(&((a.yaw.to_radians() * 10000.0).round() as i16).to_be_bytes());
            frame(FT_ATTITUDE, &p, &mut out);
        }
        if let Some(g) = cache.gps.as_ref() {
            let mut p = Vec::with_capacity(15);
            p.extend_from_slice(&((g.lat * 1e7).round() as i32).to_be_bytes());
            p.extend_from_slice(&((g.lon * 1e7).round() as i32).to_be_bytes());
            p.extend_from_slice(&((g.ground_speed * 36.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes()); // m/s → km/h*10
            p.extend_from_slice(&((g.course * 100.0).round().rem_euclid(36000.0) as u16).to_be_bytes()); // COG, centidegrees
            p.extend_from_slice(&((g.alt_msl.round() as i32 + 1000).clamp(0, 65535) as u16).to_be_bytes()); // +1000 m offset
            p.push(g.num_sat);
            frame(FT_GPS, &p, &mut out);
        }
        if let Some(al) = cache.altitude.as_ref() {
            // INAV's layout with `crsf_use_legacy_baro_packet` off: the GPS frame above is MSL, and this
            // 3-byte 0x09 (altitude above home + vertical speed) replaces the 0x07 vario frame.
            frame(FT_BARO_ALT, &baro_alt_payload(al.altitude, al.vario), &mut out);
        }
        if let Some(an) = cache.analog.as_ref() {
            let mut p = Vec::with_capacity(8);
            p.extend_from_slice(&((an.voltage * 10.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes()); // deci-volts
            p.extend_from_slice(&((an.current * 10.0).round().clamp(0.0, 65535.0) as u16).to_be_bytes()); // deci-amps
            let mah = an.mah_drawn & 0x00FF_FFFF;
            p.push(((mah >> 16) & 0xFF) as u8);
            p.push(((mah >> 8) & 0xFF) as u8);
            p.push((mah & 0xFF) as u8);
            p.push(an.battery_percentage);
            frame(FT_BATTERY, &p, &mut out);
        }
        if let Some(s) = cache.status.as_ref() {
            let text = crsf_mode_string(s.flight_mode_flags, s.arming_flags);
            let mut p = text.as_bytes().to_vec();
            p.push(0); // null terminator
            frame(FT_FLIGHT_MODE, &p, &mut out);
        }
        out
    }
}

/// Append a CRSF frame: sync, length, type, payload, CRC8-DVB-S2 (over type+payload).
fn frame(ty: u8, payload: &[u8], out: &mut Vec<u8>) {
    let mut crc_buf = Vec::with_capacity(payload.len() + 1);
    crc_buf.push(ty);
    crc_buf.extend_from_slice(payload);
    let crc = MspCodec::crc8_dvb_s2(&crc_buf);
    out.push(SYNC);
    out.push((payload.len() + 2) as u8); // type + payload + crc
    out.push(ty);
    out.extend_from_slice(payload);
    out.push(crc);
}

/// 0x09: altitude above home packed as INAV's `crsfFrameBarometerAltitudeVarioSensor` does (decimetres
/// + 10000 up to 2276.7 m, above that metres with the top bit set), then the vertical speed on TBS's
/// logarithmic scale.
fn baro_alt_payload(alt_m: f64, vario_ms: f64) -> [u8; 3] {
    const ALT_MIN_DM: i64 = 10000;
    const ALT_THRESHOLD_DM: i64 = 0x8000 - ALT_MIN_DM;
    const ALT_MAX_DM: i64 = 0x7ffe * 10 - 5;
    let dm = (alt_m * 10.0).round() as i64;
    let packed: u16 = if dm < -ALT_MIN_DM {
        0
    } else if dm > ALT_MAX_DM {
        0xfffe
    } else if dm < ALT_THRESHOLD_DM {
        (dm + ALT_MIN_DM) as u16
    } else {
        (((dm + 5) / 10) as u16) | 0x8000
    };
    let vario = ((vario_ms.abs() + 1.0).ln() / 0.026 * vario_ms.signum()).round().clamp(-128.0, 127.0) as i8;
    let alt = packed.to_be_bytes();
    [alt[0], alt[1], vario as u8]
}

/// Pick the INAV CRSF mode string. The chain is INAV's `crsfFrameFlightMode()` (`telemetry/crsf.c`)
/// verbatim — same tests, same order, same vocabulary ("AH" = alt hold, "ANGH" = angle-hold mode, "CRSH"
/// = course hold vs "CRUZ" = cruise) — minus the states we cannot derive from the unified flags: HRST/GEO
/// (RC-box / geozone state), WRTH (WP-mission RTH), LOTR (needs the airplane platform bit), and the
/// disarmed "WAIT" GPS sentinel. Disarmed reports "OK" when ready, "!ERR" when arming is blocked.
fn crsf_mode_string(flags: u32, arming_flags: u32) -> &'static str {
    if arming_flags & ARMED_FLAG == 0 {
        return if arming_flags & ARMING_DISABLED_MASK != 0 { "!ERR" } else { "OK" };
    }
    if flags & FM_NAV_FW_AUTOLAND != 0 {
        "LAND"
    } else if flags & FM_FAILSAFE != 0 {
        "!FS!"
    } else if flags & FM_MANUAL != 0 {
        "MANU"
    } else if flags & FM_TURTLE != 0 {
        "TURT"
    } else if flags & FM_NAV_RTH != 0 {
        "RTH"
    } else if flags & FM_NAV_POSHOLD != 0 {
        "HOLD"
    } else if flags & FM_NAV_COURSE_HOLD != 0 && flags & FM_NAV_ALTHOLD != 0 {
        "CRUZ"
    } else if flags & FM_NAV_COURSE_HOLD != 0 {
        "CRSH"
    } else if flags & FM_NAV_WP != 0 {
        "WP"
    } else if flags & FM_NAV_ALTHOLD != 0 {
        "AH"
    } else if flags & FM_ANGLE != 0 {
        "ANGL"
    } else if flags & FM_HORIZON != 0 {
        "HOR"
    } else if flags & FM_ANGLEHOLD != 0 {
        "ANGH"
    } else {
        "ACRO"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ready + armed (no ARMING_DISABLED_* bits set).
    const ARMED: u32 = ARMED_FLAG;
    /// A representative ARMING_DISABLED_* reason (compass not calibrated).
    const BLOCKED: u32 = 1 << 12;

    /// The collapse must match INAV's `crsfFrameFlightMode()` chain (`telemetry/crsf.c`).
    #[test]
    fn mode_string_matches_inav_chain() {
        let cases: &[(u32, &str)] = &[
            (FM_NAV_FW_AUTOLAND | FM_NAV_RTH, "LAND"),        // autoland outranks even failsafe
            (FM_FAILSAFE | FM_NAV_RTH, "!FS!"),
            (FM_MANUAL | FM_NAV_RTH, "MANU"),
            (FM_TURTLE, "TURT"),
            (FM_NAV_RTH | FM_NAV_WP, "RTH"),
            (FM_NAV_POSHOLD | FM_NAV_ALTHOLD | FM_ANGLE, "HOLD"),
            (FM_NAV_COURSE_HOLD | FM_NAV_ALTHOLD, "CRUZ"),    // cruise = course hold + alt hold
            (FM_NAV_COURSE_HOLD, "CRSH"),                     // course hold alone
            (FM_NAV_WP | FM_NAV_ALTHOLD, "WP"),
            (FM_NAV_ALTHOLD | FM_ANGLE, "AH"),                // angle+althold
            (FM_NAV_ALTHOLD | FM_HORIZON, "AH"),              // horizon+althold
            (FM_NAV_ALTHOLD | FM_ANGLEHOLD, "AH"),            // anglehold+althold
            (FM_ANGLE, "ANGL"),
            (FM_HORIZON, "HOR"),
            (FM_ANGLEHOLD, "ANGH"),                           // anglehold alone — NOT althold vocabulary
            (0, "ACRO"),
        ];
        for &(flags, expected) in cases {
            assert_eq!(crsf_mode_string(flags, ARMED), expected, "flags=0x{flags:X}");
        }
    }

    #[test]
    fn baro_altitude_is_packed_like_inav() {
        assert_eq!(baro_alt_payload(0.0, 0.0), [0x27, 0x10, 0]); // 10000
        assert_eq!(baro_alt_payload(100.0, 1.0), [0x2A, 0xF8, 27]); // 11000; ln(2) / 0.026 = 26.7
        assert_eq!(baro_alt_payload(-50.5, -1.0), [0x25, 0x17, (-27i8) as u8]); // 9495
        assert_eq!(baro_alt_payload(3000.0, 0.0), [0x8B, 0xB8, 0]); // metres | 0x8000
        assert_eq!(baro_alt_payload(-2000.0, -500.0), [0, 0, (-128i8) as u8]);
    }

    #[test]
    fn disarmed_reports_readiness() {
        assert_eq!(crsf_mode_string(0, 0), "OK");
        assert_eq!(crsf_mode_string(0, BLOCKED), "!ERR");
        // INAV checks ARMED first — an armed craft never reports !ERR.
        assert_eq!(crsf_mode_string(0, ARMED | BLOCKED), "ACRO");
    }
}
