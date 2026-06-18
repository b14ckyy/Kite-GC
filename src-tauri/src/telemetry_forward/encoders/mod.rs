// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Output-protocol encoders — the inverse of `passive_telemetry::decoders`. Each encoder turns the
//! unified telemetry cache into wire frames of its protocol.

pub mod crsf;
pub mod ltm;
pub mod mavlink;
pub mod smartport;

use super::cache::TelemetryCache;

/// A protocol encoder. `frame_set` is called once per pacer tick (see `RelayHub` — paced on the
/// attitude update) and returns one complete set of output frames built from the current cache. Pacing
/// (rather than emitting per inbound event) decouples the output rate from the input's framing/republish
/// cadence — e.g. SmartPort re-emits all cached fields at 10 Hz, which would otherwise inflate the output
/// with repeated data.
pub trait Encoder: Send {
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8>;
}
