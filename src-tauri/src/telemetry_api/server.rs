// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The running API: the transports plus one ticker thread that builds a frame at the configured rate
//! and hands it to every active output. The rate is the API's own clock — a 50 Hz MAVLink attitude
//! stream and a 1 Hz MSP link both come out at `rate_hz` (repeating the newest state in between).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::state::{ActiveProtocol, AppState};
use crate::telemetry_forward::output::{udp::UdpSink, OutputSink};

use super::frame::{Frame, Hello, LinkInfo, Protocol, GROUPS, SCHEMA_VERSION};
use super::http::{HttpServer, Shared};
use super::state::ApiState;
use super::tcp::TcpServer;
use super::ApiConfig;

/// Fixed ports (documented; not user-configurable so a consumer never has to guess).
pub const TCP_PORT: u16 = 27300;
pub const HTTP_PORT: u16 = 27301;

/// Health document served on `/api/v1/health`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    ok: bool,
    schema: u32,
    connected: bool,
    protocol: Option<Protocol>,
    clients: usize,
    rate_hz: f64,
    seq: u64,
}

pub struct Running {
    pub config: ApiConfig,
    tcp: Option<Arc<TcpServer>>,
    http: Option<HttpServer>,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
    pub tcp_addr: Option<String>,
    pub http_addr: Option<String>,
    pub udp_target: Option<String>,
}

impl Running {
    /// Bind the transports and start the ticker. Fails (without leaving anything bound) when a port
    /// is taken — the settings row shows the message.
    pub fn start(app: AppHandle, state: Arc<Mutex<ApiState>>, cfg: ApiConfig) -> Result<Self, String> {
        let bind = if cfg.lan { "0.0.0.0" } else { "127.0.0.1" };
        let rate_hz = cfg.rate_hz.clamp(1.0, 10.0);

        let tcp = if cfg.tcp {
            let hello = serde_json::to_string(&Hello { schema: SCHEMA_VERSION, hello: true, groups: GROUPS, rate_hz })
                .map_err(|e| e.to_string())?
                + "\n";
            Some(Arc::new(TcpServer::open(bind, TCP_PORT, hello).map_err(|e| format!("TCP stream: {e}"))?))
        } else {
            None
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let http = if cfg.http {
            Some(HttpServer::open(bind, HTTP_PORT, shared.clone()).map_err(|e| format!("HTTP: {e}"))?)
        } else {
            None
        };
        let mut udp = if cfg.udp.enabled {
            Some(UdpSink::open(&cfg.udp.host, cfg.udp.port).map_err(|e| format!("UDP: {e}"))?)
        } else {
            None
        };
        let tcp_addr = tcp.as_ref().map(|t| t.addr().to_string());
        let http_addr = http.as_ref().map(|h| h.addr().to_string());
        let udp_target = udp.as_ref().map(|u| u.description());

        let stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let stop = stop.clone();
            let tcp = tcp.clone();
            let shared = shared.clone();
            let period = Duration::from_secs_f64(1.0 / rate_hz);
            thread::spawn(move || {
                let mut seq: u64 = 0;
                let mut was_connected = false;
                let mut next = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    next += period;
                    let link = link_info(&app);
                    // A fresh vehicle link starts from a clean slate — no stale groups from the last one.
                    if link.connected && !was_connected {
                        state.lock().unwrap().reset_link();
                    }
                    was_connected = link.connected;

                    seq += 1;
                    let ts = now_ms();
                    let line = {
                        let st = state.lock().unwrap();
                        let frame = Frame::build(&st, &link, seq, ts);
                        serde_json::to_string(&frame).unwrap_or_default()
                    };
                    let clients = tcp.as_ref().map(|t| t.client_count()).unwrap_or(0);
                    {
                        let mut sh = shared.lock().unwrap();
                        sh.last_frame = Some(line.clone());
                        sh.health = serde_json::to_string(&Health {
                            ok: true,
                            schema: SCHEMA_VERSION,
                            connected: link.connected,
                            protocol: link.protocol,
                            clients,
                            rate_hz,
                            seq,
                        })
                        .unwrap_or_default();
                    }
                    if let Some(t) = &tcp {
                        t.broadcast(format!("{line}\n").as_bytes());
                    }
                    if let Some(u) = udp.as_mut() {
                        if let Err(e) = u.write(line.as_bytes()) {
                            log::debug!("[telemetry-api] {e}");
                        }
                    }
                    // Fixed-phase sleep so the rate does not drift with frame build time.
                    let now = Instant::now();
                    if next > now {
                        thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            })
        };

        Ok(Self { config: cfg, tcp, http, shared, stop, ticker: Some(ticker), tcp_addr, http_addr, udp_target })
    }

    pub fn clients(&self) -> usize {
        self.tcp.as_ref().map(|t| t.client_count()).unwrap_or(0)
    }

    /// The newest frame, for a one-shot read without a transport.
    #[allow(dead_code)]
    pub fn last_frame(&self) -> Option<String> {
        self.shared.lock().unwrap().last_frame.clone()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.ticker.take() {
            let _ = h.join();
        }
        // Transports close with their fields (tcp/http Drop stop their accept loops).
        self.http.take();
    }
}

/// What the link looks like right now, from the app state the connection commands maintain.
fn link_info(app: &AppHandle) -> LinkInfo {
    let st = app.state::<AppState>();
    let protocol = st.protocol.lock().ok().and_then(|p| {
        p.as_ref().map(|a| match a {
            ActiveProtocol::Msp(_) => Protocol::Msp,
            ActiveProtocol::Mavlink(_) => Protocol::Mavlink,
            ActiveProtocol::PassiveTelemetry(_) => Protocol::Passive,
        })
    });
    let fc_variant = st.fc_info.lock().ok().and_then(|f| f.as_ref().map(|i| i.fc_variant.clone()));
    LinkInfo { connected: protocol.is_some(), protocol, fc_variant }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
