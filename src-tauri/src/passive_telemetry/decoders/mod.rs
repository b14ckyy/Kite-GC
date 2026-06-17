// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Protocol decoders for passive telemetry. Each turns a locked protocol's frames into the unified
// telemetry events (same names/payloads as MSP/MAVLink) so the frontend is protocol-agnostic.

pub mod crsf;
pub mod frsky;
pub mod ltm;
