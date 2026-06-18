// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! LTM (Lightweight TeleMetry) encoder — the inverse of `passive_telemetry::decoders::ltm`.
//!
//! Frame = `'$' 'T' <type> <payload little-endian> <crc>`, crc = XOR of the payload bytes only. We emit a
//! frame whenever its source data updates (pass-through rate):
//!   • Attitude update → A-frame      (pitch/roll/yaw i16, degrees)
//!   • GPS update      → G-frame      (lat/lon i32 ×1e7, gs u8 m/s, alt i32 cm, sats<<2|fix u8)
//!   • Analog/Status/Airspeed update → S-frame (vbat u16 mV, mAh u16, rssi u8, airspeed u8, statemode u8)
//!
//! Consumers: antenna trackers (U360GTS), GCS, monitoring apps. Source: INAV `telemetry/ltm.c` + `ltm.h`.

use super::super::cache::TelemetryCache;
use super::Encoder;

// ── Unified flight-mode bits (must match scheduler::telemetry::box_id_to_flight_mode_bit output) ──
const FM_ANGLE: u32 = 1 << 0;
const FM_HORIZON: u32 = 1 << 1;
const FM_HEADING: u32 = 1 << 2;
const FM_NAV_ALTHOLD: u32 = 1 << 3;
const FM_NAV_RTH: u32 = 1 << 4;
const FM_NAV_POSHOLD: u32 = 1 << 5;
const FM_NAV_LAUNCH: u32 = 1 << 7;
const FM_MANUAL: u32 = 1 << 8;
const FM_FAILSAFE: u32 = 1 << 9;
const FM_AUTO_TUNE: u32 = 1 << 10;
const FM_NAV_WP: u32 = 1 << 11;
const FM_NAV_COURSE_HOLD: u32 = 1 << 12;
const FM_NAV_FW_AUTOLAND: u32 = 1 << 18;

/// INAV arming_flags bit 2 = ARMED.
const ARMED_FLAG: u32 = 0x04;

#[derive(Default)]
pub struct LtmEncoder;

impl LtmEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl Encoder for LtmEncoder {
    /// One full LTM set per pacer tick: A (attitude) + G (GPS) + S (status), each emitted only once its
    /// source data is present in the cache. No repeats within a tick (the S-frame is built once).
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8> {
        let mut out = Vec::with_capacity(48);
        if let Some(a) = cache.attitude.as_ref() {
            out.extend_from_slice(&a_frame(a));
        }
        if let Some(g) = cache.gps.as_ref() {
            out.extend_from_slice(&g_frame(g));
        }
        let s = s_frame(cache);
        out.extend_from_slice(&s);
        out
    }
}

/// Assemble `'$' 'T' <type> <payload> <crc=XOR(payload)>`.
fn frame(ty: u8, payload: &[u8]) -> Vec<u8> {
    let crc = payload.iter().fold(0u8, |c, &b| c ^ b);
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.push(b'$');
    out.push(b'T');
    out.push(ty);
    out.extend_from_slice(payload);
    out.push(crc);
    out
}

fn a_frame(a: &crate::scheduler::telemetry::AttitudeData) -> Vec<u8> {
    let mut p = Vec::with_capacity(6);
    p.extend_from_slice(&(a.pitch.round() as i16).to_le_bytes());
    p.extend_from_slice(&(a.roll.round() as i16).to_le_bytes());
    p.extend_from_slice(&(a.yaw.round() as i16).to_le_bytes());
    frame(b'A', &p)
}

fn g_frame(g: &crate::scheduler::telemetry::GpsData) -> Vec<u8> {
    let mut p = Vec::with_capacity(14);
    p.extend_from_slice(&((g.lat * 1e7).round() as i32).to_le_bytes());
    p.extend_from_slice(&((g.lon * 1e7).round() as i32).to_le_bytes());
    p.push(g.ground_speed.round().clamp(0.0, 255.0) as u8);
    p.extend_from_slice(&((g.alt_msl * 100.0).round() as i32).to_le_bytes()); // m → cm
    // LTM fix: 1=no fix, 2=2D, 3=3D (our fix_type is 0/2/3); sats in the upper 6 bits.
    let ltm_fix: u8 = if g.fix_type >= 3 { 3 } else if g.fix_type == 2 { 2 } else { 1 };
    let sats = g.num_sat.min(63);
    p.push((sats << 2) | ltm_fix);
    frame(b'G', &p)
}

fn s_frame(cache: &TelemetryCache) -> Vec<u8> {
    let analog = cache.analog.as_ref();
    let status = cache.status.as_ref();
    // Need at least status or analog to say anything meaningful.
    if analog.is_none() && status.is_none() {
        return Vec::new();
    }
    let vbat_mv = analog.map(|a| (a.voltage * 1000.0).round().clamp(0.0, 65535.0) as u16).unwrap_or(0);
    let mah = analog.map(|a| a.mah_drawn.min(65535) as u16).unwrap_or(0);
    // RSSI: source is MSP 0..1023; LTM carries 0..254.
    let rssi = analog.map(|a| (a.rssi as u32 * 254 / 1023).min(254) as u8).unwrap_or(0);
    let airspeed = cache.airspeed.as_ref().map(|x| x.airspeed.round().clamp(0.0, 255.0) as u8).unwrap_or(0);

    let (mode, armed, failsafe) = match status {
        Some(s) => (
            ltm_mode_from_flags(s.flight_mode_flags),
            s.arming_flags & ARMED_FLAG != 0,
            s.flight_mode_flags & FM_FAILSAFE != 0,
        ),
        None => (4u8 /* acro */, false, false),
    };
    let statemode = (mode << 2) | ((failsafe as u8) << 1) | (armed as u8);

    let mut p = Vec::with_capacity(7);
    p.extend_from_slice(&vbat_mv.to_le_bytes());
    p.extend_from_slice(&mah.to_le_bytes());
    p.push(rssi);
    p.push(airspeed);
    p.push(statemode);
    frame(b'S', &p)
}

/// Collapse the unified flight-mode bitfield into a single LTM mode number (`ltm_modes_e`). Inverse of
/// `decoders::ltm::classify_ltm_mode`; ordered most-dominant first since LTM carries only one mode.
fn ltm_mode_from_flags(f: u32) -> u8 {
    if f & FM_NAV_FW_AUTOLAND != 0 {
        15 // RTH autoland
    } else if f & FM_NAV_RTH != 0 {
        13 // RTH
    } else if f & FM_NAV_WP != 0 {
        10 // mission / waypoint
    } else if f & FM_NAV_LAUNCH != 0 {
        20 // launch
    } else if f & FM_NAV_POSHOLD != 0 {
        9 // GPS hold
    } else if f & FM_NAV_COURSE_HOLD != 0 {
        18 // cruise
    } else if f & FM_NAV_ALTHOLD != 0 {
        8 // alt hold (LTM: angle+althold)
    } else if f & FM_AUTO_TUNE != 0 {
        21 // autotune
    } else if f & FM_MANUAL != 0 {
        0 // manual
    } else if f & FM_HEADING != 0 {
        11 // heading hold
    } else if f & FM_HORIZON != 0 {
        3 // horizon
    } else if f & FM_ANGLE != 0 {
        2 // angle
    } else {
        4 // acro / rate fallback
    }
}
