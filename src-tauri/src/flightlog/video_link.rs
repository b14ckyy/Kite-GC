// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Recorded videos find their flight by time. A recording ends at disarm, the flight row may come much
//! later (End Flight dialog, a pending session resumed after a restart) or never (discarded), so finished
//! recordings wait in a small file next to the database until a flight that overlaps them is committed.
//!
//! A flight knows its video by the Matroska segment UID (`fingerprint`) and a path relative to the video
//! folder (`rel_path`) — never by an absolute path, so a logbook export carries nothing of this machine.
//! The file may be renamed or sorted into subfolders: `resolve` finds it by its UID and corrects the row.

use std::collections::{HashMap, HashSet};
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
        let extra = serde_json::json!({ "source": "rtsp-recording" }).to_string();
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

/// How deep below the video folder a recording is looked for.
const SEARCH_DEPTH: usize = 6;

/// The `.mkv` files below `dir` whose segment UID is one of `wanted`, by UID. Stops when all are found.
fn find_by_uid(dir: &Path, wanted: &mut HashSet<String>, depth: usize, found: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        if wanted.is_empty() {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("mkv")) {
            if let Some(uid) = crate::video::record::segment_uid(&path).filter(|u| wanted.contains(u)) {
                wanted.remove(&uid);
                found.insert(uid, path);
            }
        }
    }
    if depth > 0 {
        for sub in subdirs {
            find_by_uid(&sub, wanted, depth - 1, found);
        }
    }
}

struct Row {
    id: i64,
    rel_path: String,
    uid: Option<String>,
}

/// Where each video row's file is now. A file that is not at its `rel_path` (or is another recording
/// there) is searched by its UID in the whole video folder, and the row gets the path it was found at.
fn resolve(conn: &Connection, root: &Path, rows: &[Row]) -> HashMap<i64, PathBuf> {
    let mut at: HashMap<i64, PathBuf> = HashMap::new();
    let mut missing: Vec<&Row> = Vec::new();
    for row in rows {
        let path = root.join(&row.rel_path);
        let same = match row.uid.as_deref() {
            Some(uid) => path.is_file() && crate::video::record::segment_uid(&path).as_deref() == Some(uid),
            // A row without a UID (not one of our recordings) can only be taken by its path.
            None => path.is_file(),
        };
        if same {
            at.insert(row.id, path);
        } else if row.uid.is_some() {
            missing.push(row);
        }
    }
    if missing.is_empty() {
        return at;
    }
    let mut wanted: HashSet<String> = missing.iter().filter_map(|r| r.uid.clone()).collect();
    let mut found = HashMap::new();
    find_by_uid(root, &mut wanted, SEARCH_DEPTH, &mut found);
    for row in missing {
        let Some(path) = row.uid.as_ref().and_then(|u| found.get(u)) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        if let Err(e) = conn.execute("UPDATE flight_videos SET rel_path = ?1 WHERE id = ?2", params![rel, row.id]) {
            log::warn!("[video] recording found again as {rel}, but the flight's link was not updated: {e}");
        } else {
            log::info!("[video] recording {} found again as {rel}", row.rel_path);
        }
        at.insert(row.id, path.clone());
    }
    at
}

/// The flight's recordings that are on disk below `root`, with their sync anchors, in flight order.
pub fn replay_parts(conn: &Connection, flight_id: i64, root: &Path) -> rusqlite::Result<Vec<crate::video::record::replay::ReplayPart>> {
    let mut stmt = conn.prepare(
        "SELECT id, rel_path, fingerprint, sync_a_video_ms, sync_a_flight_ms, sync_b_video_ms, sync_b_flight_ms
         FROM flight_videos
         WHERE flight_id = ?1 AND sync_a_flight_ms IS NOT NULL AND sync_b_flight_ms IS NOT NULL
         ORDER BY sync_a_flight_ms, id",
    )?;
    let rows: Vec<(Row, i64, i64, i64, i64)> = stmt
        .query_map(params![flight_id], |r| {
            Ok((
                Row { id: r.get(0)?, rel_path: r.get(1)?, uid: r.get(2)? },
                r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                r.get(4)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let files: Vec<Row> = rows.iter().map(|(r, ..)| Row { id: r.id, rel_path: r.rel_path.clone(), uid: r.uid.clone() }).collect();
    let at = resolve(conn, root, &files);
    Ok(rows
        .into_iter()
        .filter_map(|(row, a_video_ms, a_flight_ms, b_video_ms, b_flight_ms)| {
            let path = at.get(&row.id)?.clone();
            Some(crate::video::record::replay::ReplayPart { path, a_video_ms, a_flight_ms, b_video_ms, b_flight_ms })
        })
        .collect())
}

/// A flight's video as the logbook shows it.
#[derive(Debug, Serialize)]
pub struct FlightVideo {
    pub id: i64,
    pub name: String,
    /// Where the file lies now; `None` when it is not below the video folder.
    pub path: Option<String>,
    pub exists: bool,
    pub duration_ms: Option<i64>,
    pub codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_size: Option<i64>,
}

pub fn list_for_flight(conn: &Connection, flight_id: i64, root: &Path) -> rusqlite::Result<Vec<FlightVideo>> {
    let mut stmt = conn.prepare(
        "SELECT id, rel_path, fingerprint, duration_ms, codec, width, height, file_size
         FROM flight_videos WHERE flight_id = ?1 ORDER BY sync_a_flight_ms, id",
    )?;
    let mut list: Vec<(Row, FlightVideo)> = stmt
        .query_map(params![flight_id], |r| {
            let row = Row { id: r.get(0)?, rel_path: r.get(1)?, uid: r.get(2)? };
            let video = FlightVideo {
                id: row.id,
                name: String::new(),
                path: None,
                exists: false,
                duration_ms: r.get(3)?,
                codec: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                file_size: r.get(7)?,
            };
            Ok((row, video))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let rows: Vec<Row> = list.iter().map(|(r, _)| Row { id: r.id, rel_path: r.rel_path.clone(), uid: r.uid.clone() }).collect();
    let at = resolve(conn, root, &rows);
    for (row, video) in &mut list {
        let path = at.get(&row.id);
        // The name as it is now: a renamed file shows its new name.
        let shown = path.map(|p| p.as_path()).unwrap_or(Path::new(&row.rel_path));
        video.name = shown.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| row.rel_path.clone());
        video.path = path.map(|p| p.to_string_lossy().to_string());
        video.exists = path.is_some();
    }
    Ok(list.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flightlog::db;

    fn video(dir: &Path, name: &str, start: i64, end: i64) -> RecordedVideo {
        RecordedVideo {
            path: dir.join(format!("{name}.mkv")).to_string_lossy().to_string(),
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

    /// The head of a Matroska file with segment UID `byte` x 16 — all `segment_uid` reads.
    fn write_mkv(path: &Path, byte: u8) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = vec![0x1A, 0x45, 0xDF, 0xA3, 0x80]; // EBML header, empty
        f.extend([0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]); // segment, size unknown
        f.extend([0xEC, 0x82, 0, 0]); // void
        f.extend([0x15, 0x49, 0xA9, 0x66, 0x93, 0x73, 0xA4, 0x90]); // info { segment uid
        f.extend([byte; 16]);
        std::fs::write(path, f).unwrap();
    }

    fn flight(start: chrono::DateTime<chrono::Utc>) -> Flight {
        Flight {
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
        }
    }

    #[test]
    fn a_committed_flight_takes_the_recordings_that_overlap_it() {
        let dir = std::env::temp_dir().join(format!("kite-vlink-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let videos = dir.join("videos");
        std::fs::create_dir_all(&videos).unwrap();
        let db_file = dir.join("flights.db");
        let conn = db::open_database(&db_file).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let start = chrono::DateTime::from_timestamp_millis(now - 600_000).unwrap();
        let flight = flight(start);
        let flight_id = db::insert_flight(&conn, &flight).unwrap();
        let s = start.timestamp_millis();
        add_waiting(&db_file, &[video(&videos, "before", s - 90_000, s - 60_000), video(&videos, "in", s - 2_000, s + 120_000)]);
        add_waiting(&db_file, &[video(&videos, "later", s + 400_000, s + 500_000)]);

        attach_waiting(&conn, &db_file, flight_id, &flight);
        let (rel, extra, anchors): (String, String, (i64, i64, i64, i64, String)) = conn
            .query_row(
                "SELECT rel_path, extra_json, sync_a_video_ms, sync_a_flight_ms, sync_b_video_ms, sync_b_flight_ms, fingerprint FROM flight_videos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, (r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))),
            )
            .unwrap();
        assert_eq!(rel, "in.mkv");
        // Nothing of this machine's folders goes into the row (and with it into an export).
        assert!(!extra.contains("videos") && !extra.contains("path"));
        assert_eq!(anchors, (0, -2_000, 122_000, 120_000, "ab".repeat(16)));
        // The other two keep waiting.
        assert_eq!(read_waiting(&db_file).len(), 2);

        // A recording that ends after its flight was saved finds it too.
        add_waiting(&db_file, &[video(&videos, "late", s + 200_000, s + 290_000)]);
        attach_to_saved_flights(&conn, &db_file);
        assert_eq!(list_for_flight(&conn, flight_id, &videos).unwrap().len(), 2);
        assert_eq!(read_waiting(&db_file).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_renamed_or_moved_recording_is_found_by_its_uid() {
        let dir = std::env::temp_dir().join(format!("kite-vuid-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let videos = dir.join("videos");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        let flight_id = db::insert_flight(&conn, &flight(chrono::Utc::now())).unwrap();
        let uid = "ab".repeat(16);
        conn.execute(
            "INSERT INTO flight_videos (flight_id, rel_path, fingerprint, sync_a_video_ms, sync_a_flight_ms, sync_b_video_ms, sync_b_flight_ms)
             VALUES (?1, 'Wing_1.mkv', ?2, 0, 0, 1000, 1000)",
            params![flight_id, uid],
        )
        .unwrap();
        let rel = || conn.query_row("SELECT rel_path FROM flight_videos", [], |r| r.get::<_, String>(0)).unwrap();

        // In place.
        write_mkv(&videos.join("Wing_1.mkv"), 0xab);
        let got = list_for_flight(&conn, flight_id, &videos).unwrap();
        assert!(got[0].exists && got[0].name == "Wing_1.mkv");

        // Sorted into a subfolder under a new name: found, and the row follows.
        std::fs::create_dir_all(videos.join("2026/Canberra")).unwrap();
        std::fs::rename(videos.join("Wing_1.mkv"), videos.join("2026/Canberra/first flight.mkv")).unwrap();
        let got = list_for_flight(&conn, flight_id, &videos).unwrap();
        assert!(got[0].exists && got[0].name == "first flight.mkv");
        assert_eq!(rel(), "2026/Canberra/first flight.mkv");
        assert_eq!(replay_parts(&conn, flight_id, &videos).unwrap().len(), 1);

        // Another recording under the old name is not taken for it.
        write_mkv(&videos.join("Wing_1.mkv"), 0x11);
        assert_eq!(list_for_flight(&conn, flight_id, &videos).unwrap()[0].name, "first flight.mkv");

        // Gone: the row stays, the file is reported missing under its last name.
        std::fs::remove_file(videos.join("2026/Canberra/first flight.mkv")).unwrap();
        let got = list_for_flight(&conn, flight_id, &videos).unwrap();
        assert!(!got[0].exists && got[0].name == "first flight.mkv" && got[0].path.is_none());
        assert!(replay_parts(&conn, flight_id, &videos).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
