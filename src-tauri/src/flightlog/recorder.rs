// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Recorder — detects arm/disarm transitions and records telemetry.
// Designed to be called from the scheduler thread with each decoded telemetry payload.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::db;
use super::msp_raw_logger::{MspRawLogger, MspRawSink};
use super::tlog_logger::TlogLogger;
use super::types::{BatteryRecord, Flight, FlightLogSettings, TelemetryRecord};
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
/// Lock rule: each map is locked on its own and never while another of the three is held; a
/// recorder may lock them while holding its own recorder lock (recorder → slots, never reverse).
#[derive(Default)]
pub struct SessionSlots {
    /// Finished sessions awaiting Save/Discard (or the re-arm grace), keyed by the vehicle key of the
    /// recorder that finalized them (`"L1:S1"`).
    pending: Mutex<HashMap<String, PendingSession>>,
    /// Sessions to continue on reconnect (ADR-042), keyed by the vehicle key they were recorded under
    /// (pending → continue) or by the temp path (recovered orphan). A reconnect gets a new vehicle key,
    /// so the next primary recorder claims one by identity instead (see `take_resume_for`).
    resume: Mutex<HashMap<String, PendingSession>>,
    /// Every temp `.ktmp` a recorder is writing right now (live-path registry).
    live: Mutex<HashSet<PathBuf>>,
    /// Vehicle keys whose recorder is connected (attached on creation, detached on teardown). A
    /// pending session whose vehicle is no longer attached came from a closed connection; the same
    /// aircraft's next recorder claims it on arm (see `take_detached_pending_for`).
    attached: Mutex<HashSet<String>>,
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

    /// Take `key`'s pending session only when `pred` holds for it (checked under the lock).
    fn take_pending_if(&self, key: &str, pred: impl FnOnce(&PendingSession) -> bool) -> Option<PendingSession> {
        let mut map = self.pending.lock().ok()?;
        if map.get(key).is_some_and(pred) { map.remove(key) } else { None }
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

    /// The continue-on-reconnect session the recorder of `fc` claims on its first status: the one
    /// recorded by the same FC (hardware id when both sides know it, else craft name + variant), else
    /// the oldest queued one — with a single queued session that is exactly the pre-multi-vehicle
    /// behaviour (the next connection takes it).
    fn take_resume_for(&self, fc: &FcInfo) -> Option<PendingSession> {
        let mut map = self.resume.lock().ok()?;
        let key = map
            .iter()
            .find(|(_, s)| same_fc(&s.flight, fc))
            .or_else(|| map.iter().min_by_key(|(_, s)| s.flight.start_time))
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

    /// Every temp file that belongs to a live workflow in this process — being written, pending
    /// Save/Discard, or queued for continue-on-reconnect. The orphan scan and the discard sweeps must
    /// never touch these.
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
        keep
    }
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
pub fn summarize_temp_session(
    temp_path: PathBuf,
    db_path: PathBuf,
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

    let start_time = chrono::DateTime::parse_from_rfc3339(&meta.start_time)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let start_lat = meta.start_lat.or_else(|| rows.iter().find_map(|r| r.lat));
    let start_lon = meta.start_lon.or_else(|| rows.iter().find_map(|r| r.lon));

    let mut max_alt = 0.0f64;
    let mut max_speed = 0.0f64;
    let mut max_distance = 0.0f64;
    let mut total_distance = 0.0f64;
    let mut last_lat: Option<f64> = None;
    let mut last_lon: Option<f64> = None;
    let mut start_mah: Option<u32> = None;
    let mut end_mah: Option<u32> = None;
    let last_timestamp_ms = rows.last().map(|r| r.timestamp_ms).unwrap_or(0);

    for r in &rows {
        if let Some(a) = r.baro_alt_m.or(r.alt_m) {
            if a > max_alt {
                max_alt = a;
            }
        }
        if let Some(s) = r.speed_ms {
            if s > max_speed {
                max_speed = s;
            }
        }
        if let Some(m) = r.mah_drawn {
            if start_mah.is_none() {
                start_mah = Some(m);
            }
            end_mah = Some(m);
        }
        if let (Some(lat), Some(lon)) = (r.lat, r.lon) {
            if let (Some(plat), Some(plon)) = (last_lat, last_lon) {
                total_distance += haversine_m(plat, plon, lat, lon);
            }
            if let (Some(slat), Some(slon)) = (start_lat, start_lon) {
                let d = haversine_m(slat, slon, lat, lon);
                if d > max_distance {
                    max_distance = d;
                }
            }
            last_lat = Some(lat);
            last_lon = Some(lon);
        }
    }

    let battery_used = match (start_mah, end_mah) {
        (Some(s), Some(e)) if e >= s => Some(e - s),
        _ => None,
    };
    let flight = Flight {
        id: 0,
        start_time,
        end_time: Some(start_time + chrono::Duration::milliseconds(last_timestamp_ms.max(0))),
        duration_sec: Some((last_timestamp_ms / 1000).max(0)),
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
        max_alt_m: Some(max_alt),
        max_speed_ms: Some(max_speed),
        max_distance_m: Some(max_distance),
        total_distance_m: Some(total_distance),
        battery_used_mah: battery_used,
        notes: None,
        linked_flight_id: None,
        pilot_name: None,
        pilot_id: None,
        battery_serial: None,
        // Live recording: the GCS sits at the flight location, so its own offset is the flight-local
        // offset (ADR-048).
        utc_offset_min: Some(super::timezone::local_offset_min_now()),
        group_id: None,
    };
    let count = rows.len() as i64;
    Ok((
        PendingSession {
            temp_path,
            db_path,
            flight,
            disarm_instant: Instant::now(),
            start_mah,
            last_timestamp_ms,
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
    emitter: VehicleEmitter,
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
    /// Multi-vehicle: a recorder for a SECONDARY vehicle on a shared link runs unattended — its flights
    /// commit on their own once the re-arm grace has passed (or on teardown) instead of opening the
    /// End-Flight dialog, which belongs to the primary vehicle.
    auto_commit: bool,
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

    /// `new` with an explicit time source (tests: `FakeClock`).
    #[allow(clippy::too_many_arguments)] // recorder construction context (settings + session metadata)
    pub fn with_clock(
        settings: FlightLogSettings,
        fc_info: FcInfo,
        protocol: &str,
        portable: bool,
        emitter: VehicleEmitter,
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
            emitter,
            slots,
            published_path: None,
            clock,
            first_status_seen: false,
            auto_commit: false,
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
        if let Err(e) = self.emitter.emit(event, payload) {
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

    /// Unattended mode (secondary vehicles): flights auto-commit after the re-arm grace / on teardown
    /// and announce `flight-recording-autocommitted` instead of opening the End-Flight dialog.
    pub fn set_auto_commit(&mut self, on: bool) {
        self.auto_commit = on;
    }

    /// Commit a parked session now (auto-commit mode) and tell the frontend to refresh the logbook.
    fn auto_commit_now(&self, p: PendingSession) {
        match commit_pending_session(p) {
            Ok(flight_id) => {
                log::info!("Flight auto-committed (secondary vehicle): id {}", flight_id);
                self.emit("flight-recording-autocommitted", FlightRecordingEvent { flight_id });
            }
            Err(e) => log::error!("Auto-commit failed: {}", e),
        }
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
        // Unattended (secondary) recorders never claim one — they had no resume slot before either.
        if !self.first_status_seen {
            self.first_status_seen = true;
            let resume = if self.auto_commit { None } else { self.slots.take_resume_for(&self.fc_info) };
            if let Some(p) = resume {
                if is_armed {
                    log::info!("Continue-on-reconnect: armed on first poll — resuming the recovered session");
                    self.resume_session(p);
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
        } else if !is_armed && self.auto_commit {
            // Unattended: once the re-arm grace has lapsed, the parked flight is final — commit it.
            let now = self.clock.instant();
            let lapsed = self.slots.take_pending_if(self.key(), |p| {
                now.saturating_duration_since(p.disarm_instant) >= REARM_GRACE
            });
            if let Some(p) = lapsed { self.auto_commit_now(p); }
        }

        self.was_armed = is_armed;
    }

    /// Move a finalized session into the shared pending slot and tell the frontend to show the
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
        // This vehicle's own pending session, else (attended recorders only) the one this aircraft
        // left on a connection that has closed since.
        let pending = self.slots.take_pending_for(self.key()).or_else(|| {
            if self.auto_commit { None } else { self.slots.take_detached_pending_for(self.key(), &self.fc_info) }
        });
        if let Some(p) = pending {
            if self.since(p.disarm_instant) < REARM_GRACE {
                self.resume_session(p);
                return;
            }
            // Grace lapsed → auto-commit the previous flight, then start a fresh session.
            if self.auto_commit {
                self.auto_commit_now(p);
                self.start_fresh_session();
                return;
            }
            match commit_pending_session(p) {
                Ok(flight_id) => {
                    self.emit("flight-recording-committed", FlightRecordingEvent { flight_id });
                }
                Err(e) => log::error!("Auto-commit on re-arm failed: {}", e),
            }
        }
        self.start_fresh_session();
    }

    /// Re-arm within the grace window — reopen the same `.ktmp` and continue the flight, with
    /// timestamps resuming where they left off (so the gap is real elapsed time, not a reset).
    fn resume_session(&mut self, p: PendingSession) {
        log::info!("Re-arm within grace — continuing the same recording");
        let temp_db = match db::open_temp_session(&p.temp_path) {
            Ok(c) => Some(c),
            Err(e) => {
                log::error!(
                    "Failed to reopen temp session {}: {} — starting a fresh one",
                    p.temp_path.display(), e,
                );
                self.start_fresh_session();
                return;
            }
        };
        let now = self.clock.instant();
        let start_instant = now
            .checked_sub(Duration::from_millis(p.last_timestamp_ms.max(0) as u64))
            .unwrap_or(now);
        self.active_flight = Some(ActiveFlight {
            temp_db,
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
        });
        self.publish_active_path();
        self.emit("flight-recording-resumed", ());
    }

    /// Open a brand-new recording session (temp store + raw logger) and announce it. Nothing is
    /// written to the main DB here — the real `flight_id` is born at commit (ADR-041).
    fn start_fresh_session(&mut self) {
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
        });
        self.publish_active_path();

        log::info!("Flight recording started (db={})", self.settings.db_enabled);

        // Id-less signal that recording is active (the frontend resets its flown-mission baseline;
        // the actual mission link happens on `flight-recording-ended` once the id exists).
        if self.settings.db_enabled {
            self.emit("flight-recording-started", ());
        }
    }

    /// Take the active flight, close its raw loggers, flush + close its temp store, and build the
    /// finalized `PendingSession` (+ its telemetry sample count). Returns `None` in raw-only mode
    /// (no DB session to commit). Shared by `on_disarm` and `shutdown` — they differ only in which
    /// lifecycle event they then emit.
    ///
    /// The file stays registered as live until the caller has parked (or committed) the session and
    /// calls `publish_active_path` — so it is protected from the discard sweeps at every moment.
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
        drop(temp_db); // checkpoint the WAL before any later ATTACH

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
            group_id: None,
        };
        Some((
            PendingSession {
                temp_path,
                db_path: self.db_file_path.clone(),
                flight: flight_row,
                disarm_instant: self.clock.instant(),
                start_mah: flight.start_mah,
                last_timestamp_ms,
            },
            sample_count,
        ))
    }

    /// Called when a disarm transition is detected. The flight is finalized as the pending session
    /// (deferred commit, ADR-041) and the frontend shows the End-Flight summary (Save / Discard); a
    /// re-arm resolves it instead.
    fn on_disarm(&mut self) {
        log::info!("DISARM detected — stopping flight recording");
        if let Some((session, _count)) = self.take_active_as_pending() {
            let dur = session.flight.duration_sec.unwrap_or(0);
            if self.auto_commit {
                // Unattended: park for the re-arm grace only; `on_status` commits it afterwards.
                self.slots.park_pending(self.key(), session);
                log::info!("Flight parked for auto-commit (disarm): {}s", dur);
            } else {
                self.stash_pending_and_emit_ended(session);
                log::info!("Flight pending commit (disarm): {}s", dur);
            }
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
        if self.auto_commit {
            let parked = self.slots.take_pending_for(self.key());
            if let Some(p) = parked { self.auto_commit_now(p); }
            if let Some((session, _)) = self.take_active_as_pending() { self.auto_commit_now(session); }
            self.publish_active_path();
            self.close_continuous_loggers();
            self.slots.detach(self.key());
            return;
        }
        if self.active_flight.is_some() {
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
        if self.auto_commit {
            self.shutdown(); // unattended: nothing to ask the operator — commit what there is
            return;
        }
        if self.active_flight.is_some() {
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
    fn resume_is_claimed_by_identity_then_oldest() {
        let slots = SessionSlots::default();
        slots.put_resume("old".into(), pending("old.ktmp", "Other", None, 0)).unwrap();
        slots.put_resume("mine".into(), pending("mine.ktmp", "Wing", Some("UID1"), 60)).unwrap();
        let fc = FcInfo { craft_name: "Renamed".into(), fc_variant: "INAV".into(), fc_uid: Some("UID1".into()), ..FcInfo::default() };
        // Hardware id wins over the older entry and over a craft-name mismatch.
        assert_eq!(slots.take_resume_for(&fc).unwrap().temp_path, PathBuf::from("mine.ktmp"));
        // No match left → the next connection takes the oldest (single-slot behaviour).
        assert_eq!(slots.take_resume_for(&fc).unwrap().temp_path, PathBuf::from("old.ktmp"));
        assert!(slots.take_resume_for(&fc).is_none());
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

        let result = summarize_temp_session(temp_path.clone(), PathBuf::from("unused.db"));
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
}
