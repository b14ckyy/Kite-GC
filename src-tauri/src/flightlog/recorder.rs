// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight Recorder — detects arm/disarm transitions and records telemetry.
// Designed to be called from the scheduler thread with each decoded telemetry payload.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use rusqlite::Connection;
use tauri::{AppHandle, Emitter};

use super::db;
use super::raw_logger::RawLogger;
use super::tlog_logger::TlogLogger;
use super::types::{Flight, FlightLogSettings, TelemetryRecord};
use crate::msp::FcInfo;
use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, GpsStatsData, NavStatusData,
    SensorStatusData, StatusData,
};

/// Bit 2 in arming_flags indicates ARMED state
const ARMED_FLAG: u32 = 0x04; // bit 2

/// Payload for the `flight-recording-ended` event. The frontend uses `flight_id` to save + link
/// the flown mission to this DB flight (see docs/archive/MISSION_LIBRARY_AND_DB.md). The flight id
/// only exists at disarm now (commit-on-disarm, ADR-040), so `flight-recording-started` is an
/// id-less signal and carries no payload.
#[derive(serde::Serialize, Clone)]
struct FlightRecordingEvent {
    flight_id: i64,
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
    db: Connection,
    raw_logger: Option<RawLogger>,
    tlog_logger: Option<TlogLogger>,
    snapshot: TelemetrySnapshot,
    active_flight: Option<ActiveFlight>,
    was_armed: bool,
    /// Continuous session log start time (for session file naming)
    session_start: Option<chrono::DateTime<Utc>>,
    /// Track continuous-mode session start Instant for raw sample timestamps
    session_instant: Option<Instant>,
    /// For emitting flight-recording lifecycle events to the frontend.
    app_handle: AppHandle,
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
    ) -> Result<Self, String> {
        let db_path = db::resolve_db_path(&settings.db_path, portable);
        log::info!("Flight log database: {}", db_path.display());

        let db = db::open_database(&db_path).map_err(|e| {
            format!("Failed to open flight log database: {}", e)
        })?;

        Ok(Self {
            settings,
            fc_info,
            protocol: protocol.to_string(),
            db_file_path: db_path,
            db,
            raw_logger: None,
            tlog_logger: None,
            snapshot: TelemetrySnapshot::default(),
            active_flight: None,
            was_armed: false,
            session_start: None,
            session_instant: None,
            app_handle,
        })
    }

    /// Start continuous raw logging immediately on connect.
    /// Called when `raw_always` is enabled. Opens a session-level raw/tlog file
    /// that records all data (including pre-arm) until disconnect.
    pub fn start_continuous_log(&mut self) {
        if !self.settings.raw_always {
            return;
        }
        let now = Utc::now();
        let log_dir = self
            .db_file_path
            .parent()
            .unwrap_or(std::path::Path::new("."));

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
            match RawLogger::new(log_dir, 0, &now) {
                Ok(logger) => {
                    log::info!("Continuous raw session started");
                    self.raw_logger = Some(logger);
                }
                Err(e) => log::warn!("Failed to create continuous raw logger: {}", e),
            }
        }
        self.session_start = Some(now);
        self.session_instant = Some(Instant::now());
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

    /// Feed status data — this is where arm/disarm transitions are detected
    pub fn on_status(&mut self, data: &StatusData) {
        self.snapshot.arming_flags = Some(data.arming_flags);
        self.snapshot.cpu_load = Some(data.cpu_load);
        self.snapshot.active_flight_mode_flags = Some(data.flight_mode_flags);

        let is_armed = (data.arming_flags & ARMED_FLAG) != 0;

        if is_armed && !self.was_armed {
            self.on_arm();
        } else if !is_armed && self.was_armed {
            self.on_disarm();
        }

        self.was_armed = is_armed;
    }

    /// Called when arm transition is detected.
    ///
    /// Commit-on-disarm (ADR-040): nothing is written to the main DB here. When DB recording is on,
    /// a per-session temp `.ktmp` is opened — it is the durable in-flight buffer and is committed
    /// into the main DB atomically on disarm. The real `flight_id` therefore exists only at disarm,
    /// so `flight-recording-started` is an id-less signal.
    fn on_arm(&mut self) {
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
        let log_dir = self
            .db_file_path
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let raw_pseudo_id = now.timestamp();

        if !self.settings.raw_always {
            // Non-continuous: create per-flight raw/tlog logger
            let (raw_logger, tlog_logger) = if self.settings.raw_enabled {
                if self.protocol == "MAVLink" {
                    match TlogLogger::new(log_dir, raw_pseudo_id, &now) {
                        Ok(logger) => (None, Some(logger)),
                        Err(e) => {
                            log::warn!("Failed to create tlog logger: {}", e);
                            (None, None)
                        }
                    }
                } else {
                    match RawLogger::new(log_dir, raw_pseudo_id, &now) {
                        Ok(logger) => (Some(logger), None),
                        Err(e) => {
                            log::warn!("Failed to create raw logger: {}", e);
                            (None, None)
                        }
                    }
                }
            } else {
                (None, None)
            };
            self.raw_logger = raw_logger;
            self.tlog_logger = tlog_logger;
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

    /// Called when disarm transition is detected. Flushes the tail of the buffer to the temp store,
    /// closes it (WAL checkpoint), then commits it into the main DB atomically as one finalized
    /// flight (commit-on-disarm, ADR-040) and removes the temp file. The real `flight_id` is born
    /// here, so this is where the frontend is told to link the flown mission.
    fn on_disarm(&mut self) {
        log::info!("DISARM detected — stopping flight recording");

        if let Some(mut flight) = self.active_flight.take() {
            let end_time = Utc::now();
            let duration = flight.start_instant.elapsed().as_secs() as i64;
            let battery_used = match (flight.start_mah, self.snapshot.mah_drawn) {
                (Some(start), Some(end)) if end >= start => Some(end - start),
                _ => None,
            };

            // Commit the temp session into the main DB (DB recording only).
            if let (Some(temp_db), Some(temp_path)) = (flight.temp_db.take(), flight.temp_path.clone())
            {
                // Flush the remaining buffer into the temp store, then close it so its WAL is
                // checkpointed before the main connection ATTACHes the file.
                if !flight.buffer.is_empty() {
                    if let Err(e) = db::insert_telemetry_batch(&temp_db, &flight.buffer) {
                        log::error!("Failed to flush final telemetry batch to temp store: {}", e);
                    }
                }
                drop(temp_db);

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
                };

                match db::commit_session_to_main(&self.db, &temp_path, &flight_row) {
                    Ok(flight_id) => {
                        db::remove_temp_session(&temp_path);
                        log::info!(
                            "Flight {} committed: {}s, max_alt={:.1}m, max_speed={:.1}m/s, distance={:.0}m",
                            flight_id, duration, flight.max_alt, flight.max_speed, flight.total_distance,
                        );

                        // Deferred enrichment: weather + geocode against the now-committed flight id.
                        if let (Some(lat), Some(lon)) = (flight.start_lat, flight.start_lon) {
                            if is_valid_gps_coord(lat, lon) {
                                let db_path = self.db_file_path.to_string_lossy().to_string();
                                tauri::async_runtime::spawn(enrich_flight_async(flight_id, lat, lon, db_path));
                            }
                        }

                        // The frontend links the flown mission now that the DB flight exists.
                        if let Err(e) = self.app_handle.emit(
                            "flight-recording-ended",
                            FlightRecordingEvent { flight_id },
                        ) {
                            log::warn!("Failed to emit flight-recording-ended: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to commit session to main DB: {} — temp file kept for recovery at {}",
                            e, temp_path.display(),
                        );
                    }
                }
            } else {
                // Raw-only mode (no temp store / no DB flight): nothing to commit.
                log::info!(
                    "Flight ended (raw-only): {}s, max_alt={:.1}m, max_speed={:.1}m/s, distance={:.0}m",
                    duration, flight.max_alt, flight.max_speed, flight.total_distance,
                );
            }

            // Close raw/tlog logger only in non-continuous mode
            if !self.settings.raw_always {
                if let Some(mut logger) = self.raw_logger.take() {
                    logger.close();
                }
                if let Some(mut logger) = self.tlog_logger.take() {
                    logger.close();
                }
            }
            // else: continuous mode — loggers stay open until disconnect
        }
    }

    /// Record a telemetry sample if we have an active flight or continuous logging.
    /// Called after attitude or GPS updates (the highest-frequency data).
    fn maybe_record_sample(&mut self) {
        // Continuous mode: write to raw logger even without an active flight
        if self.active_flight.is_none() && self.settings.raw_always {
            if let Some(session_instant) = &self.session_instant {
                let elapsed_ms = session_instant.elapsed().as_millis() as i64;
                let record = TelemetryRecord {
                    id: 0,
                    flight_id: 0, // no DB flight
                    timestamp_ms: elapsed_ms,
                    lat: self.snapshot.lat,
                    lon: self.snapshot.lon,
                    alt_m: self.snapshot.alt_gps, // GPS MSL (replay map height); baro is relative
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
                    link_quality: None,
                    baro_alt_m: self.snapshot.alt_baro,
                    gps_hdop: self.snapshot.gps_hdop, gps_eph: self.snapshot.gps_eph, gps_epv: self.snapshot.gps_epv,
                    active_wp_number: self.snapshot.active_wp_number,
                    active_flight_mode_flags: self.snapshot.active_flight_mode_flags.map(|f| f as i64),
                    state_flags: None, nav_state: self.snapshot.nav_state, nav_flags: None,
                    rx_signal_received: None, hw_health_status: self.snapshot.hw_health_status, baro_temperature: None,
                    wind_n_ms: None, wind_e_ms: None, wind_d_ms: None,
                    rc_data_json: None, rc_command_json: None,
                    nav_lat: None, nav_lon: None, nav_alt_m: None,
                };
                if let Some(ref mut logger) = self.raw_logger {
                    logger.write_record(&record);
                }
                // tlog is written via write_raw_mavlink_frame(), not here
            }
            return;
        }

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
        };

        // Write to raw logger if active
        if let Some(ref mut logger) = self.raw_logger {
            logger.write_record(&record);
        }

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

    /// Graceful shutdown — flush and finalize any active flight.
    pub fn shutdown(&mut self) {
        if self.active_flight.is_some() {
            log::info!("Recorder shutdown with active flight — forcing disarm");
            self.on_disarm();
        }
        // Always close continuous loggers on disconnect
        if let Some(mut logger) = self.raw_logger.take() {
            log::info!("Closing continuous raw log session");
            logger.close();
        }
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
