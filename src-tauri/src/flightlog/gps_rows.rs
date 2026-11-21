// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! `gps_records` rows of a log import: the decoders hand every kept track row's receiver snapshot
//! here, and each new sample of a receiver becomes one row.

use rusqlite::Connection;

use super::db;
use crate::scheduler::telemetry::GpsReceiverData;

/// A receiver's latest sample with its own log time (µs).
pub(crate) type GpsSample = (u64, GpsReceiverData);

#[derive(Default)]
pub(crate) struct GpsRows {
    /// Log time of the last sample stored per receiver (index = instance): a snapshot repeats a
    /// receiver's sample until its next message.
    last_us: [Option<u64>; 2],
    rows: Vec<(i64, GpsReceiverData)>,
}

impl GpsRows {
    /// `origin_us` is the log time the track's `timestamp_ms` counts from. A sample from before it keeps
    /// its negative time, so the replay sees its real age.
    pub fn offer(&mut self, origin_us: u64, snapshot: &[GpsSample]) {
        for (time_us, rx) in snapshot {
            let Some(last) = self.last_us.get_mut(rx.id as usize) else { continue };
            if *last == Some(*time_us) {
                continue;
            }
            *last = Some(*time_us);
            self.rows.push(((*time_us as i64 - origin_us as i64) / 1000, *rx));
        }
    }

    pub fn store(&self, conn: &Connection, flight_id: i64) -> rusqlite::Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }
        db::insert_gps_records_batch(conn, flight_id, &self.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx(id: u8, num_sat: u8) -> GpsReceiverData {
        GpsReceiverData {
            id,
            fix_type: 2,
            fix_raw: 3,
            num_sat,
            hdop: None,
            lat: 48.1,
            lon: 11.5,
            alt_msl: 500.0,
            vdop: None,
            h_acc_m: None,
            v_acc_m: None,
            speed_ms: None,
            course_deg: None,
            yaw_deg: None,
        }
    }

    /// The raw numbers of the logs on the unified scale the replay reads (0 none, 1 2D, 2 3D, 3 DGPS / RTK).
    #[test]
    fn raw_fix_numbers_map_onto_the_unified_scale() {
        use crate::scheduler::telemetry::unified_fix_from_raw;
        assert_eq!([0, 1, 2, 3, 4, 5, 6, 8].map(unified_fix_from_raw), [0, 0, 1, 2, 3, 3, 3, 3]);
    }

    /// A sample repeated by later snapshots is stored once, at its own time — negative when it is from
    /// before the track's origin; an instance beyond the two receivers is ignored.
    #[test]
    fn one_row_per_new_sample() {
        let mut rows = GpsRows::default();
        rows.offer(1_000_000, &[(900_000, rx(0, 10)), (1_050_000, rx(1, 11))]);
        rows.offer(1_000_000, &[(1_200_000, rx(0, 12)), (1_050_000, rx(1, 11)), (1_200_000, rx(2, 9))]);
        let seen: Vec<(i64, u8, u8)> = rows.rows.iter().map(|(t, r)| (*t, r.id, r.num_sat)).collect();
        assert_eq!(seen, vec![(-100, 0, 10), (50, 1, 11), (200, 0, 12)]);
    }
}
