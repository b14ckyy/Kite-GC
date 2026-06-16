// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Passive Telemetry Module
//
// A strictly listen-only connection mode for ground-side telemetry streams forwarded by the
// transmitter / radio (EdgeTX, ETHOS), ELRS backpacks or DIY bridges — FrSkyX/SmartPort, CRSF, LTM
// and MAVLink-passive. The wire protocol is **auto-detected** (see `detector`), never user-picked.
//
// This module **never transmits** on the transport: no heartbeats, no polling, no command/waypoint
// writes. See `docs/active/RADIO_TELEMETRY.md`.
//
// Phase B (current): the handler captures the raw byte stream to file for offline analysis and reports
// a detection guess + framing stats to the Debug Monitor. Decoding into the unified telemetry events
// (Phase C) is built once a real capture has been analysed.

pub mod capture;
pub mod detector;
pub mod handler;

pub use handler::{start, PassiveHandle};
