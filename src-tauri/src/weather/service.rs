// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The one weather poller: wind by height at the aircraft, for the widget and the flight recorders.
// Runs in the backend so it keeps going on Android while the WebView is frozen.
// Faster than the model's 15-min step: a fast aircraft changes model cell within minutes.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager};

use super::{fetch_wind_aloft, WindAloft};
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(3);
const REFRESH_MS: i64 = 5 * 60_000;
const RETRY_MS: i64 = 60_000;
/// A position this far from the last request's is asked for at once (launch point → aircraft, vehicle switch).
const MOVED_M: f64 = 5_000.0;
/// A fix older than this is no position any more (link gone, vehicle switched off).
const GPS_FRESH: Duration = Duration::from_secs(15);
/// An answer older than this is not written into a flight that starts now.
const RECORD_MAX_AGE_MS: i64 = 30 * 60_000;

pub const EVENT: &str = "weather-wind-aloft";
const SOURCE: &str = "open-meteo";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindAloftState {
    pub data: Option<WindAloft>,
    /// When `data` arrived, Unix ms.
    pub fetched_at: i64,
    /// The last request failed (no internet, service down).
    pub failed: bool,
    pub no_position: bool,
}

struct Answer {
    /// Counts the answers, so a recorder stores each one once.
    seq: u64,
    lat: f64,
    lon: f64,
    json: Arc<str>,
}

// Times are wall clock: the monotonic clock stands still while the device sleeps.
#[derive(Default)]
struct Inner {
    /// A weather widget is on screen.
    watched: bool,
    /// The frontend's fallback position (FC home or the launch marker) for the time without a fix.
    reference: Option<(f64, f64)>,
    gps: Option<(f64, f64, Instant)>,
    state: WindAloftState,
    answer: Option<Answer>,
    asked: Option<(f64, f64)>,
    asked_from_gps: bool,
    next_at_ms: i64,
}

#[derive(Clone, Default)]
pub struct WeatherService {
    inner: Arc<Mutex<Inner>>,
}

impl WeatherService {
    pub fn new() -> Self {
        Self::default()
    }

    // The GPS tap runs on the telemetry thread: a poisoned lock must not take that thread down.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_watched(&self, on: bool) -> WindAloftState {
        let mut i = self.lock();
        i.watched = on;
        i.state.clone()
    }

    pub fn set_reference(&self, pos: Option<(f64, f64)>) {
        self.lock().reference = pos;
    }

    /// Tap the active vehicle's position and start the poller. Once, at app setup.
    pub fn start(&self, app: &AppHandle) {
        let tap = self.clone();
        let filter = app.clone();
        app.listen("telemetry-gps", move |ev| {
            if !crate::vehicle_registry::payload_is_active(&filter, ev.payload()) {
                return;
            }
            let Ok(g) = serde_json::from_str::<crate::telemetry_api::state::Gps>(ev.payload()) else { return };
            if g.fix_type >= 1 && (g.lat != 0.0 || g.lon != 0.0) {
                tap.lock().gps = Some((g.lat, g.lon, Instant::now()));
            }
        });
        let service = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                service.tick(&app).await;
                tokio::time::sleep(TICK).await;
            }
        });
    }

    async fn tick(&self, app: &AppHandle) {
        let recorders: Vec<_> = app
            .state::<AppState>()
            .recorders
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        // Without a widget on screen the setting decides whether a recording gets weather.
        let watched = self.lock().watched;
        let recording = (watched || super::records_with_flights())
            && recorders.iter().any(|r| r.lock().is_ok_and(|r| r.records_weather()));
        if !watched && !recording {
            return;
        }

        let request = {
            let mut i = self.lock();
            let fix = i.gps.filter(|(_, _, at)| at.elapsed() < GPS_FRESH).map(|(lat, lon, _)| (lat, lon));
            match fix.or(i.reference) {
                None => {
                    if !i.state.no_position {
                        i.state.no_position = true;
                        let _ = app.emit(EVENT, &i.state);
                    }
                    None
                }
                Some(pos) => {
                    // A telemetry gap drops back to the reference: that is no move of the aircraft.
                    let fell_back = fix.is_none() && i.asked_from_gps;
                    let moved = !fell_back && i.asked.is_some_and(|a| distance_m(a, pos) > MOVED_M);
                    if moved || unix_ms() >= i.next_at_ms {
                        i.asked = Some(pos);
                        i.asked_from_gps = fix.is_some();
                        Some(pos)
                    } else {
                        None
                    }
                }
            }
        };

        if let Some((lat, lon)) = request {
            // What goes out is what gets stored.
            let (lat, lon) = (super::coarse(lat), super::coarse(lon));
            let result = fetch_wind_aloft(lat, lon).await;
            let mut i = self.lock();
            let now = unix_ms();
            match result {
                Ok(data) => {
                    let seq = i.answer.as_ref().map_or(1, |a| a.seq + 1);
                    match serde_json::to_string(&data) {
                        Ok(json) => i.answer = Some(Answer { seq, lat, lon, json: json.into() }),
                        Err(e) => log::warn!("weather: answer not serialised for the recorder: {e}"),
                    }
                    i.state = WindAloftState { data: Some(data), fetched_at: now, failed: false, no_position: false };
                    i.next_at_ms = now + REFRESH_MS;
                }
                Err(_) => {
                    i.state.failed = true;
                    i.state.no_position = false;
                    i.next_at_ms = now + RETRY_MS;
                }
            }
            let _ = app.emit(EVENT, &i.state);
        }

        if !recording {
            return;
        }
        // Every tick, not only on arrival: a flight that starts between two requests gets the answer in hand.
        // Copied out first: the GPS tap must never wait behind a recorder's lock.
        let offer = {
            let i = self.lock();
            let fresh = unix_ms() - i.state.fetched_at < RECORD_MAX_AGE_MS;
            i.answer.as_ref().filter(|_| fresh).map(|a| (a.seq, a.lat, a.lon, a.json.clone()))
        };
        let Some((seq, lat, lon, json)) = offer else { return };
        for r in &recorders {
            if let Ok(mut r) = r.lock() {
                r.on_weather(seq, lat, lon, SOURCE, &json);
            }
        }
    }
}

fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let (dla, dlo) = (la2 - la1, (b.1 - a.1).to_radians());
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * h.sqrt().asin()
}
