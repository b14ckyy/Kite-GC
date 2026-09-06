// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The Telemetry API wire contract — schema 1.
//!
//! One `Frame` per emission, serialized as one JSON object. This is a PUBLIC contract: field names and
//! units here are what external consumers program against (documented in
//! `docs/user/reference/telemetry-api.md`). Groups mirror the frontend's `TelemetryData` / the Raw
//! Telemetry popup; units are the store's canonical raw units (degrees, metres, m/s, V, A, W, mAh, %).
//! A group is `null` until its source has delivered a value on the current link — never a fake zero.
//!
//! Bump `SCHEMA_VERSION` only when an existing field changes meaning or disappears; adding fields is
//! backwards compatible and needs no bump.

use serde::Serialize;

use super::state::{
    AltRef, ApiState, Attitude, BatteryInstance, FlightMode, Gcs, Link, PassiveProtocol, Sensors,
    Vehicle, Wind,
};

/// Payload contract version.
pub const SCHEMA_VERSION: u32 = 1;

/// ARMED is bit 2 of the unified `arming_flags` bitfield for MSP and MAVLink alike (INAV's
/// `armingFlag_e`; the MAVLink path maps HEARTBEAT's armed bit onto it). Mirrors `ARMED_BIT` in
/// `src/lib/helpers/arming.ts` — keep the two in step.
const ARMED_BIT: u32 = 1 << 2;

/// Which link Kite is on — the three input paths of the unified pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Msp,
    Mavlink,
    Passive,
}

/// What the ticker knows beyond the event state: the link itself.
#[derive(Debug, Clone, Default)]
pub struct LinkInfo {
    pub connected: bool,
    pub protocol: Option<Protocol>,
    pub fc_variant: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame<'a> {
    pub schema: u32,
    /// Epoch ms, stamped when the frame was built.
    pub ts: u64,
    /// Monotonic per server start — a gap tells a consumer it missed frames.
    pub seq: u64,
    /// `false` → no vehicle link; `telemetry` then holds the last known state (or all-null after start).
    pub connected: bool,
    pub protocol: Option<Protocol>,
    pub fc_variant: Option<&'a str>,
    pub telemetry: Telemetry<'a>,
    pub home: Option<HomeOut>,
    pub gcs: Option<&'a Gcs>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry<'a> {
    pub gps: Option<GpsOut>,
    pub attitude: Option<&'a Attitude>,
    pub altitude: Option<AltitudeOut>,
    pub alt_ref: Option<&'a AltRef>,
    pub wind: Option<&'a Wind>,
    pub battery: Option<BatteryOut>,
    pub batteries: Option<&'a Vec<BatteryInstance>>,
    pub link: Option<&'a Link>,
    pub status: Option<StatusOut>,
    pub sensors: Option<&'a Sensors>,
    pub flight_mode: Option<&'a FlightMode>,
    pub nav: Option<NavOut>,
    pub ekf: Option<EkfOut>,
    pub misc: Option<MiscOut>,
    pub vehicle: Option<&'a Vehicle>,
    /// Passive links only: the carrier protocol Kite locked onto (+ a tunneled one, e.g. MAVLink).
    pub passive_protocol: Option<&'a PassiveProtocol>,
    /// Epoch ms of the last FC telemetry event of any group.
    pub last_update: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsOut {
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: f64,
    pub ground_speed: f64,
    pub course: f64,
    pub num_sat: u8,
    pub fix_type: u8,
    pub hdop: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AltitudeOut {
    pub altitude: f64,
    pub vario: f64,
    pub airspeed: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryOut {
    pub voltage: f64,
    pub current: f64,
    /// W — V × A when the source protocol carries no power of its own.
    pub power: f64,
    pub mah_drawn: u32,
    pub percentage: u8,
    pub cell_count: u8,
    /// 0–100, from the FC's throttle telemetry (INAV MISC2 / MAVLink VFR_HUD); null when not reported.
    pub throttle: Option<u8>,
    /// Raw RSSI as the source protocol reports it (INAV: 0–1023).
    pub rssi: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOut {
    /// Derived from `armingFlags` bit 2 — the one field almost every consumer wants.
    pub armed: bool,
    pub arming_flags: u32,
    pub flight_mode_flags: u32,
    pub cpu_load: u16,
    pub sensor_status: u16,
    pub msp_rc_override: bool,
    /// The FC itself is talking (false = the link carrier is up but the FC has gone quiet).
    pub fc_alive: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavOut {
    pub nav_state: u8,
    pub active_wp: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EkfOut {
    pub status: Option<u8>,
    pub max_variance: Option<f64>,
    pub flags: Option<u32>,
    pub ekf_type: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiscOut {
    pub auto_throttle: bool,
    pub uptime_s: u32,
    pub flight_time_s: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeOut {
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: f64,
    pub set: bool,
}

impl<'a> Frame<'a> {
    pub fn build(state: &'a ApiState, link: &'a LinkInfo, seq: u64, ts: u64) -> Self {
        let gps = state.gps.as_ref().map(|g| GpsOut {
            lat: g.lat,
            lon: g.lon,
            alt_msl: g.alt_msl,
            ground_speed: g.ground_speed,
            course: g.course,
            num_sat: g.num_sat,
            fix_type: g.fix_type,
            hdop: state.gps_stats.as_ref().map(|s| s.hdop),
        });
        let altitude = state.altitude.as_ref().map(|a| AltitudeOut {
            altitude: a.altitude,
            vario: a.vario,
            airspeed: state.airspeed.as_ref().map(|s| s.airspeed),
        });
        let battery = state.analog.as_ref().map(|a| BatteryOut {
            voltage: a.voltage,
            current: a.current,
            power: if a.power > 0.0 { a.power } else { a.voltage * a.current },
            mah_drawn: a.mah_drawn,
            percentage: a.battery_percentage,
            cell_count: a.cell_count,
            throttle: state.misc.as_ref().map(|m| m.throttle_pct),
            rssi: a.rssi,
        });
        let status = state.status.as_ref().map(|s| StatusOut {
            armed: s.arming_flags & ARMED_BIT != 0,
            arming_flags: s.arming_flags,
            flight_mode_flags: s.flight_mode_flags,
            cpu_load: s.cpu_load,
            sensor_status: s.sensor_status,
            msp_rc_override: s.msp_rc_override,
            fc_alive: state.fc_link.as_ref().map(|f| f.alive),
        });
        let nav = state.nav.as_ref().map(|n| NavOut { nav_state: n.nav_state, active_wp: n.active_wp_number });
        let ekf = match (&state.ekf_status, &state.ekf_type) {
            (None, None) => None,
            (st, ty) => Some(EkfOut {
                status: st.as_ref().map(|s| s.status),
                max_variance: st.as_ref().and_then(|s| s.max_variance),
                flags: st.as_ref().and_then(|s| s.flags),
                ekf_type: ty.as_ref().map(|t| t.ekf_type),
            }),
        };
        let misc = state.misc.as_ref().map(|m| MiscOut {
            auto_throttle: m.auto_throttle,
            uptime_s: m.uptime_s,
            flight_time_s: m.flight_time_s,
        });
        let home = state
            .home
            .as_ref()
            .map(|h| HomeOut { lat: h.lat, lon: h.lon, alt_msl: h.alt, set: true });

        Frame {
            schema: SCHEMA_VERSION,
            ts,
            seq,
            connected: link.connected,
            protocol: link.protocol,
            fc_variant: link.fc_variant.as_deref(),
            telemetry: Telemetry {
                gps,
                attitude: state.attitude.as_ref(),
                altitude,
                alt_ref: state.alt_ref.as_ref(),
                wind: state.wind.as_ref(),
                battery,
                batteries: state.batteries.as_ref(),
                link: state.link.as_ref(),
                status,
                sensors: state.sensors.as_ref(),
                flight_mode: state.flight_mode.as_ref(),
                nav,
                ekf,
                misc,
                vehicle: state.vehicle.as_ref(),
                passive_protocol: state.passive_protocol.as_ref(),
                last_update: state.last_update_ms,
            },
            home,
            gcs: state.gcs.as_ref(),
        }
    }
}

/// The `hello` record a TCP client receives first: schema + the top-level group names, so a consumer
/// can check compatibility before parsing frames.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub schema: u32,
    pub hello: bool,
    pub groups: &'static [&'static str],
    pub rate_hz: f64,
}

pub const GROUPS: &[&str] = &[
    "gps", "attitude", "altitude", "altRef", "wind", "battery", "batteries", "link", "status", "sensors",
    "flightMode", "nav", "ekf", "misc", "vehicle", "passiveProtocol", "lastUpdate",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry_api::state::{Analog, Status};

    #[test]
    fn armed_is_bit_two_and_power_falls_back_to_v_times_a() {
        let mut st = ApiState::default();
        st.status = Some(Status { arming_flags: 0b100, flight_mode_flags: 0, cpu_load: 0, sensor_status: 0, msp_rc_override: false });
        st.analog = Some(Analog { voltage: 12.0, mah_drawn: 0, rssi: 0, current: 2.5, power: 0.0, battery_percentage: 50, cell_count: 3 });
        let link = LinkInfo { connected: true, protocol: Some(Protocol::Msp), fc_variant: Some("INAV".into()) };
        let f = Frame::build(&st, &link, 1, 0);
        let v: serde_json::Value = serde_json::to_value(&f).unwrap();
        assert_eq!(v["telemetry"]["status"]["armed"], true);
        assert_eq!(v["telemetry"]["battery"]["power"], 30.0);
        assert_eq!(v["protocol"], "msp");
        assert!(v["telemetry"]["gps"].is_null(), "unknown groups are null, not fake zeros");
        assert!(v["home"].is_null());
    }

    #[test]
    fn dto_reads_snake_case_and_writes_camel_case() {
        let a: Analog = serde_json::from_str(r#"{"voltage":11.1,"mah_drawn":5,"rssi":900,"current":1.0,"power":11.1,"battery_percentage":80,"cell_count":3}"#).unwrap();
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"mahDrawn\":5") && s.contains("\"batteryPercentage\":80"), "{s}");
    }
}
