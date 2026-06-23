// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Protocol Module
// Handles MAVLink v1/v2 frame parsing, serialization, handshake, and handler thread.
// Uses the `mavlink` crate for message definitions (ardupilotmega dialect).

pub mod codec;
pub mod control;
pub mod handler;
pub mod handshake;
pub mod mission;
pub mod params;
pub mod params_rt;
pub mod parser;
pub mod streamrates;

// MAVLink debug-stats tracker (Debug Monitor). Compiled into all builds now (was a release no-op stub)
// so a release `--debug` run populates the MAVLink tab. Methods early-return on
// `crate::debug_mode::enabled()` — a single atomic load when debug mode is off (ADR-008 runtime-gated).
pub mod debug;

pub use handler::MavlinkHandle;
pub use handshake::perform_handshake;
pub use mission::ArduWaypoint;
