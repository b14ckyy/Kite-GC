// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Recorder — detects arm/disarm transitions and records telemetry.
// Designed to be called from the scheduler thread with each decoded telemetry payload.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::db;
use super::group_store;
use super::msp_raw_logger::{MspRawLogger, MspRawSink};
use super::tlog_logger::TlogLogger;
use super::types::{BatteryRecord, Flight, FlightEvent, FlightLogSettings, TelemetryRecord};
use crate::msp::FcInfo;
use crate::vehicle_registry::emitter::VehicleEmitter;
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, BatteryInstanceData, GpsData, GpsStatsData,
    LinkStatsData, Misc2Data, NavStatusData, SensorStatusData, StatusData, WindData,
};

/// Bit 2 in arming_flags indicates ARMED state
const ARMED_FLAG: u32 = 0x04; // bit 2

/// INAV-style disarm→re-arm grace: a re-arm within this window continues the SAME log (an accidental
/// disarm in flight is one flight, not two). Beyond it, the previous flight is committed and a new
/// session starts. See ADR-041.
const REARM_GRACE: Duration = Duration::from_secs(5);

/// Timeline event kinds written to `session_events` (→ `flight_events` at commit).
pub const EVENT_ARM: &str = "arm";
pub const EVENT_DISARM: &str = "disarm";
/// The member's link went away while its group ran (the recorder was suspended, GROUP_FLIGHTS.md §3.2).
pub const EVENT_LINK_LOST: &str = "link_lost";
/// The same aircraft came back and its new recorder adopted the suspended session.
pub const EVENT_LINK_BACK: &str = "link_back";

/// `session_meta.role` values.
const ROLE_SINGLE: &str = "single";
const ROLE_MEMBER: &str = "member";

/// Lowest `GpsData::fix_type` that counts as a position fix. The protocol handlers normalise to
/// INAV's scale (0 = none, 1 = 2D, 2 = 3D, 3 = DGPS/RTK; `mavlink_proto/handler.rs` GPS_RAW_INT).
const MIN_FIX_TYPE: u8 = 1;

/// One armed stretch of a session on its relative timeline (`timestamp_ms`, ms since the session
/// start). Group members record while disarmed too, so their stats count these only (§3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmedSegment {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Pair the arm/disarm events of a session into armed segments. Other event kinds are ignored, as are
/// an arm while armed and a disarm while disarmed. A segment still open at the end closes at `end_ms`
/// (the last sample, or "now" for a live session).
pub fn armed_segments_from(events: &[FlightEvent], end_ms: i64) -> Vec<ArmedSegment> {
    let mut out = Vec::new();
    let mut open: Option<i64> = None;
    for e in events {
        match (e.kind.as_str(), open) {
            (EVENT_ARM, None) => open = Some(e.timestamp_ms),
            (EVENT_DISARM, Some(start)) => {
                out.push(ArmedSegment { start_ms: start, end_ms: e.timestamp_ms.max(start) });
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        out.push(ArmedSegment { start_ms: start, end_ms: end_ms.max(start) });
    }
    out
}

/// Flight statistics over the armed segments of a recorded track (GROUP_FLIGHTS.md §3.8): only rows
/// inside a segment count; distance is summed per segment (no jump across a disarmed gap), battery
/// use per segment (a battery swap resets `mah_drawn`). With one segment over the whole track this is
/// exactly the pre-group whole-span rule.
struct SegmentStats {
    duration_sec: i64,
    max_alt: f64,
    max_speed: f64,
    max_distance: f64,
    total_distance: f64,
    battery_used: Option<u32>,
}

fn stats_over_segments(
    rows: &[TelemetryRecord],
    segments: &[ArmedSegment],
    start_lat: Option<f64>,
    start_lon: Option<f64>,
) -> SegmentStats {
    let mut s = SegmentStats {
        duration_sec: segments.iter().map(|g| g.end_ms - g.start_ms).sum::<i64>().max(0) / 1000,
        max_alt: 0.0,
        max_speed: 0.0,
        max_distance: 0.0,
        total_distance: 0.0,
        battery_used: None,
    };
    for seg in segments {
        let mut last: Option<(f64, f64)> = None;
        let mut first_mah: Option<u32> = None;
        let mut last_mah: Option<u32> = None;
        for r in rows.iter().filter(|r| r.timestamp_ms >= seg.start_ms && r.timestamp_ms <= seg.end_ms) {
            if let Some(a) = r.baro_alt_m.or(r.alt_m) {
                if a > s.max_alt {
                    s.max_alt = a;
                }
            }
            if let Some(v) = r.speed_ms {
                if v > s.max_speed {
                    s.max_speed = v;
                }
            }
            if let Some(m) = r.mah_drawn {
                first_mah.get_or_insert(m);
                last_mah = Some(m);
            }
            if let (Some(lat), Some(lon)) = (r.lat, r.lon) {
                if let Some((plat, plon)) = last {
                    s.total_distance += haversine_m(plat, plon, lat, lon);
                }
                if let (Some(slat), Some(slon)) = (start_lat, start_lon) {
                    let d = haversine_m(slat, slon, lat, lon);
                    if d > s.max_distance {
                        s.max_distance = d;
                    }
                }
                last = Some((lat, lon));
            }
        }
        if let (Some(a), Some(b)) = (first_mah, last_mah) {
            if b >= a {
                s.battery_used = Some(s.battery_used.unwrap_or(0) + (b - a));
            }
        }
    }
    s
}

/// A recorder's place in a group flight (set by the group coordinator, GROUP_FLIGHTS.md §3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMembership {
    pub group_id: String,
    /// The group's `.kgrp` (`group_store`).
    pub kgrp_path: PathBuf,
}

/// Where a recorder's lifecycle events go, and the vehicle key that names its slots and temp files:
/// the vehicle's `VehicleEmitter` in the app, a capturing sink in the tests (no Tauri runtime).
pub trait RecorderSink: Send {
    fn key(&self) -> &str;
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), String>;
}

impl RecorderSink for VehicleEmitter {
    fn key(&self) -> &str {
        VehicleEmitter::key(self)
    }
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), String> {
        self.emit(event, payload).map_err(|e| e.to_string())
    }
}

/// Payload for the `flight-recording-committed` event (a pending session was auto-committed on a
/// grace-lapsed re-arm). The frontend links the captured mission + closes the dialog.
#[derive(serde::Serialize, Clone)]
struct FlightRecordingEvent {
    flight_id: i64,
}

/// Payload for `flight-recording-ended` — the disarm summary stats (no `flight_id` exists yet under
/// deferred commit, ADR-041; the dialog reads these directly).
#[derive(serde::Serialize, Clone)]
struct RecordingEndedEvent {
    duration_sec: i64,
    max_alt_m: f64,
    max_speed_ms: f64,
    max_distance_m: f64,
    total_distance_m: f64,
    battery_used_mah: Option<u32>,
}

/// Payload for `flight-recording-interrupted` — a disconnect while the UAV was still armed (ADR-042).
/// The frontend shows the recovery prompt (Discard / Save / Continue on Reconnect), not the
/// End-Flight dialog: the flight is not necessarily over (port change, switch to telemetry, …).
#[derive(serde::Serialize, Clone)]
struct RecordingInterruptedEvent {
    temp_path: String,
    craft_name: String,
    start_time: String,
    duration_sec: i64,
    sample_count: i64,
}

/// A finished live-recording session awaiting commit/discard (deferred commit, ADR-041). Held in
/// app-state so it survives a disconnect while the End-Flight dialog is open. Carries everything
/// both consumers need: the finalized `Flight` + temp/db paths (to commit), and the resume fields
/// (`start_mah`, `last_timestamp_ms`) so a re-arm within grace can continue the same `.ktmp`.
pub struct PendingSession {
    pub temp_path: PathBuf,
    pub db_path: PathBuf,
    pub flight: Flight,
    pub disarm_instant: Instant,
    pub start_mah: Option<u32>,
    pub last_timestamp_ms: i64,
    /// The armed stretches of the session (from its arm/disarm events).
    #[allow(dead_code)] // read by the group coordinator: GROUP_FLIGHTS.md step 5
    pub armed_segments: Vec<ArmedSegment>,
    /// Set when the session belongs to a group flight (then `flight.group_id` is set too).
    pub membership: Option<GroupMembership>,
}

impl PendingSession {
    /// Whether the aircraft was armed at any point of the session. A group member without armed time
    /// (a passive vehicle, or one that never took off) is listed unticked in the store prompt (§5.5).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn has_armed_time(&self) -> bool {
        !self.armed_segments.is_empty()
    }
}

/// Time source of the recorder: the monotonic clock that times a session (sample offsets, re-arm
/// grace) and the wall clock that stamps it. Injected so the recorder logic can run against a
/// settable fake in tests; production uses [`SystemClock`].
pub trait Clock: Send + Sync {
    /// Monotonic "now" (session timing, re-arm grace).
    fn instant(&self) -> Instant;
    /// Wall-clock "now" (UTC).
    fn utc(&self) -> DateTime<Utc>;
}

/// The real clock: `Instant::now()` / `Utc::now()`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Settable test clock: both readings start at a fixed base and move only when the test says so,
/// always by the same amount (monotonic and wall time never drift apart).
#[cfg(test)]
pub struct FakeClock {
    base_instant: Instant,
    base_utc: DateTime<Utc>,
    offset: Mutex<Duration>,
}

#[cfg(test)]
impl FakeClock {
    pub fn new(base_utc: DateTime<Utc>) -> Self {
        Self { base_instant: Instant::now(), base_utc, offset: Mutex::new(Duration::ZERO) }
    }

    /// Move both clocks forward by `d`.
    pub fn advance(&self, d: Duration) {
        *self.offset.lock().unwrap() += d;
    }

    /// Set both clocks to `base + since_base`.
    pub fn set(&self, since_base: Duration) {
        *self.offset.lock().unwrap() = since_base;
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn instant(&self) -> Instant {
        self.base_instant + *self.offset.lock().unwrap()
    }
    fn utc(&self) -> DateTime<Utc> {
        let offset = chrono::Duration::from_std(*self.offset.lock().unwrap()).unwrap_or_default();
        self.base_utc + offset
    }
}

/// Recording slots shared by every recorder (primary or secondary, any link) and the command layer.
/// Lives in app-state so a pending session survives a disconnect while its End-Flight dialog is open
/// (ADR-041). Keyed per vehicle, so two recorders can never overwrite each other's session and the
/// protected-path set covers every file a recorder owns.
///
/// Lock rule: each map is locked on its own and never while another of the four is held; a
/// recorder may lock them while holding its own recorder lock (recorder → slots, never reverse).
#[derive(Default)]
pub struct SessionSlots {
    /// Finished sessions awaiting Save/Discard (or the re-arm grace), keyed by the vehicle key of the
    /// recorder that finalized them (`"L1:S1"`).
    pending: Mutex<HashMap<String, PendingSession>>,
    /// Sessions to continue on reconnect (ADR-042), keyed by the vehicle key they were recorded under
    /// (pending → continue) or by the temp path (a recovered orphan, or a Continue issued without a
    /// vehicle id). A reconnect gets a new vehicle key, so the next primary recorder claims one by
    /// identity instead (see `take_resume_for`).
    resume: Mutex<HashMap<String, PendingSession>>,
    /// Every temp `.ktmp` a recorder is writing right now, plus a session taken out of its slot while
    /// it is being committed or reopened (live-path registry).
    live: Mutex<HashSet<PathBuf>>,
    /// Vehicle keys whose recorder is connected (attached on creation, detached on teardown). A
    /// pending session whose vehicle is no longer attached came from a closed connection; the same
    /// aircraft's next recorder claims it on arm (see `take_detached_pending_for`).
    attached: Mutex<HashSet<String>>,
    /// Finalized group-member sessions awaiting the group's store prompt (`finalize_member`), keyed by
    /// temp path — one vehicle can have members in two groups (a new group flies while the previous
    /// prompt is still open). Never touched by the single-flight pending/grace logic.
    group_pending: Mutex<HashMap<PathBuf, PendingSession>>,
    /// Group-member sessions whose link went away while their group ran, keyed by temp path. The same
    /// aircraft's next recorder adopts one on its first status (`take_suspended_for`); the coordinator
    /// finalizes the rest when the group ends (`take_suspended_of_group`).
    suspended: Mutex<HashMap<PathBuf, PendingSession>>,
}

/// Shared handle to the recording slots (see `state::AppState::sessions`).
pub type SessionSlotsHandle = Arc<SessionSlots>;

impl SessionSlots {
    /// Park a finalized session for `key`. A vehicle holds at most one pending session (the next arm
    /// resolves it first), so a replaced entry is a logic error worth a warning — its file is then no
    /// longer protected and is left to the orphan scan.
    pub fn park_pending(&self, key: &str, session: PendingSession) {
        if let Ok(mut map) = self.pending.lock() {
            if let Some(old) = map.insert(key.to_string(), session) {
                log::warn!(
                    "Pending session of {} replaced — {} is left to the orphan scan",
                    key,
                    old.temp_path.display()
                );
            }
        }
    }

    /// Take `key`'s pending session.
    pub fn take_pending_for(&self, key: &str) -> Option<PendingSession> {
        self.pending.lock().ok().and_then(|mut map| map.remove(key))
    }

    /// A pending session left by a connection that has closed since (its vehicle key is no longer
    /// attached) — the End-Flight dialog outlives a disconnect, and a reconnect gives the aircraft a new
    /// key. Claimed by the same FC (as `take_resume_for`). A recorder that is the only one connected
    /// also claims a single such session of another identity (e.g. the same aircraft back over passive
    /// telemetry) — the pre-multi-vehicle single-slot behaviour; with other vehicles connected that
    /// guess could hand one aircraft's flight to another, so it is not made.
    fn take_detached_pending_for(&self, own_key: &str, fc: &FcInfo) -> Option<PendingSession> {
        let attached: HashSet<String> = self.attached.lock().ok()?.clone();
        let alone = attached.iter().all(|k| k == own_key);
        let mut map = self.pending.lock().ok()?;
        let detached: Vec<String> = map.keys().filter(|k| !attached.contains(*k)).cloned().collect();
        let key = detached
            .iter()
            .find(|k| map.get(*k).is_some_and(|s| same_fc(&s.flight, fc)))
            .or_else(|| if alone && detached.len() == 1 { detached.first() } else { None })?
            .clone();
        map.remove(&key)
    }

    /// Mark `key`'s recorder as connected.
    fn attach(&self, key: &str) {
        if let Ok(mut set) = self.attached.lock() {
            set.insert(key.to_string());
        }
    }

    /// Mark `key`'s recorder as torn down (idempotent).
    fn detach(&self, key: &str) {
        if let Ok(mut set) = self.attached.lock() {
            set.remove(key);
        }
    }

    /// Take the pending session a command addresses: `Some(vehicle)` → exactly that vehicle's;
    /// `None` → the only one (the single-vehicle case). Several without a vehicle is an error rather
    /// than a guess — resolving the wrong aircraft's flight cannot be undone.
    pub fn take_pending(&self, vehicle_id: Option<&str>) -> Result<Option<PendingSession>, String> {
        let mut map = self.pending.lock().map_err(|_| "Pending-session lock poisoned".to_string())?;
        match vehicle_id {
            Some(key) => Ok(map.remove(key)),
            None => match map.len() {
                0 => Ok(None),
                1 => {
                    let key = map.keys().next().cloned().unwrap_or_default();
                    Ok(map.remove(&key))
                }
                n => Err(format!("{n} pending recording sessions — the vehicle must be named")),
            },
        }
    }

    /// Queue a session for continue-on-reconnect.
    pub fn put_resume(&self, key: String, session: PendingSession) -> Result<(), String> {
        let mut map = self.resume.lock().map_err(|_| "Resume-session lock poisoned".to_string())?;
        if let Some(old) = map.insert(key.clone(), session) {
            log::warn!(
                "Continue-on-reconnect session {} replaced — {} is left to the orphan scan",
                key,
                old.temp_path.display()
            );
        }
        Ok(())
    }

    /// The continue-on-reconnect session the recorder `own_key` of `fc` claims on its first status: the
    /// one recorded by the same FC (hardware id when both sides know it, else craft name + variant),
    /// else — only while this recorder is the only one connected — the oldest queued one. With a single
    /// queued session that is exactly the pre-multi-vehicle behaviour (the next connection takes it);
    /// with other vehicles connected that guess could hand one aircraft's flight to another, so it is
    /// not made (as in `take_detached_pending_for`).
    ///
    /// Residual: a recorder whose identity is not final on its first status — INAV over MAVLink starts
    /// with the heartbeat identity until the MSP-tunnel probe replaces it (`set_fc_info`) — matches by
    /// identity only by chance, so with several links connected its Continue stays queued (protected)
    /// until a later lone connection claims it, or the startup recovery offers the file after a restart.
    fn take_resume_for(&self, own_key: &str, fc: &FcInfo) -> Option<PendingSession> {
        let alone = self.attached.lock().ok()?.iter().all(|k| k == own_key);
        let mut map = self.resume.lock().ok()?;
        let key = map
            .iter()
            .find(|(_, s)| same_fc(&s.flight, fc))
            .or_else(|| if alone { map.iter().min_by_key(|(_, s)| s.flight.start_time) } else { None })
            .map(|(k, _)| k.clone())?;
        map.remove(&key)
    }

    /// Swap a recorder's registered live file (`old` → `new`; either may be `None`).
    fn set_live(&self, old: Option<&PathBuf>, new: Option<&PathBuf>) {
        if let Ok(mut set) = self.live.lock() {
            if let Some(p) = old {
                set.remove(p);
            }
            if let Some(p) = new {
                set.insert(p.clone());
            }
        }
    }

    /// Commit a session taken out of its slot (see `commit_pending_session`) with its temp file
    /// registered as live for the whole commit — otherwise it sits in no protected set while the main
    /// DB copies it, and a concurrent discard sweep (e.g. another vehicle's Discard) could delete it.
    pub fn commit_protected(&self, session: PendingSession) -> Result<i64, String> {
        let path = session.temp_path.clone();
        self.set_live(None, Some(&path));
        let result = commit_pending_session(session);
        self.set_live(Some(&path), None);
        result
    }

    /// Every temp file that belongs to a live workflow in this process — being written, pending
    /// Save/Discard, queued for continue-on-reconnect, a finalized or suspended group member. The
    /// orphan scan and the discard sweeps must never touch these.
    pub fn protected_paths(&self) -> Vec<PathBuf> {
        let mut keep: Vec<PathBuf> = Vec::new();
        if let Ok(set) = self.live.lock() {
            keep.extend(set.iter().cloned());
        }
        for map in [&self.pending, &self.resume] {
            if let Ok(map) = map.lock() {
                keep.extend(map.values().map(|s| s.temp_path.clone()));
            }
        }
        for map in [&self.group_pending, &self.suspended] {
            if let Ok(map) = map.lock() {
                keep.extend(map.keys().cloned());
            }
        }
        keep
    }

    /// Park a finalized group-member session for its group's store prompt.
    fn park_group_member(&self, session: PendingSession) {
        if let Ok(mut map) = self.group_pending.lock() {
            map.insert(session.temp_path.clone(), session);
        }
    }

    /// Take every finalized member session of `group_id` (the group coordinator, when the group's
    /// store prompt is resolved).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn take_group_members(&self, group_id: &str) -> Vec<PendingSession> {
        take_where(&self.group_pending, |s| s.flight.group_id.as_deref() == Some(group_id))
    }

    /// Keep a suspended group-member session (its link went away while the group ran).
    fn park_suspended(&self, session: PendingSession) {
        if let Ok(mut map) = self.suspended.lock() {
            map.insert(session.temp_path.clone(), session);
        }
    }

    /// The suspended member session recorded by the FC `fc` (same identity rule as `take_resume_for`,
    /// never a guess: the group has other aircraft by definition). The oldest one when several match.
    fn take_suspended_for(&self, fc: &FcInfo) -> Option<PendingSession> {
        let mut map = self.suspended.lock().ok()?;
        let key = map
            .iter()
            .filter(|(_, s)| same_fc(&s.flight, fc))
            .min_by_key(|(_, s)| s.flight.start_time)
            .map(|(k, _)| k.clone())?;
        map.remove(&key)
    }

    /// Take every suspended member session of `group_id` (the coordinator finalizes them when the
    /// group ends — their aircraft did not come back).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn take_suspended_of_group(&self, group_id: &str) -> Vec<PendingSession> {
        take_where(&self.suspended, |s| s.flight.group_id.as_deref() == Some(group_id))
    }
}

/// Remove and return every session of `map` matching `pred`, oldest first.
fn take_where(
    map: &Mutex<HashMap<PathBuf, PendingSession>>,
    pred: impl Fn(&PendingSession) -> bool,
) -> Vec<PendingSession> {
    let Ok(mut map) = map.lock() else { return Vec::new() };
    let keys: Vec<PathBuf> = map.iter().filter(|(_, s)| pred(s)).map(|(k, _)| k.clone()).collect();
    let mut out: Vec<PendingSession> = keys.iter().filter_map(|k| map.remove(k)).collect();
    out.sort_by_key(|s| s.flight.start_time);
    out
}

/// Whether a recorded flight came from the FC `fc`: the hardware id when both sides know it, else
/// craft name + variant.
fn same_fc(f: &Flight, fc: &FcInfo) -> bool {
    match (&f.fc_uid, &fc.fc_uid) {
        (Some(a), Some(b)) => a == b,
        _ => f.craft_name == fc.craft_name && f.fc_variant == fc.fc_variant,
    }
}

/// A vehicle key (`"L1:S2"`) as a file-name fragment (`"L1-S2"`).
fn file_key(vehicle_key: &str) -> String {
    vehicle_key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Commit a pending session into the main DB: insert the finalized flight, copy the temp
/// `telemetry_records`, remove the temp file, and spawn weather/geocode enrichment. Returns the new
/// flight id. Shared by the Save command and the recorder's grace-lapsed re-arm path.
pub fn commit_pending_session(session: PendingSession) -> Result<i64, String> {
    let conn = db::open_database(&session.db_path)
        .map_err(|e| format!("Failed to open flight DB for commit: {}", e))?;
    let flight_id = db::commit_session_to_main(&conn, &session.temp_path, &session.flight)
        .map_err(|e| format!("Failed to commit session: {}", e))?;
    db::remove_temp_session(&session.temp_path);

    if let (Some(lat), Some(lon)) = (session.flight.start_lat, session.flight.start_lon) {
        if is_valid_gps_coord(lat, lon) {
            let db_path = session.db_path.to_string_lossy().to_string();
            tauri::async_runtime::spawn(enrich_flight_async(flight_id, lat, lon, db_path));
        }
    }
    log::info!("Pending session committed as flight {}", flight_id);
    Ok(flight_id)
}

/// Discard a pending session — delete the temp `.ktmp` (and its WAL/SHM); nothing reaches the main DB.
pub fn discard_pending_session(session: PendingSession) {
    db::remove_temp_session(&session.temp_path);
    log::info!("Pending session discarded: {}", session.temp_path.display());
}

/// Reconstruct a `PendingSession` from an orphan temp `.ktmp` left by a crash/close (recovery,
/// ADR-042): read its `session_meta` + telemetry, recompute the flight stats, and finalize the
/// `Flight` (`end_time` = last sample). Returns the session + its telemetry sample count. The temp
/// file is left in place (the caller decides: commit / discard / continue-on-reconnect).
///
/// Stats count the armed segments only (GROUP_FLIGHTS.md §3.8), paired from the session's arm/disarm
/// events; a segment still open ends at the last sample. A session written before those events
/// existed (`session_meta.role` NULL) counts as armed over its whole span, as before.
pub fn summarize_temp_session(
    temp_path: PathBuf,
    db_path: PathBuf,
    clock: &dyn Clock,
) -> Result<(PendingSession, i64), String> {
    let conn =
        db::open_temp_session(&temp_path).map_err(|e| format!("Cannot open temp session: {}", e))?;
    let meta = db::read_session_meta(&conn)
        .map_err(|e| format!("Cannot read session_meta: {}", e))?
        .ok_or_else(|| "Temp session has no metadata".to_string())?;
    let rows = db::read_flight_track(&conn, 0, Some(&meta.fc_variant))
        .map_err(|e| format!("Cannot read temp telemetry: {}", e))?;
    if rows.is_empty() {
        return Err("Temp session has no telemetry".into());
    }
    let events = db::read_session_events(&conn).map_err(|e| format!("Cannot read session events: {}", e))?;

    let start_time = chrono::DateTime::parse_from_rfc3339(&meta.start_time)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| clock.utc());
    let start_lat = meta.start_lat.or_else(|| rows.iter().find_map(|r| r.lat));
    let start_lon = meta.start_lon.or_else(|| rows.iter().find_map(|r| r.lon));

    let start_mah = rows.iter().find_map(|r| r.mah_drawn);
    let last_timestamp_ms = rows.last().map(|r| r.timestamp_ms).unwrap_or(0);
    let armed_segments = if meta.role.is_none() {
        vec![ArmedSegment { start_ms: 0, end_ms: last_timestamp_ms.max(0) }]
    } else {
        armed_segments_from(&events, last_timestamp_ms)
    };
    let stats = stats_over_segments(&rows, &armed_segments, start_lat, start_lon);
    let membership = match (&meta.group_id, &meta.group_file) {
        (Some(group_id), Some(file)) => Some(GroupMembership {
            group_id: group_id.clone(),
            kgrp_path: group_store::sibling_path(&temp_path, file),
        }),
        _ => None,
    };

    let flight = Flight {
        id: 0,
        start_time,
        end_time: Some(start_time + chrono::Duration::milliseconds(last_timestamp_ms.max(0))),
        duration_sec: Some(stats.duration_sec),
        source: "live".into(),
        craft_name: meta.craft_name,
        fc_variant: meta.fc_variant,
        fc_version: meta.fc_version,
        board_id: meta.board_id,
        platform_type: meta.platform_type,
        fc_uid: meta.fc_uid,
        protocol: meta.protocol,
        start_lat,
        start_lon,
        location_name: None,
        weather_temp_c: None,
        weather_wind_ms: None,
        weather_wind_deg: None,
        weather_desc: None,
        max_alt_m: Some(stats.max_alt),
        max_speed_ms: Some(stats.max_speed),
        max_distance_m: Some(stats.max_distance),
        total_distance_m: Some(stats.total_distance),
        battery_used_mah: stats.battery_used,
        notes: None,
        linked_flight_id: None,
        pilot_name: None,
        pilot_id: None,
        battery_serial: None,
        // Live recording: the GCS sits at the flight location, so its own offset is the flight-local
        // offset (ADR-048).
        utc_offset_min: Some(super::timezone::local_offset_min_now()),
        group_id: meta.group_id,
    };
    let count = rows.len() as i64;
    Ok((
        PendingSession {
            temp_path,
            db_path,
            flight,
            disarm_instant: clock.instant(),
            start_mah,
            last_timestamp_ms,
            armed_segments,
            membership,
        },
        count,
    ))
}

#[inline]
fn is_valid_gps_coord(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// Async enrichment: fetch weather + geocode for a newly armed flight.
/// Runs in the background, never blocks the recorder thread.
async fn enrich_flight_async(flight_id: i64, lat: f64, lon: f64, db_path: String) {
    // Fetch weather and geocode (sequential — no tokio::join available)
    let weather = super::weather::fetch_weather(lat, lon).await;
    let location = super::geocode::reverse_geocode(lat, lon, "en").await;

    // Open a fresh connection for the update (recorder's conn is on another thread)
    let conn = match db::open_database(std::path::Path::new(&db_path)) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Enrichment: failed to open DB: {}", e);
            return;
        }
    };

    if let Some(w) = weather {
        if let Err(e) = conn.execute(
            "UPDATE flights SET weather_temp_c = ?1, weather_wind_ms = ?2, weather_wind_deg = ?3, weather_desc = ?4 WHERE id = ?5",
            rusqlite::params![w.temp_c, w.wind_ms, w.wind_deg, w.description, flight_id],
        ) {
            log::warn!("Enrichment: failed to write weather for flight {}: {}", flight_id, e);
        } else {
            log::info!("Enrichment: weather saved for flight {}", flight_id);
        }
    }

    if let Some(name) = location {
        if let Err(e) = conn.execute(
            "UPDATE flights SET location_name = ?1 WHERE id = ?2",
            rusqlite::params![name, flight_id],
        ) {
            log::warn!("Enrichment: failed to write location for flight {}: {}", flight_id, e);
        } else {
            log::info!("Enrichment: location '{}' saved for flight {}", name, flight_id);
        }
    }
}

/// Buffer size before flushing telemetry records to database
const FLUSH_THRESHOLD: usize = 50;

/// Snapshot of the latest telemetry values, accumulated across different poll groups
#[derive(Debug, Clone, Default)]
struct TelemetrySnapshot {
    // Attitude
    roll: Option<f64>,
    pitch: Option<f64>,
    yaw: Option<f64>,
    // GPS
    lat: Option<f64>,
    lon: Option<f64>,
    alt_gps: Option<f64>,
    speed: Option<f64>,
    heading: Option<f64>,
    fix_type: Option<u8>,
    num_sat: Option<u8>,
    // Altitude (baro)
    alt_baro: Option<f64>,
    vario: Option<f64>,
    // Analog
    voltage: Option<f64>,
    current: Option<f64>,
    mah_drawn: Option<u32>,
    rssi: Option<u16>,
    battery_percentage: Option<u8>,
    // RC link (unified link-stats pipeline) — LQ / SNR / raw uplink RSSI dBm, per protocol availability
    link_quality: Option<u8>,
    link_snr: Option<i8>,
    link_rssi_dbm: Option<i16>,
    // Airspeed
    airspeed: Option<f64>,
    // Throttle output (0–100%) from MSP2_INAV_MISC2 / VFR_HUD
    throttle: Option<f64>,
    // Latest per-instance batteries (ArduPilot/PX4 multi-monitor). Empty for single-battery setups.
    batteries: Vec<BatteryInstanceData>,
    // Wind (live ArduPilot WIND), stored as the NED velocity vector (direction the air moves TOWARD)
    // to match the imported VWN/VWE convention used on replay.
    wind_n_ms: Option<f64>,
    wind_e_ms: Option<f64>,
    // Status
    arming_flags: Option<u32>,
    cpu_load: Option<u16>,
    active_flight_mode_flags: Option<u32>,
    // Canonical flight mode (protocol-agnostic) — see docs/active/FLIGHT_MODE_UNIFIED.md
    mode_primary: Option<String>,
    mode_modifiers: Option<String>,
    // Navigation (MSP_NAV_STATUS) — mission context for replay
    active_wp_number: Option<i32>,
    nav_state: Option<i32>,
    // GPS quality (MSP_GPSSTATISTICS)
    gps_hdop: Option<f64>,
    gps_eph: Option<f64>,
    gps_epv: Option<f64>,
    // Packed per-sensor hardware health (MSP_SENSOR_STATUS), 2 bits/sensor
    hw_health_status: Option<i64>,
}

/// Active flight session
struct ActiveFlight {
    /// Per-session temp SQLite store (the durable in-flight buffer). `None` in raw-only mode
    /// (`db_enabled == false`), where the only sink is the raw text/tlog logger.
    temp_db: Option<Connection>,
    /// Path of the temp `.ktmp` file (kept so it can be committed + removed on disarm).
    temp_path: Option<std::path::PathBuf>,
    /// Wall-clock flight start (the finalized `flights.start_time` written at commit).
    start_time: chrono::DateTime<Utc>,
    start_instant: Instant,
    start_lat: Option<f64>,
    start_lon: Option<f64>,
    /// Accumulated telemetry records pending flush to the temp store
    buffer: Vec<TelemetryRecord>,
    /// Accumulated per-instance battery records pending flush (multi-battery; ArduPilot/PX4).
    bat_buffer: Vec<BatteryRecord>,
    // Statistics tracking
    max_alt: f64,
    max_speed: f64,
    max_distance: f64,
    total_distance: f64,
    last_lat: Option<f64>,
    last_lon: Option<f64>,
    start_mah: Option<u32>,
    /// The session's timeline events so far (mirrors its `session_events`).
    events: Vec<FlightEvent>,
}

/// The flight recorder, shared between the scheduler and command layer.
pub struct FlightRecorder {
    settings: FlightLogSettings,
    fc_info: FcInfo,
    protocol: String,
    db_file_path: std::path::PathBuf,
    /// Base dir for raw logs (raw_logs/*.tlog | *.rawmsp) — separate from the DB folder.
    raw_log_dir: std::path::PathBuf,
    /// Shared MSP raw-log sink (ADR-049): the transport writes the raw serial bytes into it; the
    /// recorder owns its lifecycle (opens on arm / continuous, drops on disarm / disconnect). `None`
    /// inside the slot when not recording; on MAVLink it stays empty (that path uses `tlog_logger`).
    msp_raw_sink: MspRawSink,
    tlog_logger: Option<TlogLogger>,
    snapshot: TelemetrySnapshot,
    active_flight: Option<ActiveFlight>,
    was_armed: bool,
    /// Emits the flight-recording lifecycle events stamped with this recorder's vehicle
    /// (`vehicleId` / `linkId`); its key also names the vehicle's slots and temp files.
    emitter: Box<dyn RecorderSink>,
    /// Shared per-vehicle slots (app-state): this vehicle's pending session — also read on the next
    /// arm for the grace decision (ADR-041) —, the continue-on-reconnect queue consulted once on the
    /// first polled status of this connection (ADR-042), and the live-path registry.
    slots: SessionSlotsHandle,
    /// The temp path this recorder currently has registered as live in `slots`.
    published_path: Option<PathBuf>,
    /// Monotonic + wall clock (see `Clock`).
    clock: Arc<dyn Clock>,
    /// Whether the first polled status has been seen on this connection (the trustworthy point to
    /// evaluate the continue-on-reconnect decision — past any handshake residual flags).
    first_status_seen: bool,
    /// Group-flight membership (GROUP_FLIGHTS.md §3.2). While set, the session records continuously:
    /// a disarm only writes an event (no End-Flight, no re-arm grace), a teardown suspends the session
    /// for adoption, and `finalize_member` ends it with armed-segment stats.
    membership: Option<GroupMembership>,
}

/// Thread-safe handle to the flight recorder
pub type FlightRecorderHandle = Arc<Mutex<FlightRecorder>>;

impl FlightRecorder {
    /// Create a new recorder on the system clock. Returns None if logging is disabled.
    /// `protocol` should be "MSP" or "MAVLink".
    pub fn new(
        settings: FlightLogSettings,
        fc_info: FcInfo,
        protocol: &str,
        portable: bool,
        emitter: VehicleEmitter,
        slots: SessionSlotsHandle,
        msp_raw_sink: MspRawSink,
    ) -> Result<Self, String> {
        Self::with_clock(settings, fc_info, protocol, portable, emitter, slots, msp_raw_sink, Arc::new(SystemClock))
    }

    /// `new` with an explicit time source and event sink (tests: `FakeClock` and a capturing sink).
    #[allow(clippy::too_many_arguments)] // recorder construction context (settings + session metadata)
    pub fn with_clock(
        settings: FlightLogSettings,
        fc_info: FcInfo,
        protocol: &str,
        portable: bool,
        emitter: impl RecorderSink + 'static,
        slots: SessionSlotsHandle,
        msp_raw_sink: MspRawSink,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, String> {
        let db_path = db::resolve_db_path(&settings.db_path, portable);
        let raw_log_dir = db::resolve_raw_log_dir(&settings.raw_log_path, portable);
        log::info!("Flight log database: {}", db_path.display());
        log::info!("Raw log directory: {}", raw_log_dir.display());

        // Validate the flight DB is openable now (fail fast). The actual writes use their own
        // connections — the temp store on arm, and the main DB at commit (ADR-041).
        db::open_database(&db_path).map_err(|e| {
            format!("Failed to open flight log database: {}", e)
        })?;
        slots.attach(emitter.key());

        Ok(Self {
            settings,
            fc_info,
            protocol: protocol.to_string(),
            db_file_path: db_path,
            raw_log_dir,
            msp_raw_sink,
            tlog_logger: None,
            snapshot: TelemetrySnapshot::default(),
            active_flight: None,
            was_armed: false,
            emitter: Box::new(emitter),
            slots,
            published_path: None,
            clock,
            first_status_seen: false,
            membership: None,
        })
    }

    /// This recorder's vehicle key (`"L1:S1"`) — names its pending slot and its temp files.
    fn key(&self) -> &str {
        self.emitter.key()
    }

    /// Time elapsed since `t` on the recorder's monotonic clock (saturating, like `Instant::elapsed`).
    fn since(&self, t: Instant) -> Duration {
        self.clock.instant().saturating_duration_since(t)
    }

    /// Emit a lifecycle event (stamped with this vehicle's ids by the emitter).
    fn emit<S: serde::Serialize>(&self, event: &str, payload: S) {
        let result = serde_json::to_value(payload)
            .map_err(|e| e.to_string())
            .and_then(|value| self.emitter.emit_json(event, value));
        if let Err(e) = result {
            log::warn!("Failed to emit {}: {}", event, e);
        }
    }

    /// Publish (or clear) the active session's temp path in the live-path registry.
    fn publish_active_path(&mut self) {
        let path = self.active_flight.as_ref().and_then(|f| f.temp_path.clone());
        if path == self.published_path {
            return;
        }
        self.slots.set_live(self.published_path.as_ref(), path.as_ref());
        self.published_path = path;
    }

    /// Whether the latest snapshot holds a position fix (§5.6: no fix, no group recording).
    fn has_fix(&self) -> bool {
        let position = matches!(
            (self.snapshot.lat, self.snapshot.lon),
            (Some(lat), Some(lon)) if is_valid_gps_coord(lat, lon)
        );
        position && self.snapshot.fix_type.is_some_and(|f| f >= MIN_FIX_TYPE)
    }

    /// Append a timeline event to the active session (its `session_events` and the in-memory copy),
    /// stamped on the session's relative timeline and the wall clock. No-op without a session.
    fn record_event(&mut self, kind: &str) {
        let now = self.clock.instant();
        let wall_ms = self.clock.utc().timestamp_millis();
        let Some(flight) = self.active_flight.as_mut() else { return };
        let event = FlightEvent {
            id: 0,
            flight_id: 0,
            timestamp_ms: now.saturating_duration_since(flight.start_instant).as_millis() as i64,
            wall_ms: Some(wall_ms),
            kind: kind.to_string(),
            code: None,
            text: None,
            source: Some("live".into()),
        };
        if let Some(conn) = flight.temp_db.as_ref() {
            if let Err(e) = db::insert_session_event(conn, &event) {
                log::warn!("Failed to write the {} event to the temp session: {}", kind, e);
            }
        }
        flight.events.push(event);
    }

    /// Write this recorder's key and group membership into the active session's `session_meta`.
    fn write_membership_meta(&self) {
        let Some(conn) = self.active_flight.as_ref().and_then(|f| f.temp_db.as_ref()) else { return };
        let (role, group_id, group_file) = match &self.membership {
            Some(m) => (ROLE_MEMBER, Some(m.group_id.as_str()), Some(group_store::file_name_of(&m.kgrp_path))),
            None => (ROLE_SINGLE, None, None),
        };
        if let Err(e) = db::update_session_meta_membership(conn, self.key(), role, group_id, group_file.as_deref()) {
            log::warn!("Failed to write the session membership: {}", e);
        }
    }

    /// Group coordinator hook: this recorder is a member of group `group_id` (its `.kgrp` at
    /// `kgrp_path`) from now on — the running session (if any) records continuously and names the group
    /// in its `session_meta`; a session started later does so from its start.
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn enter_member_mode(&mut self, group_id: &str, kgrp_path: &Path) {
        log::info!("{} joins group flight {}", self.key(), group_id);
        self.membership = Some(GroupMembership { group_id: group_id.to_string(), kgrp_path: kgrp_path.to_path_buf() });
        self.write_membership_meta();
    }

    /// Group coordinator hook: back to single-flight rules (the running session, if any, continues as
    /// a single flight and is finalized by its next disarm).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn leave_member_mode(&mut self) {
        if let Some(m) = self.membership.take() {
            log::info!("{} leaves group flight {}", self.key(), m.group_id);
            self.write_membership_meta();
        }
    }

    /// Group coordinator hook (§5.5): the group's first arm with a fix happened — start recording this
    /// connected vehicle although it is not armed. Needs a position fix (§5.6); returns whether a
    /// session is running afterwards (true as well when one already was). The session has no armed
    /// time until the vehicle's own arm. Emits nothing (group members never announce their sessions).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn start_recording_unarmed(&mut self) -> bool {
        if self.active_flight.is_some() {
            return true;
        }
        if !self.has_fix() {
            log::warn!("{}: no GPS fix — not recorded with the group (group recording needs a position)", self.key());
            return false;
        }
        log::info!("{}: starting the group recording (vehicle not armed)", self.key());
        self.start_fresh_session(false);
        if self.was_armed {
            self.record_event(EVENT_ARM);
        }
        self.active_flight.is_some()
    }

    /// Group coordinator hook: the group ended (last disarm + grace) — finalize this member's session
    /// with armed-segment stats and park it for the group's store prompt (`SessionSlots::
    /// take_group_members`). Leaves member mode. Returns whether a session was parked.
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn finalize_member(&mut self) -> bool {
        if self.membership.is_none() {
            log::warn!("{}: finalize_member outside a group flight — ignored", self.key());
            return false;
        }
        let parked = match self.take_active_as_pending() {
            Some((session, count)) => {
                log::info!(
                    "Group member {} finalized: {}s armed, {} samples",
                    self.key(), session.flight.duration_sec.unwrap_or(0), count,
                );
                self.slots.park_group_member(session);
                true
            }
            None => false,
        };
        self.publish_active_path();
        self.membership = None;
        parked
    }

    /// The armed stretches of the running session; one still open ends now.
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn armed_segments(&self) -> Vec<ArmedSegment> {
        match &self.active_flight {
            Some(f) => armed_segments_from(&f.events, self.since(f.start_instant).as_millis() as i64),
            None => Vec::new(),
        }
    }

    /// Whether the running session has any armed time (§5.5: members without are listed unticked).
    #[allow(dead_code)] // group coordinator: GROUP_FLIGHTS.md step 5
    pub fn has_armed_time(&self) -> bool {
        !self.armed_segments().is_empty()
    }

    /// Update the protocol label (recorded in the flight metadata). Passive telemetry detects its
    /// sub-protocol from the stream after connect, so the handler refines the label (e.g.
    /// "Telemetry (SmartPort)") once locked — before a flight is created on arm.
    pub fn set_protocol(&mut self, protocol: &str) {
        self.protocol = protocol.to_string();
    }

    /// Start continuous raw logging immediately on connect.
    /// Called when `raw_always` is enabled. Opens a session-level raw/tlog file
    /// that records all data (including pre-arm) until disconnect.
    pub fn start_continuous_log(&mut self) {
        if !self.settings.raw_always {
            return;
        }
        let now = self.clock.utc();
        let log_dir = self.raw_log_dir.as_path();

        // Use session timestamp + "session" label, flight_id=0 (no DB flight yet)
        if self.protocol == "MAVLink" {
            match TlogLogger::new(log_dir, 0, &now) {
                Ok(logger) => {
                    log::info!("Continuous tlog session started");
                    self.tlog_logger = Some(logger);
                }
                Err(e) => log::warn!("Failed to create continuous tlog: {}", e),
            }
        } else {
            // MSP: open the shared raw-serial sink; the transport writes into it (ADR-049).
            self.open_msp_raw_log(0, &now);
            log::info!("Continuous MSP raw session started");
        }
    }

    /// Open the shared MSP raw-serial logger (ADR-049) into the sink the transport writes to. No-op on
    /// MAVLink (that path records via `tlog_logger`).
    fn open_msp_raw_log(&self, flight_id: i64, now: &chrono::DateTime<Utc>) {
        if self.protocol == "MAVLink" {
            return;
        }
        // Don't reopen if a logger is already running — in continuous mode the connect path opens it
        // BEFORE the handshake (so the handshake's identity frames land in the log, ADR-049); the
        // recorder then adopts that one instead of starting a fresh (handshake-less) file.
        if self.msp_raw_sink.lock().map(|g| g.is_some()).unwrap_or(false) {
            return;
        }
        match MspRawLogger::new(self.raw_log_dir.as_path(), flight_id, now) {
            Ok(logger) => {
                if let Ok(mut g) = self.msp_raw_sink.lock() {
                    *g = Some(logger);
                }
            }
            Err(e) => log::warn!("Failed to create MSP raw log: {}", e),
        }
    }

    /// Flush + clear the shared MSP raw logger (so the transport stops writing).
    fn close_msp_raw_log(&self) {
        if let Ok(mut g) = self.msp_raw_sink.lock() {
            if let Some(mut logger) = g.take() {
                logger.close();
            }
        }
    }

    /// Feed attitude data from the scheduler
    pub fn on_attitude(&mut self, data: &AttitudeData) {
        self.snapshot.roll = Some(data.roll);
        self.snapshot.pitch = Some(data.pitch);
        self.snapshot.yaw = Some(data.yaw);
        self.maybe_record_sample();
    }

    /// Feed GPS data from the scheduler
    pub fn on_gps(&mut self, data: &GpsData) {
        self.snapshot.lat = Some(data.lat);
        self.snapshot.lon = Some(data.lon);
        self.snapshot.alt_gps = Some(data.alt_msl);
        self.snapshot.speed = Some(data.ground_speed);
        // DB column `heading` = course over ground (GpsData.course); the FC fused heading is stored
        // separately in `yaw` (from on_attitude). Kept distinct for the wind/crab analysis.
        self.snapshot.heading = Some(data.course);
        self.snapshot.fix_type = Some(data.fix_type);
        self.snapshot.num_sat = Some(data.num_sat);
        self.maybe_record_sample();
    }

    /// Feed altitude data from the scheduler
    pub fn on_altitude(&mut self, data: &AltitudeData) {
        self.snapshot.alt_baro = Some(data.altitude);
        self.snapshot.vario = Some(data.vario);
    }

    /// Feed analog data from the scheduler
    pub fn on_analog(&mut self, data: &AnalogData) {
        self.snapshot.voltage = Some(data.voltage);
        self.snapshot.current = Some(data.current);
        self.snapshot.mah_drawn = Some(data.mah_drawn);
        self.snapshot.rssi = Some(data.rssi);
        self.snapshot.battery_percentage = if data.battery_percentage > 0 { Some(data.battery_percentage) } else { None };
    }

    /// Feed unified RC-link stats — only updates the fields a given frame actually carries, so a
    /// RSSI-only protocol doesn't wipe LQ/SNR seen from another (and vice versa).
    pub fn on_linkstats(&mut self, data: &LinkStatsData) {
        if data.lq.is_some() { self.snapshot.link_quality = data.lq; }
        if data.snr_db.is_some() { self.snapshot.link_snr = data.snr_db; }
        if data.rssi_dbm.is_some() { self.snapshot.link_rssi_dbm = data.rssi_dbm; }
    }

    /// Mark the connected vehicle as a QuadPlane (ArduPilot `Q_ENABLE`). A QuadPlane reports
    /// MAV_TYPE_FIXED_WING → recorded as platform_type 1 (Airplane); override it to 7 (VTOL) so
    /// replay / the flight-detail vehicle-type field show the correct type. Idempotent.
    pub fn set_quadplane(&mut self) {
        self.fc_info.platform_type = 7; // PLATFORM_VTOL (matches the frontend platform table)
    }

    /// Live platform-type override (UAV Info panel): applies to the flight being recorded — its temp
    /// session meta too, so a crash-recovered session keeps it — and to every later flight on this
    /// link. Session-only; the value lives nowhere but in the recorded flights.
    pub fn set_platform_type(&mut self, platform_type: u8) {
        self.fc_info.platform_type = platform_type;
        if let Some(conn) = self.active_flight.as_ref().and_then(|f| f.temp_db.as_ref()) {
            if let Err(e) = db::update_session_meta_platform_type(conn, platform_type) {
                log::warn!("Failed to update session_meta platform type: {}", e);
            }
        }
    }

    /// Replace the recorded FC identity (craft name, variant, version, board, platform, FC id) — for the
    /// flight being recorded (its temp session meta too) and every later flight on this link. Used when
    /// the MSP-over-MAVLink probe turns a MAVLink link into an INAV one after the recorder was created
    /// with the heartbeat identity. The protocol label stays (the link still records a .tlog).
    pub fn set_fc_info(&mut self, fc_info: FcInfo) {
        self.fc_info = fc_info;
        if let Some(conn) = self.active_flight.as_ref().and_then(|f| f.temp_db.as_ref()) {
            let i = &self.fc_info;
            if let Err(e) = db::update_session_meta_identity(
                conn,
                &i.craft_name,
                &i.fc_variant,
                &i.fc_version,
                &i.board_id,
                i.platform_type,
                i.fc_uid.as_deref(),
            ) {
                log::warn!("Failed to update session_meta identity: {}", e);
            }
        }
    }

    /// Feed airspeed data from the scheduler
    pub fn on_airspeed(&mut self, data: &AirspeedData) {
        self.snapshot.airspeed = Some(data.airspeed);
    }

    /// Feed throttle (MSP2_INAV_MISC2 / VFR_HUD). Only the throttle percent is recorded; the message's
    /// uptime/flight-time are not (the flight timer is derived from the recording itself).
    pub fn on_misc2(&mut self, data: &Misc2Data) {
        self.snapshot.throttle = Some(data.throttle_pct as f64);
    }

    /// Feed the per-instance battery list (ArduPilot/PX4 multi-monitor). Each sample writes one
    /// `battery_records` row per instance (see maybe_record_sample). Empty for single-battery setups.
    pub fn on_batteries(&mut self, data: &[BatteryInstanceData]) {
        self.snapshot.batteries = data.to_vec();
    }

    /// Feed wind data (MAVLink WIND / INAV MSP2_INAV_WIND). Stored as the NED velocity vector
    /// (direction the air moves TOWARD) to match the imported VWN/VWE convention used on replay.
    pub fn on_wind(&mut self, data: &WindData) {
        let toward = (data.direction_from_deg + 180.0).to_radians();
        self.snapshot.wind_n_ms = Some(data.speed_ms * toward.cos());
        self.snapshot.wind_e_ms = Some(data.speed_ms * toward.sin());
    }

    /// Feed navigation status (MSP_NAV_STATUS) — the FC's current target waypoint + nav state.
    /// Recorded so a live-flown mission shows active-WP tracking on replay (matching the live map).
    pub fn on_nav_status(&mut self, data: &NavStatusData) {
        self.snapshot.active_wp_number = Some(data.active_wp_number as i32);
        self.snapshot.nav_state = Some(data.nav_state as i32);
    }

    /// Feed GPS quality stats (MSP_GPSSTATISTICS) — HDOP/EPH/EPV carried in one message.
    pub fn on_gps_stats(&mut self, data: &GpsStatsData) {
        self.snapshot.gps_hdop = Some(data.hdop);
        self.snapshot.gps_eph = data.eph;
        self.snapshot.gps_epv = data.epv;
    }

    /// Feed per-sensor hardware health (MSP_SENSOR_STATUS), packed 2 bits/sensor into
    /// `hw_health_status` in the order documented in FLIGHTLOG_DATABASE.md (gyro, acc, mag, baro,
    /// gps, rangefinder, pitot). Values 0=NONE,1=OK,2=UNAVAILABLE,3=UNHEALTHY.
    pub fn on_sensor_status(&mut self, data: &SensorStatusData) {
        let packed = (data.gyro as i64 & 0x3)
            | ((data.acc as i64 & 0x3) << 2)
            | ((data.mag as i64 & 0x3) << 4)
            | ((data.baro as i64 & 0x3) << 6)
            | ((data.gps as i64 & 0x3) << 8)
            | ((data.rangefinder as i64 & 0x3) << 10)
            | ((data.pitot as i64 & 0x3) << 12);
        self.snapshot.hw_health_status = Some(packed);
    }

    /// Write a raw MAVLink frame to the tlog file (if active)
    pub fn write_raw_mavlink_frame(&mut self, raw_frame: &[u8]) {
        if let Some(ref mut logger) = self.tlog_logger {
            logger.write_frame(raw_frame);
        }
    }

    /// Feed the canonical flight mode (protocol-agnostic). Stored per telemetry row so replay reads
    /// it directly — no re-classification. See docs/active/FLIGHT_MODE_UNIFIED.md.
    pub fn on_flightmode(&mut self, fm: &crate::flightmode::FlightModeState) {
        self.snapshot.mode_primary = Some(fm.primary.clone());
        self.snapshot.mode_modifiers = if fm.modifiers.is_empty() {
            None
        } else {
            Some(fm.modifiers.join(","))
        };
    }

    /// Feed status data — this is where arm/disarm transitions are detected
    pub fn on_status(&mut self, data: &StatusData) {
        self.snapshot.arming_flags = Some(data.arming_flags);
        self.snapshot.cpu_load = Some(data.cpu_load);
        self.snapshot.active_flight_mode_flags = Some(data.flight_mode_flags);

        let is_armed = (data.arming_flags & ARMED_FLAG) != 0;

        // First polled status of this connection: settle any continue-on-reconnect session (ADR-042).
        // The poller's status is past any handshake residual flags, so it is the trustworthy point.
        // A suspended group member of the same aircraft comes first: it is adopted without a prompt.
        if !self.first_status_seen {
            self.first_status_seen = true;
            if let Some(p) = self.slots.take_suspended_for(&self.fc_info) {
                self.adopt_member(p, is_armed);
                self.was_armed = is_armed;
                return;
            }
            let resume = self.slots.take_resume_for(self.key(), &self.fc_info);
            if let Some(p) = resume {
                if is_armed {
                    log::info!("Continue-on-reconnect: armed on first poll — resuming the recovered session");
                    self.resume_session(p, false);
                } else {
                    log::info!("Continue-on-reconnect: disarmed on first poll — finalizing the recovered session");
                    self.stash_pending_and_emit_ended(p);
                }
                self.was_armed = is_armed;
                return;
            }
        }

        if is_armed && !self.was_armed {
            self.on_arm();
        } else if !is_armed && self.was_armed {
            self.on_disarm();
        }

        self.was_armed = is_armed;
    }

    /// Move a finalized session into this vehicle's pending entry and tell the frontend to show the
    /// End-Flight summary (Save/Discard). Used by `on_disarm` and the continue-on-reconnect
    /// disarmed-on-first-poll path.
    fn stash_pending_and_emit_ended(&self, session: PendingSession) {
        let ev = RecordingEndedEvent {
            duration_sec: session.flight.duration_sec.unwrap_or(0),
            max_alt_m: session.flight.max_alt_m.unwrap_or(0.0),
            max_speed_ms: session.flight.max_speed_ms.unwrap_or(0.0),
            max_distance_m: session.flight.max_distance_m.unwrap_or(0.0),
            total_distance_m: session.flight.total_distance_m.unwrap_or(0.0),
            battery_used_mah: session.flight.battery_used_mah,
        };
        self.slots.park_pending(self.key(), session);
        self.emit("flight-recording-ended", ev);
    }

    /// Called when an arm transition is detected. Resolves any pending session first (deferred
    /// commit + grace, ADR-041): a re-arm within the grace window continues the SAME log; beyond it
    /// the previous flight is auto-committed and a fresh session starts.
    fn on_arm(&mut self) {
        // Group member: the session runs on through disarms — an arm is only an event. A member
        // without a session (none was possible at the group's start) opens one now, fix permitting.
        if self.membership.is_some() {
            if self.active_flight.is_none() {
                if !self.has_fix() {
                    log::warn!("{}: armed without a GPS fix — not recorded with the group", self.key());
                    return;
                }
                self.start_fresh_session(false);
            }
            log::info!("ARM (group member {}) — recording continues", self.key());
            self.record_event(EVENT_ARM);
            return;
        }
        // This vehicle's own pending session, else the one this aircraft left on a connection that
        // has closed since.
        let pending = self
            .slots
            .take_pending_for(self.key())
            .or_else(|| self.slots.take_detached_pending_for(self.key(), &self.fc_info));
        if let Some(p) = pending {
            if self.since(p.disarm_instant) < REARM_GRACE {
                self.resume_session(p, true);
                return;
            }
            // Grace lapsed → auto-commit the previous flight, then start a fresh session.
            match self.slots.commit_protected(p) {
                Ok(flight_id) => {
                    self.emit("flight-recording-committed", FlightRecordingEvent { flight_id });
                }
                Err(e) => log::error!("Auto-commit on re-arm failed: {}", e),
            }
        }
        self.start_fresh_session(true);
        self.record_event(EVENT_ARM);
    }

    /// Reopen a parked session's `.ktmp` as the active flight, its relative timeline continuing at
    /// `offset_ms(conn)` from now. False when the file cannot be opened (logged; the caller starts a
    /// fresh session).
    fn reopen_session(&mut self, p: PendingSession, offset_ms: impl FnOnce(&Connection) -> i64) -> bool {
        // Taken out of its slot, the file is in no protected set until the flight is active again:
        // register it as live before reopening it.
        self.slots.set_live(self.published_path.as_ref(), Some(&p.temp_path));
        self.published_path = Some(p.temp_path.clone());
        let conn = match db::open_temp_session(&p.temp_path) {
            Ok(c) => c,
            Err(e) => {
                log::error!(
                    "Failed to reopen temp session {}: {} — starting a fresh one",
                    p.temp_path.display(), e,
                );
                self.slots.set_live(Some(&p.temp_path), None);
                self.published_path = None;
                return false;
            }
        };
        let mut events = db::read_session_events(&conn).unwrap_or_else(|e| {
            log::warn!("Cannot read the events of {}: {}", p.temp_path.display(), e);
            Vec::new()
        });
        // A session written before events were recorded began at its arm (see `summarize_temp_session`).
        let legacy = db::read_session_meta(&conn).ok().flatten().is_some_and(|m| m.role.is_none());
        if legacy && !events.iter().any(|e| e.kind == EVENT_ARM) {
            events.insert(0, FlightEvent {
                id: 0,
                flight_id: 0,
                timestamp_ms: 0,
                wall_ms: None,
                kind: EVENT_ARM.into(),
                code: None,
                text: None,
                source: None,
            });
        }
        let offset = offset_ms(&conn).max(0);
        let now = self.clock.instant();
        let start_instant = now.checked_sub(Duration::from_millis(offset as u64)).unwrap_or(now);
        self.active_flight = Some(ActiveFlight {
            temp_db: Some(conn),
            temp_path: Some(p.temp_path),
            start_time: p.flight.start_time,
            start_instant,
            start_lat: p.flight.start_lat,
            start_lon: p.flight.start_lon,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
            bat_buffer: Vec::new(),
            max_alt: p.flight.max_alt_m.unwrap_or(0.0),
            max_speed: p.flight.max_speed_ms.unwrap_or(0.0),
            max_distance: p.flight.max_distance_m.unwrap_or(0.0),
            total_distance: p.flight.total_distance_m.unwrap_or(0.0),
            last_lat: None,
            last_lon: None,
            start_mah: p.start_mah,
            events,
        });
        self.publish_active_path();
        true
    }

    /// Re-arm within the grace window — reopen the same `.ktmp` and continue the flight. The relative
    /// timeline (`timestamp_ms`) resumes at the last sample, so the disarmed gap is not in it; the
    /// wall-clock gap stays visible through each row's `wall_ms`. Called only while armed: `rearm` =
    /// an arm edge brought it here (recorded as an event; the continue-on-reconnect path stayed armed
    /// across the link loss). A fresh session that replaces an unreadable file starts armed.
    fn resume_session(&mut self, p: PendingSession, rearm: bool) {
        log::info!("Re-arm within grace — continuing the same recording");
        let last_timestamp_ms = p.last_timestamp_ms;
        if !self.reopen_session(p, |_| last_timestamp_ms) {
            self.start_fresh_session(true);
            self.record_event(EVENT_ARM);
            return;
        }
        if rearm {
            self.record_event(EVENT_ARM);
        }
        self.emit("flight-recording-resumed", ());
    }

    /// The same aircraft came back while its group ran: adopt its suspended member session (§3.2).
    /// Unlike the single-flight resume, the relative timeline keeps the **real** gap — it continues
    /// from the last point where `timestamp_ms` and `wall_ms` are both known, advanced by the wall
    /// time since — so a synchronised group replay stays aligned. Writes `link_back`, plus the arm or
    /// disarm that happened while the link was gone. No lifecycle event (members never emit them).
    fn adopt_member(&mut self, p: PendingSession, is_armed: bool) {
        log::info!("{}: adopting the suspended group-member session {}", self.key(), p.temp_path.display());
        self.membership = p.membership.clone();
        let fallback_ms = p.last_timestamp_ms;
        let now_wall = self.clock.utc().timestamp_millis();
        let reopened = self.reopen_session(p, |conn| match db::last_session_time_pair(conn) {
            Ok(Some((ts, wall))) => ts + (now_wall - wall).max(0),
            Ok(None) => fallback_ms, // no wall stamp in the file (pre-v20): the gap cannot be known
            Err(e) => {
                log::warn!("Cannot read the session's last wall-clock stamp: {} — the link gap is dropped", e);
                fallback_ms
            }
        });
        if !reopened {
            // The file is gone or broken: record on into a new member file rather than lose the flight.
            self.start_fresh_session(false);
            if is_armed {
                self.record_event(EVENT_ARM);
            }
            return;
        }
        let was_armed = self
            .active_flight
            .as_ref()
            .and_then(|f| f.events.iter().rev().find(|e| e.kind == EVENT_ARM || e.kind == EVENT_DISARM))
            .is_some_and(|e| e.kind == EVENT_ARM);
        self.record_event(EVENT_LINK_BACK);
        if is_armed && !was_armed {
            self.record_event(EVENT_ARM);
        } else if !is_armed && was_armed {
            self.record_event(EVENT_DISARM);
        }
    }

    /// Open a brand-new recording session (temp store + raw logger); `announce` = emit
    /// `flight-recording-started` (single flights — group members never announce). Nothing is
    /// written to the main DB here — the real `flight_id` is born at commit (ADR-041).
    fn start_fresh_session(&mut self, announce: bool) {
        log::info!("ARM detected — starting flight recording");

        let now = self.clock.utc();

        // Open the per-session temp store (DB recording only).
        let (temp_db, temp_path) = if self.settings.db_enabled {
            let sessions_dir = self
                .db_file_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("sessions");
            // The vehicle key makes the name unique per recorder: two vehicles arming in the same
            // second ("arm all") must never share a file. Not the FC id — it is missing on secondary /
            // passive links and not unique (ArduPilot SITL instances share one).
            let path = sessions_dir.join(format!(
                "active_{}_{}.ktmp",
                now.format("%Y-%m-%d_%H%M%S"),
                file_key(self.key())
            ));
            // Registered before the file exists, so a concurrent discard sweep never sees it unprotected.
            self.slots.set_live(None, Some(&path));
            match db::open_temp_session(&path) {
                Ok(conn) => {
                    if let Err(e) = db::write_session_meta(
                        &conn,
                        &now,
                        &self.fc_info.craft_name,
                        &self.fc_info.fc_variant,
                        &self.fc_info.fc_version,
                        &self.fc_info.board_id,
                        self.fc_info.platform_type,
                        self.fc_info.fc_uid.as_deref(),
                        &self.protocol,
                        self.snapshot.lat,
                        self.snapshot.lon,
                    ) {
                        log::warn!("Failed to write session_meta: {}", e);
                    }
                    log::info!("Temp session store: {}", path.display());
                    (Some(conn), Some(path))
                }
                Err(e) => {
                    log::error!("Failed to open temp session store: {} — this flight won't be recorded to the DB", e);
                    self.slots.set_live(Some(&path), None);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // Start raw/tlog logger if enabled AND not already running (continuous mode). The raw log is
        // a parallel backup; it has no DB flight id, so it is named by a timestamp pseudo-id.
        let log_dir = self.raw_log_dir.as_path();
        let raw_pseudo_id = now.timestamp();

        if !self.settings.raw_always {
            // Non-continuous: create the per-flight raw logger for this protocol (named by a
            // timestamp pseudo-id, no DB flight id yet). MAVLink → tlog; MSP → shared raw-serial sink.
            if self.settings.raw_enabled {
                if self.protocol == "MAVLink" {
                    match TlogLogger::new(log_dir, raw_pseudo_id, &now) {
                        Ok(logger) => self.tlog_logger = Some(logger),
                        Err(e) => log::warn!("Failed to create tlog logger: {}", e),
                    }
                } else {
                    self.open_msp_raw_log(raw_pseudo_id, &now);
                }
            }
        }
        // else: continuous mode — loggers already running from start_continuous_log()

        // Enrichment (weather + geocode) is deferred to disarm — it needs the committed flight id.

        self.active_flight = Some(ActiveFlight {
            temp_db,
            temp_path,
            start_time: now,
            start_instant: self.clock.instant(),
            start_lat: self.snapshot.lat,
            start_lon: self.snapshot.lon,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
            bat_buffer: Vec::new(),
            max_alt: 0.0,
            max_speed: 0.0,
            max_distance: 0.0,
            total_distance: 0.0,
            last_lat: None,
            last_lon: None,
            start_mah: self.snapshot.mah_drawn,
            events: Vec::new(),
        });
        self.publish_active_path();
        self.write_membership_meta();

        log::info!("Flight recording started (db={})", self.settings.db_enabled);

        // Id-less signal that recording is active (the frontend resets its flown-mission baseline;
        // the actual mission link happens on `flight-recording-ended` once the id exists).
        if self.settings.db_enabled && announce {
            self.emit("flight-recording-started", ());
        }
    }

    /// Take the active flight, close its raw loggers, flush + close its temp store, and build the
    /// finalized `PendingSession` (+ its telemetry sample count). Returns `None` in raw-only mode
    /// (no DB session to commit). Shared by `on_disarm` and `shutdown` — they differ only in which
    /// lifecycle event they then emit — and by the group-member paths (`finalize_member`,
    /// `suspend_member`), where the stats are recomputed over the armed segments.
    ///
    /// The file stays registered as live until the caller has parked (or committed) the session and
    /// calls `publish_active_path`, so the hand-over into the pending slot leaves no gap
    /// (`protected_paths` reads the live set before the slots). The reverse moves — out of a slot into
    /// a commit or a reopen — register the path before the file is used (`commit_protected`,
    /// `reopen_session`); only the instant between the take and that registration is uncovered, since
    /// the maps are locked one at a time.
    fn take_active_as_pending(&mut self) -> Option<(PendingSession, i64)> {
        let mut flight = self.active_flight.take()?;
        let end_time = self.clock.utc();
        let elapsed = self.since(flight.start_instant);
        let duration = elapsed.as_secs() as i64;
        let last_timestamp_ms = elapsed.as_millis() as i64;
        let battery_used = match (flight.start_mah, self.snapshot.mah_drawn) {
            (Some(start), Some(end)) if end >= start => Some(end - start),
            _ => None,
        };

        // Close raw/tlog logger (non-continuous mode) regardless of the DB path.
        if !self.settings.raw_always {
            self.close_msp_raw_log();
            if let Some(mut logger) = self.tlog_logger.take() {
                logger.close();
            }
        }

        let (Some(temp_db), Some(temp_path)) = (flight.temp_db.take(), flight.temp_path.clone())
        else {
            log::info!(
                "Flight ended (raw-only): {}s, max_alt={:.1}m, max_speed={:.1}m/s, distance={:.0}m",
                duration, flight.max_alt, flight.max_speed, flight.total_distance,
            );
            return None;
        };

        if !flight.buffer.is_empty() {
            if let Err(e) = db::insert_telemetry_batch(&temp_db, &flight.buffer) {
                log::error!("Failed to flush final telemetry batch to temp store: {}", e);
            }
        }
        if !flight.bat_buffer.is_empty() {
            if let Err(e) = db::insert_battery_records_batch(&temp_db, &flight.bat_buffer) {
                log::error!("Failed to flush final battery batch to temp store: {}", e);
            }
        }
        let sample_count = db::temp_session_row_count(&temp_db).unwrap_or(0);
        let armed_segments = armed_segments_from(&flight.events, last_timestamp_ms);
        // A group member recorded disarmed stretches too: its stats count the armed segments only
        // (§3.8), recomputed from the file — the open segment ends at the last sample, as in recovery.
        let member_stats = self.membership.as_ref().map(|_| {
            let rows = db::read_flight_track(&temp_db, 0, Some(&self.fc_info.fc_variant)).unwrap_or_else(|e| {
                log::warn!("Cannot read the member track for its stats: {}", e);
                Vec::new()
            });
            let end_ms = rows.last().map(|r| r.timestamp_ms).unwrap_or(0);
            let segments = armed_segments_from(&flight.events, end_ms);
            (stats_over_segments(&rows, &segments, flight.start_lat, flight.start_lon), segments)
        });
        drop(temp_db); // checkpoint the WAL before any later ATTACH

        let (duration, battery_used, armed_segments) = match &member_stats {
            Some((s, segments)) => {
                flight.max_alt = s.max_alt;
                flight.max_speed = s.max_speed;
                flight.max_distance = s.max_distance;
                flight.total_distance = s.total_distance;
                (s.duration_sec, s.battery_used, segments.clone())
            }
            None => (duration, battery_used, armed_segments),
        };
        let flight_row = Flight {
            id: 0,
            start_time: flight.start_time,
            end_time: Some(end_time),
            duration_sec: Some(duration),
            source: "live".into(),
            craft_name: self.fc_info.craft_name.clone(),
            fc_variant: self.fc_info.fc_variant.clone(),
            fc_version: self.fc_info.fc_version.clone(),
            board_id: self.fc_info.board_id.clone(),
            platform_type: self.fc_info.platform_type,
            fc_uid: self.fc_info.fc_uid.clone(),
            protocol: self.protocol.clone(),
            start_lat: flight.start_lat,
            start_lon: flight.start_lon,
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            max_alt_m: Some(flight.max_alt),
            max_speed_ms: Some(flight.max_speed),
            max_distance_m: Some(flight.max_distance),
            total_distance_m: Some(flight.total_distance),
            battery_used_mah: battery_used,
            notes: None,
            linked_flight_id: None,
            pilot_name: None,
            pilot_id: None,
            battery_serial: None,
            // Live recording: the GCS sits at the flight location, so its own offset is the
            // flight-local offset (ADR-048).
            utc_offset_min: Some(super::timezone::local_offset_min_now()),
            group_id: self.membership.as_ref().map(|m| m.group_id.clone()),
        };
        Some((
            PendingSession {
                temp_path,
                db_path: self.db_file_path.clone(),
                flight: flight_row,
                disarm_instant: self.clock.instant(),
                start_mah: flight.start_mah,
                last_timestamp_ms,
                armed_segments,
                membership: self.membership.clone(),
            },
            sample_count,
        ))
    }

    /// Called when a disarm transition is detected. The flight is finalized as the pending session
    /// (deferred commit, ADR-041) and the frontend shows the End-Flight summary (Save / Discard); a
    /// re-arm resolves it instead. A group member only records the event and records on.
    fn on_disarm(&mut self) {
        if self.membership.is_some() {
            log::info!("DISARM (group member {}) — recording continues", self.key());
            self.record_event(EVENT_DISARM);
            return;
        }
        log::info!("DISARM detected — stopping flight recording");
        self.record_event(EVENT_DISARM);
        if let Some((session, _count)) = self.take_active_as_pending() {
            let dur = session.flight.duration_sec.unwrap_or(0);
            self.stash_pending_and_emit_ended(session);
            log::info!("Flight pending commit (disarm): {}s", dur);
        }
        self.publish_active_path();
    }

    /// Record a telemetry sample into the active flight's temp store / statistics.
    /// Called after attitude or GPS updates (the highest-frequency data). Raw serial logging is
    /// independent — it happens at the transport (tlog frames / MSP raw sink), not here.
    fn maybe_record_sample(&mut self) {
        let flight = match &mut self.active_flight {
            Some(f) => f,
            None => return,
        };

        let elapsed_ms = self.clock.instant().saturating_duration_since(flight.start_instant).as_millis() as i64;
        // Wall-clock stamp per row (schema v20): absolute time survives gaps and the resume offset.
        let wall_ms = self.clock.utc().timestamp_millis();

        // Relative altitude (baro, GPS fallback) drives the flight's max-altitude statistic and the
        // replay widget's relative reading. The stored `alt_m` is GPS MSL (see below).
        let alt_rel = self.snapshot.alt_baro.or(self.snapshot.alt_gps);

        let record = TelemetryRecord {
            id: 0,
            flight_id: 0, // temp store local id; rewritten to the main flight id on commit
            timestamp_ms: elapsed_ms,
            lat: self.snapshot.lat,
            lon: self.snapshot.lon,
            // GPS MSL — the replay map/3D height (the adapter maps alt_m → altMsl, matching the live
            // track + Blackbox import). baro is relative-to-home, so storing it here made replay
            // AGL = baro − terrain (e.g. −84 m at a ~84 m-MSL field).
            alt_m: self.snapshot.alt_gps,
            speed_ms: self.snapshot.speed,
            airspeed_ms: self.snapshot.airspeed,
            throttle_pct: self.snapshot.throttle,
            heading: self.snapshot.heading,
            vario_ms: self.snapshot.vario,
            voltage: self.snapshot.voltage,
            current_a: self.snapshot.current,
            mah_drawn: self.snapshot.mah_drawn,
            rssi: self.snapshot.rssi,
            battery_percentage: self.snapshot.battery_percentage,
            roll: self.snapshot.roll,
            pitch: self.snapshot.pitch,
            yaw: self.snapshot.yaw,
            fix_type: self.snapshot.fix_type,
            num_sat: self.snapshot.num_sat,
            cpu_load: self.snapshot.cpu_load,
            link_quality: self.snapshot.link_quality, // live LQ from the unified link-stats pipeline
            baro_alt_m: self.snapshot.alt_baro,
            gps_hdop: self.snapshot.gps_hdop,
            gps_eph: self.snapshot.gps_eph,
            gps_epv: self.snapshot.gps_epv,
            active_wp_number: self.snapshot.active_wp_number,
            active_flight_mode_flags: self.snapshot.active_flight_mode_flags.map(|f| f as i64),
            state_flags: None, // INAV stateFlags is Blackbox-only (no live MSP source)
            nav_state: self.snapshot.nav_state,
            nav_flags: None, // MSP_NAV_STATUS exposes the target WP + state, not the nav flag bitmask
            rx_signal_received: None,
            hw_health_status: self.snapshot.hw_health_status,
            baro_temperature: None,
            wind_n_ms: self.snapshot.wind_n_ms,
            wind_e_ms: self.snapshot.wind_e_ms,
            wind_d_ms: None,
            rc_data_json: None,
            rc_command_json: None,
            nav_lat: None,
            nav_lon: None,
            nav_alt_m: None,
            mode_primary: self.snapshot.mode_primary.clone(),
            mode_modifiers: self.snapshot.mode_modifiers.clone(),
            link_snr: self.snapshot.link_snr,
            link_rssi_dbm: self.snapshot.link_rssi_dbm,
            wall_ms: Some(wall_ms),
        };

        // Update statistics (max altitude is the relative-to-home reading, like the Blackbox stats)
        if let Some(a) = alt_rel {
            if a > flight.max_alt {
                flight.max_alt = a;
            }
        }
        if let Some(s) = self.snapshot.speed {
            if s > flight.max_speed {
                flight.max_speed = s;
            }
        }

        // Distance tracking
        if let (Some(lat), Some(lon)) = (self.snapshot.lat, self.snapshot.lon) {
            if let (Some(prev_lat), Some(prev_lon)) = (flight.last_lat, flight.last_lon) {
                let dist = haversine_m(prev_lat, prev_lon, lat, lon);
                flight.total_distance += dist;
            }
            // Distance from start
            if let (Some(slat), Some(slon)) = (flight.start_lat, flight.start_lon) {
                let from_start = haversine_m(slat, slon, lat, lon);
                if from_start > flight.max_distance {
                    flight.max_distance = from_start;
                }
            }
            flight.last_lat = Some(lat);
            flight.last_lon = Some(lon);
        }

        // Per-instance battery rows for this same timestamp (multi-battery; ArduPilot/PX4). Only when
        // ≥2 monitors — single-battery flights replay from the denormalised primary on
        // telemetry_records (matches the import path), so we write nothing redundant here.
        if self.snapshot.batteries.len() >= 2 {
        for b in &self.snapshot.batteries {
            flight.bat_buffer.push(BatteryRecord {
                id: 0,
                flight_id: 0, // temp store local id; rewritten on commit
                timestamp_ms: elapsed_ms,
                instance: b.id,
                voltage: Some(b.voltage),
                current_a: Some(b.current),
                mah_drawn: Some(b.mah_drawn),
                battery_percentage: Some(b.percentage),
                cell_count: Some(b.cell_count),
                temperature: b.temperature,
            });
        }
        }

        // Buffer + flush into the temp session store (DB recording only). The temp store is the
        // durable buffer; the main DB is untouched until the commit on disarm.
        if let Some(ref temp_db) = flight.temp_db {
            flight.buffer.push(record);

            // Flush buffer when threshold reached
            if flight.buffer.len() >= FLUSH_THRESHOLD {
                let records = std::mem::replace(
                    &mut flight.buffer,
                    Vec::with_capacity(FLUSH_THRESHOLD),
                );
                if let Err(e) = db::insert_telemetry_batch(temp_db, &records) {
                    log::error!("Failed to flush telemetry batch to temp store: {}", e);
                }
                if !flight.bat_buffer.is_empty() {
                    let bat = std::mem::take(&mut flight.bat_buffer);
                    if let Err(e) = db::insert_battery_records_batch(temp_db, &bat) {
                        log::error!("Failed to flush battery batch to temp store: {}", e);
                    }
                }
            }
        }
    }

    /// Graceful shutdown (disconnect). An **active (armed) flight** is finalized as a pending session
    /// and the frontend is shown the **recovery prompt** via `flight-recording-interrupted` — the
    /// flight may not be over (the user could be changing the COM port or switching to telemetry), so
    /// Continue-on-Reconnect must be offered (ADR-042), not the End-Flight dialog.
    pub fn shutdown(&mut self) {
        if self.membership.is_some() {
            self.suspend_member();
        } else if self.active_flight.is_some() {
            log::info!("Disconnect with active flight — stashed as pending (frontend confirmed, applies the action)");
            if let Some((session, _count)) = self.take_active_as_pending() {
                // Resolved by the frontend's Save/Discard/Continue.
                self.slots.park_pending(self.key(), session);
            }
            self.publish_active_path();
        }
        self.close_continuous_loggers();
        self.mirror_to_shared_folders();
        self.slots.detach(self.key());
    }

    /// Connection lost (device gone — e.g. USB unplugged), detected by the scheduler. Like `shutdown`,
    /// but emits `flight-recording-interrupted` so the frontend shows the recovery prompt — there was
    /// no chance to pre-confirm the disconnect (ADR-042).
    pub fn shutdown_lost(&mut self) {
        if self.membership.is_some() {
            // Mid-group: no recovery prompt — the group's store prompt covers the member later.
            self.suspend_member();
        } else if self.active_flight.is_some() {
            log::info!("Connection lost with active flight — offering recovery");
            if let Some((session, sample_count)) = self.take_active_as_pending() {
                let ev = RecordingInterruptedEvent {
                    temp_path: session.temp_path.to_string_lossy().to_string(),
                    craft_name: session.flight.craft_name.clone(),
                    start_time: session.flight.start_time.to_rfc3339(),
                    duration_sec: session.flight.duration_sec.unwrap_or(0),
                    sample_count,
                };
                // Save/Discard via commands; Continue moves it to the resume queue.
                self.slots.park_pending(self.key(), session);
                self.emit("flight-recording-interrupted", ev);
            }
            self.publish_active_path();
        }
        self.close_continuous_loggers();
        self.mirror_to_shared_folders();
        self.slots.detach(self.key());
    }

    /// A group member's link went away while its group runs (§3.2): write `link_lost`, close the
    /// session and keep it — protected — for the same aircraft's next recorder to adopt
    /// (`adopt_member`) or the coordinator to finalize when the group ends. No lifecycle event.
    fn suspend_member(&mut self) {
        if self.active_flight.is_none() {
            return;
        }
        self.record_event(EVENT_LINK_LOST);
        if let Some((session, _count)) = self.take_active_as_pending() {
            log::info!("Group member {} suspended (link gone) — {} kept for adoption", self.key(), session.temp_path.display());
            self.slots.park_suspended(session);
        }
        self.publish_active_path();
    }

    /// Mirror the session's artefacts into user-granted shared folders where the settings name one
    /// (Android SAF tree URIs) — no-op everywhere else. Lives in `user_file`, the module that owns
    /// user-storage platform differences; called on both teardown paths so a lost link mirrors too.
    fn mirror_to_shared_folders(&self) {
        crate::user_file::mirror_session(
            &self.db_file_path,
            &self.raw_log_dir,
            &self.settings.db_path,
            &self.settings.raw_log_path,
        );
    }

    /// Close the continuous (pre-arm) raw/tlog loggers on disconnect.
    fn close_continuous_loggers(&mut self) {
        self.close_msp_raw_log();
        if let Some(mut logger) = self.tlog_logger.take() {
            log::info!("Closing continuous tlog session");
            logger.close();
        }
    }
}

/// Haversine distance in meters between two lat/lon points
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0; // Earth radius in meters
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let lat1_r = lat1.to_radians();
    let lat2_r = lat2.to_radians();

    let a = (d_lat / 2.0).sin().powi(2)
        + lat1_r.cos() * lat2_r.cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    R * c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(timestamp_ms: i64) -> TelemetryRecord {
        TelemetryRecord {
            id: 0,
            flight_id: 0,
            timestamp_ms,
            lat: None,
            lon: None,
            alt_m: None,
            speed_ms: None,
            airspeed_ms: None,
            throttle_pct: None,
            heading: None,
            vario_ms: None,
            voltage: None,
            current_a: None,
            mah_drawn: None,
            rssi: None,
            battery_percentage: None,
            roll: None,
            pitch: None,
            yaw: None,
            fix_type: None,
            num_sat: None,
            cpu_load: None,
            link_quality: None,
            baro_alt_m: None,
            gps_hdop: None,
            gps_eph: None,
            gps_epv: None,
            active_wp_number: None,
            // Raw INAV flags without a canonical mode, so the read path derives the mode.
            active_flight_mode_flags: Some(1),
            state_flags: None,
            nav_state: None,
            nav_flags: None,
            rx_signal_received: None,
            hw_health_status: None,
            baro_temperature: None,
            wind_n_ms: None,
            wind_e_ms: None,
            wind_d_ms: None,
            rc_data_json: None,
            rc_command_json: None,
            nav_lat: None,
            nav_lon: None,
            nav_alt_m: None,
            mode_primary: None,
            mode_modifiers: None,
            link_snr: None,
            link_rssi_dbm: None,
            wall_ms: None,
        }
    }

    fn pending(temp: &str, craft: &str, fc_uid: Option<&str>, start_offset_s: i64) -> PendingSession {
        PendingSession {
            temp_path: PathBuf::from(temp),
            db_path: PathBuf::from("unused.db"),
            flight: Flight {
                id: 0,
                start_time: DateTime::<Utc>::from_timestamp(1_700_000_000 + start_offset_s, 0).unwrap(),
                end_time: None,
                duration_sec: None,
                source: "live".into(),
                craft_name: craft.into(),
                fc_variant: "INAV".into(),
                fc_version: String::new(),
                board_id: String::new(),
                platform_type: 0,
                fc_uid: fc_uid.map(String::from),
                protocol: "MSP".into(),
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
            },
            disarm_instant: Instant::now(),
            start_mah: None,
            last_timestamp_ms: 0,
            armed_segments: Vec::new(),
            membership: None,
        }
    }

    #[test]
    fn fake_clock_moves_both_readings_together() {
        let base = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let clock = FakeClock::new(base);
        let i0 = clock.instant();
        assert_eq!(clock.utc(), base);
        clock.advance(Duration::from_millis(1500));
        assert_eq!(clock.instant().duration_since(i0), Duration::from_millis(1500));
        assert_eq!(clock.utc().timestamp_millis(), base.timestamp_millis() + 1500);
        clock.set(Duration::from_secs(10));
        assert_eq!(clock.instant().duration_since(i0), Duration::from_secs(10));
        assert_eq!(clock.utc().timestamp(), base.timestamp() + 10);
    }

    #[test]
    fn file_key_is_a_safe_file_name_fragment() {
        assert_eq!(file_key("L1:S2"), "L1-S2");
        assert_eq!(file_key("L12:S0"), "L12-S0");
    }

    /// Every file a recorder owns — live, pending (primary AND secondary), resume — is protected (M2),
    /// and one vehicle's slot never overwrites another's.
    #[test]
    fn session_slots_protect_every_live_pending_and_resume_file() {
        let slots = SessionSlots::default();
        slots.set_live(None, Some(&PathBuf::from("live_a.ktmp")));
        slots.set_live(None, Some(&PathBuf::from("live_b.ktmp")));
        slots.park_pending("L1:S1", pending("pend_primary.ktmp", "A", None, 0));
        slots.park_pending("L1:S2", pending("pend_secondary.ktmp", "B", None, 0));
        slots.put_resume("L3:S0".into(), pending("resume.ktmp", "C", None, 0)).unwrap();
        let mut keep: Vec<String> =
            slots.protected_paths().iter().map(|p| p.to_string_lossy().to_string()).collect();
        keep.sort();
        assert_eq!(
            keep,
            ["live_a.ktmp", "live_b.ktmp", "pend_primary.ktmp", "pend_secondary.ktmp", "resume.ktmp"]
        );
        // Swapping a live file drops the old one.
        slots.set_live(Some(&PathBuf::from("live_a.ktmp")), None);
        assert!(!slots.protected_paths().contains(&PathBuf::from("live_a.ktmp")));
    }

    #[test]
    fn take_pending_addresses_one_vehicle_and_refuses_to_guess() {
        let slots = SessionSlots::default();
        assert!(slots.take_pending(None).unwrap().is_none());
        // Single vehicle: no id needed (the pre-multi-vehicle command shape).
        slots.park_pending("L1:S1", pending("a.ktmp", "A", None, 0));
        assert_eq!(slots.take_pending(None).unwrap().unwrap().temp_path, PathBuf::from("a.ktmp"));
        // Two vehicles: an id is required, and it takes exactly that vehicle's session.
        slots.park_pending("L1:S1", pending("a.ktmp", "A", None, 0));
        slots.park_pending("L2:S0", pending("b.ktmp", "B", None, 0));
        assert!(slots.take_pending(None).is_err());
        assert!(slots.take_pending(Some("L9:S9")).unwrap().is_none());
        assert_eq!(slots.take_pending(Some("L2:S0")).unwrap().unwrap().temp_path, PathBuf::from("b.ktmp"));
        assert_eq!(slots.take_pending_for("L1:S1").unwrap().temp_path, PathBuf::from("a.ktmp"));
    }

    /// A pending session of a closed connection is claimed on arm by the same FC under its new key;
    /// another aircraft only takes it when it is the only one connected (single-slot behaviour).
    #[test]
    fn detached_pending_is_claimed_by_the_same_fc_or_a_lone_recorder() {
        let wing = FcInfo { craft_name: "Wing".into(), fc_variant: "INAV".into(), ..FcInfo::default() };
        let other = FcInfo { craft_name: "Quad".into(), fc_variant: "INAV".into(), ..FcInfo::default() };
        let slots = SessionSlots::default();
        slots.attach("L1:S0");
        slots.park_pending("L1:S0", pending("wing.ktmp", "Wing", None, 0));
        // Still connected → not up for grabs.
        slots.attach("L2:S0");
        assert!(slots.take_detached_pending_for("L2:S0", &wing).is_none());
        // L1 closes; L2 carries another aircraft and L3 is connected too → no guess.
        slots.detach("L1:S0");
        slots.attach("L3:S0");
        assert!(slots.take_detached_pending_for("L2:S0", &other).is_none());
        // The same FC reconnected as L3 claims it.
        assert_eq!(slots.take_detached_pending_for("L3:S0", &wing).unwrap().temp_path, PathBuf::from("wing.ktmp"));
        // Alone: the only detached session goes to whoever arms next, whatever its identity.
        slots.park_pending("L1:S0", pending("wing.ktmp", "Wing", None, 0));
        slots.detach("L3:S0");
        assert_eq!(slots.take_detached_pending_for("L2:S0", &other).unwrap().temp_path, PathBuf::from("wing.ktmp"));
    }

    #[test]
    fn resume_is_claimed_by_identity_then_oldest_when_alone() {
        let slots = SessionSlots::default();
        slots.attach("L1:S0");
        slots.attach("L2:S0");
        slots.put_resume("old".into(), pending("old.ktmp", "Other", None, 0)).unwrap();
        slots.put_resume("mine".into(), pending("mine.ktmp", "Wing", Some("UID1"), 60)).unwrap();
        let fc = FcInfo { craft_name: "Renamed".into(), fc_variant: "INAV".into(), fc_uid: Some("UID1".into()), ..FcInfo::default() };
        // Hardware id wins over the older entry and over a craft-name mismatch.
        assert_eq!(slots.take_resume_for("L2:S0", &fc).unwrap().temp_path, PathBuf::from("mine.ktmp"));
        // No match left and another vehicle connected → no guess.
        assert!(slots.take_resume_for("L2:S0", &fc).is_none());
        // Alone → the next connection takes the oldest (single-slot behaviour).
        slots.detach("L1:S0");
        assert_eq!(slots.take_resume_for("L2:S0", &fc).unwrap().temp_path, PathBuf::from("old.ktmp"));
        assert!(slots.take_resume_for("L2:S0", &fc).is_none());
    }

    /// Crash recovery reads an orphan `.ktmp`, which has no `flights` table: summarizing it must
    /// take the flight variant from `session_meta` instead of querying `flights`.
    #[test]
    fn summarize_temp_session_reads_a_ktmp_without_flights_table() {
        let temp_path = std::env::temp_dir()
            .join(format!("kite-recorder-test-recover-{}.ktmp", std::process::id()));
        db::remove_temp_session(&temp_path);
        {
            let conn = db::open_temp_session(&temp_path).unwrap();
            db::write_session_meta(
                &conn, &Utc::now(), "TestCraft", "INAV", "8.0.0", "TEST", 1, None, "MSP", None, None,
            )
            .unwrap();
            db::insert_telemetry_batch(&conn, &[record(0), record(100)]).unwrap();
        }

        let result = summarize_temp_session(temp_path.clone(), PathBuf::from("unused.db"), &SystemClock);
        let derived = db::open_temp_session(&temp_path)
            .and_then(|conn| db::read_flight_track(&conn, 0, Some("INAV")));
        db::remove_temp_session(&temp_path);

        let (session, count) = result.expect("recovery must read the .ktmp");
        assert_eq!(count, 2);
        assert_eq!(session.flight.fc_variant, "INAV");
        let expected = crate::flightmode::classify_inav(1).primary;
        let derived = derived.unwrap();
        assert_eq!(derived.len(), 2);
        for r in derived {
            assert_eq!(r.mode_primary.as_deref(), Some(expected.as_str()));
        }
    }

    // ── Group flights, steps 3–4 (Dev-Docs active/GROUP_FLIGHTS.md) ─────────────────────────────

    /// Captures the lifecycle events a recorder emits (no Tauri runtime).
    struct TestSink {
        key: String,
        emitted: Arc<Mutex<Vec<String>>>,
    }

    impl RecorderSink for TestSink {
        fn key(&self) -> &str {
            &self.key
        }
        fn emit_json(&self, event: &str, _payload: serde_json::Value) -> Result<(), String> {
            self.emitted.lock().unwrap().push(event.to_string());
            Ok(())
        }
    }

    /// A throw-away DB folder, the shared slots and one fake clock for every recorder of a test.
    struct Rig {
        dir: PathBuf,
        clock: Arc<FakeClock>,
        slots: SessionSlotsHandle,
    }

    impl Rig {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("kite-rec-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let base = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
            Rig { dir, clock: Arc::new(FakeClock::new(base)), slots: Arc::new(SessionSlots::default()) }
        }

        fn base_ms(&self) -> i64 {
            1_700_000_000_000
        }

        fn recorder(&self, key: &str, fc: FcInfo) -> (FlightRecorder, Arc<Mutex<Vec<String>>>) {
            let emitted = Arc::new(Mutex::new(Vec::new()));
            let settings = FlightLogSettings {
                enabled: true,
                db_enabled: true,
                db_path: self.dir.to_string_lossy().to_string(),
                raw_log_path: self.dir.join("raw").to_string_lossy().to_string(),
                raw_enabled: false,
                raw_always: false,
            };
            let sink = TestSink { key: key.to_string(), emitted: emitted.clone() };
            let rec = FlightRecorder::with_clock(
                settings, fc, "MAVLink", false, sink, self.slots.clone(), Arc::new(Mutex::new(None)), self.clock.clone(),
            )
            .unwrap();
            (rec, emitted)
        }

        fn kgrp(&self, group_id: &str) -> PathBuf {
            group_store::group_file_path(&self.dir.join("sessions"), group_id)
        }

        fn advance_s(&self, s: u64) {
            self.clock.advance(Duration::from_secs(s));
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fc(craft: &str, uid: Option<&str>) -> FcInfo {
        FcInfo { craft_name: craft.into(), fc_variant: "ArduPilot".into(), fc_uid: uid.map(String::from), ..FcInfo::default() }
    }

    fn status(armed: bool) -> StatusData {
        StatusData {
            arming_flags: if armed { ARMED_FLAG } else { 0 },
            flight_mode_flags: 0,
            cpu_load: 0,
            sensor_status: 0,
            msp_rc_override: false,
        }
    }

    fn gps(fix_type: u8, lat: f64, lon: f64) -> GpsData {
        GpsData { fix_type, num_sat: 10, lat, lon, alt_msl: 100.0, ground_speed: 5.0, course: 0.0 }
    }

    fn active_path(rec: &FlightRecorder) -> PathBuf {
        rec.active_flight.as_ref().and_then(|f| f.temp_path.clone()).expect("an active session")
    }

    /// `(kind, timestamp_ms, wall_ms)` of a temp session's events (read through its own connection).
    fn file_events(path: &Path) -> Vec<(String, i64, Option<i64>)> {
        let conn = Connection::open(path).unwrap();
        db::read_session_events(&conn).unwrap().into_iter().map(|e| (e.kind, e.timestamp_ms, e.wall_ms)).collect()
    }

    fn file_rows(path: &Path) -> Vec<TelemetryRecord> {
        db::read_flight_track(&Connection::open(path).unwrap(), 0, None).unwrap()
    }

    fn kinds(events: &[(String, i64, Option<i64>)]) -> Vec<(&str, i64)> {
        events.iter().map(|(k, t, _)| (k.as_str(), *t)).collect()
    }

    fn seg(start_ms: i64, end_ms: i64) -> ArmedSegment {
        ArmedSegment { start_ms, end_ms }
    }

    fn event(kind: &str, timestamp_ms: i64) -> FlightEvent {
        FlightEvent {
            id: 0,
            flight_id: 0,
            timestamp_ms,
            wall_ms: Some(1_700_000_000_000 + timestamp_ms),
            kind: kind.into(),
            code: None,
            text: None,
            source: Some("live".into()),
        }
    }

    fn test_group(id: &str) -> crate::flightlog::types::FlightGroup {
        crate::flightlog::types::FlightGroup {
            id: id.into(),
            start_time: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(),
            end_time: None,
            utc_offset_min: Some(60),
            start_lat: None,
            start_lon: None,
            start_alt_m: None,
            location_name: None,
            weather_temp_c: None,
            weather_wind_ms: None,
            weather_wind_deg: None,
            weather_desc: None,
            notes: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn armed_segments_pair_arm_and_disarm_events() {
        let events = [
            event(EVENT_DISARM, 0), // disarm while disarmed: ignored
            event(EVENT_ARM, 100),
            event(EVENT_ARM, 150), // arm while armed: ignored
            event(EVENT_LINK_LOST, 200),
            event(EVENT_DISARM, 300),
            event(EVENT_ARM, 900),
        ];
        assert_eq!(armed_segments_from(&events, 1000), [seg(100, 300), seg(900, 1000)]);
        assert!(armed_segments_from(&[], 1000).is_empty());
    }

    /// (a) Single-flight mode is unchanged — End-Flight on disarm, grace resume with the compressed
    /// timeline, grace-lapsed commit — and every arm/disarm is now an event, carried into the main DB.
    #[test]
    fn solo_arm_disarm_grace_unchanged_and_events_written() {
        let rig = Rig::new("solo");
        let (mut rec, emitted) = rig.recorder("L1:S1", fc("Solo", None));
        rec.on_gps(&gps(2, 0.0, 0.0)); // (0,0) = no position → no enrichment request on the commit below
        rec.on_status(&status(false));
        rec.on_status(&status(true));
        let path = active_path(&rec);
        assert!(path.file_name().unwrap().to_string_lossy().ends_with("_L1-S1.ktmp"));
        rig.advance_s(2);
        rec.on_gps(&gps(2, 0.0, 0.0));
        rec.on_status(&status(false));
        assert!(rec.active_flight.is_none());
        assert_eq!(*emitted.lock().unwrap(), ["flight-recording-started", "flight-recording-ended"]);
        assert!(rig.slots.protected_paths().contains(&path));

        // Re-arm within grace → same file, timeline continues at the last sample (gap compressed).
        rig.advance_s(3);
        rec.on_status(&status(true));
        assert_eq!(active_path(&rec), path);
        assert_eq!(emitted.lock().unwrap().last().map(String::as_str), Some("flight-recording-resumed"));
        rig.advance_s(1);
        rec.on_gps(&gps(2, 0.0, 0.0));
        rec.on_status(&status(false));
        let events = file_events(&path);
        assert_eq!(kinds(&events), [("arm", 0), ("disarm", 2000), ("arm", 2000), ("disarm", 3000)]);
        // …while the wall clock keeps the 3 s the aircraft sat disarmed.
        assert_eq!(events[2].2, Some(rig.base_ms() + 5000));
        let meta = db::read_session_meta(&Connection::open(&path).unwrap()).unwrap().unwrap();
        assert_eq!((meta.role.as_deref(), meta.vehicle_key.as_deref(), meta.group_id), (Some("single"), Some("L1:S1"), None));

        // Grace lapsed → the previous flight is committed (with its events), a new one starts.
        rig.advance_s(6);
        rec.on_status(&status(true));
        let tail: Vec<String> = emitted.lock().unwrap().iter().skip(3).cloned().collect();
        assert_eq!(tail, ["flight-recording-ended", "flight-recording-committed", "flight-recording-started"]);
        assert!(!path.exists());
        assert_ne!(active_path(&rec), path);
        let main = db::open_database(&rig.dir.join("flights.db")).unwrap();
        let flight_id: i64 = main.query_row("SELECT MAX(id) FROM flights", [], |r| r.get(0)).unwrap();
        let committed = db::get_flight_events(&main, flight_id).unwrap();
        assert_eq!(committed.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["arm", "disarm", "arm", "disarm"]);
        assert_eq!(db::get_flight(&main, flight_id).unwrap().unwrap().group_id, None);
    }

    /// (b) A group member records through a disarm: the disarm and the re-arm are events in ONE file —
    /// no pending session, no End-Flight, no grace (the re-arm comes 10 s later), no second start.
    #[test]
    fn member_disarm_rearm_is_one_file_without_pending_or_grace() {
        let rig = Rig::new("member");
        let kgrp = rig.kgrp("g1");
        let (mut rec, emitted) = rig.recorder("L1:S1", fc("A", None));
        rec.on_gps(&gps(2, 48.1, 11.5));
        rec.on_status(&status(false));
        rec.on_status(&status(true));
        let path = active_path(&rec);
        rec.enter_member_mode("g1", &kgrp);
        rig.advance_s(1);
        rec.on_gps(&gps(2, 48.1, 11.5));
        rec.on_status(&status(false));
        assert_eq!(active_path(&rec), path);
        rig.advance_s(10);
        rec.on_gps(&gps(2, 48.1, 11.5));
        rec.on_status(&status(true));
        assert_eq!(active_path(&rec), path);
        assert!(rig.slots.take_pending(None).unwrap().is_none());
        assert_eq!(*emitted.lock().unwrap(), ["flight-recording-started"]);
        assert_eq!(kinds(&file_events(&path)), [("arm", 0), ("disarm", 1000), ("arm", 11000)]);
        let meta = db::read_session_meta(&Connection::open(&path).unwrap()).unwrap().unwrap();
        assert_eq!(meta.role.as_deref(), Some("member"));
        assert_eq!(meta.group_id.as_deref(), Some("g1"));
        assert_eq!(meta.group_file.as_deref(), Some("group_g1.kgrp"));
        assert_eq!(rec.armed_segments(), [seg(0, 1000), seg(11000, 11000)]);
        assert!(rec.has_armed_time());
    }

    /// (c) `finalize_member` parks the session for the group prompt (never as a single-flight pending),
    /// with stats over the armed segments only — the climb while disarmed does not count.
    #[test]
    fn finalize_member_parks_for_the_group_with_armed_segment_stats() {
        let rig = Rig::new("finalize");
        let kgrp = rig.kgrp("g1");
        let (mut rec, emitted) = rig.recorder("L1:S2", fc("B", None));
        rec.on_gps(&gps(2, 48.1, 11.5));
        rec.on_status(&status(false));
        rec.on_status(&status(true));
        rec.enter_member_mode("g1", &kgrp);
        let path = active_path(&rec);
        let alt = |rec: &mut FlightRecorder, m: f64| rec.on_altitude(&AltitudeData { altitude: m, vario: 0.0 });
        alt(&mut rec, 10.0);
        rig.advance_s(2);
        rec.on_gps(&gps(2, 48.1, 11.5)); // t=2 s, armed, 10 m
        rec.on_status(&status(false)); // disarm at 2 s
        alt(&mut rec, 500.0);
        rig.advance_s(3);
        rec.on_gps(&gps(2, 48.1, 11.5)); // t=5 s, disarmed, 500 m — must not count
        rig.advance_s(1);
        rec.on_status(&status(true)); // arm at 6 s
        alt(&mut rec, 20.0);
        rig.advance_s(4);
        rec.on_gps(&gps(2, 48.1, 11.5)); // t=10 s, armed, 20 m
        rec.on_status(&status(false)); // last disarm at 10 s

        assert!(rec.finalize_member());
        assert!(rec.active_flight.is_none() && rec.membership.is_none());
        assert!(rig.slots.take_pending(None).unwrap().is_none());
        assert!(rig.slots.protected_paths().contains(&path));
        assert_eq!(*emitted.lock().unwrap(), ["flight-recording-started"]);
        let parked = rig.slots.take_group_members("g1");
        assert_eq!(parked.len(), 1);
        let p = &parked[0];
        assert_eq!(p.temp_path, path);
        assert_eq!(p.flight.group_id.as_deref(), Some("g1"));
        assert_eq!(p.membership, Some(GroupMembership { group_id: "g1".into(), kgrp_path: kgrp }));
        assert_eq!(p.armed_segments, [seg(0, 2000), seg(6000, 10000)]);
        assert!(p.has_armed_time());
        assert_eq!(p.flight.duration_sec, Some(6));
        assert_eq!(p.flight.max_alt_m, Some(20.0));
        assert!(rig.slots.take_group_members("g1").is_empty());
        assert!(!rig.slots.protected_paths().contains(&path));
    }

    /// (d) + §5.5: a connected vehicle recorded from the group's first arm but never armed itself has no
    /// armed time (listed unticked). Nothing is announced to the frontend.
    #[test]
    fn unarmed_member_has_no_armed_time() {
        let rig = Rig::new("unarmed");
        let (mut rec, emitted) = rig.recorder("L2:S1", fc("Passive", None));
        rec.on_gps(&gps(2, 48.1, 11.5));
        rec.on_status(&status(false));
        assert!(rec.start_recording_unarmed());
        rec.enter_member_mode("g1", &rig.kgrp("g1"));
        rig.advance_s(5);
        rec.on_gps(&gps(2, 48.2, 11.5));
        assert!(rec.armed_segments().is_empty());
        assert!(!rec.has_armed_time());
        assert!(rec.finalize_member());
        let p = rig.slots.take_group_members("g1").pop().unwrap();
        assert!(!p.has_armed_time());
        assert_eq!(p.flight.duration_sec, Some(0));
        assert_eq!(p.flight.total_distance_m, Some(0.0));
        assert!(emitted.lock().unwrap().is_empty());
        assert!(file_events(&p.temp_path).is_empty());
        assert_eq!(file_rows(&p.temp_path).len(), 1);
    }

    /// (j) §5.6: no fix, no group recording — neither the unarmed start nor a member's own arm opens a
    /// session without a position fix.
    #[test]
    fn start_recording_unarmed_refuses_without_fix() {
        let rig = Rig::new("nofix");
        let (mut rec, _) = rig.recorder("L1:S3", fc("NoFix", None));
        assert!(!rec.start_recording_unarmed()); // no GPS at all
        rec.on_gps(&gps(0, 48.1, 11.5)); // position but no fix
        assert!(!rec.start_recording_unarmed());
        rec.on_gps(&gps(2, 0.0, 0.0)); // fix flag but no position
        assert!(!rec.start_recording_unarmed());
        rec.enter_member_mode("g1", &rig.kgrp("g1"));
        rec.on_status(&status(false));
        rec.on_status(&status(true));
        assert!(rec.active_flight.is_none());
        assert!(!rig.dir.join("sessions").exists() || std::fs::read_dir(rig.dir.join("sessions")).unwrap().next().is_none());
        rec.on_gps(&gps(1, 48.1, 11.5)); // a 2D fix is a position
        assert!(rec.start_recording_unarmed());
        // Already armed when it starts → the session is armed from its first moment.
        assert_eq!(rec.armed_segments(), [seg(0, 0)]);
    }

    /// (h) A member suspended by its link and adopted by the same aircraft's new recorder keeps the real
    /// gap: `timestamp_ms` and `wall_ms` stay one timeline (constant difference) across the outage.
    #[test]
    fn adopt_inside_a_group_keeps_the_wall_clock_gap() {
        let rig = Rig::new("adopt");
        let kgrp = rig.kgrp("g1");
        let (mut a, emitted_a) = rig.recorder("L1:S2", fc("Wing", Some("UID1")));
        a.on_gps(&gps(2, 48.1, 11.5));
        a.on_status(&status(false));
        a.on_status(&status(true));
        a.enter_member_mode("g1", &kgrp);
        let path = active_path(&a);
        rig.advance_s(1);
        a.on_gps(&gps(2, 48.1, 11.5));
        a.shutdown_lost();
        drop(a);
        assert_eq!(*emitted_a.lock().unwrap(), ["flight-recording-started"]); // no recovery prompt
        assert!(rig.slots.take_pending(None).unwrap().is_none());
        assert!(rig.slots.protected_paths().contains(&path));

        rig.advance_s(30);
        // Another aircraft reconnecting first does not take it.
        let (mut other, _) = rig.recorder("L3:S1", fc("Quad", Some("UID9")));
        other.on_status(&status(true));
        assert!(other.membership.is_none());
        // The same FC under a new link key adopts it on its first status.
        let (mut b, emitted_b) = rig.recorder("L2:S2", fc("Wing", Some("UID1")));
        b.on_gps(&gps(2, 48.1, 11.5));
        b.on_status(&status(true));
        assert_eq!(active_path(&b), path);
        assert_eq!(b.membership, Some(GroupMembership { group_id: "g1".into(), kgrp_path: kgrp }));
        rig.advance_s(1);
        b.on_gps(&gps(2, 48.1, 11.5));
        b.on_status(&status(false)); // member: an event, the session goes on
        assert!(emitted_b.lock().unwrap().is_empty());
        assert!(b.finalize_member());

        let events = file_events(&path);
        assert_eq!(kinds(&events), [("arm", 0), ("link_lost", 1000), ("link_back", 31000), ("disarm", 32000)]);
        let rows = file_rows(&path);
        assert_eq!(rows.iter().map(|r| r.timestamp_ms).collect::<Vec<_>>(), [1000, 32000]);
        for r in &rows {
            assert_eq!(r.wall_ms.unwrap() - r.timestamp_ms, rig.base_ms());
        }
        for (_, t, wall) in &events {
            assert_eq!(wall.unwrap() - t, rig.base_ms());
        }
        let p = rig.slots.take_group_members("g1").pop().unwrap();
        // Armed through the outage (it never disarmed), so the 30 s gap counts as armed time.
        assert_eq!(p.armed_segments, [seg(0, 32000)]);
    }

    /// A member that comes back disarmed after disarming during the outage gets that disarm recorded.
    #[test]
    fn adopt_records_an_arm_change_during_the_outage() {
        let rig = Rig::new("adopt-disarmed");
        let (mut a, _) = rig.recorder("L1:S1", fc("Wing", Some("UID1")));
        a.on_gps(&gps(2, 48.1, 11.5));
        a.on_status(&status(true)); // first status armed, nothing to resume → an ordinary arm edge
        a.enter_member_mode("g1", &rig.kgrp("g1"));
        let path = active_path(&a);
        rig.advance_s(2);
        a.shutdown();
        drop(a);
        rig.advance_s(8);
        let (mut b, _) = rig.recorder("L2:S1", fc("Wing", Some("UID1")));
        b.on_status(&status(false));
        assert_eq!(active_path(&b), path);
        assert_eq!(b.armed_segments(), [seg(0, 10000)]);
        assert_eq!(kinds(&file_events(&path)), [("arm", 0), ("link_lost", 2000), ("link_back", 10000), ("disarm", 10000)]);
    }

    /// Builds a `.ktmp` with the current schema: `rows`, `events`, and the given role / group.
    fn write_ktmp(path: &Path, role: Option<&str>, group: Option<&str>, rows: &[TelemetryRecord], events: &[FlightEvent]) {
        db::remove_temp_session(path);
        let conn = db::open_temp_session(path).unwrap();
        let start = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        db::write_session_meta(&conn, &start, "Craft", "INAV", "8.0.0", "TEST", 1, None, "MSP", Some(48.0), Some(11.0))
            .unwrap();
        if let Some(role) = role {
            db::update_session_meta_membership(&conn, "L1:S1", role, group, group.map(|g| format!("group_{g}.kgrp")).as_deref())
                .unwrap();
        }
        db::insert_telemetry_batch(&conn, rows).unwrap();
        for e in events {
            db::insert_session_event(&conn, e).unwrap();
        }
    }

    fn row(ts: i64, lat: f64, alt: f64, mah: u32) -> TelemetryRecord {
        TelemetryRecord { lat: Some(lat), lon: Some(11.0), baro_alt_m: Some(alt), mah_drawn: Some(mah), wall_ms: Some(1_700_000_000_000 + ts), ..record(ts) }
    }

    /// (i) Recovery stats count the armed segments only: rows before the first arm and between a
    /// disarm and the next arm are out, distance restarts per segment, battery use is summed per
    /// segment (the swap reset `mah_drawn`), and the open last segment ends at the last sample.
    #[test]
    fn summarize_counts_two_armed_segments_only() {
        let rig = Rig::new("summarize-segments");
        let path = rig.dir.join("sessions").join("seg.ktmp");
        let rows = [
            row(0, 48.000, 999.0, 50),     // before the arm
            row(1000, 48.001, 10.0, 100),  // segment 1
            row(2000, 48.002, 15.0, 150),
            row(3000, 48.003, 12.0, 200),
            row(4000, 48.100, 800.0, 200), // disarmed (carried away for the battery swap)
            row(6000, 48.101, 20.0, 0),    // segment 2, fresh pack
            row(8000, 48.102, 30.0, 50),
            row(10000, 48.103, 25.0, 120),
        ];
        let events = [event(EVENT_ARM, 1000), event(EVENT_DISARM, 3000), event(EVENT_ARM, 6000)];
        write_ktmp(&path, Some("member"), Some("g1"), &rows, &events);

        let (p, count) = summarize_temp_session(path.clone(), rig.dir.join("flights.db"), rig.clock.as_ref()).unwrap();
        assert_eq!(count, 8);
        assert_eq!(p.armed_segments, [seg(1000, 3000), seg(6000, 10000)]);
        assert_eq!(p.flight.duration_sec, Some(6));
        assert_eq!(p.flight.max_alt_m, Some(30.0));
        assert_eq!(p.flight.battery_used_mah, Some(100 + 120));
        let d = |a: f64, b: f64| haversine_m(a, 11.0, b, 11.0);
        let expected = d(48.001, 48.002) + d(48.002, 48.003) + d(48.101, 48.102) + d(48.102, 48.103);
        assert!((p.flight.total_distance_m.unwrap() - expected).abs() < 1e-6);
        assert!((p.flight.max_distance_m.unwrap() - haversine_m(48.0, 11.0, 48.103, 11.0)).abs() < 1e-6);
        // The group rides along for the commit.
        assert_eq!(p.flight.group_id.as_deref(), Some("g1"));
        assert_eq!(p.membership.as_ref().map(|m| m.kgrp_path.clone()), Some(rig.dir.join("sessions").join("group_g1.kgrp")));
        assert_eq!(p.disarm_instant, rig.clock.instant());
        // The same file as a (new-format) single flight without events: no armed time at all.
        write_ktmp(&path, Some("single"), None, &rows, &[]);
        let (p, _) = summarize_temp_session(path.clone(), rig.dir.join("flights.db"), rig.clock.as_ref()).unwrap();
        assert!(!p.has_armed_time());
        assert_eq!(p.flight.duration_sec, Some(0));
        db::remove_temp_session(&path);
    }

    /// Turns a current-format `.ktmp` into one written before group flights: no `session_events`, no
    /// membership columns, and with `pre_v20` no `wall_ms` either (DROP COLUMN needs SQLite ≥ 3.35 —
    /// the bundled one is newer).
    fn make_legacy(path: &Path, pre_v20: bool) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "DROP TABLE session_events;
             ALTER TABLE session_meta DROP COLUMN vehicle_key;
             ALTER TABLE session_meta DROP COLUMN role;
             ALTER TABLE session_meta DROP COLUMN group_id;
             ALTER TABLE session_meta DROP COLUMN group_file;",
        )
        .unwrap();
        if pre_v20 {
            conn.execute_batch("ALTER TABLE telemetry_records DROP COLUMN wall_ms;").unwrap();
        }
    }

    /// (e) A `.ktmp` from before `session_events` opens, summarizes over its whole span (the pre-group
    /// rule) and commits — a pre-v20 one after the recovery open back-filled it, a v20 one without
    /// `session_events` (written before step 3) even straight from disk.
    #[test]
    fn legacy_ktmp_without_events_opens_summarizes_and_commits() {
        let rig = Rig::new("legacy");
        let sessions = rig.dir.join("sessions");
        let rows = [row(0, 48.0, 5.0, 10), row(1000, 48.001, 40.0, 30), row(4000, 48.002, 20.0, 90)];
        let main = db::open_database(&rig.dir.join("flights.db")).unwrap();

        let recovered = sessions.join("legacy_a.ktmp");
        write_ktmp(&recovered, None, None, &rows, &[]);
        make_legacy(&recovered, true);
        let (p, count) = summarize_temp_session(recovered.clone(), rig.dir.join("flights.db"), rig.clock.as_ref()).unwrap();
        assert_eq!(count, 3);
        assert_eq!(p.armed_segments, [seg(0, 4000)]);
        assert_eq!((p.flight.duration_sec, p.flight.max_alt_m, p.flight.battery_used_mah), (Some(4), Some(40.0), Some(80)));
        assert_eq!(p.flight.group_id, None);
        assert!(file_events(&recovered).is_empty()); // back-filled, empty
        let id = db::commit_session_to_main(&main, &recovered, &p.flight).unwrap();
        assert_eq!(db::read_flight_track(&main, id, None).unwrap().len(), 3);
        assert!(db::get_flight_events(&main, id).unwrap().is_empty());

        let untouched = sessions.join("legacy_b.ktmp");
        write_ktmp(&untouched, None, None, &rows, &[]);
        make_legacy(&untouched, false);
        let id = db::commit_session_to_main(&main, &untouched, &p.flight).unwrap();
        assert_eq!(db::read_flight_track(&main, id, None).unwrap().len(), 3);
        assert_eq!(db::get_flight(&main, id).unwrap().unwrap().group_id, None);
    }

    /// (f) The commit copies the session's events into `flight_events` under the new flight id and sets
    /// `flights.group_id` from `session_meta`; a group id whose row is missing fails the commit.
    #[test]
    fn commit_copies_events_and_sets_group_id_only_for_a_known_group() {
        let rig = Rig::new("commit-group");
        let path = rig.dir.join("sessions").join("member.ktmp");
        let rows = [row(0, 48.0, 5.0, 10), row(1000, 48.001, 6.0, 20)];
        let events = [event(EVENT_ARM, 0), event(EVENT_DISARM, 800), event(EVENT_LINK_LOST, 900)];
        write_ktmp(&path, Some("member"), Some("g1"), &rows, &events);
        let (p, _) = summarize_temp_session(path.clone(), rig.dir.join("flights.db"), rig.clock.as_ref()).unwrap();
        let main = db::open_database(&rig.dir.join("flights.db")).unwrap();

        let err = db::commit_session_to_main(&main, &path, &p.flight).unwrap_err();
        assert!(err.to_string().contains("group flight g1"), "{err}");
        let n: i64 = main.query_row("SELECT COUNT(*) FROM flights", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);

        db::insert_flight_group(&main, &test_group("g1")).unwrap();
        // session_meta wins over the Flight passed in.
        let mut flight = p.flight.clone();
        flight.group_id = None;
        let id = db::commit_session_to_main(&main, &path, &flight).unwrap();
        assert_eq!(db::get_flight(&main, id).unwrap().unwrap().group_id.as_deref(), Some("g1"));
        let copied = db::get_flight_events(&main, id).unwrap();
        assert_eq!(
            copied.iter().map(|e| (e.flight_id, e.kind.as_str(), e.timestamp_ms, e.wall_ms)).collect::<Vec<_>>(),
            [
                (id, "arm", 0, Some(1_700_000_000_000)),
                (id, "disarm", 800, Some(1_700_000_000_800)),
                (id, "link_lost", 900, Some(1_700_000_000_900)),
            ]
        );
        assert_eq!(copied[0].source.as_deref(), Some("live"));
        assert_eq!(db::list_flight_group_member_ids(&main, "g1").unwrap(), [id]);
    }
}
