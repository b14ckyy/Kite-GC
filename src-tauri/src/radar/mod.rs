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

/// Config pushed from the frontend (`settings.radar`). Phase 0 carries the master switch + the
/// dev-only sim; per-source params arrive in later phases.
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
}

impl Default for RadarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RadarManager {
    pub fn new() -> Self {
        Self { running: None }
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
