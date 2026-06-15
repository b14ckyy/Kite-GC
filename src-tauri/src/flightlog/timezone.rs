// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Timezone resolution for flight-local time display (ADR-048).
//
// Flights are stored with a true-UTC `start_time` plus `utc_offset_min` — the local UTC offset at the
// flight location, DST-aware. This module resolves that offset:
//   - from coordinates (imports): tzf-rs maps lat/lon → IANA zone name, chrono-tz gives the offset for
//     the flight's date (handles DST).
//   - from the ground-station PC (live): the GCS sits at the field, so its own local offset is exact.

use std::sync::OnceLock;

use chrono::{DateTime, Offset, Utc};
use chrono_tz::Tz;
use tzf_rs::DefaultFinder;

/// The embedded timezone-boundary finder is expensive to build (loads the bundled polygon data), so
/// it is constructed once on first use and shared.
fn finder() -> &'static DefaultFinder {
    static FINDER: OnceLock<DefaultFinder> = OnceLock::new();
    FINDER.get_or_init(DefaultFinder::new)
}

/// Resolve the local UTC offset (minutes, east-positive) at `lat`/`lon` for the instant `utc`,
/// DST-aware. Returns `None` if the coordinate maps to no timezone (e.g. open ocean) or the zone
/// name can't be parsed — the caller then falls back to UTC display.
pub fn offset_min_at(lat: f64, lon: f64, utc: DateTime<Utc>) -> Option<i32> {
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    // tzf-rs takes (longitude, latitude); returns "" when no zone covers the point.
    let name = finder().get_tz_name(lon, lat);
    if name.is_empty() {
        return None;
    }
    let tz: Tz = name.parse().ok()?;
    let secs = utc.with_timezone(&tz).offset().fix().local_minus_utc();
    Some(secs / 60)
}

/// The ground-station PC's own UTC offset right now (minutes, east-positive), DST-aware. Used for
/// live recordings — the GCS is at the flight location, so this is the flight-local offset.
pub fn local_offset_min_now() -> i32 {
    chrono::Local::now().offset().fix().local_minus_utc() / 60
}
