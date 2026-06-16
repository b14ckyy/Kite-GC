// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Recorder — detects arm/disarm transitions and records telemetry.
// Designed to be called from the scheduler thread with each decoded telemetry payload.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::Connection;
use tauri::{AppHandle, Emitter};

use super::db;
use super::msp_raw_logger::{MspRawLogger, MspRawSink};
use super::tlog_logger::TlogLogger;
use super::types::{Flight, FlightLogSettings, TelemetryRecord};
use crate::msp::FcInfo;
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, GpsStatsData, NavStatusData,
    SensorStatusData, StatusData,
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

/// Shared, connection-independent slot for the one pending session (see `state::AppState`).
pub type PendingSessionHandle = Arc<Mutex<Option<PendingSession>>>;

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
    let rows = db::get_flight_track(&conn, 0)
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
    yaw: Option<i16>,
    // GPS
    lat: Option<f64>,
    lon: Option<f64>,
    alt_gps: Option<f64>,
    speed: Option<f64>,
    heading: Option<i16>,
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
    // Airspeed
    airspeed: Option<f64>,
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
    /// For emitting flight-recording lifecycle events to the frontend.
    app_handle: AppHandle,
    /// Shared slot for the pending (awaiting commit/discard) session — also read on the next arm for
    /// the grace decision. Shared with app-state so it outlives this connection (ADR-041).
    pending: PendingSessionHandle,
    /// A recovered session the user chose to continue on reconnect (ADR-042), consulted once on the
    /// first polled status of this connection (armed → resume; disarmed → finalize).
    resume: PendingSessionHandle,
    /// Whether the first polled status has been seen on this connection (the trustworthy point to
    /// evaluate the continue-on-reconnect decision — past any handshake residual flags).
    first_status_seen: bool,
}

/// Thread-safe handle to the flight recorder
pub type FlightRecorderHandle = Arc<Mutex<FlightRecorder>>;

impl FlightRecorder {
    /// Create a new recorder. Returns None if logging is disabled.
    /// `protocol` should be "MSP" or "MAVLink".
    pub fn new(
        settings: FlightLogSettings,
        fc_info: FcInfo,
        protocol: &str,
        portable: bool,
        app_handle: AppHandle,
        pending: PendingSessionHandle,
        resume: PendingSessionHandle,
        msp_raw_sink: MspRawSink,
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
            app_handle,
            pending,
            resume,
            first_status_seen: false,
        })
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
        let now = Utc::now();
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
        self.snapshot.heading = Some(data.course as i16);
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

    /// Feed airspeed data from the scheduler
    pub fn on_airspeed(&mut self, data: &AirspeedData) {
        self.snapshot.airspeed = Some(data.airspeed);
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
        if !self.first_status_seen {
            self.first_status_seen = true;
            let resume = self.resume.lock().ok().and_then(|mut s| s.take());
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
            battery_used_mah: session.flight.battery_used_mah,
        };
        if let Ok(mut slot) = self.pending.lock() {
            *slot = Some(session);
        }
        if let Err(e) = self.app_handle.emit("flight-recording-ended", ev) {
            log::warn!("Failed to emit flight-recording-ended: {}", e);
        }
    }

    /// Called when an arm transition is detected. Resolves any pending session first (deferred
    /// commit + grace, ADR-041): a re-arm within the grace window continues the SAME log; beyond it
    /// the previous flight is auto-committed and a fresh session starts.
    fn on_arm(&mut self) {
        let pending = self.pending.lock().ok().and_then(|mut s| s.take());
        if let Some(p) = pending {
            if p.disarm_instant.elapsed() < REARM_GRACE {
                self.resume_session(p);
                return;
            }
            // Grace lapsed → auto-commit the previous flight, then start a fresh session.
            match commit_pending_session(p) {
                Ok(flight_id) => {
                    if let Err(e) = self
                        .app_handle
                        .emit("flight-recording-committed", FlightRecordingEvent { flight_id })
                    {
                        log::warn!("Failed to emit flight-recording-committed: {}", e);
                    }
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
        let start_instant = Instant::now()
            .checked_sub(Duration::from_millis(p.last_timestamp_ms.max(0) as u64))
            .unwrap_or_else(Instant::now);
        self.active_flight = Some(ActiveFlight {
            temp_db,
            temp_path: Some(p.temp_path),
            start_time: p.flight.start_time,
            start_instant,
            start_lat: p.flight.start_lat,
            start_lon: p.flight.start_lon,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
            max_alt: p.flight.max_alt_m.unwrap_or(0.0),
            max_speed: p.flight.max_speed_ms.unwrap_or(0.0),
            max_distance: p.flight.max_distance_m.unwrap_or(0.0),
            total_distance: p.flight.total_distance_m.unwrap_or(0.0),
            last_lat: None,
            last_lon: None,
            start_mah: p.start_mah,
        });
        if let Err(e) = self.app_handle.emit("flight-recording-resumed", ()) {
            log::warn!("Failed to emit flight-recording-resumed: {}", e);
        }
    }

    /// Open a brand-new recording session (temp store + raw logger) and announce it. Nothing is
    /// written to the main DB here — the real `flight_id` is born at commit (ADR-041).
    fn start_fresh_session(&mut self) {
        log::info!("ARM detected — starting flight recording");

        let now = Utc::now();

        // Open the per-session temp store (DB recording only).
        let (temp_db, temp_path) = if self.settings.db_enabled {
            let sessions_dir = self
                .db_file_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("sessions");
            let path = sessions_dir.join(format!("active_{}.ktmp", now.format("%Y-%m-%d_%H%M%S")));
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
            start_instant: Instant::now(),
            start_lat: self.snapshot.lat,
            start_lon: self.snapshot.lon,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
            max_alt: 0.0,
            max_speed: 0.0,
            max_distance: 0.0,
            total_distance: 0.0,
            last_lat: None,
            last_lon: None,
            start_mah: self.snapshot.mah_drawn,
        });

        log::info!("Flight recording started (db={})", self.settings.db_enabled);

        // Id-less signal that recording is active (the frontend resets its flown-mission baseline;
        // the actual mission link happens on `flight-recording-ended` once the id exists).
        if self.settings.db_enabled {
            if let Err(e) = self.app_handle.emit("flight-recording-started", ()) {
                log::warn!("Failed to emit flight-recording-started: {}", e);
            }
        }
    }

    /// Take the active flight, close its raw loggers, flush + close its temp store, and build the
    /// finalized `PendingSession` (+ its telemetry sample count). Returns `None` in raw-only mode
    /// (no DB session to commit). Shared by `on_disarm` and `shutdown` — they differ only in which
    /// lifecycle event they then emit.
    fn take_active_as_pending(&mut self) -> Option<(PendingSession, i64)> {
        let mut flight = self.active_flight.take()?;
        let end_time = Utc::now();
        let duration = flight.start_instant.elapsed().as_secs() as i64;
        let last_timestamp_ms = flight.start_instant.elapsed().as_millis() as i64;
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
        };
        Some((
            PendingSession {
                temp_path,
                db_path: self.db_file_path.clone(),
                flight: flight_row,
                disarm_instant: Instant::now(),
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
            self.stash_pending_and_emit_ended(session);
            log::info!("Flight pending commit (disarm): {}s", dur);
        }
    }

    /// Record a telemetry sample into the active flight's temp store / statistics.
    /// Called after attitude or GPS updates (the highest-frequency data). Raw serial logging is
    /// independent — it happens at the transport (tlog frames / MSP raw sink), not here.
    fn maybe_record_sample(&mut self) {
        let flight = match &mut self.active_flight {
            Some(f) => f,
            None => return,
        };

        let elapsed_ms = flight.start_instant.elapsed().as_millis() as i64;

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
            link_quality: None, // MSP does not expose LQ; populated via Blackbox import
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
            wind_n_ms: None,
            wind_e_ms: None,
            wind_d_ms: None,
            rc_data_json: None,
            rc_command_json: None,
            nav_lat: None,
            nav_lon: None,
            nav_alt_m: None,
            mode_primary: self.snapshot.mode_primary.clone(),
            mode_modifiers: self.snapshot.mode_modifiers.clone(),
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
            }
        }
    }

    /// Graceful shutdown (disconnect). An **active (armed) flight** is finalized as a pending session
    /// and the frontend is shown the **recovery prompt** via `flight-recording-interrupted` — the
    /// flight may not be over (the user could be changing the COM port or switching to telemetry), so
    /// Continue-on-Reconnect must be offered (ADR-042), not the End-Flight dialog.
    pub fn shutdown(&mut self) {
        if self.active_flight.is_some() {
            log::info!("Disconnect with active flight — stashed as pending (frontend confirmed, applies the action)");
            if let Some((session, _count)) = self.take_active_as_pending() {
                if let Ok(mut slot) = self.pending.lock() {
                    *slot = Some(session); // resolved by the frontend's Save/Discard/Continue
                }
            }
        }
        self.close_continuous_loggers();
    }

    /// Connection lost (device gone — e.g. USB unplugged), detected by the scheduler. Like `shutdown`,
    /// but emits `flight-recording-interrupted` so the frontend shows the recovery prompt — there was
    /// no chance to pre-confirm the disconnect (ADR-042).
    pub fn shutdown_lost(&mut self) {
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
                if let Ok(mut slot) = self.pending.lock() {
                    *slot = Some(session); // Save/Discard via commands; Continue moves it to resume
                }
                if let Err(e) = self.app_handle.emit("flight-recording-interrupted", ev) {
                    log::warn!("Failed to emit flight-recording-interrupted: {}", e);
                }
            }
        }
        self.close_continuous_loggers();
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
