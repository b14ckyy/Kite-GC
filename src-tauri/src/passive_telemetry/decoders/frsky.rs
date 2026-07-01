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

use std::time::Instant;

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::flightmode::{classify_inav, FlightModeState, ARM_DISABLE_BLOCKED};
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, LinkStatsData, StatusData,
};

use super::ap_passthrough::ApPassthroughDecoder;

/// ArduPilot FrSky-passthrough DIY appID range (Yaapu). Frames in this range are routed to the shared
/// AP-passthrough sub-decoder instead of the INAV-native sensor map (level-2 sub-detection).
const AP_PASSTHROUGH_RANGE: std::ops::RangeInclusive<u16> = 0x5000..=0x52FF;

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
const ID_LQ: u16 = 0xF010; // link quality, sent as the "VFR"/RxQuality sensor (FrSky ACCESS, 2019+)

/// S.Port physical ID of the receiver/TX — its frames (RSSI/RxBt) keep coming after an FC-link loss,
/// so anything else marks the FC link alive.
const RECEIVER_PHYS_ID: u8 = 0x98;

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
/// arming-disabled bits, normalized-flags)`. Mirrors smartport.c exactly. The ones column encodes readiness:
/// +1 = arming enabled (ready), +2 = arming disabled (not ready), +4 = ARMED. We surface the "not ready"
/// state as a synthetic disabled bit (shared with CRSF) so the toolbar shows ready vs not-ready — 0 when
/// ready or armed.
fn decode_modes(value: u32) -> (bool, u32, u32) {
    let ones = value % 10;
    let armed = ones & 4 != 0;
    let arm_disable_bits = if ones & 2 != 0 { ARM_DISABLE_BLOCKED } else { 0 };
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
    (armed, arm_disable_bits, f)
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

    // RC link health (RC Link widget). RSSI (0xF101) and LQ (0xF010) are both 0–100 % on FrSky.
    link_rssi: Option<u8>,
    lq: Option<u8>,

    airspeed: f64,
    seen_airspeed: bool,

    armed: bool,
    /// Synthetic INAV "arming disabled" bit for the disarmed not-ready state (0 when armed or ready).
    arm_disable_bits: u32,
    mode_flags: u32,
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
    fresh_link: bool,
}

pub struct FrskyDecoder {
    acc: Vec<u8>,
    state: State,
    /// Lazily created when ArduPilot passthrough (0x5000-range) frames appear (level-2 sub-detection).
    ap: Option<ApPassthroughDecoder>,
    /// Last time a frame from the flight controller (physID ≠ 0x98 receiver) was seen — drives the
    /// FC-link-alive signal. The receiver/TX keeps sending RSSI etc. after the FC link drops, so "any
    /// data" isn't enough; we track FC-origin frames specifically.
    last_fc: Option<Instant>,
}

impl FrskyDecoder {
    pub fn new() -> Self {
        Self { acc: Vec::with_capacity(16), state: State::default(), ap: None, last_fc: None }
    }

    /// True once ArduPilot-passthrough (0x5000-range) frames have appeared → a secondary protocol.
    pub fn ap_active(&self) -> bool {
        self.ap.is_some()
    }

    /// Age (ms) of the last FC-origin frame, or None if none seen yet.
    pub fn fc_age_ms(&self) -> Option<u128> {
        self.last_fc.map(|t| t.elapsed().as_millis())
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
        // physID 0x98 = receiver/TX (RSSI/RxBt keep coming after an FC-link loss); anything else is the
        // flight controller → marks the FC link as alive.
        if f[0] != RECEIVER_PHYS_ID {
            self.last_fc = Some(Instant::now());
        }
        let appid = (f[2] as u16) | ((f[3] as u16) << 8);
        let value = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
        if AP_PASSTHROUGH_RANGE.contains(&appid) {
            // ArduPilot source: route to the passthrough sub-decoder. The native sensor map (lat/lon,
            // alt, speed, battery, …) is still fed by the standard FrSky sensors ArduPilot also sends.
            self.ap.get_or_insert_with(ApPassthroughDecoder::new).apply_packet(appid, value);
        } else {
            self.apply(appid, value);
        }
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
                let (armed, disable_bits, flags) = decode_modes(value);
                s.armed = armed;
                s.arm_disable_bits = disable_bits;
                s.mode_flags = flags;
                s.seen_status = true;
            }
            ID_ASPD => { s.airspeed = value as f64 * 0.514444; s.seen_airspeed = true; }
            ID_RSSI => {
                s.rssi = value as u16;
                s.seen_analog = true;
                // The FC also emits 0xF101 from its (often-unconfigured) RSSI channel — a phantom 0 that
                // alternates with the receiver's real value (0/100 flicker). Ignore 0 for the RC link: a
                // genuine 0 RSSI means the link is dead, in which case no frames would be arriving anyway.
                if value > 0 {
                    s.link_rssi = Some((value as u16).min(100) as u8);
                    s.fresh_link = true;
                }
            }
            ID_LQ => {
                // 0xF010 carries packet LOSS %, not quality (matches the radio's VFR = 100 − raw).
                s.lq = Some(100u32.saturating_sub(value) as u8);
                s.fresh_link = true;
            }
            _ => {}
        }
        // Mark the relevant emit-group fresh (gates publish; mirrors the appID→field mapping above).
        match appid {
            ID_PITCH | ID_ROLL | ID_HEADING => s.fresh_attitude = true,
            ID_FPV | ID_SPEED | ID_GPS_ALT | ID_LATLONG | ID_GNSS | ID_LEGACY_GNSS => s.fresh_gps = true,
            ID_ALTITUDE | ID_VARIO => s.fresh_altitude = true,
            ID_CURRENT | ID_VFAS | ID_FUEL | ID_RSSI => s.fresh_analog = true,
            ID_ASPD => s.fresh_airspeed = true,
            ID_MODES | ID_LEGACY_MODES => s.fresh_status = true,
            _ => {}
        }
    }

    /// Emit the accumulated state as the unified telemetry events and feed the flight recorder. Each
    /// event is only sent once its relevant fields have been seen, so widgets/recorder aren't fed
    /// placeholder zeros.
    pub fn publish(&mut self, app: &AppHandle, recorder: Option<&FlightRecorderHandle>) {
        // ArduPilot passthrough (if present) owns flight mode / armed / EKF / status-text; the native
        // INAV status fields are simply never seen for an AP source, so there is no conflict.
        if let Some(ap) = self.ap.as_mut() {
            ap.publish(app, recorder);
        }
        // Only emit a type when a fresh frame updated it since the last publish (no fixed-tick re-publish
        // of cached state). Capture + clear the flags, then emit from the immutably-borrowed state.
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

        // Status first: drives the recorder's arm/disarm edge + the ARMED indicator.
        if f_status {
            let status = StatusData {
                arming_flags: if s.armed { ARMED_FLAG } else { s.arm_disable_bits },
                flight_mode_flags: s.mode_flags,
                cpu_load: 0,
                sensor_status: 0,
                msp_rc_override: false,
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

        if f_att {
            let att = AttitudeData { roll: s.roll, pitch: s.pitch, yaw: s.yaw };
            let _ = app.emit("telemetry-attitude", &att);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_attitude(&att); }
            }
        }

        if f_gps {
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

        if f_asp {
            let aspd = AirspeedData { airspeed: s.airspeed };
            let _ = app.emit("telemetry-airspeed", &aspd);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_airspeed(&aspd); }
            }
        }

        // RC link: RSSI (0xF101) + LQ (0xF010), both native %; no SNR over S.Port.
        if f_link {
            let ls = LinkStatsData {
                rssi_percent: s.link_rssi.map(|v| v as f32),
                lq: s.lq,
                ..Default::default()
            };
            let _ = app.emit("telemetry-linkstats", &ls);
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() { r.on_linkstats(&ls); }
            }
        }
    }
}
