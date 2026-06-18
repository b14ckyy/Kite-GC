// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Telemetry forwarding / conversion ("Relay").
//!
//! Re-encodes the live inbound telemetry (MSP / MAVLink / passive) into a chosen wire protocol and emits
//! it out a second link — for antenna trackers, monitoring apps or other GCS. See
//! `docs/active/TELEMETRY_FORWARDING.md`.
//!
//! Tap: the `RelayHub` registers backend listeners for the same `telemetry-*` events the decoders emit,
//! deserializes them into a latest-values cache, and fans each update out to N active relays. Zero
//! changes to the producers. Live-only (no replay). Rate = pass-through (one output frame per source
//! update; periodic frames come later for MAVLink).

pub mod cache;
pub mod encoders;
pub mod output;
pub mod relay;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, State};

use crate::scheduler::telemetry::{
    AirspeedData, AltitudeData, AnalogData, AttitudeData, GpsData, StatusData,
};

use cache::{TelemKind, TelemetryCache};
use relay::{Relay, RelayConfig, RelayStatusInfo};

/// Result of (re)configuring one relay, returned to the frontend so it can show device-missing/errors.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayResult {
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
    pub target: Option<String>,
}

/// Shared relay state, managed by Tauri. Listeners + stats thread are started once (lazily, on the first
/// `relay_configure`, which has the `AppHandle`).
pub struct RelayHub {
    cache: Arc<Mutex<TelemetryCache>>,
    relays: Arc<Mutex<Vec<Relay>>>,
    started: AtomicBool,
}

impl RelayHub {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(TelemetryCache::default())),
            relays: Arc::new(Mutex::new(Vec::new())),
            started: AtomicBool::new(false),
        }
    }

    /// Register the backend telemetry-event listeners + the stats thread (once).
    fn ensure_started(&self, app: &AppHandle) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }

        // One listener per unified event → update cache → fan out to relays.
        macro_rules! tap {
            ($event:literal, $ty:ty, $field:ident, $kind:expr) => {{
                let cache = self.cache.clone();
                let relays = self.relays.clone();
                app.listen($event, move |ev| {
                    if let Ok(d) = serde_json::from_str::<$ty>(ev.payload()) {
                        cache.lock().unwrap().$field = Some(d);
                        dispatch(&cache, &relays, $kind);
                    }
                });
            }};
        }
        tap!("telemetry-attitude", AttitudeData, attitude, TelemKind::Attitude);
        tap!("telemetry-gps", GpsData, gps, TelemKind::Gps);
        tap!("telemetry-altitude", AltitudeData, altitude, TelemKind::Altitude);
        tap!("telemetry-analog", AnalogData, analog, TelemKind::Analog);
        tap!("telemetry-status", StatusData, status, TelemKind::Status);
        tap!("telemetry-airspeed", AirspeedData, airspeed, TelemKind::Airspeed);

        // Stats thread: every 1 s push per-relay byte-rate + status to the frontend.
        let relays = self.relays.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let mut last: HashMap<String, u64> = HashMap::new();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let infos: Vec<RelayStatusInfo> = {
                    let r = relays.lock().unwrap();
                    r.iter()
                        .map(|relay| {
                            let prev = last.insert(relay.id.clone(), relay.bytes_out).unwrap_or(relay.bytes_out);
                            RelayStatusInfo {
                                id: relay.id.clone(),
                                protocol: relay.protocol.clone(),
                                target: relay.target.clone(),
                                ok: relay.ok,
                                waiting: relay.pending(),
                                bytes_per_sec: relay.bytes_out.saturating_sub(prev),
                                frames_out: relay.frames_out,
                                errors: relay.errors,
                            }
                        })
                        .collect()
                };
                let _ = app.emit("relay-stats", &infos);
            }
        });
    }

    /// Reconcile the active relays with the given configs. Relays whose config is **unchanged** are reused
    /// as-is — so editing one relay never rebinds another's TCP port or drops its connected clients.
    /// Removed/changed relays are dropped **first** (freeing their ports/sockets), then the new/changed
    /// ones are built. A short pause between the two lets a just-closed TCP listener fully release its
    /// port before the same port is potentially re-bound. (Build is async — no lock held across `.await`.)
    async fn configure(&self, configs: Vec<RelayConfig>) -> Vec<RelayResult> {
        let mut existing: Vec<Relay> = std::mem::take(&mut *self.relays.lock().unwrap());
        let mut built: Vec<Relay> = Vec::new();
        let mut to_build: Vec<RelayConfig> = Vec::new();
        let mut results = Vec::new();

        // First pass: keep exact-match relays, queue the rest for (re)building.
        for cfg in configs.into_iter().filter(|c| c.enabled) {
            if let Some(pos) = existing.iter().position(|r| r.config == cfg) {
                let relay = existing.remove(pos);
                results.push(RelayResult { id: cfg.id.clone(), ok: true, error: None, target: Some(relay.target.clone()) });
                built.push(relay);
            } else {
                to_build.push(cfg);
            }
        }

        // Drop the removed/changed relays now (releases ports/sockets), then pause so a closed TCP
        // listener's port is free before we possibly re-bind it.
        let had_removed = !existing.is_empty();
        drop(existing);
        if had_removed && !to_build.is_empty() {
            tokio::time::sleep(Duration::from_millis(120)).await;
        }

        for cfg in &to_build {
            match Relay::build(cfg).await {
                Ok(relay) => {
                    results.push(RelayResult { id: cfg.id.clone(), ok: true, error: None, target: Some(relay.target.clone()) });
                    built.push(relay);
                }
                Err(e) => {
                    log::warn!("[RELAY {}] not started: {}", cfg.id, e);
                    results.push(RelayResult { id: cfg.id.clone(), ok: false, error: Some(e), target: None });
                }
            }
        }

        *self.relays.lock().unwrap() = built;
        results
    }

    /// Tear down all relays (on primary disconnect).
    fn clear(&self) {
        self.relays.lock().unwrap().clear();
        *self.cache.lock().unwrap() = TelemetryCache::default();
    }
}

impl Default for RelayHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Decide whether this update should trigger a relay emission, and if so, push a full frame set to each
/// relay. Emission is **paced on the attitude update** (the always-present, highest-rate field) so the
/// output rate follows the real data rate, not the input's framing/republish cadence — e.g. SmartPort
/// re-emits all cached fields at 10 Hz. Attitude-less sources fall back to pacing on GPS.
fn dispatch(cache: &Mutex<TelemetryCache>, relays: &Mutex<Vec<Relay>>, kind: TelemKind) {
    let snapshot = cache.lock().unwrap().clone();
    let pace = match kind {
        TelemKind::Attitude => true,
        TelemKind::Gps => snapshot.attitude.is_none(),
        _ => false, // other updates only refresh the cache
    };
    if !pace {
        return;
    }
    let mut active = relays.lock().unwrap();
    if active.is_empty() {
        return;
    }
    for relay in active.iter_mut() {
        relay.emit_set(&snapshot);
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// (Re)configure the relays — called by the frontend on primary connect with the persisted configs.
#[tauri::command]
pub async fn relay_configure(app: AppHandle, hub: State<'_, RelayHub>, configs: Vec<RelayConfig>) -> Result<Vec<RelayResult>, String> {
    hub.ensure_started(&app);
    Ok(hub.configure(configs).await)
}

/// Tear down all relays — called on primary disconnect.
#[tauri::command]
pub fn relay_clear(hub: State<RelayHub>) {
    hub.clear();
}
