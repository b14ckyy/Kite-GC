// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The recording FILES of flights: they travel beside a `.kflight` in the folder `<name>_video` (the file
//! holds only the `flight_videos` rows — a recording runs to gigabytes), go with a deleted flight, and the
//! ones no flight knows can be listed. A file is matched to its row by the segment UID, never by its name.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

use super::video_link::{self, Row};

/// Recording files and their total size; each further field belongs to one use.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct VideoSet {
    pub count: usize,
    pub bytes: u64,
    /// Export only: recordings the flights are linked to that are not below the video folder.
    pub missing: usize,
    /// Import only: recordings of the imported flights that were below the video folder already.
    pub present: usize,
    /// Delete and clean-up only: the files, relative to the video folder — shown in the question and
    /// handed back as what may go.
    pub names: Vec<String>,
}

pub fn sidecar_dir(kflight: &Path) -> PathBuf {
    let stem = kflight.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    kflight.with_file_name(format!("{stem}_video"))
}

fn rows(conn: &Connection, flight_ids: Option<&[i64]>) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare("SELECT id, rel_path, fingerprint, flight_id FROM flight_videos ORDER BY id")?;
    let wanted: Option<HashSet<i64>> = flight_ids.map(|ids| ids.iter().copied().collect());
    let all = stmt
        .query_map([], |r| Ok((Row { id: r.get(0)?, rel_path: r.get(1)?, uid: r.get(2)? }, r.get::<_, i64>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(all.into_iter().filter(|(_, flight)| wanted.as_ref().is_none_or(|w| w.contains(flight))).map(|(row, _)| row).collect())
}

/// The recordings of these flights (`None` = every flight) that are on disk below `root`, each once.
fn export_files(conn: &Connection, flight_ids: Option<&[i64]>, root: &Path) -> rusqlite::Result<(Vec<PathBuf>, usize)> {
    let rows = rows(conn, flight_ids)?;
    let at = video_link::resolve(conn, root, &rows);
    let mut files: Vec<PathBuf> = Vec::new();
    let mut missing = 0;
    for row in &rows {
        match at.get(&row.id) {
            Some(path) if !files.contains(path) => files.push(path.clone()),
            Some(_) => {}
            None => missing += 1,
        }
    }
    Ok((files, missing))
}

fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn export_set(conn: &Connection, flight_ids: Option<&[i64]>, root: &Path) -> rusqlite::Result<VideoSet> {
    let (files, missing) = export_files(conn, flight_ids, root)?;
    Ok(VideoSet { count: files.len(), bytes: files.iter().map(|p| size_of(p)).sum(), missing, ..Default::default() })
}

/// `name` in `dir`, or `name_2`, `name_3`… when that is taken.
fn free_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let as_path = Path::new(name);
    let stem = as_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = as_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| dir.join(format!("{stem}_{n}{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Written as `<name>.part` and renamed when complete: an interrupted copy never carries a `.mkv` name,
/// which would pass for the whole recording by its UID.
fn copy_file(from: &Path, to: &Path, progress: &mut dyn FnMut(u64)) -> Result<u64, String> {
    let mut part = to.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let mut run = || -> std::io::Result<u64> {
        let mut src = std::fs::File::open(from)?;
        let mut dst = std::fs::File::create(&part)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n])?;
            done += n as u64;
            progress(done);
        }
        dst.sync_all()?;
        drop(dst);
        std::fs::rename(&part, to)?;
        Ok(done)
    };
    run().map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("Copying {} failed: {e}", from.display())
    })
}

/// Copy the flights' recordings into the sidecar folder of `kflight`. `progress(done, total)` in bytes.
pub fn export_videos(
    conn: &Connection,
    flight_ids: Option<&[i64]>,
    root: &Path,
    kflight: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<VideoSet, String> {
    let (files, missing) = export_files(conn, flight_ids, root).map_err(|e| format!("Database error: {e}"))?;
    let total: u64 = files.iter().map(|p| size_of(p)).sum();
    if files.is_empty() {
        return Ok(VideoSet { missing, ..Default::default() });
    }
    let dir = sidecar_dir(kflight);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Creating {} failed: {e}", dir.display()))?;
    // A recording already in the folder (an export repeated onto the same file) is not written twice.
    let present: HashSet<String> = sidecar_files(&dir).into_iter().map(|(uid, _)| uid).collect();
    let mut before = 0u64;
    let mut set = VideoSet { missing, ..Default::default() };
    for file in &files {
        let size = size_of(file);
        let known = crate::video::record::segment_uid(file).is_some_and(|uid| present.contains(&uid));
        if !known {
            let Some(name) = file.file_name() else { continue };
            copy_file(file, &free_name(&dir, name), &mut |done| progress(before + done, total))?;
        }
        before += size;
        progress(before, total);
        set.count += 1;
        set.bytes += size;
    }
    Ok(set)
}

/// The recordings in a sidecar folder, by segment UID (top level only: Kite writes them flat).
fn sidecar_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mkv")))
        .filter_map(|p| crate::video::record::segment_uid(&p).map(|uid| (uid, p)))
        .collect();
    files.sort();
    files
}

/// Recordings beside `kflight` that a flight of this database is linked to and that are not below `root`.
fn import_files(conn: &Connection, kflight: &Path, root: &Path) -> rusqlite::Result<Vec<(String, PathBuf)>> {
    let offered = sidecar_files(&sidecar_dir(kflight));
    if offered.is_empty() {
        return Ok(offered);
    }
    let linked: Vec<Row> =
        rows(conn, None)?.into_iter().filter(|r| r.uid.as_ref().is_some_and(|u| offered.iter().any(|(o, _)| o == u))).collect();
    let at = video_link::resolve(conn, root, &linked);
    let wanted: HashSet<&String> = linked.iter().filter(|r| !at.contains_key(&r.id)).filter_map(|r| r.uid.as_ref()).collect();
    let mut seen = HashSet::new();
    Ok(offered.into_iter().filter(|(uid, _)| wanted.contains(uid) && seen.insert(uid.clone())).collect())
}

/// `imported` = the flights the import just added: how many of their recordings were here already.
pub fn import_set(conn: &Connection, kflight: &Path, root: &Path, imported: &[i64]) -> rusqlite::Result<VideoSet> {
    let files = import_files(conn, kflight, root)?;
    let own = rows(conn, Some(imported))?;
    let present = if own.is_empty() { 0 } else { video_link::resolve(conn, root, &own).len() };
    Ok(VideoSet { count: files.len(), bytes: files.iter().map(|(_, p)| size_of(p)).sum(), present, ..Default::default() })
}

/// The files only these flights are linked to — what may go when the flights are deleted.
pub fn files_of_only(conn: &Connection, flight_ids: &[i64], root: &Path) -> rusqlite::Result<Vec<PathBuf>> {
    let own = rows(conn, Some(flight_ids))?;
    let own_ids: HashSet<i64> = own.iter().map(|r| r.id).collect();
    let shared: HashSet<String> = rows(conn, None)?.into_iter().filter(|r| !own_ids.contains(&r.id)).filter_map(|r| r.uid).collect();
    let at = video_link::resolve(conn, root, &own);
    let mut files: Vec<PathBuf> = Vec::new();
    for row in own.iter().filter(|r| !r.uid.as_ref().is_some_and(|u| shared.contains(u))) {
        if let Some(path) = at.get(&row.id).filter(|p| !files.contains(p)) {
            files.push(path.clone());
        }
    }
    Ok(files)
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/")
}

pub fn set_of(root: &Path, files: &[PathBuf]) -> VideoSet {
    VideoSet {
        count: files.len(),
        bytes: files.iter().map(|p| size_of(p)).sum(),
        names: files.iter().map(|p| relative(root, p)).collect(),
        ..Default::default()
    }
}

/// Kite's recordings below `root` that no flight of this database is linked to. `keep` = segment UIDs
/// that are spoken for (waiting for their flight), `busy` = the file being written right now. With
/// `only`, just those of them (as `relative` names): what the user was shown, nothing found since.
pub fn orphans(
    conn: &Connection,
    root: &Path,
    keep: &HashSet<String>,
    busy: Option<&Path>,
    only: Option<&[String]>,
) -> rusqlite::Result<Vec<PathBuf>> {
    let linked: HashSet<String> = rows(conn, None)?.into_iter().filter_map(|r| r.uid).collect();
    let mut files = Vec::new();
    video_link::recordings_below(root, &mut |uid, path| {
        let wanted = only.is_none_or(|names| names.contains(&relative(root, path)));
        if wanted && !linked.contains(uid) && !keep.contains(uid) && busy != Some(path) {
            files.push(path.to_path_buf());
        }
    });
    files.sort();
    Ok(files)
}

/// Copy the recordings beside `kflight` into the video folder and point their flights at them.
pub fn import_videos(
    conn: &Connection,
    kflight: &Path,
    root: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<VideoSet, String> {
    let files = import_files(conn, kflight, root).map_err(|e| format!("Database error: {e}"))?;
    let total: u64 = files.iter().map(|(_, p)| size_of(p)).sum();
    if files.is_empty() {
        return Ok(VideoSet::default());
    }
    std::fs::create_dir_all(root).map_err(|e| format!("Creating {} failed: {e}", root.display()))?;
    let mut before = 0u64;
    let mut set = VideoSet::default();
    for (uid, file) in &files {
        let Some(name) = file.file_name() else { continue };
        let dest = free_name(root, name);
        let size = copy_file(file, &dest, &mut |done| progress(before + done, total))?;
        before += size;
        progress(before, total);
        let rel = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        conn.execute("UPDATE flight_videos SET rel_path = ?1 WHERE fingerprint = ?2", params![rel, uid])
            .map_err(|e| format!("Database error: {e}"))?;
        video_link::forget_missing(root, uid);
        set.count += 1;
        set.bytes += size;
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flightlog::db;

    /// A minimal Matroska head as Kite writes it: EBML header, then a Segment whose Info carries the
    /// writing app and `uid`.
    fn mkv(path: &Path, uid: [u8; 16], filler: usize) {
        mkv_by(path, b"Kite GC", uid, filler)
    }

    fn mkv_by(path: &Path, app: &[u8], uid: [u8; 16], filler: usize) {
        let mut info = vec![0x57, 0x41, 0x80 | app.len() as u8];
        info.extend_from_slice(app);
        info.extend_from_slice(&[0x73, 0xA4, 0x90]);
        info.extend_from_slice(&uid);
        let mut segment = vec![0x15, 0x49, 0xA9, 0x66, 0x80 | info.len() as u8];
        segment.extend_from_slice(&info);
        let mut file = vec![0x1A, 0x45, 0xDF, 0xA3, 0x80, 0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        file.extend_from_slice(&segment);
        file.extend(std::iter::repeat_n(0u8, filler));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kite-vx-{tag}-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()))
    }

    fn hex(uid: [u8; 16]) -> String {
        uid.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn flight(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO flights (craft_name, start_time, source) VALUES (?1, '2026-11-20T10:00:00Z', 'live')",
            params![name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn link(conn: &Connection, flight: i64, rel: &str, uid: [u8; 16]) {
        conn.execute(
            "INSERT INTO flight_videos (flight_id, rel_path, fingerprint) VALUES (?1, ?2, ?3)",
            params![flight, rel, hex(uid)],
        )
        .unwrap();
    }

    #[test]
    fn the_test_file_carries_its_uid() {
        let dir = scratch("1");
        let file = dir.join("a.mkv");
        mkv(&file, [7; 16], 10);
        assert_eq!(crate::video::record::segment_uid(&file), Some(hex([7; 16])));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn export_copies_the_flights_recordings_and_counts_the_missing() {
        let dir = scratch("2");
        let root = dir.join("videos");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        let a = flight(&conn, "A");
        let b = flight(&conn, "B");
        mkv(&root.join("a.mkv"), [1; 16], 3000);
        mkv(&root.join("sub/b.mkv"), [2; 16], 500);
        link(&conn, a, "a.mkv", [1; 16]);
        link(&conn, a, "gone.mkv", [9; 16]);
        link(&conn, b, "sub/b.mkv", [2; 16]);

        let only_a = export_set(&conn, Some(&[a]), &root).unwrap();
        assert_eq!((only_a.count, only_a.missing), (1, 1));
        assert_eq!(export_set(&conn, None, &root).unwrap().count, 2);

        let kflight = dir.join("out/Flights_2.kflight");
        std::fs::create_dir_all(kflight.parent().unwrap()).unwrap();
        let mut last = (0, 0);
        let set = export_videos(&conn, None, &root, &kflight, &mut |done, total| last = (done, total)).unwrap();
        assert_eq!((set.count, set.missing), (2, 1));
        assert_eq!(last, (set.bytes, set.bytes));
        let side = dir.join("out/Flights_2_video");
        assert_eq!(std::fs::read(side.join("a.mkv")).unwrap(), std::fs::read(root.join("a.mkv")).unwrap());
        assert!(side.join("b.mkv").is_file());
        assert!(!side.join("a.mkv.part").exists());

        // A second export onto the same file adds nothing.
        export_videos(&conn, None, &root, &kflight, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read_dir(&side).unwrap().count(), 2);
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn import_takes_what_a_flight_is_linked_to_and_does_not_have() {
        let dir = scratch("3");
        let root = dir.join("videos");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        let a = flight(&conn, "A");
        link(&conn, a, "old/place/a.mkv", [1; 16]);
        link(&conn, a, "have.mkv", [2; 16]);
        mkv(&root.join("have.mkv"), [2; 16], 100);
        // Another recording already sits under the name the import would use.
        mkv(&root.join("a.mkv"), [5; 16], 100);

        let kflight = dir.join("in/x.kflight");
        let side = sidecar_dir(&kflight);
        mkv(&side.join("a.mkv"), [1; 16], 2000);
        mkv(&side.join("have.mkv"), [2; 16], 100);
        mkv(&side.join("stranger.mkv"), [3; 16], 100);

        let offered = import_set(&conn, &kflight, &root, &[a]).unwrap();
        assert_eq!((offered.count, offered.present), (1, 1));

        let set = import_videos(&conn, &kflight, &root, &mut |_, _| {}).unwrap();
        assert_eq!(set.count, 1);
        let rel: String =
            conn.query_row("SELECT rel_path FROM flight_videos WHERE fingerprint = ?1", params![hex([1; 16])], |r| r.get(0)).unwrap();
        assert_eq!(rel, "a_2.mkv");
        assert_eq!(crate::video::record::segment_uid(&root.join("a_2.mkv")), Some(hex([1; 16])));
        assert!(!root.join("stranger.mkv").exists());
        assert_eq!(import_set(&conn, &kflight, &root, &[]).unwrap().count, 0);
        let listed = video_link::list_for_flight(&conn, a, &root).unwrap();
        assert!(listed.iter().all(|v| v.exists));
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_deleted_flight_takes_only_the_recordings_nobody_else_has() {
        let dir = scratch("del");
        let root = dir.join("videos");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        let a = flight(&conn, "A");
        let b = flight(&conn, "B");
        mkv(&root.join("own.mkv"), [1; 16], 100);
        mkv(&root.join("shared.mkv"), [2; 16], 100);
        link(&conn, a, "own.mkv", [1; 16]);
        link(&conn, a, "shared.mkv", [2; 16]);
        link(&conn, b, "shared.mkv", [2; 16]);
        link(&conn, a, "gone.mkv", [3; 16]);
        assert_eq!(files_of_only(&conn, &[a], &root).unwrap(), vec![root.join("own.mkv")]);
        assert_eq!(files_of_only(&conn, &[a, b], &root).unwrap().len(), 2);
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn orphans_are_kites_own_recordings_no_flight_knows() {
        let dir = scratch("orph");
        let root = dir.join("videos");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        let a = flight(&conn, "A");
        mkv(&root.join("linked.mkv"), [1; 16], 100);
        mkv(&root.join("sub/orphan.mkv"), [2; 16], 100);
        mkv(&root.join("waiting.mkv"), [3; 16], 100);
        mkv(&root.join("running.mkv"), [4; 16], 100);
        std::fs::write(root.join("holiday.mkv"), b"not a matroska file").unwrap();
        mkv_by(&root.join("obs_capture.mkv"), b"Lavf62.13.102", [6; 16], 100);
        mkv(&root.join("later.mkv"), [7; 16], 100);
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        link(&conn, a, "elsewhere.mkv", [1; 16]);
        let keep: HashSet<String> = [hex([3; 16])].into();
        let busy = root.join("running.mkv");
        let found = orphans(&conn, &root, &keep, Some(&busy), None).unwrap();
        assert_eq!(found, vec![root.join("later.mkv"), root.join("sub").join("orphan.mkv")]);
        assert_eq!(set_of(&root, &found).names, vec!["later.mkv", "sub/orphan.mkv"]);
        // Only what was confirmed goes.
        let confirmed = vec!["sub/orphan.mkv".to_string(), "linked.mkv".to_string()];
        let found = orphans(&conn, &root, &keep, Some(&busy), Some(&confirmed)).unwrap();
        assert_eq!(found, vec![root.join("sub").join("orphan.mkv")]);
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_sidecar_folder_offers_nothing() {
        let dir = scratch("4");
        let conn = db::open_database(&dir.join("flights.db")).unwrap();
        assert_eq!(import_set(&conn, &dir.join("x.kflight"), &dir, &[]).unwrap(), VideoSet::default());
        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }
}
