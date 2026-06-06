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
use vehicle::{TrackedVehicle, VehicleSystem};

/// Consolidated snapshot event name — same regardless of which sources are active.
pub const RADAR_EVENT: &str = "radar-vehicles";

/// Per-provider ADS-B status event (contact counts + error flags) — drives the panel's per-source dots.
pub const ADSB_STATUS_EVENT: &str = "radar-adsb-status";

/// Aggregator emit throttle + idle tick.
const EMIT_THROTTLE: Duration = Duration::from_millis(200);
const AGG_TICK: Duration = Duration::from_millis(250);

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

struct Running {
    update_tx: mpsc::Sender<SourceUpdate>,
    stop: Arc<AtomicBool>,
    agg: Option<JoinHandle<()>>,
    sources: Vec<SourceHandle>,
    snapshot: Arc<Mutex<RadarSnapshot>>,
}

/// Owns the running radar pipeline. Idle (no threads) until enabled. Lives in `AppState`, behind a
/// `Mutex`, completely separate from `AppState.protocol`.
pub struct RadarManager {
    running: Option<Running>,
    /// Live ADS-B query centre (`[lat, lon]`), shared with the online source so it can follow the
    /// map viewport / UAV without restarting the pipeline. Updated via `set_center`.
    query_center: Arc<Mutex<Option<(f64, f64)>>>,
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
        }
    }

    /// Update the live ADS-B query centre (cheap; no pipeline restart).
    pub fn set_center(&self, lat: f64, lon: f64) {
        if let Ok(mut c) = self.query_center.lock() {
            *c = Some((lat, lon));
        }
    }

    /// Current consolidated state (for `radar_snapshot` on panel open).
    pub fn snapshot(&self) -> RadarSnapshot {
        self.running
            .as_ref()
            .and_then(|r| r.snapshot.lock().ok().map(|s| s.clone()))
            .unwrap_or_default()
    }

    /// Apply a config: start/stop the pipeline and (Phase 0) the dev sim source. For simplicity and
    /// correctness this restarts cleanly on every change; per-source diffing comes in later phases.
    pub fn configure(&mut self, config: &RadarConfig, app: &AppHandle) {
        self.stop(app);
        if !config.enabled {
            return;
        }

        let (tx, rx) = mpsc::channel::<SourceUpdate>();
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(RadarSnapshot::default()));
        let agg = spawn_aggregator(rx, stop.clone(), snapshot.clone(), app.clone());

        let mut sources: Vec<SourceHandle> = Vec::new();

        // Dev-only synthetic source — present only in debug builds.
        #[cfg(debug_assertions)]
        if config.sim {
            let center = config
                .sim_center
                .map(|c| (c[0], c[1]))
                .unwrap_or((48.0, 11.0));
            let src = Box::new(sources::sim::SimSource::new(center));
            sources.push(src.start(tx.clone()));
        }

        // ADS-B online (Phase 1): a shared, live query centre (map viewport / UAV) + ≥1 provider.
        if config.adsb.enabled {
            // Seed the shared centre from the config (the frontend keeps it live via radar_set_center).
            if let Some(c) = config.adsb.center {
                *self.query_center.lock().unwrap() = Some((c[0], c[1]));
            }
            let providers: Vec<AdsbOnlineProvider> =
                config.adsb.online.iter().filter(|p| p.enabled).cloned().collect();
            if providers.is_empty() {
                eprintln!("[radar][adsb] enabled but no online providers");
            } else {
                let radius_km = if config.adsb.radius_km > 0.0 { config.adsb.radius_km.min(100.0) } else { 25.0 };
                let poll_sec = if config.adsb.poll_sec > 0.0 { config.adsb.poll_sec } else { 5.0 };
                let src = Box::new(sources::adsb_online::AdsbOnlineSource::new(
                    providers, self.query_center.clone(), radius_km, poll_sec, app.clone(),
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

        eprintln!("[radar] configured: enabled, {} source(s)", sources.len());
        self.running = Some(Running {
            update_tx: tx,
            stop,
            agg: Some(agg),
            sources,
            snapshot,
        });
    }

    /// Tear down all sources + the aggregator and clear the UI with an empty snapshot.
    pub fn stop(&mut self, app: &AppHandle) {
        if let Some(mut r) = self.running.take() {
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
        let mut last_emit = Instant::now() - EMIT_THROTTLE;
        let mut dirty = true;

        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(AGG_TICK) {
                Ok(update) => {
                    for v in update.vehicles {
                        let m = maps.entry(v.system).or_default();
                        match m.get_mut(&v.id) {
                            // Per-system merge by id: latest fields win, accumulate the source set.
                            Some(existing) => {
                                let mut srcs = existing.sources.clone();
                                for s in &v.sources {
                                    if !srcs.contains(s) {
                                        srcs.push(*s);
                                    }
                                }
                                *existing = v;
                                existing.sources = srcs;
                            }
                            None => {
                                m.insert(v.id.clone(), v);
                            }
                        }
                    }
                    dirty = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            // TTL prune.
            let now = now_ms();
            for (sys, m) in maps.iter_mut() {
                let ttl = ttl_ms(*sys);
                let before = m.len();
                m.retain(|_, v| now - v.last_seen_ms <= ttl);
                if m.len() != before {
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
