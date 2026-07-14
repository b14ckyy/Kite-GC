// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JSON encoder — the read-only telemetry export, served over HTTP by `output/http.rs`.
//!
//! Unlike the FC protocols, this one has a **public** wire contract, so it does not serialize the
//! internal cache structs directly (those are snake_case and free to change with the frontend). It maps
//! them into an explicit, versioned camelCase DTO instead. Bump `SCHEMA_VERSION` on any incompatible
//! change to it.
//!
//! Every frame carries the mission ID, so a consumer can attribute samples without out-of-band context.
//! A relay with no mission ID never gets built at all (see `relay.rs`).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::super::cache::TelemetryCache;
use super::Encoder;

/// Payload contract version. Bump on any incompatible change to `Frame` and friends.
const SCHEMA_VERSION: u32 = 1;

/// ARMED is bit 2 of the normalized `arming_flags` bitfield, for MSP and MAVLink alike (INAV's
/// `armingFlag_e`; the MAVLink path maps HEARTBEAT's armed bit onto it). Mirrors `ARMED_BIT` in
/// `src/lib/helpers/arming.ts` — keep the two in step.
const ARMED_BIT: u32 = 1 << 2;

/// Fall back to this when the configured rate is absent or nonsensical.
const DEFAULT_RATE_HZ: f32 = 5.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame<'a> {
    /// Payload contract version — see `SCHEMA_VERSION`.
    schema: u32,
    mission_id: &'a str,
    /// Unix epoch milliseconds, stamped at encode time.
    ts: u64,
    /// Monotonic counter — a gap tells the consumer it dropped frames.
    seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    attitude: Option<Attitude>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gps: Option<Gps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    altitude: Option<Altitude>,
    #[serde(skip_serializing_if = "Option::is_none")]
    battery: Option<Battery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    airspeed: Option<Airspeed>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Attitude {
    /// Degrees, ±180.
    roll: f64,
    /// Degrees, ±90.
    pitch: f64,
    /// Heading, 0–360.
    yaw: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Gps {
    fix_type: u8,
    num_sat: u8,
    /// Decimal degrees.
    lat: f64,
    lon: f64,
    /// Metres.
    alt_msl: f64,
    /// m/s.
    ground_speed: f64,
    /// Degrees.
    course: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Altitude {
    /// Metres. Whether this is true MSL or relative-to-home depends on the source protocol.
    altitude: f64,
    /// m/s, positive up.
    vario: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Battery {
    /// Volts.
    voltage: f64,
    /// Amps.
    current: f64,
    /// Watts.
    power: f64,
    mah_drawn: u32,
    /// 0–100.
    percentage: u8,
    cell_count: u8,
    /// Raw RSSI as the source protocol reports it (scale is protocol-dependent).
    rssi: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// Derived from `armingFlags` bit 2 — the one field a consumer almost always wants.
    armed: bool,
    arming_flags: u32,
    flight_mode_flags: u32,
    cpu_load: u16,
    sensor_status: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Airspeed {
    /// m/s.
    airspeed: f64,
}

pub struct JsonEncoder {
    mission_id: String,
    /// Minimum wall-clock gap between emitted frames — see `frame_set`.
    min_interval: Duration,
    last_emit: Option<Instant>,
    seq: u64,
}

impl JsonEncoder {
    /// `rate_hz` is clamped to a sane range: below ~0.1 Hz the export looks dead, and above 50 Hz we'd
    /// just be reserializing the same cache faster than any source updates it. An absent or nonsensical
    /// value falls back to the default.
    pub fn new(mission_id: String, rate_hz: Option<f32>) -> Self {
        let hz = match rate_hz {
            Some(hz) if hz.is_finite() && hz > 0.0 => hz,
            _ => DEFAULT_RATE_HZ,
        };
        let hz = hz.clamp(0.1, 50.0);
        Self {
            mission_id,
            min_interval: Duration::from_secs_f32(1.0 / hz),
            last_emit: None,
            seq: 0,
        }
    }
}

impl Encoder for JsonEncoder {
    /// The relay paces `frame_set` on the *attitude* update, which on MAVLink can run at 10–50 Hz — far
    /// more than a JSON/HTTP consumer wants. So we rate-limit here and return an **empty** Vec when
    /// called too soon; `Relay::emit_set` early-returns on that, so no frame is written and the byte/frame
    /// counters correctly stay put.
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8> {
        let now = Instant::now();
        if let Some(last) = self.last_emit {
            if now.duration_since(last) < self.min_interval {
                return Vec::new();
            }
        }
        self.last_emit = Some(now);
        self.seq += 1;

        let frame = Frame {
            schema: SCHEMA_VERSION,
            mission_id: &self.mission_id,
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            seq: self.seq,
            attitude: cache.attitude.as_ref().map(|a| Attitude { roll: a.roll, pitch: a.pitch, yaw: a.yaw }),
            gps: cache.gps.as_ref().map(|g| Gps {
                fix_type: g.fix_type,
                num_sat: g.num_sat,
                lat: g.lat,
                lon: g.lon,
                alt_msl: g.alt_msl,
                ground_speed: g.ground_speed,
                course: g.course,
            }),
            altitude: cache.altitude.as_ref().map(|a| Altitude { altitude: a.altitude, vario: a.vario }),
            battery: cache.analog.as_ref().map(|a| Battery {
                voltage: a.voltage,
                current: a.current,
                power: a.power,
                mah_drawn: a.mah_drawn,
                percentage: a.battery_percentage,
                cell_count: a.cell_count,
                rssi: a.rssi,
            }),
            status: cache.status.as_ref().map(|s| Status {
                armed: (s.arming_flags & ARMED_BIT) != 0,
                arming_flags: s.arming_flags,
                flight_mode_flags: s.flight_mode_flags,
                cpu_load: s.cpu_load,
                sensor_status: s.sensor_status,
            }),
            airspeed: cache.airspeed.as_ref().map(|a| Airspeed { airspeed: a.airspeed }),
        };

        match serde_json::to_vec(&frame) {
            Ok(mut bytes) => {
                bytes.push(b'\n');
                bytes
            }
            Err(e) => {
                log::warn!("[RELAY json] encode failed: {e}");
                Vec::new()
            }
        }
    }
}
