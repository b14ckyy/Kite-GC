// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight recording (arm → disarm) into SQLite, the logbook and its enrichment (geocode, weather).

pub mod db;
pub mod ardupilot;
pub mod blackbox;
pub mod decoder;
pub mod exchange;
pub mod geocode;
pub mod group;
pub mod group_store;
pub mod hires;
mod inav;
pub mod msp_raw_logger;
pub mod raw_import;
pub mod recorder;
pub mod timezone;
pub mod tlog_logger;
pub mod track_export;
pub mod types;
pub mod ulog;
pub mod weather;
