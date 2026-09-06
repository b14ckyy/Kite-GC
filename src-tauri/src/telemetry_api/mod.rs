// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Telemetry API — Kite's read-only live-telemetry feed for external programs.
//!
//! Serves everything the unified telemetry model holds (the Raw Telemetry popup's content) as JSON:
//! an NDJSON stream over TCP (port 27300), one datagram per frame over UDP (configured target), and
//! HTTP GET snapshots (port 27301). Backend-only by decision (Dev-Docs `active/TELEMETRY_API.md`): it
//! keeps serving on Android while the WebView is paused, and it needs no frontend to be open.
//!
//! Tap: like the relay hub, the API registers backend listeners for the `telemetry-*` events the
//! decoders emit (plus `home-position`) and keeps the newest value of every group in `state.rs`. The
//! frontend contributes exactly one thing — the resolved GCS position (`telemetry_api_set_gcs`).
//! Producers are untouched. Wire contract: `frame.rs`.

pub mod frame;
pub mod http;
pub mod server;
pub mod state;
pub mod tcp;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Listener, State};

use server::Running;
use state::{ApiState, Gcs};

/// `settings.telemetryApi`, pushed by the frontend on every change (idempotent reconfigure).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiConfig {
    pub enabled: bool,
    /// NDJSON stream server on TCP 27300.
    pub tcp: bool,
    /// GET snapshot/health server on TCP 27301.
    pub http: bool,
    pub udp: UdpTarget,
    /// Bind the TCP listeners on all interfaces (LAN) instead of loopback only.
    pub lan: bool,
    /// Frames per second, 1–10.
    pub rate_hz: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UdpTarget {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}

/// What the settings row shows.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub running: bool,
    pub tcp_endpoint: Option<String>,
    pub http_endpoint: Option<String>,
    pub udp_target: Option<String>,
    pub clients: usize,
    pub error: Option<String>,
}

/// Managed by Tauri. Listeners are registered once, lazily, on the first `telemetry_api_configure`
/// (which has the `AppHandle`); the state fills regardless of whether the server is running, so
/// enabling it mid-flight serves the current values immediately.
pub struct TelemetryApi {
    state: Arc<Mutex<ApiState>>,
    running: Mutex<Option<Running>>,
    last_error: Mutex<Option<String>>,
    started: AtomicBool,
}

impl TelemetryApi {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ApiState::default())),
            running: Mutex::new(None),
            last_error: Mutex::new(None),
            started: AtomicBool::new(false),
        }
    }

    fn ensure_started(&self, app: &AppHandle) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        // One listener per unified event → its slot. `stamp` marks FC telemetry (not home / link
        // housekeeping) so `lastUpdate` means "the FC last said something".
        macro_rules! tap {
            ($event:literal, $ty:ty, $field:ident, $stamp:expr) => {{
                let state = self.state.clone();
                app.listen($event, move |ev| match serde_json::from_str::<$ty>(ev.payload()) {
                    Ok(d) => {
                        let mut st = state.lock().unwrap();
                        st.$field = Some(d);
                        if $stamp {
                            st.last_update_ms = Some(now_ms());
                        }
                    }
                    Err(e) => log::debug!("[telemetry-api] {} payload not understood: {e}", $event),
                });
            }};
        }
        tap!("telemetry-attitude", state::Attitude, attitude, true);
        tap!("telemetry-gps", state::Gps, gps, true);
        tap!("telemetry-gps-stats", state::GpsStats, gps_stats, true);
        tap!("telemetry-altitude", state::Altitude, altitude, true);
        tap!("telemetry-alt-ref", state::AltRef, alt_ref, false);
        tap!("telemetry-analog", state::Analog, analog, true);
        tap!("telemetry-batteries", Vec<state::BatteryInstance>, batteries, true);
        tap!("telemetry-status", state::Status, status, true);
        tap!("telemetry-sensor-status", state::Sensors, sensors, true);
        tap!("telemetry-flightmode", state::FlightMode, flight_mode, true);
        tap!("telemetry-nav-status", state::Nav, nav, true);
        tap!("telemetry-ekf-status", state::EkfStatus, ekf_status, true);
        tap!("telemetry-ekf-type", state::EkfType, ekf_type, false);
        tap!("telemetry-linkstats", state::Link, link, true);
        tap!("telemetry-fc-link", state::FcLink, fc_link, false);
        tap!("telemetry-misc2", state::Misc, misc, true);
        tap!("telemetry-wind", state::Wind, wind, true);
        tap!("telemetry-airspeed", state::Airspeed, airspeed, true);
        tap!("telemetry-vehicle", state::Vehicle, vehicle, false);
        tap!("telemetry-protocol", state::PassiveProtocol, passive_protocol, false);
        tap!("home-position", state::Home, home, false);
    }

    /// Apply a config: start, stop, or rebuild when it changed. Unchanged → keep the running server
    /// (and its clients). Returns the status the settings row shows.
    fn configure(&self, app: &AppHandle, cfg: ApiConfig) -> ApiStatus {
        let mut running = self.running.lock().unwrap();
        if !cfg.enabled {
            if running.take().is_some() {
                log::info!("[telemetry-api] stopped");
            }
            *self.last_error.lock().unwrap() = None;
            return self.status_locked(&running);
        }
        if running.as_ref().is_some_and(|r| r.config == cfg) {
            return self.status_locked(&running);
        }
        // Drop the old server first so its ports are free for the rebuild.
        if running.take().is_some() {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        match Running::start(app.clone(), self.state.clone(), cfg) {
            Ok(r) => {
                log::info!(
                    "[telemetry-api] started: tcp={} http={} udp={}",
                    r.tcp_addr.as_deref().unwrap_or("off"),
                    r.http_addr.as_deref().unwrap_or("off"),
                    r.udp_target.as_deref().unwrap_or("off"),
                );
                *self.last_error.lock().unwrap() = None;
                *running = Some(r);
            }
            Err(e) => {
                log::warn!("[telemetry-api] not started: {e}");
                *self.last_error.lock().unwrap() = Some(e);
            }
        }
        self.status_locked(&running)
    }

    fn status_locked(&self, running: &Option<Running>) -> ApiStatus {
        match running {
            Some(r) => ApiStatus {
                running: true,
                tcp_endpoint: r.tcp_addr.clone(),
                http_endpoint: r.http_addr.clone(),
                udp_target: r.udp_target.clone(),
                clients: r.clients(),
                error: None,
            },
            None => ApiStatus { error: self.last_error.lock().unwrap().clone(), ..Default::default() },
        }
    }

    pub fn status(&self) -> ApiStatus {
        self.status_locked(&self.running.lock().unwrap())
    }
}

impl Default for TelemetryApi {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// (Re)configure the API — called on every `settings.telemetryApi` change and once at start-up.
#[tauri::command]
pub fn telemetry_api_configure(app: AppHandle, api: State<TelemetryApi>, config: ApiConfig) -> ApiStatus {
    api.ensure_started(&app);
    api.configure(&app, config)
}

/// Current status for the settings row (endpoints, client count, last error).
#[tauri::command]
pub fn telemetry_api_status(api: State<TelemetryApi>) -> ApiStatus {
    api.status()
}

/// The frontend's resolved GCS position (the map's GCS marker). `None` clears it (marker off).
#[tauri::command]
pub fn telemetry_api_set_gcs(
    api: State<TelemetryApi>,
    gcs: Option<GcsInput>,
) {
    let mut st = api.state.lock().unwrap();
    st.gcs = gcs.map(|g| Gcs { lat: g.lat, lon: g.lon, alt_msl: g.alt_msl, accuracy_m: g.accuracy_m });
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GcsInput {
    pub lat: f64,
    pub lon: f64,
    pub alt_msl: Option<f64>,
    pub accuracy_m: Option<f64>,
}

