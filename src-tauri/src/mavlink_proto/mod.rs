// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink Protocol Module
// Handles MAVLink v1/v2 frame parsing, serialization, handshake, and handler thread.
// Uses the `mavlink` crate for message definitions (ardupilotmega dialect).

pub mod codec;
pub mod handler;
pub mod handshake;
pub mod mission;
pub mod params;
pub mod parser;
pub mod streamrates;

// MAVLink debug-stats tracker (Debug Monitor). Real implementation only in debug builds;
// in release it is a zero-sized no-op struct, so all tracking calls compile away.
#[cfg(debug_assertions)]
pub mod debug;

#[cfg(not(debug_assertions))]
pub mod debug {
    pub struct MavlinkDebugTracker;
    impl MavlinkDebugTracker {
        pub fn new() -> Self { Self }
        #[inline(always)]
        pub fn on_rx(&mut self, _: u32, _: usize) {}
        #[inline(always)]
        pub fn on_tx(&mut self, _: u32, _: usize) {}
        #[inline(always)]
        pub fn maybe_emit(&mut self, _: &tauri::AppHandle) {}
    }
}

pub use handler::MavlinkHandle;
pub use handshake::perform_handshake;
pub use mission::ArduWaypoint;
