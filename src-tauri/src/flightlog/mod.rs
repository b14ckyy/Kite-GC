// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight recording (arm → disarm) into SQLite, the logbook and its enrichment (geocode, weather).

pub mod db;
pub mod db_move;
pub mod ardupilot;
pub mod blackbox;
pub mod decoder;
pub mod exchange;
pub mod geocode;
mod gps_rows;
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
pub mod video_files;
pub mod video_link;
pub mod ulog;
pub mod weather;

/// A POSITION_TARGET_GLOBAL_INT as the recorded nav target `(lat, lon, alt_m)`; `None` for the 0 / 0 an FC
/// sends while it has no target.
pub fn nav_target(lat_int: i32, lon_int: i32, alt: f32) -> Option<(f64, f64, f64)> {
    if lat_int == 0 && lon_int == 0 {
        return None;
    }
    Some((lat_int as f64 / 1e7, lon_int as f64 / 1e7, alt as f64))
}
