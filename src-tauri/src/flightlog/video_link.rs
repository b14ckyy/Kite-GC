// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Recorded videos find their flight by time. A recording ends at disarm, the flight row may come much
//! later (End Flight dialog, a pending session resumed after a restart) or never (discarded), so finished
//! recordings wait in a small file next to the database until a flight that overlaps them is committed.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

use super::types::Flight;
use crate::video::record::RecordedVideo;

const WAITING_FILE: &str = "video_recordings.json";
/// A recording nobody claimed is forgotten after this long; its file stays where it is.
const WAITING_MAX_MS: i64 = 30 * 24 * 3600 * 1000;

fn waiting_path(db_file: &Path) -> PathBuf {
    db_file.parent().unwrap_or(Path::new(".")).join(WAITING_FILE)
}

fn read_waiting(db_file: &Path) -> Vec<RecordedVideo> {
    std::fs::read(waiting_path(db_file))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_waiting(db_file: &Path, list: &[RecordedVideo]) {
    let path = waiting_path(db_file);
    let result = if list.is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        serde_json::to_vec_pretty(list).map_err(std::io::Error::other).and_then(|b| std::fs::write(&path, b))
    };
    if let Err(e) = result {
        log::warn!("[video] recordings waiting for their flight not stored ({}): {e}", path.display());
    }
}

/// Finished recordings wait for the flight they belong to.
pub fn add_waiting(db_file: &Path, videos: &[RecordedVideo]) {
    if videos.is_empty() {
        return;
    }
    let mut list = read_waiting(db_file);
    list.extend(videos.iter().cloned());
    write_waiting(db_file, &list);
}

/// Attach every waiting recording that overlaps the flight in time. The two sync anchors are the
/// recording's first and last picture on the flight's clock (ms from `start_time`).
pub fn attach_waiting(conn: &Connection, db_file: &Path, flight_id: i64, flight: &Flight) {
    let list = read_waiting(db_file);
    if list.is_empty() {
        return;
    }
    let start = flight.start_time.timestamp_millis();
    let end = flight.end_time.map(|t| t.timestamp_millis()).unwrap_or(start + flight.duration_sec.unwrap_or(0) * 1000);
    let now = chrono::Utc::now().timestamp_millis();
    let mut keep = Vec::new();
    for v in list {
        if v.start_unix_ms > end || v.end_unix_ms < start {
            if now - v.end_unix_ms < WAITING_MAX_MS {
                keep.push(v);
            }
            continue;
        }
        let name = Path::new(&v.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| v.path.clone());
        let extra = serde_json::json!({ "source": "rtsp-recording", "path": v.path }).to_string();
        let inserted = conn.execute(
            "INSERT INTO flight_videos (flight_id, rel_path, duration_ms, codec, width, height, frame_count, file_size,
                                        fingerprint, sync_a_video_ms, sync_a_flight_ms, sync_b_video_ms, sync_b_flight_ms, extra_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?3, ?11, ?12)",
            params![
                flight_id,
                name,
                v.duration_ms,
                v.codec,
                v.width,
                v.height,
                v.frames as i64,
                v.size as i64,
                v.uid,
                v.start_unix_ms - start,
                v.end_unix_ms - start,
                extra,
            ],
        );
        match inserted {
            Ok(_) => log::info!("[video] recording {name} attached to flight {flight_id}"),
            Err(e) => {
                log::warn!("[video] recording {name} not attached to flight {flight_id}: {e}");
                keep.push(v);
            }
        }
    }
    write_waiting(db_file, &keep);
}

/// The flight may already be in the database when its recording ends (auto-save at disarm): offer the
/// waiting recordings to the newest flights.
pub fn attach_to_saved_flights(conn: &Connection, db_file: &Path) {
    if read_waiting(db_file).is_empty() {
        return;
    }
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM flights ORDER BY id DESC LIMIT 10")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .unwrap_or_default();
    for id in ids {
        if let Ok(Some(flight)) = super::db::get_flight(conn, id) {
            attach_waiting(conn, db_file, id, &flight);
        }
    }
}

/// A flight's video as the logbook shows it.
#[derive(Debug, Serialize)]
pub struct FlightVideo {
    pub id: i64,
    pub name: String,
    /// Where the file was written; `None` for a row without a known location.
    pub path: Option<String>,
    pub exists: bool,
    pub duration_ms: Option<i64>,
    pub codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_size: Option<i64>,
}

pub fn list_for_flight(conn: &Connection, flight_id: i64) -> rusqlite::Result<Vec<FlightVideo>> {
    let mut stmt = conn.prepare(
        "SELECT id, rel_path, duration_ms, codec, width, height, file_size, extra_json
         FROM flight_videos WHERE flight_id = ?1 ORDER BY sync_a_flight_ms, id",
    )?;
    let rows = stmt.query_map(params![flight_id], |r| {
        let extra: Option<String> = r.get(7)?;
        let path = extra
            .and_then(|e| serde_json::from_str::<serde_json::Value>(&e).ok())
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(str::to_string));
        Ok(FlightVideo {
            id: r.get(0)?,
            name: r.get(1)?,
            exists: path.as_deref().is_some_and(|p| Path::new(p).is_file()),
            path,
            duration_ms: r.get(2)?,
            codec: r.get(3)?,
            width: r.get(4)?,
            height: r.get(5)?,
            file_size: r.get(6)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flightlog::db;

    fn video(name: &str, start: i64, end: i64) -> RecordedVideo {
        RecordedVideo {
            path: format!("/videos/{name}.mkv"),
            uid: "ab".repeat(16),
            start_unix_ms: start,
            end_unix_ms: end,
            duration_ms: end - start,
            codec: "h264".into(),
            width: 1920,
            height: 1080,
            frames: 100,
            size: 1234,
        }
    }

    #[test]
    fn a_committed_flight_takes_the_recordings_that_overlap_it() {
        let dir = std::env::temp_dir().join(format!("kite-vlink-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_file = dir.join("flights.db");
        let conn = db::open_database(&db_file).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let start = chrono::DateTime::from_timestamp_millis(now - 600_000).unwrap();
        let flight = Flight {
            id: 0,
            start_time: start,
            end_time: Some(start + chrono::Duration::seconds(300)),
            duration_sec: Some(300),
            source: "live".into(),
            craft_name: "Wing".into(),
            fc_variant: "ArduPlane".into(),
            fc_version: "4.6.0".into(),
            board_id: String::new(),
            platform_type: 1,
            fc_uid: None,
            protocol: "MAVLink".into(),
            start_lat: None,
            start_lon: None,
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            max_alt_m: None,
            max_speed_ms: None,
            max_distance_m: None,
            total_distance_m: None,
            battery_used_mah: None,
            notes: None,
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            utc_offset_min: None,
            group_id: None,
            vehicle_id: None,
        };
        let flight_id = db::insert_flight(&conn, &flight).unwrap();
        let s = start.timestamp_millis();
        add_waiting(&db_file, &[video("before", s - 90_000, s - 60_000), video("in", s - 2_000, s + 120_000)]);
        add_waiting(&db_file, &[video("later", s + 400_000, s + 500_000)]);

        attach_waiting(&conn, &db_file, flight_id, &flight);
        let got = list_for_flight(&conn, flight_id).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].name.as_str(), got[0].path.as_deref()), ("in.mkv", Some("/videos/in.mkv")));
        let anchors: (i64, i64, i64, i64, String) = conn
            .query_row(
                "SELECT sync_a_video_ms, sync_a_flight_ms, sync_b_video_ms, sync_b_flight_ms, fingerprint FROM flight_videos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(anchors, (0, -2_000, 122_000, 120_000, "ab".repeat(16)));
        // The other two keep waiting.
        assert_eq!(read_waiting(&db_file).len(), 2);

        // A recording that ends after its flight was saved finds it too.
        add_waiting(&db_file, &[video("late", s + 200_000, s + 290_000)]);
        attach_to_saved_flights(&conn, &db_file);
        assert_eq!(list_for_flight(&conn, flight_id).unwrap().len(), 2);
        assert_eq!(read_waiting(&db_file).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
