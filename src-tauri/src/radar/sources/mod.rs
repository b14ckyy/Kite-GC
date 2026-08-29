// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — data sources. One module per source family (added incrementally, see the phasing in
// docs/active/RADAR_TRACKING_CORE.md §9). Phase 0 ships only the dev-only `sim` source.

// adsb_mavlink and formation_flight read from a serial device through `transport::serial`, which
// exists on every target (on iOS as the stand-in whose opens fail cleanly) — so every source
// compiles everywhere and platform capability is a runtime property, not a compile-time one.
pub mod adsb_mavlink;
pub mod adsb_msp;
pub mod adsb_online;
pub mod formation_flight;
pub mod sim;
