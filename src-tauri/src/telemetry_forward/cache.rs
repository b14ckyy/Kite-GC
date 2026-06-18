// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Latest-values telemetry cache for the relay layer.
//!
//! The relay tap (see `mod.rs`) deserializes the unified `telemetry-*` events into these structs and
//! stores the most recent value of each. Encoders read the cache to build output frames — directly the
//! ones that bundle multiple fields (LTM S-frame = battery + airspeed + status), and on a timer the ones
//! that need periodic emission (MAVLink heartbeat, later).

use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, StatusData,
};

/// Which unified telemetry type just updated — drives encoders that emit per source frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemKind {
    Attitude,
    Gps,
    Altitude,
    Analog,
    Status,
    Airspeed,
}

/// Most recent value of each unified telemetry type. `None` until first seen.
#[derive(Debug, Clone, Default)]
pub struct TelemetryCache {
    pub attitude: Option<AttitudeData>,
    pub gps: Option<GpsData>,
    pub altitude: Option<AltitudeData>,
    pub analog: Option<AnalogData>,
    pub status: Option<StatusData>,
    pub airspeed: Option<AirspeedData>,
}
