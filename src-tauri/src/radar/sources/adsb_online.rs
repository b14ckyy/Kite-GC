// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — ADS-B online source: polls REST providers in the readsb shape (URL templates with
// {lat}/{lon}/{dist}) around the query centre; the aggregator merges them by ICAO.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::radar::now_ms;
use crate::radar::source::{RadarSource, SourceHandle, SourceUpdate};
use crate::radar::vehicle::{AltRef, TrackedVehicle, VehicleSource, VehicleSystem};
use crate::radar::{AdsbOnlineProvider, ADSB_STATUS_EVENT};

/// Per-provider result for one poll cycle (emitted as `radar-adsb-status`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdsbProviderStatus {
    name: String,
    count: usize,
    ok: bool,
    /// Not polled because the provider named a time to wait (HTTP 429).
    limited: bool,
}

const KM_PER_NM: f64 = 1.852;
const FT_TO_M: f64 = 0.3048;
const KT_TO_MS: f64 = 0.514_444;
const FPM_TO_MS: f64 = 0.00508;
const POLL_GRANULARITY: Duration = Duration::from_millis(200);
/// Cap for the per-provider failure backoff (poll interval × 2^n, never beyond this).
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Longest pause taken from a provider's rate-limit headers (daily quotas reset within this).
const MAX_LIMIT_PAUSE: Duration = Duration::from_secs(24 * 3600);

/// Per-provider failure backoff, independent per provider. A failed fetch doubles the spacing (in poll
/// cycles) up to `MAX_BACKOFF`; a success resets it.
struct Backoff {
    /// Current spacing multiplier in poll cycles (1 = every cycle).
    mult: u32,
    /// Cycles still to sit out before the next attempt.
    skip: u32,
    /// Cycles skipped since the failure began (for the recovery log line).
    skipped: u32,
    /// The provider's own wait after a 429: no request before this, whatever the cycle count says.
    until: Option<Instant>,
}

impl Default for Backoff {
    fn default() -> Self { Self { mult: 1, skip: 0, skipped: 0, until: None } }
}

struct FetchError {
    msg: String,
    /// Wait named by the provider with a 429.
    retry_after: Option<Duration>,
}

pub struct AdsbOnlineSource {
    providers: Vec<AdsbOnlineProvider>,
    /// Live query centre (`[lat, lon]`), shared with the manager so it follows the viewport/UAV.
    center: Arc<Mutex<Option<(f64, f64)>>>,
    /// Live query radius (km), shared so the 3D view can size the query to what's visible. `None` ⇒
    /// fall back to the configured `radius_km`.
    radius: Arc<Mutex<Option<f64>>>,
    radius_km: f64,
    poll: Duration,
    /// Instant of the last fetch, **shared by the manager** so it survives a reconfigure (which rebuilds
    /// this source). Lets a fresh instance wait out the remaining poll interval instead of fetching
    /// immediately — otherwise rapid reconfigures (connect/disconnect, settings toggles) would bypass
    /// the poll rate and hammer the provider (HTTP 429 / IP ban).
    last_fetch: Arc<Mutex<Option<Instant>>>,
    app: AppHandle,
}

impl AdsbOnlineSource {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        providers: Vec<AdsbOnlineProvider>,
        center: Arc<Mutex<Option<(f64, f64)>>>,
        radius: Arc<Mutex<Option<f64>>>,
        radius_km: f64,
        poll_sec: f64,
        last_fetch: Arc<Mutex<Option<Instant>>>,
        app: AppHandle,
    ) -> Self {
        Self {
            providers,
            center,
            radius,
            radius_km,
            poll: Duration::from_secs_f64(poll_sec.max(1.0)),
            last_fetch,
            app,
        }
    }
}

impl RadarSource for AdsbOnlineSource {
    fn system(&self) -> VehicleSystem {
        VehicleSystem::Adsb
    }
    fn source(&self) -> VehicleSource {
        VehicleSource::AdsbOnline
    }
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = stop.clone();

        let handle = tauri::async_runtime::spawn(async move {
            // No connection reuse: some providers rate-limit a kept-alive connection far harder than fresh ones.
            let client = reqwest::Client::builder()
                .user_agent("Kite-GC/0.5 radar")
                .timeout(Duration::from_secs(10))
                .pool_max_idle_per_host(0)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            // Per-provider rate-limit backoff state, index-aligned with `self.providers`.
            let mut backoff: Vec<Backoff> = self.providers.iter().map(|_| Backoff::default()).collect();
            // Longest backoff in poll cycles: never more than a minute between attempts.
            let max_mult = ((MAX_BACKOFF.as_secs_f64() / self.poll.as_secs_f64()).floor() as u32).max(1);

            loop {
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }

                // Need a query centre; skip cheaply until we have one (no slot consumed).
                let center = self.center.lock().ok().and_then(|c| *c);
                let (clat, clon) = match center {
                    Some(c) => c,
                    None => {
                        tokio::time::sleep(POLL_GRANULARITY).await;
                        continue;
                    }
                };

                // Atomic rate claim across ALL source loops (incl. leftovers of a reconfigure): fetch only if `poll`
                // elapsed and stamp the slot under the SAME lock — a read-then-wait gate let every loop pass (HTTP 429).
                let claimed = match self.last_fetch.lock() {
                    Ok(mut g) => {
                        let due = g.map(|t| t.elapsed() >= self.poll).unwrap_or(true);
                        if due {
                            *g = Some(Instant::now());
                        }
                        due
                    }
                    Err(_) => false,
                };
                if !claimed {
                    tokio::time::sleep(POLL_GRANULARITY).await;
                    continue;
                }

                // Live radius each cycle (3D sizes the query to the visible area); fall back to config.
                let radius_km = self.radius.lock().ok().and_then(|r| *r).unwrap_or(self.radius_km);
                let dist_nm = (radius_km / KM_PER_NM).clamp(1.0, 250.0).round() as i64;

                let now = now_ms();
                let mut vehicles: Vec<TrackedVehicle> = Vec::new();
                let mut statuses: Vec<AdsbProviderStatus> = Vec::with_capacity(self.providers.len());

                for (p, b) in self.providers.iter().zip(backoff.iter_mut()) {
                    if let Some(until) = b.until {
                        if Instant::now() < until {
                            b.skipped += 1;
                            statuses.push(AdsbProviderStatus { name: p.name.clone(), count: 0, ok: false, limited: true });
                            continue;
                        }
                        b.until = None;
                    }
                    // Backing off after a failure: sit this cycle out (the panel keeps the error dot).
                    if b.skip > 0 {
                        b.skip -= 1;
                        b.skipped += 1;
                        statuses.push(AdsbProviderStatus { name: p.name.clone(), count: 0, ok: false, limited: false });
                        continue;
                    }
                    let url = p
                        .url
                        .replace("{lat}", &format!("{:.5}", clat))
                        .replace("{lon}", &format!("{:.5}", clon))
                        .replace("{dist}", &dist_nm.to_string());
                    // Verbose per-request trace (opt-in via Debug level).
                    log::debug!("[radar][adsb] {} GET {}", p.name, url);
                    match fetch(&client, &url, p.api_key.as_deref()).await {
                        Ok(root) => {
                            let before = vehicles.len();
                            parse_into(&root, now, &mut vehicles);
                            statuses.push(AdsbProviderStatus { name: p.name.clone(), count: vehicles.len() - before, ok: true, limited: false });
                            if b.mult > 1 {
                                log::info!(
                                    "[radar][adsb] {} recovered after {} skipped cycle(s), back to the {:.0} s interval",
                                    p.name, b.skipped, self.poll.as_secs_f64()
                                );
                            }
                            *b = Backoff::default();
                        }
                        Err(FetchError { msg, retry_after: Some(wait) }) => {
                            // Requests during the wait still count at some providers (ban, revoked key).
                            let wait = wait.min(MAX_LIMIT_PAUSE);
                            b.until = Some(Instant::now() + wait);
                            log::warn!("[radar][adsb] {} rate limit ({}) — paused for {} s", p.name, msg, wait.as_secs());
                            statuses.push(AdsbProviderStatus { name: p.name.clone(), count: 0, ok: false, limited: true });
                        }
                        Err(FetchError { msg: e, .. }) => {
                            // Back off this provider: double the spacing per consecutive failure (capped).
                            let first = b.mult == 1;
                            b.mult = (b.mult * 2).min(max_mult);
                            b.skip = b.mult - 1;
                            let wait = self.poll.as_secs_f64() * b.mult as f64;
                            if first {
                                // Tester-relevant → default-level warn, but only on the transition so a
                                // rate-limited provider doesn't fill the log once per cycle.
                                log::warn!("[radar][adsb] {} fetch failed: {} — backing off (next try in {:.0} s)", p.name, e, wait);
                            } else {
                                log::debug!("[radar][adsb] {} still failing: {} — next try in {:.0} s", p.name, e, wait);
                            }
                            statuses.push(AdsbProviderStatus { name: p.name.clone(), count: 0, ok: false, limited: false });
                        }
                    }
                }

                // Per-provider status (counts / errors) for the panel.
                let _ = self.app.emit(ADSB_STATUS_EVENT, &statuses);

                if !vehicles.is_empty()
                    && tx
                        .send(SourceUpdate {
                            source: VehicleSource::AdsbOnline,
                            vehicles,
                        })
                        .is_err()
                    {
                        break; // aggregator gone
                    }
                // Spacing is handled by the rate gate at the top of the loop (shared across restarts).
            }
        });

        SourceHandle::new(move || {
            stop.store(true, Ordering::Relaxed);
            handle.abort();
        })
    }
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
) -> Result<Value, FetchError> {
    let plain = |msg: String| FetchError { msg, retry_after: None };
    let mut req = client.get(url);
    if let Some(k) = api_key {
        if !k.is_empty() {
            req = req.header("api-key", k);
        }
    }
    let resp = req.send().await.map_err(|e| plain(e.to_string()))?;
    if !resp.status().is_success() {
        let retry_after = (resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
            .then(|| {
                let header = |name: &str| resp.headers().get(name)?.to_str().ok()?.trim().parse::<u64>().ok();
                limit_wait(header("retry-after"), header("x-ratelimit-reset"), now_ms() / 1000)
            })
            .flatten();
        return Err(FetchError { msg: format!("HTTP {}", resp.status()), retry_after });
    }
    resp.json::<Value>().await.map_err(|e| plain(e.to_string()))
}

/// The wait a 429 names: `Retry-After` in seconds, else `X-RateLimit-Reset` — seconds to go, or at some
/// providers the Unix time of the reset.
fn limit_wait(retry_after: Option<u64>, reset: Option<u64>, now_s: i64) -> Option<Duration> {
    let secs = retry_after.or_else(|| {
        let r = reset?;
        Some(if r > 1_000_000_000 { r.saturating_sub(now_s.max(0) as u64) } else { r })
    })?;
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Map the provider's `ac[]` array (readsb v2 shape) into TrackedVehicles. Defensive: ADS-B JSON is
/// messy (fields missing, `alt_baro` may be the string "ground"), so every field is optional.
fn parse_into(root: &Value, now: i64, out: &mut Vec<TrackedVehicle>) {
    let Some(ac) = root.get("ac").and_then(Value::as_array) else {
        return;
    };
    for a in ac {
        let (Some(lat), Some(lon)) = (
            a.get("lat").and_then(Value::as_f64),
            a.get("lon").and_then(Value::as_f64),
        ) else {
            continue;
        };
        // Uppercase so the id matches the MAVLink/MSP receivers' `{:06X}` form → cross-source merge.
        let hex = a
            .get("hex")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_uppercase();
        if hex.is_empty() {
            continue;
        }

        let mut v = TrackedVehicle::new(hex, VehicleSystem::Adsb, VehicleSource::AdsbOnline, lat, lon, now);
        v.callsign = a
            .get("flight")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // alt_baro: feet (number) or "ground"; only map the numeric case.
        v.alt_m = a.get("alt_baro").and_then(Value::as_f64).map(|ft| ft * FT_TO_M);
        v.alt_ref = AltRef::BaroMsl;
        v.heading_deg = a.get("track").and_then(Value::as_f64);
        v.ground_speed_ms = a.get("gs").and_then(Value::as_f64).map(|kt| kt * KT_TO_MS);
        v.vertical_speed_ms = a.get("baro_rate").and_then(Value::as_f64).map(|fpm| fpm * FPM_TO_MS);
        v.squawk = a
            .get("squawk")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u16>().ok());
        v.category = a.get("category").and_then(Value::as_str).map(str::to_string);
        out.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_wait_reads_both_headers() {
        assert_eq!(limit_wait(Some(30), Some(4842), 0), Some(Duration::from_secs(30)));
        assert_eq!(limit_wait(None, Some(4842), 1_795_000_000), Some(Duration::from_secs(4842)));
        // A reset given as Unix time.
        assert_eq!(limit_wait(None, Some(1_795_000_600), 1_795_000_000), Some(Duration::from_secs(600)));
        assert_eq!(limit_wait(None, Some(1_795_000_000), 1_795_000_600), None);
        assert_eq!(limit_wait(None, None, 0), None);
    }
}
