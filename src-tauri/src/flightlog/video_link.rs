// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Finished recordings wait in a file next to the database until their flight is committed, then move
//! into `Flights/<flight>/` below the video folder. A flight knows its video by segment UID and a path
//! relative to that folder, never an absolute one; `resolve` finds a moved file by its UID.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

use super::types::Flight;
use crate::video::record::RecordedVideo;

const WAITING_FILE: &str = "video_recordings.json";
pub(super) const FLIGHTS_DIR: &str = "Flights";
/// A recording nobody claimed is forgotten after this long; its file stays where it is.
const WAITING_MAX_MS: i64 = 30 * 24 * 3600 * 1000;

/// The list is read, changed and written back from the command pool and from the telemetry thread.
static WAITING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// One entry per recording: the closed file replaces what was announced when it was opened.
pub fn add_waiting(db_file: &Path, videos: &[RecordedVideo]) {
    if videos.is_empty() {
        return;
    }
    let _lock = WAITING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read_waiting(db_file);
    list.retain(|known| !videos.iter().any(|v| v.uid == known.uid));
    list.extend(videos.iter().cloned());
    write_waiting(db_file, &list);
}

/// When this process started (Unix ms): a part announced before it and never closed is what a crash
/// left, one announced since is being written. Forced at start-up (`note_startup`).
static STARTED_MS: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| chrono::Utc::now().timestamp_millis());

pub fn note_startup() {
    std::sync::LazyLock::force(&STARTED_MS);
}

/// Give the unclosed parts of an earlier run their real end (`read`), drop the ones whose file is gone.
fn complete_unclosed(list: Vec<RecordedVideo>, started_ms: i64, read: &dyn Fn(&RecordedVideo) -> Option<RecordedVideo>) -> Vec<RecordedVideo> {
    list.into_iter()
        .filter_map(|v| {
            if v.end_unix_ms != 0 || v.start_unix_ms >= started_ms {
                return Some(v);
            }
            match read(&v) {
                Some(done) => {
                    log::warn!("[video] recording {} was never closed — {} s of it are on disk", v.path, done.duration_ms / 1000);
                    Some(done)
                }
                None if Path::new(&v.path).is_file() => Some(v),
                None => None,
            }
        })
        .collect()
}

/// `Wing_2026-11-20_10-00-00_part4.mkv` → `Wing_2026-11-20_10-00-00`.
fn recording_stem(name: &str) -> String {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    match stem.rfind("_part") {
        Some(i) if stem.len() > i + 5 && stem[i + 5..].bytes().all(|b| b.is_ascii_digit()) => stem[..i].to_string(),
        _ => stem,
    }
}

/// The flight's folder below `Flights/`: the one its recordings name already, else its first recording's
/// name, else `new_name`'s. A `rel_path` may come from an imported file: only a name that is one path
/// component on every system is taken from it.
pub(super) fn flight_folder(conn: &Connection, flight_id: i64, new_name: &str) -> String {
    let known: Vec<String> = conn
        .prepare("SELECT rel_path FROM flight_videos WHERE flight_id = ?1 ORDER BY sync_a_flight_ms, id")
        .and_then(|mut s| s.query_map(params![flight_id], |r| r.get(0))?.collect())
        .unwrap_or_default();
    let plain = |name: &str| {
        !name.is_empty()
            && name.trim_matches(['.', ' ']) == name
            && !name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
            && !name.chars().any(char::is_control)
    };
    let in_folder = known.iter().find_map(|rel| {
        let mut parts = rel.split(['/', '\\']);
        (parts.next() == Some(FLIGHTS_DIR)).then(|| parts.next()).flatten().filter(|_| parts.next().is_some()).filter(|f| plain(f)).map(str::to_string)
    });
    let stem = |name: &str| Some(recording_stem(name)).filter(|s| plain(s));
    in_folder.or_else(|| known.first().and_then(|n| stem(n))).or_else(|| stem(new_name)).unwrap_or_else(|| "Video".into())
}

/// `rel` counts from the file's own folder, which is the video folder for a waiting recording. `None` =
/// not moved (held open on Windows, read-only medium); its old path stays valid.
fn move_to_flight(file: &Path, folder: &str) -> Option<(PathBuf, String)> {
    let (base, name) = (file.parent()?, file.file_name()?);
    if !file.is_file() {
        return None;
    }
    let dir = base.join(FLIGHTS_DIR).join(folder);
    let moved = std::fs::create_dir_all(&dir).and_then(|_| {
        let dest = super::video_files::free_name(&dir, name);
        std::fs::rename(file, &dest).map(|_| dest)
    });
    match moved {
        Ok(dest) => {
            let rel = format!("{FLIGHTS_DIR}/{folder}/{}", dest.file_name()?.to_string_lossy());
            Some((dest, rel))
        }
        Err(e) => {
            log::warn!("[video] recording {} not moved into {}: {e}", file.display(), dir.display());
            let _ = std::fs::remove_dir(&dir);
            None
        }
    }
}

/// Attach every waiting recording whose middle lies in the flight: the pre-roll of a quick re-arm reaches
/// back into the flight before, which must not take it. The two sync anchors are the recording's first
/// and last picture on the flight's clock (ms from `start_time`).
pub fn attach_waiting(conn: &Connection, db_file: &Path, flight_id: i64, flight: &Flight) {
    attach_waiting_with(conn, db_file, flight_id, flight, *STARTED_MS, &crate::video::record::read_unfinished)
}

fn attach_waiting_with(
    conn: &Connection,
    db_file: &Path,
    flight_id: i64,
    flight: &Flight,
    started_ms: i64,
    read: &dyn Fn(&RecordedVideo) -> Option<RecordedVideo>,
) {
    let _lock = WAITING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let list = read_waiting(db_file);
    if list.is_empty() {
        return;
    }
    let list = complete_unclosed(list, started_ms, read);
    let start = flight.start_time.timestamp_millis();
    let end = flight.end_time.map(|t| t.timestamp_millis()).unwrap_or(start + flight.duration_sec.unwrap_or(0) * 1000);
    let now = chrono::Utc::now().timestamp_millis();
    let mut keep = Vec::new();
    let mut folder: Option<String> = None;
    for v in list {
        // Still open: being written, or unreadable for now.
        if v.end_unix_ms == 0 {
            if now - v.start_unix_ms < WAITING_MAX_MS {
                keep.push(v);
            }
            continue;
        }
        let middle = v.start_unix_ms + (v.end_unix_ms - v.start_unix_ms) / 2;
        if middle > end || middle < start {
            if now - v.end_unix_ms < WAITING_MAX_MS {
                keep.push(v);
            }
            continue;
        }
        let mut name = Path::new(&v.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| v.path.clone());
        let folder = folder.get_or_insert_with(|| flight_folder(conn, flight_id, &name));
        if let Some((_, rel)) = move_to_flight(Path::new(&v.path), folder) {
            name = rel;
        }
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
                // With the path it had: the next try then links the plain name, and the UID finds the file.
                keep.push(v);
            }
        }
    }
    write_waiting(db_file, &keep);
}

/// The flight may already be in the database when its recording ends (auto-save at disarm): offer the
/// waiting recordings to the newest flights.
pub fn attach_to_saved_flights(conn: &Connection, db_file: &Path) {
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

const SEARCH_DEPTH: usize = 6;
/// A recording that was not found is not searched again for this long: the details and the replay ask
/// one after the other, and the search reads the head of every file below the folder.
const MISSING_TTL: std::time::Duration = std::time::Duration::from_secs(20);
static MISSING: std::sync::Mutex<Vec<(PathBuf, String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

/// `None` when the list is there but unreadable: then nothing tells which recordings wait for a flight.
pub fn waiting_uids(db_file: &Path) -> Option<HashSet<String>> {
    let _guard = WAITING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(bytes) = std::fs::read(waiting_path(db_file)) else { return Some(HashSet::new()) };
    let list: Vec<RecordedVideo> = serde_json::from_slice(&bytes).ok()?;
    Some(list.into_iter().map(|v| v.uid).collect())
}

/// Every recording Kite wrote below `root`, as deep as a linked recording is searched.
pub(super) fn recordings_below(root: &Path, each: &mut dyn FnMut(&str, &Path)) {
    fn walk(dir: &Path, depth: usize, each: &mut dyn FnMut(&str, &Path)) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, each);
                }
            } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("mkv")) {
                if let Some(uid) = crate::video::record::kite_segment_uid(&path) {
                    each(&uid, &path);
                }
            }
        }
    }
    walk(root, SEARCH_DEPTH, each);
}

/// A recording that was just put below `root` is looked for again at once.
pub(super) fn forget_missing(root: &Path, uid: &str) {
    MISSING.lock().unwrap_or_else(|e| e.into_inner()).retain(|(dir, known, _)| !(dir == root && known == uid));
}

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

pub(super) struct Row {
    pub id: i64,
    pub rel_path: String,
    pub uid: Option<String>,
}

/// Where each video row's file is now. A file that is not at its `rel_path` (or is another recording
/// there) is searched by its UID in the whole video folder, and the row gets the path it was found at.
pub(super) fn resolve(conn: &Connection, root: &Path, rows: &[Row]) -> HashMap<i64, PathBuf> {
    let mut at: HashMap<i64, PathBuf> = HashMap::new();
    let mut missing: Vec<&Row> = Vec::new();
    for row in rows {
        // Piece by piece: a `rel_path` is written with `/`, and the path goes to the user as it is.
        let path = row.rel_path.split('/').fold(root.to_path_buf(), |p, part| p.join(part));
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
    let mut known = MISSING.lock().unwrap_or_else(|e| e.into_inner());
    known.retain(|(_, _, at)| at.elapsed() < MISSING_TTL);
    let mut wanted: HashSet<String> = missing
        .iter()
        .filter_map(|r| r.uid.clone())
        .filter(|u| !known.iter().any(|(dir, uid, _)| dir == root && uid == u))
        .collect();
    let mut found = HashMap::new();
    if !wanted.is_empty() {
        find_by_uid(root, &mut wanted, SEARCH_DEPTH, &mut found);
        known.extend(wanted.into_iter().map(|u| (root.to_path_buf(), u, std::time::Instant::now())));
    }
    drop(known);
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
            uid: format!("{:0<32.32}", name.bytes().map(|b| format!("{b:02x}")).collect::<String>()),
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
        assert_eq!(anchors, (0, -2_000, 122_000, 120_000, video(&videos, "in", 0, 0).uid));
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
    fn a_recording_a_crash_left_open_joins_the_flight_it_falls_into() {
        let dir = std::env::temp_dir().join(format!("kite-vcrash-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let videos = dir.join("videos");
        std::fs::create_dir_all(&videos).unwrap();
        let db_file = dir.join("flights.db");
        let conn = db::open_database(&db_file).unwrap();
        let start = chrono::Utc::now() - chrono::Duration::seconds(600);
        let t0 = start.timestamp_millis();
        let fl = flight(start);
        let flight_id = db::insert_flight(&conn, &fl).unwrap();

        // Announced at open by an earlier run, never closed; the file is there. A second one of this run
        // is still being written; a third lost its file.
        let open = |name: &str, at: i64| RecordedVideo { end_unix_ms: 0, duration_ms: 0, uid: name.repeat(16), ..video(&videos, name, at, at) };
        for name in ["aa", "cc"] {
            std::fs::write(videos.join(format!("{name}.mkv")), b"x").unwrap();
        }
        add_waiting(&db_file, &[open("aa", t0 + 10_000), open("cc", t0 + 200_000), open("dd", t0 + 20_000)]);
        // The closed file replaces its announcement instead of standing beside it.
        add_waiting(&db_file, &[open("dd", t0 + 20_000)]);
        assert_eq!(read_waiting(&db_file).len(), 3);

        let this_run_started = t0 + 100_000;
        let read = |v: &RecordedVideo| (v.uid == "aa".repeat(16)).then(|| RecordedVideo { end_unix_ms: v.start_unix_ms + 60_000, duration_ms: 60_000, ..v.clone() });
        attach_waiting_with(&conn, &db_file, flight_id, &fl, this_run_started, &read);

        let rows: Vec<(String, i64, i64, i64)> = conn
            .prepare("SELECT fingerprint, duration_ms, sync_a_flight_ms, sync_b_flight_ms FROM flight_videos")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![("aa".repeat(16), 60_000, 10_000, 70_000)]);
        // The one being written keeps waiting; the one without a file is forgotten.
        let left: Vec<String> = read_waiting(&db_file).into_iter().map(|v| v.uid).collect();
        assert_eq!(left, vec!["cc".repeat(16)]);
        assert_eq!(waiting_uids(&db_file).unwrap().len(), 1);
        drop(conn);
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

    #[test]
    fn attached_recordings_move_into_the_folder_of_their_flight() {
        let dir = std::env::temp_dir().join(format!("kite-vfold-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let videos = dir.join("videos");
        let db_file = dir.join("flights.db");
        std::fs::create_dir_all(&videos).unwrap();
        let conn = db::open_database(&db_file).unwrap();
        let start = chrono::DateTime::from_timestamp_millis(chrono::Utc::now().timestamp_millis() - 600_000).unwrap();
        let flight = flight(start);
        let flight_id = db::insert_flight(&conn, &flight).unwrap();
        let s = start.timestamp_millis();
        let mut first = video(&videos, "Wing_2026-11-20_10-00-00_part2", s + 1_000, s + 50_000);
        let mut other = video(&videos, "Wing_2026-11-20_10-05-00", s + 60_000, s + 90_000);
        let waits = video(&videos, "Wing_2026-11-20_11-00-00", s + 900_000, s + 950_000);
        (first.uid, other.uid) = ("11".repeat(16), "22".repeat(16));
        write_mkv(Path::new(&first.path), 0x11);
        write_mkv(Path::new(&other.path), 0x22);
        write_mkv(Path::new(&waits.path), 0x33);
        add_waiting(&db_file, &[first, waits.clone()]);
        attach_waiting(&conn, &db_file, flight_id, &flight);
        // A part that ends after the flight was saved joins the folder the flight has.
        add_waiting(&db_file, &[other]);
        attach_to_saved_flights(&conn, &db_file);

        let folder = videos.join(FLIGHTS_DIR).join("Wing_2026-11-20_10-00-00");
        assert!(folder.join("Wing_2026-11-20_10-00-00_part2.mkv").is_file() && folder.join("Wing_2026-11-20_10-05-00.mkv").is_file());
        assert!(Path::new(&waits.path).is_file(), "a recording without a flight stays in the video folder");
        let got = list_for_flight(&conn, flight_id, &videos).unwrap();
        assert!(got.len() == 2 && got.iter().all(|v| v.exists));
        let rel: String = conn.query_row("SELECT rel_path FROM flight_videos ORDER BY id LIMIT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(rel, "Flights/Wing_2026-11-20_10-00-00/Wing_2026-11-20_10-00-00_part2.mkv");

        // No row of an imported file leads out of `Flights/`, whichever way the name is found.
        for (rel, folder) in [
            ("Flights/../x.mkv", "x"),
            ("../../..", "new"),
            ("C:\\Users\\Public\\..", "new"),
            ("/", "new"),
            ("Flights/a:b/x.mkv", "x"),
            ("Flights/con. /x.mkv", "x"),
        ] {
            conn.execute("UPDATE flight_videos SET rel_path = ?1", params![rel]).unwrap();
            assert_eq!(flight_folder(&conn, flight_id, "new.mkv"), folder, "{rel}");
        }
        conn.execute("UPDATE flight_videos SET rel_path = '..'", []).unwrap();
        assert_eq!(flight_folder(&conn, flight_id, ".."), "Video");
        let _ = std::fs::remove_dir_all(dir);
    }
}
