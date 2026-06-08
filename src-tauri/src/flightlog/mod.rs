// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Recording & Logbook Module
// Records telemetry data during flights (arm→disarm) into a SQLite database.
// Supports optional raw text logging, reverse geocoding, and weather metadata.

pub mod db;
pub mod ardupilot;
pub mod blackbox;
pub mod exchange;
pub mod geocode;
pub mod raw_logger;
pub mod recorder;
pub mod tlog_logger;
pub mod track_export;
pub mod types;
pub mod weather;
