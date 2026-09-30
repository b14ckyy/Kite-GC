// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Group-flight coordinator (Dev-Docs active/GROUP_FLIGHTS.md §3.2, step 5): ties the flights of the
//! vehicles that fly at the same time into one group flight.
//!
//! Protocol-agnostic: it knows recorders by vehicle key (`"L1:S2"`) through `register` and learns arm,
//! disarm, first status and teardown from the recorders themselves — never a MAVLink or MSP detail.
//!
//! Rules (Marc, GROUP_FLIGHTS.md §1, §5, §5b): with the fleet feature enabled
//! (`AppState::fleet_enabled`) and at least two recorders registered, the first arm WITH a 3D fix
//! starts a group. From then on every registered recorder records (member mode; an unarmed session
//! where none runs), the group starts at the earliest arm among the recordings already running, and it
//! ends when the last armed member has disarmed and nothing re-armed within `GROUP_END_GRACE` — or,
//! when the last armed member was lost (its link went away while armed, or its recorder reported no
//! status for `MEMBER_STALL`), after `LOST_ARMED_TIMEOUT` without an arm. Then every member is finalized
//! for the group's store prompt (`PendingGroup`, `group-flight-ended`).
//!
//! Lock rule (deadlock-free by construction): recorder lock → coordinator lock, never the reverse. The
//! coordinator never locks a recorder while it holds its own lock; the recorder-facing calls answer with
//! a `GroupDirective` the calling recorder applies to itself. Work on other recorders — members that get
//! no status, the finalization at the group end — runs in `tick` (the worker thread's, or a test's),
//! which snapshots their `Weak` handles under the coordinator lock, releases it and then locks one
//! recorder at a time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use sha1::{Digest, Sha1};

use super::group_store::{self, GroupFileHeader, GroupFileMember};
use super::recorder::{
    Clock, FlightRecorder, FlightRecorderHandle, GroupMembership, PendingSession, SessionSlotsHandle, REARM_GRACE,
};

/// A group ends this long after its last armed member disarmed, unless a member re-arms (§5.1: the
/// single-flight re-arm grace applies to the group end).
pub const GROUP_END_GRACE: Duration = REARM_GRACE;

/// A group whose last armed member was LOST (link gone while armed) stays open this long for it — or
/// for any other arm — before it ends (§5.2).
pub const LOST_ARMED_TIMEOUT: Duration = Duration::from_secs(60);

/// An ARMED member whose recorder reported no status for longer than this counts as lost-armed from
/// its last status (§5.2 — a stalled link keeps its recorder, so no teardown ever reports the loss);
/// its next status clears it.
pub const MEMBER_STALL: Duration = Duration::from_secs(5);

/// Worker wake-up period while a group runs or ends (group-end timers, members without status).
const TICK_WITH_GROUP: Duration = Duration::from_millis(250);
/// Worker wake-up period without a group (only prunes dead recorder handles).
const TICK_IDLE: Duration = Duration::from_secs(30);

/// What the calling recorder does after reporting to the coordinator.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupDirective {
    /// Nothing changes.
    Stay,
    /// Become a member of this running group (`enter_member_mode`, and record from now on:
    /// `start_recording_unarmed` where no session runs).
    Join(GroupMembership),
    /// The group this recorder is a member of has ended: `finalize_member`.
    Leave,
}

/// A single-flight arm, reported once its session runs (`FlightRecorder::on_arm`).
#[derive(Debug, Clone)]
pub struct ArmReport {
    /// A 3D fix with a valid position (`FlightRecorder::has_3d_fix`) — needed by the group initiator.
    pub has_3d_fix: bool,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// GPS MSL altitude.
    pub alt_m: Option<f64>,
    /// Start (UTC) of the session now running — its original start after a re-arm within grace.
    pub session_start: DateTime<Utc>,
}

/// A recorder's state right after its first status (adoption and continue-on-reconnect are settled).
#[derive(Debug, Clone)]
pub struct FirstStatusReport {
    pub armed: bool,
    /// The group of an adopted member session (the same aircraft back over a new link).
    pub member_of: Option<String>,
    /// The session's file, when one runs.
    pub file: Option<MemberFile>,
    pub session_start: Option<DateTime<Utc>>,
}

/// A member's temp file and identity, as listed in the group's `.kgrp`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberFile {
    pub temp_path: PathBuf,
    pub craft_name: String,
    pub fc_variant: String,
    pub fc_uid: Option<String>,
}

/// Where the coordinator's events go: the app handle in production (`AppSink`), a capturing sink in tests.
pub trait GroupEventSink: Send + Sync {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

/// Emits the group events app-wide (they belong to no single vehicle).
pub struct AppSink(pub tauri::AppHandle);

impl GroupEventSink for AppSink {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        if let Err(e) = self.0.emit(event, payload) {
            log::warn!("Failed to emit {}: {}", event, e);
        }
    }
}

/// `group-flight-started` payload.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupStartedEvent {
    group_id: String,
    /// Group start, RFC 3339 UTC (the earliest arm among the recordings running at the start).
    start_time: String,
    /// Vehicle keys of the recorders taken into the group.
    members: Vec<String>,
}

/// One member flight of an ended group (`group-flight-ended`, `PendingGroup`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberSummary {
    /// The member's temp file name — how the store prompt (step 7/8) addresses it.
    pub member_id: String,
    /// Vehicle key of the recorder that recorded it last (empty when unknown).
    pub vehicle_id: String,
    pub craft_name: String,
    pub fc_variant: String,
    /// Armed at any point — members without armed time are listed unticked (§5.5).
    pub has_armed_time: bool,
    /// Lost while armed and never came back: the flight may have gone on unrecorded.
    pub incomplete: bool,
    /// Armed time (the member's stats count its armed segments only, §3.8).
    pub armed_sec: i64,
    pub max_alt_m: f64,
    pub max_speed_ms: f64,
    pub max_distance_m: f64,
    pub total_distance_m: f64,
    pub battery_used_mah: Option<u32>,
    #[serde(skip)]
    pub temp_path: PathBuf,
}

/// `group-flight-ended` payload; also the summary of a pending group.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupEndedEvent {
    pub group_id: String,
    pub start_time: String,
    pub end_time: String,
    pub members: Vec<GroupMemberSummary>,
}

/// An ended group awaiting its store prompt (GROUP_FLIGHTS.md §3.4, step 7). Its member sessions stay
/// in `SessionSlots::group_pending` (protected) until the commit/discard command takes them
/// (`SessionSlots::take_group_members`).
#[derive(Debug, Clone)]
pub struct PendingGroup {
    pub id: String,
    pub header: GroupFileHeader,
    /// Removed by the store prompt once every member is committed or discarded (§3.4).
    #[allow(dead_code)] // the commit / discard commands: GROUP_FLIGHTS.md step 7
    pub kgrp_path: PathBuf,
    /// As of `finish` (the `group-flight-ended` payload); `take_pending_group` rebuilds it from the slots.
    pub members: Vec<GroupMemberSummary>,
    /// The group's members with a file, as the coordinator knew them at the end (vehicle key, lost).
    roster: Vec<RosterEntry>,
}

/// A member of an ended group as the coordinator knew it — to name the finalized sessions in the slots.
#[derive(Debug, Clone)]
struct RosterEntry {
    temp_path: PathBuf,
    key: String,
    /// Lost (or stalled) while armed and never came back.
    incomplete: bool,
}

impl PendingGroup {
    fn summary(&self) -> GroupEndedEvent {
        GroupEndedEvent {
            group_id: self.id.clone(),
            start_time: self.header.start_time.to_rfc3339(),
            end_time: self.header.end_time.unwrap_or(self.header.start_time).to_rfc3339(),
            members: self.members.clone(),
        }
    }
}

/// Read-only snapshot of the coordinator for the Debug Monitor's "Fleet (gated)" tab
/// (`debug_group_state`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDebugState {
    /// The fleet feature flag as the coordinator sees it.
    pub enabled: bool,
    /// Registered recorders, by key.
    pub recorders: Vec<DebugRecorder>,
    pub group: Option<DebugGroup>,
    /// Ids of the ended groups awaiting their store prompt, oldest first.
    pub pending: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugRecorder {
    pub key: String,
    pub first_status_seen: bool,
    pub armed: bool,
    /// Time since its last status (ms).
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugGroup {
    pub id: String,
    /// `running` | `ending`.
    pub phase: &'static str,
    /// Group start, RFC 3339 UTC.
    pub start_time: String,
    /// Time left until the group ends (ms, 0 once due); `None` while no end timer runs.
    pub end_in_ms: Option<u64>,
    pub members: Vec<DebugMember>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugMember {
    pub key: String,
    pub armed: bool,
    pub gone: bool,
    pub lost_armed: bool,
    pub stalled: bool,
    /// The member's temp file name (`None` before it reported one).
    pub file: Option<String>,
}

/// The group id (§3.1): the first 16 hex chars of `sha1("{start_ms}|{lat_e7}|{lon_e7}")` — group start as
/// UTC epoch ms, position in 1e-7 degrees (rounded). Plain ASCII, reproducible anywhere; not security.
pub fn group_id_for(start: DateTime<Utc>, lat: f64, lon: f64) -> String {
    let input = format!(
        "{}|{}|{}",
        start.timestamp_millis(),
        (lat * 1e7).round() as i64,
        (lon * 1e7).round() as i64
    );
    Sha1::digest(input.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A registered recorder.
struct Registered {
    recorder: Weak<Mutex<FlightRecorder>>,
    /// Where its temp files live (the `.kgrp` of a group it starts goes there too).
    sessions_dir: PathBuf,
    /// Last reported arm state.
    armed: bool,
    /// Start of its running session (single flight or member), `None` without one.
    session_start: Option<DateTime<Utc>>,
    /// Its first status has been handled (adoption / continue-on-reconnect settled) — only then may it
    /// be given an unarmed member session.
    first_status_seen: bool,
    /// When it last reported a status (registration counts as one) — an armed member silent for
    /// `MEMBER_STALL` counts as lost-armed.
    last_seen: Instant,
    /// The "other sessions folder, not taken into the group" warning was logged for it.
    warned_dir: bool,
}

/// A member of the running group, by the vehicle key that records it now.
struct Member {
    key: String,
    /// Its temp file, once it reported one (`member_recording`).
    file: Option<MemberFile>,
    joined_at: DateTime<Utc>,
    armed: bool,
    /// Its recorder was torn down (suspended session, or none).
    gone: bool,
    /// Torn down while armed, not back yet.
    lost_armed: bool,
    /// Armed, but its recorder reported no status for `MEMBER_STALL` — counts as lost-armed until its
    /// next status.
    stalled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    /// Decided to end; `tick` finalizes the members.
    Ending,
}

struct Group {
    header: GroupFileHeader,
    kgrp_path: PathBuf,
    members: Vec<Member>,
    phase: Phase,
    /// The group ends at this instant unless a member arms first.
    end_deadline: Option<Instant>,
    /// When the last armed member was lost.
    last_armed_loss: Option<Instant>,
}

impl Group {
    fn membership(&self) -> GroupMembership {
        GroupMembership { group_id: self.header.id.clone(), kgrp_path: self.kgrp_path.clone() }
    }

    fn running(&self, id: &str) -> bool {
        self.phase == Phase::Running && self.header.id == id
    }

    fn member_mut(&mut self, key: &str) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.key == key)
    }

    /// Whether `key` records into the group (joined and reported its file).
    fn is_joined(&self, key: &str) -> bool {
        self.members.iter().any(|m| m.key == key && m.file.is_some())
    }

    fn present_armed(&self) -> bool {
        self.members.iter().any(|m| !m.gone && m.armed && !m.stalled)
    }

    /// Whether a recorder with temp files in `sessions_dir` may join: its files must live next to the
    /// group's `.kgrp` (the store prompt finds the members there).
    fn same_dir(&self, sessions_dir: &Path) -> bool {
        self.kgrp_path.parent() == Some(sessions_dir)
    }

    /// `key` joins with a session that began before the group start: the group starts there instead
    /// (Marc, 2026-10-01 — e.g. a vehicle back via continue-on-reconnect whose flight began before the
    /// group formed). The id stays the one formed at the start; the `.kgrp` header is rewritten.
    fn pull_start(&mut self, key: &str, session_start: Option<DateTime<Utc>>) {
        let Some(start) = session_start.filter(|s| *s < self.header.start_time) else { return };
        log::info!(
            "Group flight {}: {} joins with a session from {} — the group start moves back from {}",
            self.header.id,
            key,
            start.to_rfc3339(),
            self.header.start_time.to_rfc3339()
        );
        self.header.start_time = start;
        write_kgrp(&self.kgrp_path, |c| group_store::write_group_header(c, &self.header));
    }

    /// List `key` as a member (or update its arm state). An armed member stops the end timer.
    fn add_member(&mut self, key: &str, armed: bool, now_utc: DateTime<Utc>) {
        match self.member_mut(key) {
            Some(m) => {
                m.armed = armed;
                m.gone = false;
            }
            None => self.members.push(Member {
                key: key.to_string(),
                file: None,
                joined_at: now_utc,
                armed,
                gone: false,
                lost_armed: false,
                stalled: false,
            }),
        }
        if armed {
            self.end_deadline = None;
        }
    }

    /// A member disarmed: with no member armed any more, the group ends after the grace — or, while a
    /// lost member may still be flying, not before the lost-member timeout.
    fn after_disarm(&mut self, now: Instant) {
        if self.present_armed() {
            return;
        }
        let mut deadline = now + GROUP_END_GRACE;
        if self.members.iter().any(|m| m.lost_armed || m.stalled) {
            if let Some(loss) = self.last_armed_loss {
                deadline = deadline.max(loss + LOST_ARMED_TIMEOUT);
            }
        }
        self.end_deadline = Some(deadline);
    }

    /// An armed member was lost at `now` (a stall reports its last status, an earlier instant): when it
    /// was the last armed one, the group stays open for it.
    fn after_armed_loss(&mut self, now: Instant) {
        self.last_armed_loss = Some(self.last_armed_loss.map_or(now, |l| l.max(now)));
        if !self.present_armed() {
            let deadline = now + LOST_ARMED_TIMEOUT;
            self.end_deadline = Some(self.end_deadline.map_or(deadline, |d| d.max(deadline)));
        }
    }

    /// `key`'s recorder is gone (torn down or dropped).
    fn member_gone(&mut self, key: &str, armed: bool, now: Instant) {
        let kgrp = self.kgrp_path.clone();
        let Some(m) = self.member_mut(key) else { return };
        m.gone = true;
        m.armed = false;
        m.lost_armed = armed;
        m.stalled = false;
        if let Some(f) = &m.file {
            let path = f.temp_path.clone();
            write_kgrp(&kgrp, |c| group_store::set_group_member_state(c, &path, group_store::MEMBER_SUSPENDED).map(|_| ()));
        }
        if armed {
            log::warn!(
                "Group flight {}: {} lost while armed — the group stays open up to {} s for it",
                self.header.id,
                key,
                LOST_ARMED_TIMEOUT.as_secs()
            );
            self.after_armed_loss(now);
        } else {
            log::info!("Group flight {}: {} went away disarmed — it stays a member", self.header.id, key);
        }
    }
}

#[derive(Default)]
struct Inner {
    registered: HashMap<String, Registered>,
    group: Option<Group>,
    pending: Vec<PendingGroup>,
    /// The "no 3D fix" warning was logged since the last group start.
    warned_no_fix: bool,
    /// Work for the worker (a group started or is ending).
    wake: bool,
}

/// The group-flight coordinator (one per app, `AppState::groups`).
pub struct GroupCoordinator {
    inner: Mutex<Inner>,
    wake: Condvar,
    slots: SessionSlotsHandle,
    clock: Arc<dyn Clock>,
    /// The fleet feature flag (`AppState::fleet_enabled`): off → never a group.
    enabled: Arc<AtomicBool>,
    sink: OnceLock<Box<dyn GroupEventSink>>,
}

impl GroupCoordinator {
    pub fn new(slots: SessionSlotsHandle, clock: Arc<dyn Clock>, enabled: Arc<AtomicBool>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            wake: Condvar::new(),
            slots,
            clock,
            enabled,
            sink: OnceLock::new(),
        }
    }

    /// Where the group events go (set once; later calls are ignored).
    pub fn set_sink(&self, sink: Box<dyn GroupEventSink>) {
        if self.sink.set(sink).is_err() {
            log::debug!("Group coordinator: event sink already set");
        }
    }

    /// Set the event sink and start the worker thread that drives the group-end timers and the work on
    /// recorders that report nothing themselves (app setup).
    pub fn start(self: &Arc<Self>, sink: Box<dyn GroupEventSink>) {
        self.set_sink(sink);
        let weak = Arc::downgrade(self);
        let spawned = std::thread::Builder::new().name("group-flights".into()).spawn(move || loop {
            let Some(coordinator) = weak.upgrade() else { return };
            coordinator.wait_for_work();
            coordinator.tick();
        });
        if let Err(e) = spawned {
            log::error!("Group coordinator: worker thread failed to start: {}", e);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify_worker(&self, inner: &mut Inner) {
        inner.wake = true;
        self.wake.notify_all();
    }

    fn wait_for_work(&self) {
        let inner = self.lock();
        if inner.wake {
            return;
        }
        let timeout = if inner.group.is_some() { TICK_WITH_GROUP } else { TICK_IDLE };
        let _ = self.wake.wait_timeout(inner, timeout);
    }

    fn emit<S: serde::Serialize>(&self, event: &str, payload: &S) {
        match serde_json::to_value(payload) {
            Ok(value) => match self.sink.get() {
                Some(sink) => sink.emit_json(event, value),
                None => log::debug!("{} not emitted: no event sink", event),
            },
            Err(e) => log::warn!("Failed to serialise {}: {}", event, e),
        }
    }

    /// Register a recorder (on creation). Recorders without DB recording are not taken: a group member
    /// is a temp file. A recorder registered while a group runs joins it on its first status (after its
    /// own adoption / continue-on-reconnect decision, so a returning member continues its file).
    pub fn register(self: &Arc<Self>, recorder: &FlightRecorderHandle) {
        let (key, sessions_dir) = {
            let Ok(mut rec) = recorder.lock() else {
                log::warn!("Group coordinator: a recorder's lock is poisoned — not registered");
                return;
            };
            if !rec.records_to_db() {
                return;
            }
            rec.attach_coordinator(self.clone());
            (rec.vehicle_key().to_string(), rec.sessions_dir())
        };
        let mut inner = self.lock();
        inner.registered.insert(
            key.clone(),
            Registered {
                recorder: Arc::downgrade(recorder),
                sessions_dir,
                armed: false,
                session_start: None,
                first_status_seen: false,
                last_seen: self.clock.instant(),
                warned_dir: false,
            },
        );
        log::info!("Group coordinator: {} registered ({} recorders)", key, inner.registered.len());
    }

    /// `key` reported a status at `now`: stamp it; a stalled member is back (armed, it stops the end
    /// timer — a disarm in the same status sets it again).
    fn seen(inner: &mut Inner, key: &str, now: Instant) {
        let Inner { registered, group, .. } = inner;
        if let Some(r) = registered.get_mut(key) {
            r.last_seen = now;
        }
        let Some(g) = group.as_mut().filter(|g| g.phase == Phase::Running) else { return };
        let id = g.header.id.clone();
        let Some(m) = g.member_mut(key).filter(|m| m.stalled) else { return };
        m.stalled = false;
        let armed = m.armed;
        log::info!("Group flight {}: {} reports again", id, key);
        if armed {
            g.end_deadline = None;
        }
    }

    /// Advance the group-end timer: an armed member silent for `MEMBER_STALL` counts as lost-armed
    /// from its last status; with no member armed and no end timer (a state no path should leave), the
    /// timer starts now; past its deadline with no member armed, the group starts ending (the worker
    /// finalizes the members; a member's own next sync does the same for itself).
    fn advance_timers(&self, inner: &mut Inner, now: Instant) {
        let Inner { registered, group, .. } = &mut *inner;
        let Some(group) = group.as_mut() else { return };
        if group.phase != Phase::Running {
            return;
        }
        let stalls: Vec<(String, Instant)> = group
            .members
            .iter()
            .filter(|m| !m.gone && m.armed && !m.stalled)
            .filter_map(|m| {
                let r = registered.get(&m.key)?;
                (now.saturating_duration_since(r.last_seen) > MEMBER_STALL).then(|| (m.key.clone(), r.last_seen))
            })
            .collect();
        for (key, last_seen) in stalls {
            if let Some(m) = group.member_mut(&key) {
                m.stalled = true;
            }
            log::warn!(
                "Group flight {}: {} armed but silent for {} s — counts as lost while armed until its next status",
                group.header.id,
                key,
                now.saturating_duration_since(last_seen).as_secs()
            );
            group.after_armed_loss(last_seen);
        }
        if group.present_armed() {
            return;
        }
        if group.end_deadline.is_none() {
            log::warn!("Group flight {}: no member armed and no end timer — the group-end timer starts now", group.header.id);
            group.after_disarm(now);
        }
        if group.end_deadline.is_some_and(|d| now >= d) {
            log::info!("Group flight {}: no member armed any more — ending", group.header.id);
            group.phase = Phase::Ending;
            write_kgrp(&group.kgrp_path.clone(), |c| {
                let mut h = group.header.clone();
                h.state = group_store::GROUP_ENDING.into();
                group_store::write_group_header(c, &h)
            });
            self.notify_worker(inner);
        }
    }

    /// The directive for `key` (a member of `member_of`, or of nothing): stay; join the running group
    /// (first status seen, no single flight of its own awaiting its End-Flight dialog); or leave a group
    /// that has ended.
    fn directive(&self, inner: &mut Inner, key: &str, member_of: Option<&str>) -> GroupDirective {
        let Inner { registered, group, .. } = inner;
        match (member_of, group.as_mut()) {
            (Some(id), Some(g)) if g.running(id) => GroupDirective::Stay,
            (Some(_), _) => {
                if let Some(r) = registered.get_mut(key) {
                    r.session_start = None; // it finalizes its member session now
                }
                GroupDirective::Leave
            }
            (None, Some(g)) if g.phase == Phase::Running => {
                let Some(r) = registered.get_mut(key) else { return GroupDirective::Stay };
                if !r.first_status_seen || self.slots.has_pending_for(key) || !dir_ok(r, key, g) {
                    return GroupDirective::Stay;
                }
                g.add_member(key, r.armed, self.clock.utc());
                g.pull_start(key, r.session_start);
                GroupDirective::Join(g.membership())
            }
            _ => GroupDirective::Stay,
        }
    }

    /// Every status after the first (recorder lock held; `from_status` false: the worker's `tick` for a
    /// recorder that got no status — it does not count as one): follow the group, advance the timers.
    pub(crate) fn sync(&self, key: &str, member_of: Option<&str>, from_status: bool) -> GroupDirective {
        let now = self.clock.instant();
        let mut inner = self.lock();
        self.advance_timers(&mut inner, now);
        if from_status {
            Self::seen(&mut inner, key, now);
        }
        self.directive(&mut inner, key, member_of)
    }

    /// The recorder's first status has been handled.
    pub(crate) fn on_first_status(&self, key: &str, report: FirstStatusReport) -> GroupDirective {
        let now = self.clock.instant();
        let now_utc = self.clock.utc();
        let mut inner = self.lock();
        self.advance_timers(&mut inner, now);
        Self::seen(&mut inner, key, now);
        {
            let Inner { registered, group, .. } = &mut *inner;
            if let Some(r) = registered.get_mut(key) {
                r.first_status_seen = true;
                r.armed = report.armed;
                r.session_start = report.session_start;
            }
            // An adopted member session: the same aircraft is back (under a new key) and records on.
            if let (Some(id), Some(g)) = (report.member_of.as_deref(), group.as_mut()) {
                if g.running(id) {
                    let path = report.file.as_ref().map(|f| f.temp_path.clone());
                    let idx = g.members.iter().position(|m| {
                        m.key == key || (path.is_some() && m.file.as_ref().map(|f| &f.temp_path) == path.as_ref())
                    });
                    let m = match idx {
                        Some(i) => &mut g.members[i],
                        None => {
                            g.members.push(Member {
                                key: key.to_string(),
                                file: None,
                                joined_at: now_utc,
                                armed: false,
                                gone: false,
                                lost_armed: false,
                                stalled: false,
                            });
                            g.members.last_mut().expect("just pushed")
                        }
                    };
                    // Its suspended file was reopened (the member listed with it), or it records on into
                    // a new member file (the reopen failed).
                    let adopted = path.is_some() && m.file.as_ref().map(|f| &f.temp_path) == path.as_ref();
                    if adopted {
                        log::info!("Group flight {}: {} continues as {} (link back)", id, m.key, key);
                    } else {
                        log::info!("Group flight {}: {} is back as {} with a new member file", id, m.key, key);
                    }
                    m.key = key.to_string();
                    m.gone = false;
                    m.lost_armed = false;
                    m.stalled = false;
                    m.armed = report.armed;
                    if report.file.is_some() {
                        m.file = report.file.clone();
                    }
                    if let Some(f) = m.file.clone() {
                        let row = kgrp_member(key, &f, m.joined_at, group_store::MEMBER_RECORDING);
                        write_kgrp(&g.kgrp_path, |c| group_store::upsert_group_member(c, &row));
                    }
                    if report.armed {
                        g.end_deadline = None;
                    } else {
                        g.after_disarm(now);
                    }
                }
            }
        }
        self.directive(&mut inner, key, report.member_of.as_deref())
    }

    /// A single-flight arm (its session runs): forms a group, joins the running one, or changes nothing.
    pub(crate) fn on_arm(&self, key: &str, report: ArmReport) -> GroupDirective {
        let now = self.clock.instant();
        let (directive, started) = {
            let mut inner = self.lock();
            self.advance_timers(&mut inner, now);
            Self::seen(&mut inner, key, now);
            if let Some(r) = inner.registered.get_mut(key) {
                r.armed = true;
                r.session_start = Some(report.session_start);
            }
            let now_utc = self.clock.utc();
            let Inner { registered, group, .. } = &mut *inner;
            let other_dir = match (group.as_ref(), registered.get_mut(key)) {
                (Some(g), Some(r)) if g.phase == Phase::Running => !dir_ok(r, key, g),
                _ => false,
            };
            match inner.group.as_mut() {
                Some(_) if other_dir => (GroupDirective::Stay, None),
                Some(g) if g.phase == Phase::Running => {
                    g.add_member(key, true, now_utc);
                    g.pull_start(key, Some(report.session_start));
                    (GroupDirective::Join(g.membership()), None)
                }
                // Ending: arms start single flights until the ended group is finalized.
                Some(_) => (GroupDirective::Stay, None),
                None => self.try_form(&mut inner, key, &report),
            }
        };
        if let Some(ev) = started {
            self.emit("group-flight-started", &ev);
        }
        directive
    }

    /// Start a group on `key`'s arm when the rules allow it (flag, ≥ 2 recorders, 3D fix, ≥ 2 recorders
    /// without a pending single flight and with their temp files in the initiator's sessions folder).
    /// The group's position — and with it the id — is the INITIATOR's (Marc, 2026-10-01): its arm with a
    /// 3D fix anchors the group, even when the start is an earlier arm of another member.
    fn try_form(&self, inner: &mut Inner, key: &str, report: &ArmReport) -> (GroupDirective, Option<GroupStartedEvent>) {
        let none = (GroupDirective::Stay, None);
        if !self.enabled.load(Ordering::Relaxed) {
            return none;
        }
        let live = inner.registered.values().filter(|r| r.recorder.strong_count() > 0).count();
        if live < 2 {
            return none;
        }
        let position = match (report.has_3d_fix, report.lat, report.lon) {
            (true, Some(lat), Some(lon)) => (lat, lon),
            _ => {
                if !inner.warned_no_fix {
                    log::warn!(
                        "Group flight not started: {} armed without a 3D GPS fix — the first arm of a group needs one; a later arm with a fix starts it",
                        key
                    );
                    inner.warned_no_fix = true;
                }
                return none;
            }
        };
        // A recorder whose single flight awaits its End-Flight dialog is not taken (§4 test f); it joins
        // once that is resolved. Nor is one whose temp files live in another sessions folder than the
        // initiator's (where the `.kgrp` goes): it records single flights.
        let sessions_dir = inner.registered.get(key).map(|r| r.sessions_dir.clone()).unwrap_or_default();
        let mut members: Vec<String> = inner
            .registered
            .iter()
            .filter(|(k, r)| {
                r.recorder.strong_count() > 0
                    && (k.as_str() == key || (!self.slots.has_pending_for(k) && r.sessions_dir == sessions_dir))
            })
            .map(|(k, _)| k.clone())
            .collect();
        members.sort();
        if members.len() < 2 {
            log::info!(
                "Group flight not started on {}'s arm: the other recorders have a finished flight awaiting its End-Flight dialog",
                key
            );
            return none;
        }
        let start = members
            .iter()
            .filter_map(|k| inner.registered.get(k).and_then(|r| r.session_start))
            .min()
            .unwrap_or(report.session_start);
        let id = group_id_for(start, position.0, position.1);
        let kgrp_path = group_store::group_file_path(&sessions_dir, &id);
        for (k, r) in inner.registered.iter_mut() {
            if r.sessions_dir != sessions_dir && !r.warned_dir && r.recorder.strong_count() > 0 {
                r.warned_dir = true;
                log::warn!(
                    "Group flight {}: {} records to another sessions folder ({}) than the group ({}) — not taken, it records single flights",
                    id,
                    k,
                    r.sessions_dir.display(),
                    sessions_dir.display()
                );
            }
        }
        let header = GroupFileHeader {
            id: id.clone(),
            start_time: start,
            end_time: None,
            start_lat: Some(position.0),
            start_lon: Some(position.1),
            start_alt_m: report.alt_m,
            utc_offset_min: Some(super::timezone::local_offset_min_now()),
            notes: None,
            state: group_store::GROUP_RUNNING.into(),
        };
        write_kgrp(&kgrp_path, |c| group_store::write_group_header(c, &header));
        let now_utc = self.clock.utc();
        let group = Group {
            header,
            kgrp_path,
            members: members
                .iter()
                .map(|k| Member {
                    key: k.clone(),
                    file: None,
                    joined_at: now_utc,
                    armed: k == key || inner.registered.get(k).is_some_and(|r| r.armed),
                    gone: false,
                    lost_armed: false,
                    stalled: false,
                })
                .collect(),
            phase: Phase::Running,
            end_deadline: None,
            last_armed_loss: None,
        };
        log::info!(
            "Group flight {} started by {}: {} members, start {}",
            id,
            key,
            members.len(),
            start.to_rfc3339()
        );
        let membership = group.membership();
        inner.group = Some(group);
        inner.warned_no_fix = false;
        self.notify_worker(inner);
        let ev = GroupStartedEvent { group_id: id, start_time: start.to_rfc3339(), members };
        (GroupDirective::Join(membership), Some(ev))
    }

    /// A member arms (before it records the event). `false`: its group is no longer running — it
    /// finalizes its member session, and the arm starts a new flight.
    pub(crate) fn on_member_arm(&self, key: &str, group_id: &str) -> bool {
        let now = self.clock.instant();
        let now_utc = self.clock.utc();
        let mut inner = self.lock();
        self.advance_timers(&mut inner, now);
        Self::seen(&mut inner, key, now);
        let Inner { registered, group, .. } = &mut *inner;
        match group.as_mut() {
            Some(g) if g.running(group_id) => {
                if let Some(r) = registered.get_mut(key) {
                    r.armed = true;
                }
                g.add_member(key, true, now_utc);
                true
            }
            _ => {
                if let Some(r) = registered.get_mut(key) {
                    r.session_start = None;
                }
                false
            }
        }
    }

    /// A single flight disarms. While a group runs, it takes the session over (`Join`: the recorder
    /// records on as a member) instead of letting it end — every connected vehicle records (§5.5).
    pub(crate) fn on_disarm(&self, key: &str) -> GroupDirective {
        let now = self.clock.instant();
        let now_utc = self.clock.utc();
        let mut inner = self.lock();
        self.advance_timers(&mut inner, now);
        Self::seen(&mut inner, key, now);
        let Inner { registered, group, .. } = &mut *inner;
        if let Some(r) = registered.get_mut(key) {
            r.armed = false;
        }
        let joinable = match (group.as_ref(), registered.get_mut(key)) {
            (Some(g), Some(r)) if g.phase == Phase::Running => dir_ok(r, key, g),
            _ => false,
        };
        match group.as_mut() {
            Some(g) if joinable => {
                g.add_member(key, false, now_utc);
                g.after_disarm(now);
                g.pull_start(key, registered.get(key).and_then(|r| r.session_start));
                GroupDirective::Join(g.membership())
            }
            _ => {
                if let Some(r) = registered.get_mut(key) {
                    r.session_start = None;
                }
                GroupDirective::Stay
            }
        }
    }

    /// A member disarms (its session records on).
    pub(crate) fn on_member_disarm(&self, key: &str, group_id: &str) {
        let now = self.clock.instant();
        let mut inner = self.lock();
        Self::seen(&mut inner, key, now);
        let Inner { registered, group, .. } = &mut *inner;
        if let Some(r) = registered.get_mut(key) {
            r.armed = false;
        }
        if let Some(g) = group.as_mut().filter(|g| g.running(group_id)) {
            if let Some(m) = g.member_mut(key) {
                m.armed = false;
            }
            g.after_disarm(now);
        }
    }

    /// A member records into `file` (after joining): list it in the group's `.kgrp`.
    pub(crate) fn member_recording(&self, key: &str, group_id: &str, file: MemberFile) {
        let now_utc = self.clock.utc();
        let mut inner = self.lock();
        let Some(g) = inner.group.as_mut().filter(|g| g.header.id == group_id) else { return };
        if g.member_mut(key).is_none() {
            g.add_member(key, false, now_utc);
        }
        let Some(m) = g.member_mut(key) else { return };
        m.file = Some(file.clone());
        let row = kgrp_member(key, &file, m.joined_at, group_store::MEMBER_RECORDING);
        let kgrp = g.kgrp_path.clone();
        write_kgrp(&kgrp, |c| group_store::upsert_group_member(c, &row));
    }

    /// A recorder is torn down (link closed or lost) — it unregisters. A member stays a member: a
    /// disarmed one does not end the group; an armed one counts as "armed but lost" (§5.2). The session
    /// of a member whose group is ending or over (suspended just now) goes to the group's finalized
    /// members, so a later connection of the same aircraft cannot adopt it into a finished group.
    pub(crate) fn on_teardown(&self, key: &str, armed: bool, member_of: Option<&str>) {
        let now = self.clock.instant();
        let mut inner = self.lock();
        let Inner { registered, group, .. } = &mut *inner;
        registered.remove(key);
        match (member_of, group.as_mut()) {
            (Some(id), Some(g)) if g.running(id) => g.member_gone(key, armed, now),
            (Some(id), _) => {
                let n = self.slots.drain_suspended_of_group(id);
                if n > 0 {
                    log::info!("Group flight {}: {} session(s) of an ended group closed", id, n);
                }
            }
            (None, Some(g)) if g.phase == Phase::Running => g.member_gone(key, false, now),
            _ => {}
        }
    }

    /// Drop registrations whose recorder no longer exists (dropped without a teardown).
    fn prune_dead(&self, inner: &mut Inner, now: Instant) {
        let dead: Vec<(String, bool)> = inner
            .registered
            .iter()
            .filter(|(_, r)| r.recorder.strong_count() == 0)
            .map(|(k, r)| (k.clone(), r.armed))
            .collect();
        for (key, armed) in dead {
            inner.registered.remove(&key);
            log::warn!("Group coordinator: recorder {} dropped without a teardown", key);
            if let Some(g) = inner.group.as_mut().filter(|g| g.phase == Phase::Running) {
                g.member_gone(&key, armed, now);
            }
        }
    }

    /// Periodic work, run without any recorder lock held (the worker thread; tests call it directly):
    /// advance the group-end timer, take recorders that got no status into the running group, and
    /// finish an ending group.
    pub fn tick(&self) {
        let now = self.clock.instant();
        let (to_sync, ending) = {
            let mut inner = self.lock();
            inner.wake = false;
            self.prune_dead(&mut inner, now);
            self.advance_timers(&mut inner, now);
            match inner.group.as_ref() {
                None => (Vec::new(), None),
                // Every registered recorder: the members of the ending group leave on their sync.
                Some(g) if g.phase == Phase::Ending => (
                    inner.registered.values().map(|r| r.recorder.clone()).collect(),
                    Some(g.header.id.clone()),
                ),
                Some(g) => (
                    inner
                        .registered
                        .iter()
                        .filter(|(k, r)| {
                            r.first_status_seen
                                && !g.is_joined(k)
                                && g.same_dir(&r.sessions_dir)
                                && !self.slots.has_pending_for(k)
                        })
                        .map(|(_, r)| r.recorder.clone())
                        .collect::<Vec<Weak<Mutex<FlightRecorder>>>>(),
                    None,
                ),
            }
        };
        // One recorder at a time, no coordinator lock held (it takes it itself in `group_sync`).
        for weak in to_sync {
            if let Some(rec) = weak.upgrade() {
                if let Ok(mut r) = rec.lock() {
                    r.group_sync(false);
                }
            }
        }
        if let Some(id) = ending {
            self.finish(&id);
        }
    }

    /// The members of the ending group `group_id` are finalized (their `group_sync` in `tick`): add the
    /// suspended ones, park the package as a `PendingGroup` and announce `group-flight-ended`.
    fn finish(&self, group_id: &str) {
        self.slots.drain_suspended_of_group(group_id);
        let event = {
            let mut inner = self.lock();
            let Some(mut group) = inner.group.take_if(|g| g.header.id == group_id && g.phase == Phase::Ending) else {
                return;
            };
            group.header.end_time = Some(self.clock.utc());
            group.header.state = group_store::GROUP_PENDING.into();
            let roster: Vec<RosterEntry> = group
                .members
                .iter()
                .filter_map(|m| {
                    m.file.as_ref().map(|f| RosterEntry {
                        temp_path: f.temp_path.clone(),
                        key: m.key.clone(),
                        incomplete: m.lost_armed || m.stalled,
                    })
                })
                .collect();
            let members = self.slots.map_group_members(group_id, |s| member_summary(s, &roster));
            write_kgrp(&group.kgrp_path, |c| {
                group_store::write_group_header(c, &group.header)?;
                for m in &members {
                    let state = if m.incomplete { group_store::MEMBER_INCOMPLETE } else { group_store::MEMBER_ENDED };
                    group_store::set_group_member_state(c, &m.temp_path, state)?;
                }
                Ok(())
            });
            log::info!(
                "Group flight {} ended: {} member flight(s) await the store prompt",
                group_id,
                members.len()
            );
            let pending = PendingGroup {
                id: group.header.id.clone(),
                header: group.header,
                kgrp_path: group.kgrp_path,
                members,
                roster,
            };
            let summary = pending.summary();
            inner.pending.push(pending);
            summary
        };
        self.emit("group-flight-ended", &event);
    }

    /// Take an ended group for its store prompt: `Some(id)` → that group, `None` → the oldest. Its
    /// members are read from the slots NOW, not taken from the `finish` summary: a member session
    /// parked after `finish` (a teardown racing the group end) is included.
    #[allow(dead_code)] // the commit / discard commands: GROUP_FLIGHTS.md step 7
    pub fn take_pending_group(&self, group_id: Option<&str>) -> Option<PendingGroup> {
        let mut pending = {
            let mut inner = self.lock();
            let idx = match group_id {
                Some(id) => inner.pending.iter().position(|p| p.id == id)?,
                None if inner.pending.is_empty() => return None,
                None => 0,
            };
            inner.pending.remove(idx)
        };
        pending.members = self.slots.map_group_members(&pending.id, |s| member_summary(s, &pending.roster));
        Some(pending)
    }

    /// The ended groups awaiting their store prompt, oldest first (payload shape of `group-flight-ended`).
    /// The member lists are those of `finish` — the store prompt (step 8) must re-query them
    /// (`take_pending_group`) before it acts on them.
    #[allow(dead_code)] // the store prompt after a frontend reload: GROUP_FLIGHTS.md step 7/8
    pub fn pending_group_summary(&self) -> Vec<GroupEndedEvent> {
        self.lock().pending.iter().map(PendingGroup::summary).collect()
    }

    /// The coordinator's live state for the Debug Monitor (read-only).
    pub fn debug_state(&self) -> GroupDebugState {
        let now = self.clock.instant();
        let inner = self.lock();
        let mut recorders: Vec<DebugRecorder> = inner
            .registered
            .iter()
            .map(|(k, r)| DebugRecorder {
                key: k.clone(),
                first_status_seen: r.first_status_seen,
                armed: r.armed,
                last_seen_ms: now.saturating_duration_since(r.last_seen).as_millis() as u64,
            })
            .collect();
        recorders.sort_by(|a, b| a.key.cmp(&b.key));
        let group = inner.group.as_ref().map(|g| DebugGroup {
            id: g.header.id.clone(),
            phase: match g.phase {
                Phase::Running => "running",
                Phase::Ending => "ending",
            },
            start_time: g.header.start_time.to_rfc3339(),
            end_in_ms: g.end_deadline.map(|d| d.saturating_duration_since(now).as_millis() as u64),
            members: g
                .members
                .iter()
                .map(|m| DebugMember {
                    key: m.key.clone(),
                    armed: m.armed,
                    gone: m.gone,
                    lost_armed: m.lost_armed,
                    stalled: m.stalled,
                    file: m.file.as_ref().map(|f| group_store::file_name_of(&f.temp_path)),
                })
                .collect(),
        });
        GroupDebugState {
            enabled: self.enabled.load(Ordering::Relaxed),
            recorders,
            group,
            pending: inner.pending.iter().map(|p| p.id.clone()).collect(),
        }
    }
}

/// Whether the recorder `r` (vehicle `key`) may join `g`: its temp files must live next to the group's
/// `.kgrp`. One of another sessions folder records single flights (warned once).
fn dir_ok(r: &mut Registered, key: &str, g: &Group) -> bool {
    if g.same_dir(&r.sessions_dir) {
        return true;
    }
    if !r.warned_dir {
        r.warned_dir = true;
        log::warn!(
            "Group flight {}: {} records to another sessions folder ({}) than the group — not taken, it records single flights",
            g.header.id,
            key,
            r.sessions_dir.display()
        );
    }
    false
}

/// The store-prompt line of one finalized member session.
fn member_summary(s: &PendingSession, roster: &[RosterEntry]) -> GroupMemberSummary {
    let member = roster.iter().find(|m| m.temp_path == s.temp_path);
    GroupMemberSummary {
        member_id: group_store::file_name_of(&s.temp_path),
        vehicle_id: member.map(|m| m.key.clone()).unwrap_or_default(),
        craft_name: s.flight.craft_name.clone(),
        fc_variant: s.flight.fc_variant.clone(),
        has_armed_time: s.has_armed_time(),
        incomplete: member.is_some_and(|m| m.incomplete),
        armed_sec: s.flight.duration_sec.unwrap_or(0),
        max_alt_m: s.flight.max_alt_m.unwrap_or(0.0),
        max_speed_ms: s.flight.max_speed_ms.unwrap_or(0.0),
        max_distance_m: s.flight.max_distance_m.unwrap_or(0.0),
        total_distance_m: s.flight.total_distance_m.unwrap_or(0.0),
        battery_used_mah: s.flight.battery_used_mah,
        temp_path: s.temp_path.clone(),
    }
}

fn kgrp_member(key: &str, f: &MemberFile, joined_at: DateTime<Utc>, state: &str) -> GroupFileMember {
    GroupFileMember {
        temp_path: f.temp_path.clone(),
        vehicle_key: key.to_string(),
        craft_name: f.craft_name.clone(),
        fc_variant: f.fc_variant.clone(),
        fc_uid: f.fc_uid.clone(),
        joined_at,
        state: state.to_string(),
    }
}

/// Write to a group's `.kgrp` (opened per write; the coordinator lock serialises the writers). A
/// failure is logged — the members record on regardless; only a crash recovery would miss the entry.
fn write_kgrp(path: &Path, f: impl FnOnce(&Connection) -> rusqlite::Result<()>) {
    if let Err(e) = group_store::open_group_file(path).and_then(|c| f(&c)) {
        log::warn!("Group file {}: {}", path.display(), e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flightlog::recorder::{FakeClock, RecorderSink, SessionSlots};
    use crate::flightlog::types::FlightLogSettings;
    use crate::msp::FcInfo;
    use crate::scheduler::telemetry::{GpsData, StatusData};

    /// Captures lifecycle events of a recorder.
    struct RecSink {
        key: String,
        emitted: Arc<Mutex<Vec<String>>>,
    }

    impl RecorderSink for RecSink {
        fn key(&self) -> &str {
            &self.key
        }
        fn emit_json(&self, event: &str, _payload: serde_json::Value) -> Result<(), String> {
            self.emitted.lock().unwrap().push(event.to_string());
            Ok(())
        }
    }

    /// Captures the coordinator's events.
    #[derive(Default)]
    struct Events(Mutex<Vec<(String, serde_json::Value)>>);

    impl GroupEventSink for Arc<Events> {
        fn emit_json(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    impl Events {
        fn named(&self, event: &str) -> Vec<serde_json::Value> {
            self.0.lock().unwrap().iter().filter(|(e, _)| e == event).map(|(_, p)| p.clone()).collect()
        }
    }

    /// One throw-away DB folder, shared slots, one fake clock and the coordinator under test.
    struct Rig {
        dir: PathBuf,
        clock: Arc<FakeClock>,
        slots: SessionSlotsHandle,
        enabled: Arc<AtomicBool>,
        coord: Arc<GroupCoordinator>,
        events: Arc<Events>,
    }

    struct Rec {
        handle: FlightRecorderHandle,
        emitted: Arc<Mutex<Vec<String>>>,
    }

    impl Rec {
        fn status(&self, armed: bool) {
            let data = StatusData {
                arming_flags: if armed { 0x04 } else { 0 },
                flight_mode_flags: 0,
                cpu_load: 0,
                sensor_status: 0,
                msp_rc_override: false,
            };
            self.handle.lock().unwrap().on_status(&data);
        }
        fn gps(&self, fix_type: u8, lat: f64, lon: f64) {
            let data = GpsData { fix_type, num_sat: 10, lat, lon, alt_msl: 100.0, ground_speed: 0.0, course: 0.0 };
            self.handle.lock().unwrap().on_gps(&data);
        }
        fn group(&self) -> Option<String> {
            self.handle.lock().unwrap().group_id().map(String::from)
        }
        fn path(&self) -> Option<PathBuf> {
            self.handle.lock().unwrap().active_temp_path()
        }
        fn emitted(&self) -> Vec<String> {
            self.emitted.lock().unwrap().clone()
        }
    }

    impl Rig {
        fn new(name: &str, fleet: bool) -> Self {
            let dir = std::env::temp_dir().join(format!("kite-group-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let clock = Arc::new(FakeClock::new(DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap()));
            let slots: SessionSlotsHandle = Arc::new(SessionSlots::default());
            let enabled = Arc::new(AtomicBool::new(fleet));
            let coord = Arc::new(GroupCoordinator::new(slots.clone(), clock.clone(), enabled.clone()));
            let events = Arc::new(Events::default());
            coord.set_sink(Box::new(events.clone()));
            Rig { dir, clock, slots, enabled, coord, events }
        }

        /// A registered recorder for vehicle `key`.
        fn recorder(&self, key: &str, craft: &str) -> Rec {
            let rec = self.recorder_unregistered(key, craft);
            self.coord.register(&rec.handle);
            rec
        }

        /// A recorder for vehicle `key` the coordinator does not know (yet).
        fn recorder_unregistered(&self, key: &str, craft: &str) -> Rec {
            let emitted = Arc::new(Mutex::new(Vec::new()));
            let settings = FlightLogSettings {
                enabled: true,
                db_enabled: true,
                db_path: self.dir.to_string_lossy().to_string(),
                raw_log_path: self.dir.join("raw").to_string_lossy().to_string(),
                raw_enabled: false,
                raw_always: false,
            };
            let fc = FcInfo { craft_name: craft.into(), fc_variant: "ArduPilot".into(), ..FcInfo::default() };
            let sink = RecSink { key: key.to_string(), emitted: emitted.clone() };
            let rec = FlightRecorder::with_clock(
                settings, fc, "MAVLink", false, sink, self.slots.clone(), Arc::new(Mutex::new(None)), self.clock.clone(),
            )
            .unwrap();
            let handle: FlightRecorderHandle = Arc::new(Mutex::new(rec));
            Rec { handle, emitted }
        }

        fn advance_ms(&self, ms: u64) {
            self.clock.advance(Duration::from_millis(ms));
        }

        fn base_ms(&self) -> i64 {
            1_700_000_000_000
        }

        fn running(&self) -> Option<String> {
            self.coord.lock().group.as_ref().filter(|g| g.phase == Phase::Running).map(|g| g.header.id.clone())
        }

        /// The coordinator's view of member `key` (Debug Monitor snapshot).
        fn member(&self, key: &str) -> Option<DebugMember> {
            self.coord.debug_state().group?.members.into_iter().find(|m| m.key == key)
        }

        /// The group start as the `.kgrp` header holds it (UTC ms).
        fn kgrp_start_ms(&self, id: &str) -> i64 {
            let kgrp = group_store::group_file_path(&self.dir.join("sessions"), id);
            let conn = group_store::open_group_file(&kgrp).unwrap();
            group_store::read_group_header(&conn).unwrap().unwrap().start_time.timestamp_millis()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn group_id_is_the_documented_sha1_prefix() {
        let start = DateTime::<Utc>::from_timestamp_millis(1_700_000_000_000).unwrap();
        // sha1("1700000000000|481000000|115000000") — reproduced with Python's hashlib.
        assert_eq!(group_id_for(start, 48.1, 11.5), "546055e111e82851");
        assert_eq!(group_id_for(start, 48.2, 11.6), "ba28bab88e8664d0");
        assert_eq!(group_id_for(start, 48.2, 11.6).len(), 16);
    }

    /// (a) With the fleet flag off nothing ever forms a group: both vehicles fly single flights with
    /// their own End-Flight summaries.
    #[test]
    fn flag_off_never_forms_a_group() {
        let rig = Rig::new("flagoff", false);
        let a = rig.recorder("L1:S1", "A");
        let b = rig.recorder("L2:S1", "B");
        for r in [&a, &b] {
            r.gps(2, 48.1, 11.5);
            r.status(false);
        }
        a.status(true);
        b.status(true);
        rig.coord.tick();
        assert!(a.group().is_none() && b.group().is_none());
        rig.advance_ms(3000);
        a.status(false);
        b.status(false);
        rig.advance_ms(10_000);
        rig.coord.tick();
        assert!(rig.running().is_none());
        assert!(rig.events.0.lock().unwrap().is_empty());
        assert_eq!(a.emitted(), ["flight-recording-started", "flight-recording-ended"]);
        assert_eq!(b.emitted(), ["flight-recording-started", "flight-recording-ended"]);
        assert!(rig.slots.has_pending_for("L1:S1") && rig.slots.has_pending_for("L2:S1"));
        // Turning the flag on later changes nothing for flights already finished.
        rig.enabled.store(true, Ordering::Relaxed);
        rig.coord.tick();
        assert!(rig.running().is_none());
    }

    /// (b) B arms (with a fix) while A records: the group starts at A's arm — retroactively — and its
    /// id is the hash of that start and the initiator's position. Both record as members; no second
    /// solo lifecycle event for A.
    #[test]
    fn b_arms_while_a_records_group_starts_at_a_arm() {
        let rig = Rig::new("retro", true);
        let a = rig.recorder("L1:S1", "A");
        a.gps(2, 48.1, 11.5);
        a.status(false);
        a.status(true); // alone: a single flight
        assert!(a.group().is_none());
        rig.advance_ms(10_000);
        let b = rig.recorder("L2:S1", "B");
        b.gps(2, 48.2, 11.6);
        b.status(false);
        assert!(rig.running().is_none(), "a registration alone forms no group");
        b.status(true);
        let id = rig.running().expect("group");
        assert_eq!(id, "ba28bab88e8664d0"); // sha1("1700000000000|482000000|116000000")
        assert_eq!(b.group().as_deref(), Some(id.as_str()));
        let started = rig.events.named("group-flight-started");
        assert_eq!(started.len(), 1);
        assert_eq!(started[0]["groupId"], id.as_str());
        assert_eq!(started[0]["startTime"], DateTime::<Utc>::from_timestamp_millis(rig.base_ms()).unwrap().to_rfc3339());
        assert_eq!(started[0]["members"], serde_json::json!(["L1:S1", "L2:S1"]));
        // A follows on its next status (pull) — its running single flight becomes its member session.
        let a_path = a.path().unwrap();
        a.status(true);
        assert_eq!(a.group().as_deref(), Some(id.as_str()));
        assert_eq!(a.path().unwrap(), a_path);
        let kgrp = group_store::group_file_path(&rig.dir.join("sessions"), &id);
        let conn = group_store::open_group_file(&kgrp).unwrap();
        let header = group_store::read_group_header(&conn).unwrap().unwrap();
        assert_eq!(header.start_time.timestamp_millis(), rig.base_ms());
        assert_eq!(header.start_lat, Some(48.2));
        let members = group_store::read_group_members(&conn, &kgrp).unwrap();
        // Same join time → ordered by file name, i.e. by session start: A's file first.
        assert_eq!(members.iter().map(|m| m.vehicle_key.as_str()).collect::<Vec<_>>(), ["L1:S1", "L2:S1"]);
        assert_eq!(members[0].temp_path, a_path);
    }

    /// (c) The initiator needs a 3D fix: an arm without one forms no group (A records a single flight);
    /// a later arm with a fix starts it — at A's earlier arm.
    #[test]
    fn initiator_without_3d_fix_forms_no_group_until_an_arm_with_one() {
        let rig = Rig::new("nofix", true);
        let a = rig.recorder("L1:S1", "A");
        let b = rig.recorder("L2:S1", "B");
        a.gps(1, 48.1, 11.5); // 2D only
        b.gps(2, 48.2, 11.6);
        a.status(false);
        b.status(false);
        a.status(true);
        assert!(rig.running().is_none());
        assert!(a.path().is_some(), "A still records its single flight");
        rig.advance_ms(10_000);
        b.status(true);
        let id = rig.running().expect("group on the arm with a fix");
        assert_eq!(id, "ba28bab88e8664d0"); // start = A's arm at base, position = B's
        a.status(true);
        assert_eq!(a.group().as_deref(), Some(id.as_str()));
    }

    /// Group of two recorders, both joined: A armed (initiator) at base, B joined unarmed.
    fn group_of_two(rig: &Rig) -> (Rec, Rec, String) {
        let a = rig.recorder("L1:S1", "A");
        let b = rig.recorder("L2:S1", "B");
        a.gps(2, 48.1, 11.5);
        b.gps(2, 48.2, 11.6);
        a.status(false);
        b.status(false);
        a.status(true);
        let id = rig.running().expect("group");
        assert_eq!(id, "546055e111e82851");
        rig.coord.tick(); // B got no status since: the worker takes it in
        assert_eq!(b.group().as_deref(), Some(id.as_str()));
        assert!(b.path().is_some(), "B records unarmed");
        (a, b, id)
    }

    /// (d) A member's disarm → re-arm within the grace keeps the group; the last disarm + grace ends it:
    /// every member finalized, `group-flight-ended` once, the package pending.
    #[test]
    fn last_disarm_plus_grace_ends_the_group_once() {
        let rig = Rig::new("end", true);
        let (a, b, id) = group_of_two(&rig);
        rig.advance_ms(1000);
        b.status(true); // t=1 s
        rig.advance_ms(1000);
        a.status(false); // t=2 s, B still armed
        rig.advance_ms(10_000);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "B armed: no end");
        b.status(false); // t=12 s: last disarm
        rig.advance_ms(3000);
        b.status(true); // t=15 s: re-arm within the grace
        rig.advance_ms(10_000);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()));
        b.status(false); // t=25 s: last disarm
        rig.advance_ms(4999);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "still inside the grace");
        rig.advance_ms(1);
        rig.coord.tick();
        assert!(rig.running().is_none());
        assert!(a.group().is_none() && b.group().is_none());
        assert!(a.path().is_none() && b.path().is_none());
        let ended = rig.events.named("group-flight-ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0]["groupId"], id.as_str());
        let members = ended[0]["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|m| m["hasArmedTime"] == true && m["incomplete"] == false));
        rig.advance_ms(10_000);
        rig.coord.tick();
        a.status(false);
        assert_eq!(rig.events.named("group-flight-ended").len(), 1, "emitted once");
        // Members never announce their sessions; no single-flight dialog for either.
        assert_eq!(a.emitted(), ["flight-recording-started"]); // A's arm before the group existed
        assert!(b.emitted().is_empty());
        assert!(!rig.slots.has_pending_for("L1:S1") && !rig.slots.has_pending_for("L2:S1"));
        let summary = rig.coord.pending_group_summary();
        assert_eq!(summary.len(), 1);
        let pending = rig.coord.take_pending_group(None).unwrap();
        assert_eq!(pending.id, id);
        // Re-read from the slots at take time, named by the roster of the ended group.
        let mut keys: Vec<&str> = pending.members.iter().map(|m| m.vehicle_id.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["L1:S1", "L2:S1"]);
        assert_eq!(pending.header.state, group_store::GROUP_PENDING);
        assert!(pending.header.end_time.is_some());
        let sessions = rig.slots.take_group_members(&id);
        assert_eq!(sessions.len(), 2);
        let conn = group_store::open_group_file(&pending.kgrp_path).unwrap();
        let kgrp_members = group_store::read_group_members(&conn, &pending.kgrp_path).unwrap();
        assert!(kgrp_members.iter().all(|m| m.state == group_store::MEMBER_ENDED));
        // The next arm is a new flight again.
        rig.advance_ms(1000);
        a.status(true);
        assert!(a.path().is_some());
    }

    /// (e) An ARMED member whose recorder goes away keeps the group open for a minute when it was the
    /// last armed one; a DISARMED member going away does not end the group.
    #[test]
    fn armed_member_lost_keeps_the_group_one_minute() {
        let rig = Rig::new("lost", true);
        let (a, b, id) = group_of_two(&rig);
        b.status(true);
        rig.advance_ms(1000);
        a.status(false); // only B armed now
        // A (disarmed) goes away: the group runs on.
        a.handle.lock().unwrap().shutdown_lost();
        rig.advance_ms(30_000);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()));
        // B (armed, the last armed) is lost.
        b.handle.lock().unwrap().shutdown_lost();
        rig.advance_ms(59_000);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "open for the lost member");
        rig.advance_ms(1000);
        rig.coord.tick();
        assert!(rig.running().is_none());
        let ended = rig.events.named("group-flight-ended");
        assert_eq!(ended.len(), 1);
        let members = ended[0]["members"].as_array().unwrap();
        assert_eq!(members.len(), 2, "both suspended sessions are in the package");
        let incomplete: Vec<bool> = members.iter().map(|m| m["incomplete"].as_bool().unwrap()).collect();
        assert_eq!(incomplete.iter().filter(|i| **i).count(), 1, "only B was lost armed");
        // Nothing of the finished group stays adoptable.
        assert!(rig.slots.take_suspended_of_group(&id).is_empty());
        assert_eq!(rig.slots.take_group_members(&id).len(), 2);
    }

    /// (e'') An armed member lost while another is still armed: when the last present member disarms,
    /// the group still waits out the lost member's minute, not just the grace.
    #[test]
    fn last_disarm_waits_for_a_lost_armed_member() {
        let rig = Rig::new("lostfirst", true);
        let (a, b, id) = group_of_two(&rig); // A armed
        b.status(true);
        b.handle.lock().unwrap().shutdown_lost(); // B lost armed at t=0; A still armed
        rig.advance_ms(10_000);
        a.status(false); // t=10 s: the last present member disarms
        rig.advance_ms(5_000);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "B's minute runs to t=60 s");
        rig.advance_ms(44_999);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()));
        rig.advance_ms(1);
        rig.coord.tick();
        assert!(rig.running().is_none());
        assert_eq!(rig.events.named("group-flight-ended").len(), 1);
    }

    /// (e') An armed lost member that comes back (adoption under a new key) keeps the group alive.
    #[test]
    fn lost_member_back_armed_keeps_the_group() {
        let rig = Rig::new("back", true);
        let (a, b, id) = group_of_two(&rig);
        b.status(true);
        a.status(false); // only B armed now
        let b_path = b.path().unwrap();
        rig.advance_ms(1000);
        b.handle.lock().unwrap().shutdown_lost();
        rig.advance_ms(40_000);
        let b2 = rig.recorder("L3:S1", "B");
        b2.gps(2, 48.2, 11.6);
        b2.status(true); // first status: adopts B's suspended member session
        assert_eq!(b2.group().as_deref(), Some(id.as_str()));
        assert_eq!(b2.path().unwrap(), b_path);
        // Past B's lost minute; B2 keeps reporting (a silent armed member would count as stalled).
        for _ in 0..30 {
            rig.advance_ms(2000);
            b2.status(true);
        }
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "B is armed again");
        b2.status(false);
        rig.advance_ms(5000);
        rig.coord.tick();
        assert!(rig.running().is_none());
        let ended = rig.events.named("group-flight-ended");
        let members = ended[0]["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|m| m["incomplete"] == false));
        assert!(members.iter().any(|m| m["vehicleId"] == "L3:S1"));
    }

    /// (f) A recorder registered after the group started joins on its first status, with an unarmed
    /// session, and is listed in the group's `.kgrp`.
    #[test]
    fn late_recorder_joins_with_an_unarmed_session() {
        let rig = Rig::new("late", true);
        let (_a, _b, id) = group_of_two(&rig);
        let c = rig.recorder("L3:S1", "C");
        rig.coord.tick();
        assert!(c.group().is_none(), "no status yet: it may still adopt a suspended member file");
        c.status(false);
        assert_eq!(c.group().as_deref(), Some(id.as_str()));
        let path = c.path().expect("an unarmed member session");
        assert!(c.emitted().is_empty());
        let kgrp = group_store::group_file_path(&rig.dir.join("sessions"), &id);
        let conn = group_store::open_group_file(&kgrp).unwrap();
        let members = group_store::read_group_members(&conn, &kgrp).unwrap();
        assert!(members.iter().any(|m| m.vehicle_key == "L3:S1" && m.temp_path == path));
        assert_eq!(members.len(), 3);
    }

    /// (g) A's single flight awaits its End-Flight dialog when B arms: no group, A is not touched. A's
    /// re-arm within the grace resumes its flight and starts the group at A's ORIGINAL arm.
    #[test]
    fn pending_single_flight_is_not_taken_into_a_group() {
        let rig = Rig::new("pending", true);
        let a = rig.recorder("L1:S1", "A");
        a.gps(2, 48.1, 11.5);
        a.status(false);
        a.status(true);
        rig.advance_ms(2000);
        a.status(false); // single flight pending (End-Flight dialog)
        let b = rig.recorder("L2:S1", "B");
        b.gps(2, 48.2, 11.6);
        b.status(false);
        b.status(true);
        assert!(rig.running().is_none());
        assert!(rig.slots.has_pending_for("L1:S1"));
        rig.coord.tick();
        assert!(a.group().is_none() && a.path().is_none());
        rig.advance_ms(2000);
        a.status(true); // within the grace: resume
        let id = rig.running().expect("group on A's re-arm");
        assert_eq!(id, "546055e111e82851"); // A's original start (base) + A's position
        assert!(a.emitted().contains(&"flight-recording-resumed".to_string()));
        b.status(true);
        assert_eq!(b.group().as_deref(), Some(id.as_str()));
    }

    /// (h) Lock order: two recorders arming / disarming from two threads while a third ticks — no
    /// deadlock (smoke).
    #[test]
    fn concurrent_arms_do_not_deadlock() {
        let rig = Rig::new("race", true);
        let a = rig.recorder("L1:S1", "A");
        let b = rig.recorder("L2:S1", "B");
        a.gps(2, 48.1, 11.5);
        b.gps(2, 48.2, 11.6);
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        for (rec, tx) in [(a.handle.clone(), tx.clone()), (b.handle.clone(), tx.clone())] {
            threads.push(std::thread::spawn(move || {
                for i in 0..200 {
                    let data = StatusData {
                        arming_flags: if i % 2 == 1 { 0x04 } else { 0 },
                        flight_mode_flags: 0,
                        cpu_load: 0,
                        sensor_status: 0,
                        msp_rc_override: false,
                    };
                    rec.lock().unwrap().on_status(&data);
                }
                tx.send(()).unwrap();
            }));
        }
        drop(tx); // a panicking recorder thread then shows as a disconnect, not as a hang
        let (coord, clock, stop2) = (rig.coord.clone(), rig.clock.clone(), stop.clone());
        let ticker = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                clock.advance(Duration::from_millis(700));
                coord.tick();
            }
        });
        for _ in 0..2 {
            rx.recv_timeout(Duration::from_secs(30)).expect("a recorder thread hung — lock-order deadlock?");
        }
        stop.store(true, Ordering::Relaxed);
        ticker.join().unwrap();
        for t in threads {
            t.join().unwrap();
        }
        assert!(!rig.events.named("group-flight-started").is_empty(), "the race exercised group formation");
    }

    /// M1: an ARMED member whose recorder goes silent (a stalled link keeps its recorder — no teardown)
    /// counts as lost-armed from its last status: once the other member disarms, the group ends after
    /// the lost minute counted from that status, not never.
    #[test]
    fn stalled_armed_member_ends_the_group_after_the_lost_minute() {
        let rig = Rig::new("stall", true);
        let (a, b, id) = group_of_two(&rig); // A armed at t=0
        b.status(true); // t=0: B armed — its last status
        rig.advance_ms(3000);
        a.status(true); // t=3 s: A reports on, B silent for 3 s: no stall yet
        rig.coord.tick();
        assert!(!rig.member("L2:S1").unwrap().stalled);
        rig.advance_ms(3000);
        a.status(false); // t=6 s: B silent for 6 s → stalled from t=0; A disarms
        assert!(rig.member("L2:S1").unwrap().stalled);
        rig.advance_ms(10_000);
        rig.coord.tick(); // t=16 s: past A's grace, but B's minute runs to t=60 s
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "open for the stalled member");
        rig.advance_ms(43_999);
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()));
        rig.advance_ms(1);
        rig.coord.tick();
        assert!(rig.running().is_none(), "ends 60 s after B's last status");
        let ended = rig.events.named("group-flight-ended");
        assert_eq!(ended.len(), 1);
        let members = ended[0]["members"].as_array().unwrap();
        let incomplete = |key: &str| members.iter().find(|m| m["vehicleId"] == key).unwrap()["incomplete"].clone();
        assert_eq!(incomplete("L2:S1"), true, "B may have flown on unrecorded");
        assert_eq!(incomplete("L1:S1"), false);
    }

    /// M1: a stalled member whose status returns within the lost minute is present again — the group
    /// stays open past that minute while it flies.
    #[test]
    fn stalled_member_back_within_the_minute_keeps_the_group() {
        let rig = Rig::new("stallback", true);
        let (a, b, id) = group_of_two(&rig);
        b.status(true); // t=0
        rig.advance_ms(1000);
        a.status(false); // t=1 s: only B armed
        rig.advance_ms(9000);
        rig.coord.tick(); // t=10 s: B silent → stalled, the group ends at t=60 s unless it returns
        assert!(rig.member("L2:S1").unwrap().stalled);
        assert!(rig.coord.debug_state().group.unwrap().end_in_ms.is_some());
        rig.advance_ms(20_000);
        b.status(true); // t=30 s: B is back, still armed
        assert!(!rig.member("L2:S1").unwrap().stalled);
        assert_eq!(rig.coord.debug_state().group.unwrap().end_in_ms, None);
        for _ in 0..40 {
            rig.advance_ms(2000);
            b.status(true);
        }
        rig.coord.tick(); // t=110 s
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "B flies on: no end");
        b.status(false);
        rig.advance_ms(5000);
        rig.coord.tick();
        assert!(rig.running().is_none());
        let ended = rig.events.named("group-flight-ended");
        assert!(ended[0]["members"].as_array().unwrap().iter().all(|m| m["incomplete"] == false));
    }

    /// m4 (Marc, 2026-10-01): a recorder joining with a session that began before the group start pulls
    /// the group start back to it — here A, lost while armed before the group existed, back via
    /// continue-on-reconnect. The id stays the one formed at the start.
    #[test]
    fn earlier_session_joining_pulls_the_group_start_back() {
        let rig = Rig::new("pullstart", true);
        let a = rig.recorder("L1:S1", "A");
        a.gps(2, 48.0, 11.0);
        a.status(false);
        a.status(true); // t=0: alone — a single flight
        rig.advance_ms(2000);
        a.handle.lock().unwrap().shutdown_lost(); // t=2 s: lost armed → recovery prompt
        assert_eq!(a.emitted().last().map(String::as_str), Some("flight-recording-interrupted"));
        let p = rig.slots.take_pending_for("L1:S1").expect("the interrupted flight");
        rig.slots.put_resume("L1:S1".into(), p).unwrap(); // the operator chose Continue
        rig.advance_ms(8000);
        let b = rig.recorder("L2:S1", "B");
        let c = rig.recorder("L3:S1", "C");
        b.gps(2, 48.2, 11.6);
        c.gps(2, 48.3, 11.7);
        b.status(false);
        c.status(false);
        b.status(true); // t=10 s: B and C form the group
        let id = rig.running().expect("group");
        assert_eq!(rig.kgrp_start_ms(&id), rig.base_ms() + 10_000);
        rig.advance_ms(5000);
        let a2 = rig.recorder("L4:S1", "A");
        a2.gps(2, 48.0, 11.0);
        a2.status(true); // t=15 s, first status: resumes A's flight from t=0 and joins
        assert_eq!(a2.emitted(), ["flight-recording-resumed"]);
        assert_eq!(a2.group().as_deref(), Some(id.as_str()));
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "the id stays");
        assert_eq!(rig.kgrp_start_ms(&id), rig.base_ms(), "the group starts at A's session start");
        let state = rig.coord.debug_state();
        assert_eq!(state.group.unwrap().start_time, DateTime::<Utc>::from_timestamp_millis(rig.base_ms()).unwrap().to_rfc3339());
    }

    /// Flag ON with ONE registered recorder: every single-flight path stays as it was — grace resume,
    /// the recovery prompt of a lost link, continue-on-reconnect — and no group ever forms.
    #[test]
    fn flag_on_with_one_recorder_stays_solo() {
        let rig = Rig::new("solo", true);
        let a = rig.recorder("L1:S1", "A");
        a.gps(2, 48.1, 11.5);
        a.status(false);
        a.status(true); // t=0
        rig.advance_ms(2000);
        a.status(false); // t=2 s: End-Flight dialog
        assert!(rig.slots.has_pending_for("L1:S1"));
        rig.advance_ms(2000);
        a.status(true); // t=4 s: re-arm within the grace → the same flight
        assert_eq!(a.emitted(), ["flight-recording-started", "flight-recording-ended", "flight-recording-resumed"]);
        let path = a.path().unwrap();
        rig.advance_ms(2000);
        a.handle.lock().unwrap().shutdown_lost(); // armed → recovery prompt
        assert_eq!(a.emitted().last().map(String::as_str), Some("flight-recording-interrupted"));
        let p = rig.slots.take_pending_for("L1:S1").unwrap();
        assert_eq!(p.temp_path, path);
        rig.slots.put_resume("L1:S1".into(), p).unwrap();
        let a2 = rig.recorder("L2:S1", "A");
        a2.gps(2, 48.1, 11.5);
        a2.status(true); // continue-on-reconnect
        assert_eq!(a2.emitted(), ["flight-recording-resumed"]);
        assert_eq!(a2.path().unwrap(), path);
        rig.advance_ms(1000);
        rig.coord.tick();
        assert!(rig.running().is_none() && a2.group().is_none());
        assert!(rig.events.0.lock().unwrap().is_empty());
        a2.status(false);
        assert_eq!(a2.emitted().last().map(String::as_str), Some("flight-recording-ended"));
    }

    /// `on_disarm` take-over: a single flight that has not joined the running group when it disarms is
    /// taken into it instead of ending. Here the recorder registers only after its first status and
    /// arm, so the coordinator never saw that first status and its sync never joins it — in production
    /// the same happens when a group forms between a recorder's sync and its disarm edge.
    #[test]
    fn disarm_of_a_single_flight_is_taken_into_the_running_group() {
        let rig = Rig::new("takeover", true);
        let c = rig.recorder_unregistered("L3:S1", "C");
        c.gps(2, 48.3, 11.7);
        c.status(false);
        c.status(true); // t=0: a single flight, no coordinator yet
        let c_path = c.path().unwrap();
        rig.coord.register(&c.handle);
        let (_a, _b, id) = group_of_two(&rig);
        assert!(c.group().is_none(), "not joined: the coordinator never saw its first status");
        rig.advance_ms(1000);
        c.status(false); // t=1 s: its disarm edge
        assert_eq!(c.group().as_deref(), Some(id.as_str()));
        assert_eq!(c.path().unwrap(), c_path, "the single flight records on as the member session");
        assert_eq!(c.emitted(), ["flight-recording-started"], "no End-Flight dialog");
        assert!(!rig.slots.has_pending_for("L3:S1"));
        let kgrp = group_store::group_file_path(&rig.dir.join("sessions"), &id);
        let conn = group_store::open_group_file(&kgrp).unwrap();
        let members = group_store::read_group_members(&conn, &kgrp).unwrap();
        assert!(members.iter().any(|m| m.vehicle_key == "L3:S1" && m.temp_path == c_path));
    }

    /// An arm while the group is ending (its end is decided, the worker has not finalized it yet)
    /// starts a single flight; the ended group still finishes with both member sessions.
    #[test]
    fn arm_during_ending_starts_a_single_flight() {
        let rig = Rig::new("armending", true);
        let (a, _b, _id) = group_of_two(&rig);
        let a_member = a.path().unwrap();
        a.status(false); // t=0: the last armed member disarms → the group ends at t=5 s
        rig.advance_ms(5000);
        a.status(true); // t=5 s: A's sync ends the group and finalizes A; the arm is a new flight
        assert!(a.group().is_none());
        let a_single = a.path().expect("a single flight");
        assert_ne!(a_single, a_member);
        assert_eq!(a.emitted(), ["flight-recording-started", "flight-recording-started"]);
        rig.coord.tick(); // the worker finishes the ended group
        assert!(rig.running().is_none());
        let ended = rig.events.named("group-flight-ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(rig.events.named("group-flight-started").len(), 1, "the arm during Ending formed no group");
        assert_eq!(a.path().unwrap(), a_single, "the single flight records on");
        rig.advance_ms(2000);
        a.status(false);
        assert_eq!(a.emitted().last().map(String::as_str), Some("flight-recording-ended"));
        assert!(rig.slots.has_pending_for("L1:S1"));
    }

    /// A member's link going away while the group is ending: its session goes straight to the ended
    /// group's members (no recovery prompt, nothing left to adopt).
    #[test]
    fn teardown_during_ending_goes_to_the_ended_group() {
        let rig = Rig::new("teardownending", true);
        let (a, b, id) = group_of_two(&rig);
        a.status(false); // t=0 → the group ends at t=5 s
        rig.advance_ms(5000);
        a.status(false); // t=5 s: A's sync ends the group and finalizes A
        assert!(a.group().is_none());
        assert_eq!(rig.coord.debug_state().group.map(|g| g.phase), Some("ending"));
        b.handle.lock().unwrap().shutdown_lost(); // before the worker ran
        rig.coord.tick();
        assert!(rig.running().is_none());
        let ended = rig.events.named("group-flight-ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0]["members"].as_array().unwrap().len(), 2);
        assert!(b.emitted().is_empty(), "no recovery prompt for a member");
        assert!(rig.slots.take_suspended_of_group(&id).is_empty());
        assert_eq!(rig.slots.take_group_members(&id).len(), 2);
    }

    /// The flag switched off mid-group: the running group records on to its normal end; afterwards the
    /// arms are single flights.
    #[test]
    fn flag_off_mid_group_records_on_to_the_end() {
        let rig = Rig::new("flagoffmid", true);
        let (a, b, id) = group_of_two(&rig);
        rig.enabled.store(false, Ordering::Relaxed);
        rig.advance_ms(1000);
        b.status(true); // t=1 s: a member arm — the group runs on
        a.status(false);
        rig.advance_ms(4000);
        b.status(true); // t=5 s
        rig.coord.tick();
        assert_eq!(rig.running().as_deref(), Some(id.as_str()), "never cut mid-flight");
        b.status(false); // t=5 s: the last armed member disarms
        rig.advance_ms(5000);
        rig.coord.tick();
        assert!(rig.running().is_none());
        assert_eq!(rig.events.named("group-flight-ended").len(), 1);
        rig.advance_ms(1000);
        a.status(true);
        b.status(true);
        assert!(rig.running().is_none() && a.group().is_none() && b.group().is_none());
        assert_eq!(rig.events.named("group-flight-started").len(), 1);
    }
}
