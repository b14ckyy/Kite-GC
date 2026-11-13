// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — foreign-vehicle tracking subsystem.
//
// A standalone background subsystem that tracks vehicles OTHER than the main MSP/MAVLink-connected
// UAV (ADS-B / FormationFlight / radio telemetry), fully independent of the exclusive telemetry path
// (it never opens a second handle to the scheduler's link). Sources push batches into an aggregator
// thread that does per-system merge (by id) + TTL expiry and emits one consolidated `radar-vehicles`
// snapshot event. See docs/active/RADAR_TRACKING_CORE.md.

pub mod link;
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

use link::LinkSpec;
use source::{NodePos, RadarSource, SourceHandle, SourceUpdate};
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

/// FormationFlight (INAV-Radar / ESP32) system config — a single module Kite speaks MSP to as an
/// emulated FC, over serial or a TCP / UDP bridge. See docs/active/RADAR_FORMATION_FLIGHT.md.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FormationFlightConfig {
    pub enabled: bool,
    /// `"serial"` (also when empty — configs stored before the field existed), `"tcp"` or `"udp"`.
    #[serde(default)]
    pub transport: String,
    /// Serial port the ESP32 module is on.
    #[serde(default)]
    pub port: String,
    /// Baud (0 ⇒ default 115200).
    #[serde(default)]
    pub baud: u32,
    /// Host of a TCP / UDP link.
    #[serde(default)]
    pub host: String,
    /// Port of a TCP / UDP link (0 ⇒ not started).
    #[serde(default)]
    pub net_port: u32,
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
    /// Local hardware receivers (MAVLink ADSB_VEHICLE over serial, TCP or UDP).
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

/// A local hardware ADS-B receiver streaming MAVLink ADSB_VEHICLE. `transport` is `"serial"` (also when
/// empty — configs stored before the field existed), `"tcp"` (Kite connects) or `"udp"` (Kite listens).
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
    /// Host of a TCP / UDP link.
    #[serde(default)]
    pub host: String,
    /// Port of a TCP / UDP link (0 ⇒ not started).
    #[serde(default)]
    pub net_port: u32,
    #[serde(default)]
    pub enabled: bool,
}

impl AdsbLocalSource {
    /// The link this receiver reads from, or `None` when it cannot be started (see `LinkSpec::from_config`).
    fn link(&self) -> Option<LinkSpec> {
        let baud = if self.baud > 0 { self.baud } else { 57600 };
        LinkSpec::from_config(&self.transport, &self.port, baud, &self.host, self.net_port)
    }
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
    /// FormationFlight source kept separate: it holds a link to a resettable ESP32, so we restart it ONLY
    /// when its link changes (reopening a serial port resets the board).
    ff: Option<FfRunning>,
    snapshot: Arc<Mutex<RadarSnapshot>>,
}

/// The running FormationFlight source + the link it was opened with (for the restart-on-change check).
struct FfRunning {
    #[allow(dead_code)]
    handle: SourceHandle,
    link: LinkSpec,
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
    /// Instant of the last ADS-B online fetch, shared with the source so a source rebuilt by a
    /// reconfigure waits out the poll interval instead of fetching immediately (would bypass the poll
    /// rate on rapid reconfigures → provider rate-limit / IP ban).
    adsb_last_fetch: Arc<Mutex<Option<Instant>>>,
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
            adsb_last_fetch: Arc::new(Mutex::new(None)),
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
        let adsb_last_fetch = self.adsb_last_fetch.clone();
        let node_pos = self.node_pos.clone();
        let node_name = self.node_name.clone();
        // Seed the live node name from config (also pushed live via radar_set_node_name) — updating it
        // here never restarts the FF source, so the ESP32 isn't reset.
        if !config.formation_flight.node_name.trim().is_empty() {
            if let Ok(mut n) = node_name.lock() {
                *n = config.formation_flight.node_name.clone();
            }
        }

        // Already running → swap the data workers, but reconcile the FF source by its link only (keep it
        // open across unrelated reconfigures so the ESP32 isn't reset).
        if let Some(r) = self.running.as_mut() {
            r.sources.clear(); // drop old workers (each SourceHandle's Drop stops its thread)
            r.sources = build_sources(config, &r.update_tx, app, &query_center, &query_radius, &adsb_last_fetch);
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
        let sources = build_sources(config, &tx, app, &query_center, &query_radius, &adsb_last_fetch);
        let ff = ff_link(config).map(|link| make_ff(link, &tx, app, &node_pos, &node_name));
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
    adsb_last_fetch: &Arc<Mutex<Option<Instant>>>,
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
                providers, query_center.clone(), query_radius.clone(), radius_km, poll_sec,
                adsb_last_fetch.clone(), app.clone(),
            ));
            sources.push(src.start(tx.clone()));
        }

        // Local hardware receivers (MAVLink over serial / TCP / UDP). Independent of online providers.
        for ls in config.adsb.local.iter().filter(|l| l.enabled) {
            match ls.link() {
                Some(link) => {
                    let src = Box::new(sources::adsb_mavlink::AdsbMavlinkSource::new(ls.name.clone(), link, app.clone()));
                    sources.push(src.start(tx.clone()));
                }
                None => log::info!(
                    "[radar][adsb] local source '{}' ({}) has no usable link — not started",
                    ls.name,
                    if ls.transport.is_empty() { "serial" } else { ls.transport.as_str() }
                ),
            }
        }
    }

    sources
}

/// The link the FormationFlight module should run on, or `None` when FF is off or its link is incomplete.
/// A serial link carries the effective baud (0 ⇒ 115200). This is the whole restart-on-change key: the
/// fields of the transport NOT selected (a stale serial port behind a TCP link) and the node name are
/// not part of it, so changing them never reopens the link.
fn ff_link(config: &RadarConfig) -> Option<LinkSpec> {
    let ff = &config.formation_flight;
    if !ff.enabled {
        return None;
    }
    let baud = if ff.baud > 0 { ff.baud } else { 115200 };
    LinkSpec::from_config(&ff.transport, &ff.port, baud, &ff.host, ff.net_port)
}

/// Start the FormationFlight source (INAV-Radar / ESP32) on `link`, returning its handle + the link.
fn make_ff(
    link: LinkSpec,
    tx: &mpsc::Sender<SourceUpdate>,
    app: &AppHandle,
    node_pos: &NodePos,
    node_name: &Arc<Mutex<String>>,
) -> FfRunning {
    let src = Box::new(sources::formation_flight::FormationFlightSource::new(
        "FormationFlight".to_string(),
        link.clone(),
        node_name.clone(),
        node_pos.clone(),
        app.clone(),
    ));
    let handle = src.start(tx.clone());
    eprintln!("[radar][ff] started on {}", link.describe());
    FfRunning { handle, link }
}

/// Reconcile the FormationFlight source against a new config WITHOUT reopening its link unless the link
/// itself changes (transport, serial port/baud, host/port) — so an unrelated reconfigure (or a name
/// change) doesn't reset the ESP32.
fn reconcile_ff(
    cur: &mut Option<FfRunning>,
    config: &RadarConfig,
    tx: &mpsc::Sender<SourceUpdate>,
    app: &AppHandle,
    node_pos: &NodePos,
    node_name: &Arc<Mutex<String>>,
) {
    let Some(link) = ff_link(config) else {
        if cur.take().is_some() {
            eprintln!("[radar][ff] stopped");
        }
        return;
    };
    if cur.as_ref().is_some_and(|f| f.link == link) {
        return; // keep the link open — no ESP reset
    }
    // Link changed → drop the old source first (its Drop stops the worker), then open the new one.
    *cur = None;
    *cur = Some(make_ff(link, tx, app, node_pos, node_name));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: serde_json::Value) -> RadarConfig {
        serde_json::from_value(json).expect("radar config parses")
    }

    #[test]
    fn stored_local_source_without_link_fields_is_serial() {
        // Shape stored before TCP / UDP existed: no host / netPort, transport sometimes empty.
        let c = config(serde_json::json!({
            "enabled": true,
            "adsb": { "enabled": true, "local": [
                { "name": "A", "port": "COM3", "baud": 115200, "enabled": true },
                { "name": "B", "transport": "", "port": "/dev/ttyUSB0", "baud": 0, "enabled": true },
            ]},
        }));
        let l = &c.adsb.local;
        assert_eq!(l[0].host, "");
        assert_eq!(l[0].net_port, 0);
        assert_eq!(l[0].link(), Some(LinkSpec::Serial { port: "COM3".into(), baud: 115200 }));
        assert_eq!(l[1].link(), Some(LinkSpec::Serial { port: "/dev/ttyUSB0".into(), baud: 57600 }));
    }

    #[test]
    fn tcp_and_udp_local_sources_parse() {
        let c = config(serde_json::json!({
            "enabled": true,
            "adsb": { "enabled": true, "local": [
                { "name": "T", "transport": "tcp", "port": "COM3", "baud": 57600, "host": "192.168.4.1", "netPort": 5760, "enabled": true },
                { "name": "U", "transport": "udp", "port": "", "baud": 57600, "host": "0.0.0.0", "netPort": 14550, "enabled": true },
                { "name": "X", "transport": "tcp", "port": "", "baud": 57600, "host": "", "netPort": 5760, "enabled": true },
            ]},
        }));
        let l = &c.adsb.local;
        assert_eq!(l[0].link(), Some(LinkSpec::Tcp { host: "192.168.4.1".into(), port: 5760 }));
        assert_eq!(l[1].link(), Some(LinkSpec::Udp { host: "0.0.0.0".into(), port: 14550 }));
        assert_eq!(l[2].link(), None, "an IP source without a host is not started");
    }

    #[test]
    fn stored_formation_flight_without_link_fields_is_serial() {
        let c = config(serde_json::json!({
            "enabled": true,
            "formationFlight": { "enabled": true, "port": "COM7", "baud": 0, "nodeName": "GCS" },
        }));
        assert_eq!(c.formation_flight.transport, "");
        assert_eq!(ff_link(&c), Some(LinkSpec::Serial { port: "COM7".into(), baud: 115200 }));
    }

    #[test]
    fn formation_flight_ip_links_parse() {
        let tcp = config(serde_json::json!({
            "enabled": true,
            "formationFlight": { "enabled": true, "transport": "tcp", "port": "", "baud": 115200, "host": "10.0.0.5", "netPort": 5761, "nodeName": "" },
        }));
        assert_eq!(ff_link(&tcp), Some(LinkSpec::Tcp { host: "10.0.0.5".into(), port: 5761 }));
        let udp = config(serde_json::json!({
            "enabled": true,
            "formationFlight": { "enabled": true, "transport": "udp", "host": "10.0.0.5", "netPort": 0 },
        }));
        assert_eq!(ff_link(&udp), None, "port 0 is not started");
    }

    #[test]
    fn formation_flight_restart_key_is_the_link_only() {
        let base = serde_json::json!({
            "enabled": true,
            "adsb": { "enabled": true },
            "formationFlight": { "enabled": true, "transport": "serial", "port": "COM7", "baud": 115200, "host": "10.0.0.5", "netPort": 5761, "nodeName": "A" },
        });
        let with = |patch: serde_json::Value| {
            let mut v = base.clone();
            for (k, val) in patch.as_object().unwrap() {
                v["formationFlight"][k] = val.clone();
            }
            ff_link(&config(v))
        };
        let cur = ff_link(&config(base.clone()));
        assert!(cur.is_some());

        // Unrelated changes keep the link (no reopen, no ESP32 reset).
        assert_eq!(with(serde_json::json!({ "nodeName": "B" })), cur);
        assert_eq!(with(serde_json::json!({ "host": "10.0.0.9", "netPort": 1 })), cur, "IP fields behind a serial link");
        let mut adsb_off = base.clone();
        adsb_off["adsb"]["enabled"] = serde_json::json!(false);
        assert_eq!(ff_link(&config(adsb_off)), cur);
        // 0 and the default baud are the same link.
        assert_eq!(with(serde_json::json!({ "baud": 0 })), cur);

        // Link changes restart it.
        assert_ne!(with(serde_json::json!({ "port": "COM8" })), cur);
        assert_ne!(with(serde_json::json!({ "baud": 57600 })), cur);
        assert_ne!(with(serde_json::json!({ "transport": "tcp" })), cur);
        assert_ne!(with(serde_json::json!({ "transport": "udp" })), cur);

        // Behind a TCP link the serial fields are the unrelated ones.
        let tcp = with(serde_json::json!({ "transport": "tcp" }));
        assert_eq!(with(serde_json::json!({ "transport": "tcp", "port": "COM9", "baud": 9600 })), tcp);
        assert_ne!(with(serde_json::json!({ "transport": "tcp", "host": "10.0.0.6" })), tcp);
        assert_ne!(with(serde_json::json!({ "transport": "tcp", "netPort": 5762 })), tcp);

        // Off → no link (the source is stopped).
        assert_eq!(with(serde_json::json!({ "enabled": false })), None);
    }
}
