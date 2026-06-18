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
const FT_VARIO: u8 = 0x07;
const FT_BATTERY: u8 = 0x08;
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
const ARMED_FLAG: u32 = 0x04;

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
            let p = ((al.vario * 100.0).round() as i16).to_be_bytes();
            frame(FT_VARIO, &p, &mut out);
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
            let text = crsf_mode_string(s.flight_mode_flags, s.arming_flags & ARMED_FLAG != 0);
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

/// Inverse of `decoders::crsf::classify_crsf_mode`: pick the dominant INAV CRSF mode string. Disarmed →
/// "OK" (the disarmed sentinel).
fn crsf_mode_string(flags: u32, armed: bool) -> &'static str {
    if !armed {
        return "OK";
    }
    if flags & FM_FAILSAFE != 0 {
        "!FS!"
    } else if flags & FM_NAV_RTH != 0 {
        "RTH"
    } else if flags & FM_NAV_WP != 0 {
        "WP"
    } else if flags & FM_NAV_COURSE_HOLD != 0 {
        "CRUZ"
    } else if flags & FM_NAV_POSHOLD != 0 {
        "HOLD"
    } else if flags & FM_MANUAL != 0 {
        "MANU"
    } else if flags & FM_HORIZON != 0 {
        "HOR"
    } else if flags & FM_ANGLE != 0 {
        if flags & FM_NAV_ALTHOLD != 0 {
            "ANGH"
        } else {
            "ANGL"
        }
    } else {
        "ACRO"
    }
}
