// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Group-flight temp file `sessions/group_<id>.kgrp` (Dev-Docs active/GROUP_FLIGHTS.md §3.3): the
//! header of a running group flight and its member list, next to the members' `.ktmp` files.
//!
//! A small SQLite file with its own extension on purpose: the `.ktmp` orphan scan and the discard
//! sweeps match `*.ktmp` only (`commands/flightlog.rs` `flightlog_scan_orphan_sessions`,
//! `db::sweep_temp_sessions`), so a header-only group file is never opened, counted as empty or
//! deleted by them. The reference goes both ways — this file lists its members, and every member's
//! `session_meta.group_id` / `group_file` names the group (`db::update_session_meta_membership`) —
//! so recovery can rebuild the set when one side is damaged or a member joined after the last write
//! here. Member paths are stored as file names and resolved against this file's directory, so the
//! set survives a moved sessions folder.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

/// File extension of a group temp file.
pub const GROUP_FILE_EXT: &str = "kgrp";

/// Group states (`GroupFileHeader::state`).
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const GROUP_RUNNING: &str = "running";
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const GROUP_ENDING: &str = "ending";
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const GROUP_PENDING: &str = "pending";

/// Member states (`GroupFileMember::state`).
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const MEMBER_RECORDING: &str = "recording";
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const MEMBER_SUSPENDED: &str = "suspended";
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const MEMBER_ENDED: &str = "ended";
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub const MEMBER_INCOMPLETE: &str = "incomplete";

/// The group header — the future `flight_groups` row (§3.1) while the group is still temporary.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
#[derive(Debug, Clone, PartialEq)]
pub struct GroupFileHeader {
    /// Immutable group id (16 hex chars; hash of the anchor's start time + position, §3.1).
    pub id: String,
    /// Group start = the earliest arm among the member recordings (UTC).
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    /// Anchor model's position (and GPS MSL altitude) at its arm.
    pub start_lat: Option<f64>,
    pub start_lon: Option<f64>,
    pub start_alt_m: Option<f64>,
    /// GCS UTC offset in minutes (ADR-048).
    pub utc_offset_min: Option<i32>,
    /// Group notes (filled by the store prompt; kept here so a recovered set keeps them).
    pub notes: Option<String>,
    /// `running` | `ending` | `pending`.
    pub state: String,
}

/// One member recording of the group.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
#[derive(Debug, Clone, PartialEq)]
pub struct GroupFileMember {
    /// The member's `.ktmp` (a file name on disk, resolved against the `.kgrp`'s directory on read).
    pub temp_path: PathBuf,
    /// Vehicle key of the recorder that wrote it (`"L1:S2"`).
    pub vehicle_key: String,
    pub craft_name: String,
    pub fc_variant: String,
    pub fc_uid: Option<String>,
    pub joined_at: DateTime<Utc>,
    /// `recording` | `suspended` | `ended` | `incomplete`.
    pub state: String,
}

/// `<sessions_dir>/group_<id>.kgrp`.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn group_file_path(sessions_dir: &Path, group_id: &str) -> PathBuf {
    sessions_dir.join(format!("group_{group_id}.{GROUP_FILE_EXT}"))
}

/// The file-name part of a path, as stored in the group file and in `session_meta.group_file`.
pub fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// A file stored by name next to `of` (both live in the sessions dir).
pub fn sibling_path(of: &Path, file_name: &str) -> PathBuf {
    of.parent().map(|dir| dir.join(file_name)).unwrap_or_else(|| PathBuf::from(file_name))
}

/// Open (creating it and its parent dir) a group temp file. WAL + `synchronous = NORMAL`, like the
/// member `.ktmp` files, so `db::remove_temp_session` removes it with its sidecars.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn open_group_file(path: &Path) -> SqlResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS group_meta (
            id             INTEGER PRIMARY KEY CHECK (id = 1),
            group_id       TEXT NOT NULL,
            start_time     TEXT NOT NULL,
            end_time       TEXT,
            start_lat      REAL,
            start_lon      REAL,
            start_alt_m    REAL,
            utc_offset_min INTEGER,
            notes          TEXT,
            state          TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS group_members (
            member_file TEXT PRIMARY KEY,
            vehicle_key TEXT NOT NULL,
            craft_name  TEXT,
            fc_variant  TEXT,
            fc_uid      TEXT,
            joined_at   TEXT NOT NULL,
            state       TEXT NOT NULL
         );",
    )?;
    Ok(conn)
}

/// Write (replace) the group header.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn write_group_header(conn: &Connection, h: &GroupFileHeader) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO group_meta
            (id, group_id, start_time, end_time, start_lat, start_lon, start_alt_m, utc_offset_min,
             notes, state)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            h.id,
            h.start_time.to_rfc3339(),
            h.end_time.map(|t| t.to_rfc3339()),
            h.start_lat,
            h.start_lon,
            h.start_alt_m,
            h.utc_offset_min,
            h.notes,
            h.state,
        ],
    )?;
    Ok(())
}

/// The group header (None for a file without one — created but never written).
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn read_group_header(conn: &Connection) -> SqlResult<Option<GroupFileHeader>> {
    conn.query_row(
        "SELECT group_id, start_time, end_time, start_lat, start_lon, start_alt_m, utc_offset_min,
                notes, state
         FROM group_meta WHERE id = 1",
        [],
        |row| {
            let start: String = row.get(1)?;
            let end: Option<String> = row.get(2)?;
            Ok(GroupFileHeader {
                id: row.get(0)?,
                start_time: parse_utc(&start)?,
                end_time: end.as_deref().map(parse_utc).transpose()?,
                start_lat: row.get(3)?,
                start_lon: row.get(4)?,
                start_alt_m: row.get(5)?,
                utc_offset_min: row.get(6)?,
                notes: row.get(7)?,
                state: row.get(8)?,
            })
        },
    )
    .optional()
}

/// Add a member, or update it (keyed by its `.ktmp` file name).
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn upsert_group_member(conn: &Connection, m: &GroupFileMember) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO group_members
            (member_file, vehicle_key, craft_name, fc_variant, fc_uid, joined_at, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            file_name_of(&m.temp_path),
            m.vehicle_key,
            m.craft_name,
            m.fc_variant,
            m.fc_uid,
            m.joined_at.to_rfc3339(),
            m.state,
        ],
    )?;
    Ok(())
}

/// Change one member's state. Returns whether the member is listed.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn set_group_member_state(conn: &Connection, temp_path: &Path, state: &str) -> SqlResult<bool> {
    let n = conn.execute(
        "UPDATE group_members SET state = ?1 WHERE member_file = ?2",
        params![state, file_name_of(temp_path)],
    )?;
    Ok(n > 0)
}

/// The members listed in the group file `kgrp_path` (opened as `conn`), in join order, with their
/// `.ktmp` paths resolved next to it.
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn read_group_members(conn: &Connection, kgrp_path: &Path) -> SqlResult<Vec<GroupFileMember>> {
    let mut stmt = conn.prepare(
        "SELECT member_file, vehicle_key, craft_name, fc_variant, fc_uid, joined_at, state
         FROM group_members ORDER BY joined_at ASC, member_file ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let file: String = row.get(0)?;
        let joined: String = row.get(5)?;
        Ok(GroupFileMember {
            temp_path: sibling_path(kgrp_path, &file),
            vehicle_key: row.get(1)?,
            craft_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            fc_variant: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            fc_uid: row.get(4)?,
            joined_at: parse_utc(&joined)?,
            state: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Delete a group file with its WAL/SHM sidecars (best effort, like a `.ktmp`).
#[allow(dead_code)] // group coordinator / set recovery: GROUP_FLIGHTS.md step 5/7
pub fn remove_group_file(path: &Path) {
    super::db::remove_temp_session(path);
}

fn parse_utc(s: &str) -> SqlResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flightlog::db;

    fn utc(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_700_000_000 + s, 0).unwrap()
    }

    /// (g) A group file with two members round-trips, and each member's `session_meta` names it.
    #[test]
    fn group_file_round_trip_with_back_references() {
        let dir = std::env::temp_dir().join(format!("kite-kgrp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let kgrp = group_file_path(&dir, "0123456789abcdef");
        assert_eq!(kgrp.extension().and_then(|e| e.to_str()), Some("kgrp"));

        let header = GroupFileHeader {
            id: "0123456789abcdef".into(),
            start_time: utc(0),
            end_time: None,
            start_lat: Some(48.1),
            start_lon: Some(11.5),
            start_alt_m: Some(520.0),
            utc_offset_min: Some(120),
            notes: None,
            state: GROUP_RUNNING.into(),
        };
        let members = [
            ("active_2023-11-14_221320_L1-S1.ktmp", "L1:S1", "Wing", Some("UID1")),
            ("active_2023-11-14_221330_L1-S2.ktmp", "L1:S2", "ArduCopter #2", None),
        ];
        for (i, (file, key, _, _)) in members.iter().enumerate() {
            let conn = db::open_temp_session(&dir.join(file)).unwrap();
            db::write_session_meta(&conn, &utc(i as i64), "x", "ArduPilot", "", "", 1, None, "MAVLink", None, None)
                .unwrap();
            db::update_session_meta_membership(&conn, key, "member", Some(&header.id), Some(&file_name_of(&kgrp)))
                .unwrap();
        }
        {
            let conn = open_group_file(&kgrp).unwrap();
            write_group_header(&conn, &header).unwrap();
            for (i, (file, key, craft, uid)) in members.iter().enumerate() {
                upsert_group_member(
                    &conn,
                    &GroupFileMember {
                        temp_path: dir.join(file),
                        vehicle_key: key.to_string(),
                        craft_name: craft.to_string(),
                        fc_variant: "ArduPilot".into(),
                        fc_uid: uid.map(String::from),
                        joined_at: utc(i as i64 * 10),
                        state: MEMBER_RECORDING.into(),
                    },
                )
                .unwrap();
            }
            assert!(set_group_member_state(&conn, &dir.join(members[1].0), MEMBER_SUSPENDED).unwrap());
            assert!(!set_group_member_state(&conn, &dir.join("unknown.ktmp"), MEMBER_ENDED).unwrap());
        }

        let conn = open_group_file(&kgrp).unwrap();
        assert_eq!(read_group_header(&conn).unwrap(), Some(header.clone()));
        let read = read_group_members(&conn, &kgrp).unwrap();
        drop(conn);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].temp_path, dir.join(members[0].0));
        assert_eq!(read[0].fc_uid.as_deref(), Some("UID1"));
        assert_eq!(read[0].state, MEMBER_RECORDING);
        assert_eq!(read[1].vehicle_key, "L1:S2");
        assert_eq!(read[1].craft_name, "ArduCopter #2");
        assert_eq!(read[1].state, MEMBER_SUSPENDED);
        // The other direction: every listed member names this group file and id.
        for m in &read {
            let meta = db::read_session_meta(&db::open_temp_session(&m.temp_path).unwrap()).unwrap().unwrap();
            assert_eq!(meta.group_id.as_deref(), Some(header.id.as_str()));
            assert_eq!(meta.role.as_deref(), Some("member"));
            assert_eq!(sibling_path(&m.temp_path, meta.group_file.as_deref().unwrap()), kgrp);
        }
        // The `.ktmp` sweep leaves the group file alone.
        assert_eq!(db::sweep_temp_sessions(&dir, &[]), 2);
        assert!(kgrp.exists());
        remove_group_file(&kgrp);
        assert!(!kgrp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
