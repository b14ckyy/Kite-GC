// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Link state for the OS layer (Dev-Docs active/BACKGROUND_TELEMETRY.md): the values the Android
//! foreground-service notification shows, plus the current flight's track points so the frontend
//! can close its trail gap after being hidden (`telemetry_track_since`). Fed next to the recorder
//! feeds in every protocol path — not inside the recorder, which only exists while logging is on.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::flightmode::FlightModeState;
use crate::msp::types::FcInfo;
use crate::scheduler::telemetry::{AnalogData, GpsData, StatusData};

/// INAV `armingFlags` ARMED bit; MAVLink and the passive decoders map onto the same bit.
const ARMED_FLAG: u32 = 1 << 2;

/// Track-point spacing (m) — the same gate as the recorder and the frontend `liveTrack`.
const MIN_TRACK_SPACING_M: f64 = 5.0;

/// Current flight only (the DB is the archive); the oldest points go first.
const MAX_TRACK_POINTS: usize = 20_000;

/// One flown point, as the frontend's `liveTrack` wants it.
#[derive(Clone, Serialize)]
pub struct TrackPoint {
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: f64,
    /// Canonical flight-mode id (`FlightModeState::primary`), for the per-mode trail colour.
    pub mode: String,
    pub ts_ms: i64,
}

struct Status {
    vehicle: String,
    /// "MSP over Serial" — protocol + transport, for the notification title.
    protocol: String,
    transport: String,
    armed: bool,
    mode: String,
    voltage: f64,
    percent: u8,
    fix: Option<(f64, f64)>,
    home: Option<(f64, f64)>,
    /// Home came from the FC (MSP_WP 0 / HOME_POSITION) rather than the arm-edge fallback.
    home_from_fc: bool,
    /// Arm-edge time of the buffered flight (0 = no flight seen on this link yet).
    flight_start_ms: i64,
    track: Vec<TrackPoint>,
}

const EMPTY: Status = Status {
    vehicle: String::new(),
    protocol: String::new(),
    transport: String::new(),
    armed: false,
    mode: String::new(),
    voltage: 0.0,
    percent: 0,
    fix: None,
    home: None,
    home_from_fc: false,
    flight_start_ms: 0,
    track: Vec::new(),
};

static STATUS: Mutex<Status> = Mutex::new(EMPTY);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn lock() -> std::sync::MutexGuard<'static, Status> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner())
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (la1, la2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * h.sqrt().min(1.0).asin()
}

fn valid_fix(d: &GpsData) -> bool {
    d.fix_type >= 2 && (d.lat != 0.0 || d.lon != 0.0) && d.lat.abs() <= 90.0 && d.lon.abs() <= 180.0
}

/// Set by `connect()` before the protocol path runs — the inner functions know only the protocol.
pub fn set_transport(transport: &str) {
    lock().transport = transport.to_string();
}

/// A link came up: forget the previous one, remember who is on the other end.
pub fn on_link_up(fc: &FcInfo, protocol: &str) {
    let mut s = lock();
    let transport = std::mem::take(&mut s.transport);
    *s = EMPTY;
    s.transport = transport;
    s.protocol = protocol.to_string();
    s.vehicle = if fc.craft_name.trim().is_empty() {
        format!("{} {}", fc.fc_variant, fc.board_id).trim().to_string()
    } else {
        fc.craft_name.trim().to_string()
    };
}

/// The link is gone. The track stays readable until the next link (a frontend may still backfill).
pub fn on_link_down() {
    let mut s = lock();
    s.armed = false;
    s.protocol.clear();
    s.transport.clear();
}

pub fn on_status(d: &StatusData) {
    let armed = d.arming_flags & ARMED_FLAG != 0;
    let mut s = lock();
    if armed && !s.armed {
        // Arm edge = new flight; the FC sets home at launch, so a known home is stale.
        s.track.clear();
        s.flight_start_ms = now_ms();
        s.home = None;
        s.home_from_fc = false;
    }
    s.armed = armed;
}

pub fn on_flightmode(fm: &FlightModeState) {
    lock().mode = fm.primary.clone();
}

pub fn on_analog(d: &AnalogData) {
    let mut s = lock();
    s.voltage = d.voltage;
    s.percent = d.battery_percentage;
}

/// Authoritative home from the FC (MSP_WP 0 at connect, MAVLink HOME_POSITION).
pub fn on_home(lat: f64, lon: f64) {
    if lat == 0.0 && lon == 0.0 {
        return;
    }
    let mut s = lock();
    s.home = Some((lat, lon));
    s.home_from_fc = true;
}

pub fn on_gps(d: &GpsData) {
    if !valid_fix(d) {
        return;
    }
    let mut s = lock();
    s.fix = Some((d.lat, d.lon));
    if !s.armed {
        return;
    }
    // First armed fix without an FC home = the launch point (the frontend's fallback).
    if s.home.is_none() {
        s.home = Some((d.lat, d.lon));
    }
    if let Some(last) = s.track.last() {
        if haversine_m(last.lat, last.lon, d.lat, d.lon) < MIN_TRACK_SPACING_M {
            return;
        }
    }
    if s.track.len() >= MAX_TRACK_POINTS {
        let drop = s.track.len() - MAX_TRACK_POINTS + 1;
        s.track.drain(..drop);
    }
    let mode = s.mode.clone();
    s.track.push(TrackPoint { lat: d.lat, lon: d.lon, alt_msl: d.alt_msl, mode, ts_ms: now_ms() });
}

/// Notification (title, text): vehicle · link, then the values the link carries (metric).
pub fn notification() -> (String, String) {
    let s = lock();
    let vehicle = if s.vehicle.is_empty() { "Kite Ground Control".to_string() } else { s.vehicle.clone() };
    let title = if s.protocol.is_empty() {
        vehicle
    } else if s.transport.is_empty() {
        format!("{vehicle} · {}", s.protocol)
    } else {
        format!("{vehicle} · {} over {}", s.protocol, s.transport)
    };

    let mut parts: Vec<String> = Vec::new();
    parts.push(if s.armed { "Armed".to_string() } else { "Disarmed".to_string() });
    if !s.mode.is_empty() {
        parts.push(s.mode.to_uppercase());
    }
    match (s.percent > 0, s.voltage > 0.5) {
        (true, true) => parts.push(format!("{} % · {:.1} V", s.percent, s.voltage)),
        (true, false) => parts.push(format!("{} %", s.percent)),
        (false, true) => parts.push(format!("{:.1} V", s.voltage)),
        (false, false) => {}
    }
    if s.armed {
        if let (Some((hl, ho)), Some((fl, fo))) = (s.home, s.fix) {
            let m = haversine_m(hl, ho, fl, fo);
            parts.push(if m >= 1000.0 { format!("Home {:.1} km", m / 1000.0) } else { format!("Home {} m", m.round() as i64) });
        }
    }
    (title, parts.join(" · "))
}

/// The buffered flight's arm time (new flight vs continuation) and its points after `since_ms`.
#[derive(Serialize)]
pub struct TrackSince {
    pub flight_start_ms: i64,
    pub points: Vec<TrackPoint>,
}

/// Points newer than `since_ms` — asked on `visibilitychange → visible` with the last point's time.
#[tauri::command]
pub fn telemetry_track_since(since_ms: i64) -> TrackSince {
    let s = lock();
    let points: Vec<TrackPoint> = s.track.iter().filter(|p| p.ts_ms > since_ms).cloned().collect();
    // Info: the only trace of what the frontend missed (a release WebView logs no console output).
    log::info!(
        "[track] backfill: {} of {} buffered points newer than {since_ms} (flight start {})",
        points.len(),
        s.track.len(),
        s.flight_start_ms
    );
    TrackSince { flight_start_ms: s.flight_start_ms, points }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gps(lat: f64, lon: f64) -> GpsData {
        GpsData { fix_type: 3, num_sat: 12, lat, lon, alt_msl: 100.0, ground_speed: 10.0, course: 0.0 }
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

    /// The tests share the process-wide state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn track_buffers_only_while_armed_and_spaced() {
        let _g = TEST_LOCK.lock().unwrap();
        *lock() = EMPTY;
        on_gps(&gps(48.0, 11.0)); // disarmed: fix only
        assert!(telemetry_track_since(0).points.is_empty());
        on_status(&status(true));
        on_gps(&gps(48.0, 11.0));
        on_gps(&gps(48.00001, 11.0)); // ~1 m: gated
        on_gps(&gps(48.001, 11.0)); // ~111 m: kept
        let t = telemetry_track_since(0);
        assert_eq!(t.points.len(), 2);
        assert!(t.flight_start_ms > 0);
        // A re-arm starts a fresh buffer.
        on_status(&status(false));
        on_status(&status(true));
        assert!(telemetry_track_since(0).points.is_empty());
    }

    #[test]
    fn notification_names_what_the_link_carries() {
        let _g = TEST_LOCK.lock().unwrap();
        *lock() = EMPTY;
        let fc = FcInfo { craft_name: "Skywalker".into(), fc_variant: "INAV".into(), board_id: "MATF".into(), ..Default::default() };
        set_transport("Serial");
        on_link_up(&fc, "MSP");
        let (title, text) = notification();
        assert_eq!(title, "Skywalker · MSP over Serial");
        assert_eq!(text, "Disarmed");
        on_status(&status(true));
        on_flightmode(&FlightModeState { primary: "cruise".into(), modifiers: vec![] });
        on_analog(&AnalogData { voltage: 12.43, mah_drawn: 0, rssi: 0, current: 0.0, power: 0.0, battery_percentage: 78, cell_count: 3 });
        on_gps(&gps(48.0, 11.0)); // launch = home fallback
        on_gps(&gps(48.01, 11.0)); // ~1.1 km out
        let (_, text) = notification();
        assert_eq!(text, "Armed · CRUISE · 78 % · 12.4 V · Home 1.1 km");
        on_home(48.005, 11.0); // FC home overrides the fallback
        assert!(notification().1.ends_with("Home 556 m"));
    }
}
