// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — data sources. One module per source family (added incrementally, see the phasing in
// docs/active/RADAR_TRACKING_CORE.md §9). Phase 0 ships only the dev-only `sim` source.

// adsb_mavlink and formation_flight read from a serial device (serialport), unavailable on iOS.
// The MSP/online/sim sources are transport-independent and build everywhere.
#[cfg(not(target_os = "ios"))]
pub mod adsb_mavlink;
pub mod adsb_msp;
pub mod adsb_online;
#[cfg(not(target_os = "ios"))]
pub mod formation_flight;
pub mod sim;
