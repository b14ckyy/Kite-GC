// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — foreign-vehicle tracking subsystem.
//
// A standalone background subsystem that tracks vehicles OTHER than the main MSP/MAVLink-connected
// UAV (ADS-B / FormationFlight / radio telemetry), fully independent of the exclusive telemetry path
// (it never opens a second handle to the scheduler's link). Sources push batches into an aggregator
// thread that does per-system merge (by id) + TTL expiry and emits one consolidated `radar-vehicles`
// snapshot event. See docs/active/RADAR_TRACKING_CORE.md.

pub mod source;
pub mod sources;
pub mod vehicle;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use source::{RadarSource, SourceHandle, SourceUpdate};
use vehicle::{TrackedVehicle, VehicleSource, VehicleSystem};

/// Consolidated snapshot event name — same regardless of which sources are active.
pub const RADAR_EVENT: &str = "radar-vehicles";

/// Per-provider ADS-B status event (contact counts + error flags) — drives the panel's per-source dots.
pub const ADSB_STATUS_EVENT: &str = "radar-adsb-status";

/// Aggregator emit throttle + idle tick.
const EMIT_THROTTLE: Duration = Duration::from_millis(200);
const AGG_TICK: Duration = Duration::from_millis(250);

/// Cross-source debounce: once a contact's position is updated by one source, positional updates from a
/// DIFFERENT source are ignored for this long. Online services poll ≤ every 2 s and lag real time, so a
/// faster realtime feed (hardware ≈ 1 s) keeps ownership and online only fills gaps — no marker jitter
/// from mixing delayed/leading feeds. Same-source refreshes are always accepted.
const SOURCE_DEBOUNCE_MS: i64 = 2000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Per-system TTL: a contact older than this (no update) is pruned.
fn ttl_ms(system: VehicleSystem) -> i64 {
    match system {
        VehicleSystem::Adsb => 45_000,
        VehicleSystem::FormationFlight => 10_000,
        VehicleSystem::Radio => 8_000,
    }
}

/// Config pushed from the frontend (`settings.radar`). Carries the master switch, the dev-only sim,
/// and the per-system source configs (ADS-B online from Phase 1; more added per phase).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadarConfig {
    pub enabled: bool,
    /// Dev-only synthetic source (ignored in release builds).
    #[serde(default)]
    pub sim: bool,
    /// Optional `[lat, lon]` centre for the dev sim (defaults applied if absent).
    #[serde(default)]
    pub sim_center: Option<[f64; 2]>,
    #[serde(default)]
    pub adsb: AdsbConfig,
    #[serde(default)]
    pub formation_flight: FormationFlightConfig,
}

/// FormationFlight (INAV-Radar / ESP32) system config — a single serial module Kite speaks MSP to as an
/// emulated FC. See docs/active/RADAR_FORMATION_FLIGHT.md.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FormationFlightConfig {
    pub enabled: bool,
    /// Serial port the ESP32 module is on.
    #[serde(default)]
    pub port: String,
    /// Baud (0 ⇒ default 115200).
    #[serde(default)]
    pub baud: u32,
    /// Name we advertise via MSP_NAME (our node's broadcast name). Empty ⇒ a default is used.
    #[serde(default)]
    pub node_name: String,
}

/// ADS-B system config (Phase 1: online providers).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdsbConfig {
    pub enabled: bool,
    /// Online REST providers (polled in parallel; merged by ICAO).
    #[serde(default)]
    pub online: Vec<AdsbOnlineProvider>,
    /// Local hardware receivers (MAVLink ADSB_VEHICLE over serial; TCP later).
    #[serde(default)]
    pub local: Vec<AdsbLocalSource>,
    /// Pull the ADS-B list from the connected UAV via MSP (INAV 8.0+). Bandwidth-heavy → opt-in.
    #[serde(default)]
    pub msp_from_fc: bool,
    /// Query radius in km (dropdown 10/25/50/75/100; capped at 100). 0 ⇒ default 25.
    #[serde(default)]
    pub radius_km: f64,
    /// Poll interval in seconds. 0 ⇒ default 5.
    #[serde(default)]
    pub poll_sec: f64,
    /// `[lat, lon]` query centre — the resolved user location.
    #[serde(default)]
    pub center: Option<[f64; 2]>,
}

/// One online ADS-B provider. `url` is a template with `{lat}` / `{lon}` / `{dist}` placeholders
/// (`dist` is filled in NM); this covers both the `/v2/point/{lat}/{lon}/{dist}` (adsb.lol/.one) and
/// the `/api/v2/lat/{lat}/lon/{lon}/dist/{dist}` (adsb.fi) shapes without special-casing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdsbOnlineProvider {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// A local hardware ADS-B receiver. Phase 2: `transport = "serial"` (MAVLink ADSB_VEHICLE);
/// TCP/WiFi (same decode) follows once the device's TCP API is confirmed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdsbLocalSource {
    pub name: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub baud: u32,
    #[serde(default)]
    pub enabled: bool,
}

/// The consolidated state delivered to the frontend (three separate, never-merged lists).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadarSnapshot {
    pub adsb: Vec<TrackedVehicle>,
    pub formation_flight: Vec<TrackedVehicle>,
    pub radio: Vec<TrackedVehicle>,
    pub last_update: i64,
}

/// Per-source status entry (emitted as `radar-adsb-status`, keyed by `name`, merged in the frontend).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdsbStatus {
    pub name: String,
    pub count: usize,
    pub ok: bool,
}

struct Running {
    update_tx: mpsc::Sender<SourceUpdate>,
    stop: Arc<AtomicBool>,
    agg: Option<JoinHandle<()>>,
    /// Data sources rebuilt on every reconfigure (ADS-B online/local). Cheap to recycle.
    sources: Vec<SourceHandle>,
    /// FormationFlight source kept separate: it holds a serial port to a resettable ESP32, so we restart
    /// it ONLY when its port/baud change (reopening the port resets the board).
    ff: Option<FfRunning>,
    snapshot: Arc<Mutex<RadarSnapshot>>,
}

/// The running FormationFlight source + the port/baud it was opened with (for the restart-on-change check).
struct FfRunning {
    #[allow(dead_code)]
    handle: SourceHandle,
    port: String,
    baud: u32,
}

/// Owns the running radar pipeline. Idle (no threads) until enabled. Lives in `AppState`, behind a
/// `Mutex`, completely separate from `AppState.protocol`.
pub struct RadarManager {
    running: Option<Running>,
    /// Live ADS-B query centre (`[lat, lon]`), shared with the online source so it can follow the
    /// map viewport / UAV without restarting the pipeline. Updated via `set_center`.
    query_center: Arc<Mutex<Option<(f64, f64)>>>,
    /// Live ADS-B query radius (km), shared so the 3D view can size the query to the visible area.
    query_radius: Arc<Mutex<Option<f64>>>,
    /// Live GCS node position `(lat, lon, alt_m)` advertised to a FormationFlight module (we emulate an
    /// FC at this spot). Shared so it follows the resolved GCS location without restarting the source.
    node_pos: Arc<Mutex<Option<(f64, f64, f64)>>>,
    /// Live craft name advertised via MSP_NAME. Shared so a name change never restarts the FF source
    /// (which would reopen the serial port and reset the ESP32).
    node_name: Arc<Mutex<String>>,
    /// Aggregator ingest channel, exposed so scheduler-fed sources (ADS-B via MSP) can push into the
    /// same pipeline. `Some` while running, `None` when idle.
    ingest: Arc<Mutex<Option<mpsc::Sender<SourceUpdate>>>>,
}

impl Default for RadarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RadarManager {
    pub fn new() -> Self {
        Self {
            running: None,
            query_center: Arc::new(Mutex::new(None)),
            query_radius: Arc::new(Mutex::new(None)),
            node_pos: Arc::new(Mutex::new(None)),
            node_name: Arc::new(Mutex::new(String::new())),
            ingest: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the GCS node position `(lat, lon, alt_m)` advertised to a FormationFlight module (cheap;
    /// no restart — the running source reads it live when answering MSP_RAW_GPS).
    pub fn set_node_pos(&self, lat: f64, lon: f64, alt_m: f64) {
        if let Ok(mut p) = self.node_pos.lock() {
            *p = Some((lat, lon, alt_m));
        }
    }

    /// Handle to the aggregator ingest channel — handed to the scheduler so ADS-B-via-MSP can push
    /// into the radar pipeline (merged by ICAO like every other ADS-B source).
    pub fn ingest_handle(&self) -> Arc<Mutex<Option<mpsc::Sender<SourceUpdate>>>> {
        self.ingest.clone()
    }

    /// Update the live ADS-B query centre + optional radius (km) (cheap; no pipeline restart).
    pub fn set_center(&self, lat: f64, lon: f64, radius_km: Option<f64>) {
        if let Ok(mut c) = self.query_center.lock() {
            *c = Some((lat, lon));
        }
        if let Some(r) = radius_km {
            if let Ok(mut rr) = self.query_radius.lock() {
                *rr = Some(r);
            }
        }
    }

    /// Current consolidated state (for `radar_snapshot` on panel open).
    pub fn snapshot(&self) -> RadarSnapshot {
        self.running
            .as_ref()
            .and_then(|r| r.snapshot.lock().ok().map(|s| s.clone()))
            .unwrap_or_default()
    }

    /// Apply a config: start/stop the pipeline. When already running, reconfigure **in place** — keep the
    /// aggregator thread (and its accumulated, merged contacts) + the ingest channel alive and only swap
    /// the source threads. So toggling one source no longer wipes the others' contacts (they persist via
    /// TTL; the restarted sources just re-poll, invisibly). A full teardown only happens on disable.
    pub fn configure(&mut self, config: &RadarConfig, app: &AppHandle) {
        if !config.enabled {
            self.stop(app);
            return;
        }
        // Cheap Arc clones so we can build sources while `running` is mutably borrowed below.
        let query_center = self.query_center.clone();
        let query_radius = self.query_radius.clone();
        let node_pos = self.node_pos.clone();
        let node_name = self.node_name.clone();
        // Seed the live node name from config (also pushed live via radar_set_node_name) — updating it
        // here never restarts the FF source, so the ESP32 isn't reset.
        if !config.formation_flight.node_name.trim().is_empty() {
            if let Ok(mut n) = node_name.lock() {
                *n = config.formation_flight.node_name.clone();
            }
        }

        // Already running → swap the data workers, but reconcile the FF source by port/baud only (keep its
        // serial port open across unrelated reconfigures so the ESP32 isn't reset).
        if let Some(r) = self.running.as_mut() {
            r.sources.clear(); // drop old workers (each SourceHandle's Drop stops its thread)
            r.sources = build_sources(config, &r.update_tx, app, &query_center, &query_radius);
            reconcile_ff(&mut r.ff, config, &r.update_tx, app, &node_pos, &node_name);
            eprintln!(
                "[radar] reconfigured in place: {} source(s){}",
                r.sources.len(),
                if r.ff.is_some() { " + FF" } else { "" }
            );
            return;
        }

        // Cold start → build the aggregator + the initial sources.
        let (tx, rx) = mpsc::channel::<SourceUpdate>();
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(RadarSnapshot::default()));
        let agg = spawn_aggregator(rx, stop.clone(), snapshot.clone(), app.clone());
        // Expose the ingest channel so scheduler-fed sources (ADS-B via MSP) push into this pipeline.
        *self.ingest.lock().unwrap() = Some(tx.clone());
        let sources = build_sources(config, &tx, app, &query_center, &query_radius);
        let ff = make_ff(config, &tx, app, &node_pos, &node_name);
        eprintln!(
            "[radar] configured: enabled, {} source(s){}",
            sources.len(),
            if ff.is_some() { " + FF" } else { "" }
        );
        self.running = Some(Running {
            update_tx: tx,
            stop,
            agg: Some(agg),
            sources,
            ff,
            snapshot,
        });
    }

    /// Tear down all sources + the aggregator and clear the UI with an empty snapshot.
    pub fn stop(&mut self, app: &AppHandle) {
        if let Some(mut r) = self.running.take() {
            *self.ingest.lock().unwrap() = None; // scheduler-fed sources stop pushing
            r.sources.clear(); // each SourceHandle's Drop signals its worker to stop
            r.stop.store(true, Ordering::Relaxed);
            drop(r.update_tx);
            if let Some(h) = r.agg.take() {
                let _ = h.join();
            }
            let empty = RadarSnapshot {
                last_update: now_ms(),
                ..Default::default()
            };
            let _ = app.emit(RADAR_EVENT, &empty);
            eprintln!("[radar] stopped");
        }
    }
}

/// Build the configured source workers, all pushing into `tx`. Shared by the cold start and the in-place
/// reconfigure path so toggling one source doesn't disturb the aggregator's accumulated contacts.
fn build_sources(
    config: &RadarConfig,
    tx: &mpsc::Sender<SourceUpdate>,
    app: &AppHandle,
    query_center: &Arc<Mutex<Option<(f64, f64)>>>,
    query_radius: &Arc<Mutex<Option<f64>>>,
) -> Vec<SourceHandle> {
    let mut sources: Vec<SourceHandle> = Vec::new();

    // Dev-only synthetic source — present only in debug builds.
    #[cfg(debug_assertions)]
    if config.sim {
        let center = config.sim_center.map(|c| (c[0], c[1])).unwrap_or((48.0, 11.0));
        let src = Box::new(sources::sim::SimSource::new(center));
        sources.push(src.start(tx.clone()));
    }

    // ADS-B online (Phase 1): a shared, live query centre (map viewport / UAV) + ≥1 provider.
    if config.adsb.enabled {
        // Seed the shared centre from the config (the frontend keeps it live via radar_set_center).
        if let Some(c) = config.adsb.center {
            *query_center.lock().unwrap() = Some((c[0], c[1]));
        }
        let providers: Vec<AdsbOnlineProvider> =
            config.adsb.online.iter().filter(|p| p.enabled).cloned().collect();
        if providers.is_empty() {
            eprintln!("[radar][adsb] enabled but no online providers");
        } else {
            let radius_km = if config.adsb.radius_km > 0.0 { config.adsb.radius_km.min(100.0) } else { 25.0 };
            // Seed the live radius from the config; the 3D view overrides it via set_center.
            *query_radius.lock().unwrap() = Some(radius_km);
            let poll_sec = if config.adsb.poll_sec > 0.0 { config.adsb.poll_sec } else { 5.0 };
            let src = Box::new(sources::adsb_online::AdsbOnlineSource::new(
                providers, query_center.clone(), query_radius.clone(), radius_km, poll_sec, app.clone(),
            ));
            sources.push(src.start(tx.clone()));
        }

        // Local hardware receivers (Phase 2: serial MAVLink). Independent of online providers.
        for ls in config.adsb.local.iter().filter(|l| l.enabled) {
            let transport = if ls.transport.is_empty() { "serial" } else { ls.transport.as_str() };
            match transport {
                "serial" if !ls.port.is_empty() => {
                    let baud = if ls.baud > 0 { ls.baud } else { 57600 };
                    let src = Box::new(sources::adsb_mavlink::AdsbMavlinkSource::new(
                        ls.name.clone(), ls.port.clone(), baud, app.clone(),
                    ));
                    sources.push(src.start(tx.clone()));
                }
                other => eprintln!("[radar][adsb] local source '{}' transport '{}' not supported yet", ls.name, other),
            }
        }
    }

    sources
}

/// Effective baud for the FormationFlight module (0 ⇒ 115200).
fn ff_baud(config: &RadarConfig) -> u32 {
    if config.formation_flight.baud > 0 { config.formation_flight.baud } else { 115200 }
}

/// Start the FormationFlight source (INAV-Radar / ESP32) if configured, returning its handle + port/baud.
fn make_ff(
    config: &RadarConfig,
    tx: &mpsc::Sender<SourceUpdate>,
    app: &AppHandle,
    node_pos: &Arc<Mutex<Option<(f64, f64, f64)>>>,
    node_name: &Arc<Mutex<String>>,
) -> Option<FfRunning> {
    let ff = &config.formation_flight;
    if !ff.enabled || ff.port.is_empty() {
        return None;
    }
    let baud = ff_baud(config);
    let src = Box::new(sources::formation_flight::FormationFlightSource::new(
        "FormationFlight".to_string(),
        ff.port.clone(),
        baud,
        node_name.clone(),
        node_pos.clone(),
        app.clone(),
    ));
    let handle = src.start(tx.clone());
    eprintln!("[radar][ff] started on {} @ {}", ff.port, baud);
    Some(FfRunning { handle, port: ff.port.clone(), baud })
}

/// Reconcile the FormationFlight source against a new config WITHOUT reopening its serial port unless the
/// port/baud actually change — so an unrelated reconfigure (or a name change) doesn't reset the ESP32.
fn reconcile_ff(
    cur: &mut Option<FfRunning>,
    config: &RadarConfig,
    tx: &mpsc::Sender<SourceUpdate>,
    app: &AppHandle,
    node_pos: &Arc<Mutex<Option<(f64, f64, f64)>>>,
    node_name: &Arc<Mutex<String>>,
) {
    let ff = &config.formation_flight;
    if !ff.enabled || ff.port.is_empty() {
        if cur.take().is_some() {
            eprintln!("[radar][ff] stopped");
        }
        return;
    }
    let same = cur
        .as_ref()
        .is_some_and(|f| f.port == ff.port && f.baud == ff_baud(config));
    if same {
        return; // keep the port open — no ESP reset
    }
    *cur = make_ff(config, tx, app, node_pos, node_name); // port/baud changed → (re)open
}

/// Aggregator thread: drain source updates, per-system merge by id, TTL-prune, throttled emit.
fn spawn_aggregator(
    rx: mpsc::Receiver<SourceUpdate>,
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<RadarSnapshot>>,
    app: AppHandle,
) -> JoinHandle<()> {
    thread::spawn(move || {
        // One map per system, keyed by vehicle id.
        let mut maps: HashMap<VehicleSystem, HashMap<String, TrackedVehicle>> = HashMap::new();
        // Per (system,id): the source that last set the position + when (ms), for the cross-source debounce.
        let mut owners: HashMap<VehicleSystem, HashMap<String, (VehicleSource, i64)>> = HashMap::new();
        let mut last_emit = Instant::now() - EMIT_THROTTLE;
        let mut dirty = true;

        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(AGG_TICK) {
                Ok(update) => {
                    for v in update.vehicles {
                        let its_source = v.sources.first().copied();
                        let m = maps.entry(v.system).or_default();
                        let ow = owners.entry(v.system).or_default();
                        match m.get_mut(&v.id) {
                            Some(existing) => {
                                // Cross-source debounce: accept same-source refreshes always; accept a
                                // different source only after SOURCE_DEBOUNCE_MS without an update. This
                                // gives the fastest realtime feed ownership and avoids position jitter
                                // from mixing leading/lagging feeds.
                                let accept = match (its_source, ow.get(&v.id)) {
                                    (Some(s), Some((last_src, last_t))) => {
                                        s == *last_src || v.last_seen_ms - *last_t >= SOURCE_DEBOUNCE_MS
                                    }
                                    _ => true,
                                };
                                if accept {
                                    // Latest dynamic fields win; keep stable identity fields
                                    // (callsign/category/squawk) when the newer source lacks them, and
                                    // accumulate the source set.
                                    let mut srcs = existing.sources.clone();
                                    for s in &v.sources {
                                        if !srcs.contains(s) {
                                            srcs.push(*s);
                                        }
                                    }
                                    let prev_callsign = existing.callsign.take();
                                    let prev_category = existing.category.take();
                                    let prev_squawk = existing.squawk.take();
                                    let stamp = v.last_seen_ms;
                                    *existing = v;
                                    existing.sources = srcs;
                                    existing.callsign = existing.callsign.take().or(prev_callsign);
                                    existing.category = existing.category.take().or(prev_category);
                                    existing.squawk = existing.squawk.take().or(prev_squawk);
                                    if let Some(s) = its_source {
                                        ow.insert(existing.id.clone(), (s, stamp));
                                    }
                                } else if let Some(s) = its_source {
                                    // Rejected positional update — still note the feed saw this contact.
                                    if !existing.sources.contains(&s) {
                                        existing.sources.push(s);
                                    }
                                }
                            }
                            None => {
                                if let Some(s) = its_source {
                                    ow.insert(v.id.clone(), (s, v.last_seen_ms));
                                }
                                m.insert(v.id.clone(), v);
                            }
                        }
                    }
                    dirty = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            // TTL prune (drop owner entries alongside their vehicles).
            let now = now_ms();
            for (sys, m) in maps.iter_mut() {
                let ttl = ttl_ms(*sys);
                let before = m.len();
                m.retain(|_, v| now - v.last_seen_ms <= ttl);
                if m.len() != before {
                    if let Some(ow) = owners.get_mut(sys) {
                        ow.retain(|id, _| m.contains_key(id));
                    }
                    dirty = true;
                }
            }

            if dirty && last_emit.elapsed() >= EMIT_THROTTLE {
                let snap = build_snapshot(&maps, now);
                if let Ok(mut s) = snapshot.lock() {
                    *s = snap.clone();
                }
                if let Err(e) = app.emit(RADAR_EVENT, &snap) {
                    log::warn!("Failed to emit {}: {}", RADAR_EVENT, e);
                }
                last_emit = Instant::now();
                dirty = false;
            }
        }
    })
}

fn build_snapshot(
    maps: &HashMap<VehicleSystem, HashMap<String, TrackedVehicle>>,
    now: i64,
) -> RadarSnapshot {
    let list = |sys: VehicleSystem| -> Vec<TrackedVehicle> {
        let mut v: Vec<TrackedVehicle> = maps
            .get(&sys)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    };
    RadarSnapshot {
        adsb: list(VehicleSystem::Adsb),
        formation_flight: list(VehicleSystem::FormationFlight),
        radio: list(VehicleSystem::Radio),
        last_update: now,
    }
}
