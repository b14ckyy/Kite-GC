// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Latest-values state for the Telemetry API — one slot per unified `telemetry-*` event group.
//!
//! The API deliberately keeps its OWN mirror of the unified model instead of reusing the relay cache
//! (`telemetry_forward::cache`): the relay needs six groups to encode FC protocols, the API needs every
//! group the frontend's `TelemetryData` holds, so that what an external consumer reads is exactly what
//! the Raw Telemetry popup shows. The producers (scheduler / mavlink_proto / passive_telemetry) are not
//! touched: the taps in `mod.rs` deserialize the same event payloads the frontend receives.
//!
//! Every DTO here deserializes the event's snake_case payload and serializes camelCase for the wire —
//! the public contract is stated once, in `frame.rs`, and the field names below are the ones that
//! reach consumers. **When a new `telemetry-*` event appears, add its slot here, its tap in `mod.rs`
//! and its place in `frame.rs`.**

use serde::{Deserialize, Serialize};

/// Applies to every payload DTO: read the backend event (snake_case), write the API (camelCase).
macro_rules! dto {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
    };
}

dto! { pub struct Attitude { pub roll: f64, pub pitch: f64, pub yaw: f64 } }
dto! { pub struct Gps {
    pub fix_type: u8, pub num_sat: u8, pub lat: f64, pub lon: f64, pub alt_msl: f64,
    pub ground_speed: f64, pub course: f64,
} }
dto! { pub struct GpsStats { pub hdop: f64, #[serde(default)] pub eph: Option<f64>, #[serde(default)] pub epv: Option<f64> } }
dto! { pub struct Altitude { pub altitude: f64, pub vario: f64 } }
dto! { pub struct AltRef { pub msl: bool } }
dto! { pub struct Analog {
    pub voltage: f64, pub mah_drawn: u32, pub rssi: u16, pub current: f64, pub power: f64,
    pub battery_percentage: u8, pub cell_count: u8,
} }
dto! { pub struct BatteryInstance {
    pub id: u8, pub voltage: f64, pub current: f64, pub mah_drawn: u32, pub percentage: u8,
    pub cell_count: u8, #[serde(default)] pub temperature: Option<f64>,
} }
dto! { pub struct Status {
    pub arming_flags: u32, pub flight_mode_flags: u32, pub cpu_load: u16, pub sensor_status: u16,
    #[serde(default)] pub msp_rc_override: bool,
} }
dto! { pub struct Sensors {
    pub gyro: u8, pub acc: u8, pub mag: u8, pub baro: u8, pub gps: u8, pub rangefinder: u8,
    pub pitot: u8, pub opflow: u8, #[serde(default)] pub prearm: u8, #[serde(default)] pub rc_receiver: u8,
} }
dto! { pub struct FlightMode { pub primary: String, #[serde(default)] pub modifiers: Vec<String> } }
dto! { pub struct Nav { pub active_wp_number: u8, pub nav_state: u8 } }
dto! { pub struct EkfStatus { pub status: u8, #[serde(default)] pub max_variance: Option<f64>, #[serde(default)] pub flags: Option<u32> } }
dto! { pub struct EkfType { pub ekf_type: i32 } }
dto! { pub struct Link {
    #[serde(default)] pub rssi_percent: Option<f32>, #[serde(default)] pub rssi_dbm: Option<i16>,
    #[serde(default)] pub lq: Option<u8>, #[serde(default)] pub snr_db: Option<i8>,
} }
dto! { pub struct FcLink { pub alive: bool } }
dto! { pub struct Misc { pub throttle_pct: u8, pub auto_throttle: bool, pub uptime_s: u32, pub flight_time_s: u32 } }
dto! { pub struct Wind { pub direction_from_deg: f64, pub speed_ms: f64 } }
dto! { pub struct Airspeed { pub airspeed: f64 } }
dto! { pub struct Vehicle { pub quadplane: bool } }
dto! { pub struct PassiveProtocol { pub primary: String, #[serde(default)] pub secondary: Option<String> } }
dto! { pub struct Home { pub lat: f64, pub lon: f64, pub alt: f64 } }

/// Ground-station position, pushed by the frontend (the resolved GCS marker) — not an FC event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gcs {
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: Option<f64>,
    pub accuracy_m: Option<f64>,
}

/// Newest value of every group. `None` until first seen on the current link.
#[derive(Debug, Clone, Default)]
pub struct ApiState {
    pub attitude: Option<Attitude>,
    pub gps: Option<Gps>,
    pub gps_stats: Option<GpsStats>,
    pub altitude: Option<Altitude>,
    pub alt_ref: Option<AltRef>,
    pub analog: Option<Analog>,
    pub batteries: Option<Vec<BatteryInstance>>,
    pub status: Option<Status>,
    pub sensors: Option<Sensors>,
    pub flight_mode: Option<FlightMode>,
    pub nav: Option<Nav>,
    pub ekf_status: Option<EkfStatus>,
    pub ekf_type: Option<EkfType>,
    pub link: Option<Link>,
    pub fc_link: Option<FcLink>,
    pub misc: Option<Misc>,
    pub wind: Option<Wind>,
    pub airspeed: Option<Airspeed>,
    pub vehicle: Option<Vehicle>,
    pub passive_protocol: Option<PassiveProtocol>,
    /// Slow-changing, kept between updates (Marc, 2026-09-06): the FC home and the GCS position.
    pub home: Option<Home>,
    pub gcs: Option<Gcs>,
    /// Epoch ms of the last FC telemetry event (any group). `None` before the first one.
    pub last_update_ms: Option<u64>,
}

impl ApiState {
    /// Forget everything that belongs to a vehicle link (a new connection starts clean). The GCS
    /// position is ours, not the vehicle's, and survives.
    pub fn reset_link(&mut self) {
        let gcs = self.gcs.take();
        *self = Self::default();
        self.gcs = gcs;
    }
}
