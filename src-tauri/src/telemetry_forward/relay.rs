// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! One relay = a protocol encoder + an output transport + stats. Built from a `RelayConfig` (persisted
//! frontend-side, handed over on primary connect).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::cache::TelemetryCache;
use super::encoders::crsf::CrsfEncoder;
use super::encoders::ltm::LtmEncoder;
use super::encoders::mavlink::MavlinkEncoder;
use super::encoders::smartport::SmartportEncoder;
use super::encoders::Encoder;
use super::output::ble::BleSink;
use super::output::serial::SerialSink;
use super::output::tcp::TcpSink;
use super::output::udp::UdpSink;
use super::output::OutputSink;

/// Shortest gap between two frame sets of one relay (100 Hz). No real source is faster, and it bounds a
/// loop: output that finds its way back into the link (through a second ground station) is relayed
/// again, and without the limit every vehicle frame adds one more that circulates for good.
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(10);

/// Relay configuration from the frontend settings store.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfig {
    pub id: String,
    pub enabled: bool,
    /// Output protocol: "ltm" (more in later phases: "mavlink" / "crsf" / "smartport").
    pub protocol: String,
    pub output: RelayOutput,
}

/// Output transport configuration. Supported `kind`: `serial` / `ble` / `tcp` (server) / `udp`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayOutput {
    pub kind: String,
    /// serial
    pub port: Option<String>,
    pub baud: Option<u32>,
    /// ble (device id)
    pub ble_device_id: Option<String>,
    /// tcp (server listen port)
    pub listen_port: Option<u16>,
    /// udp (send target)
    pub host: Option<String>,
    pub udp_port: Option<u16>,
}

/// Per-relay status snapshot pushed to the frontend (`relay-stats` event).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatusInfo {
    pub id: String,
    pub protocol: String,
    pub target: String,
    /// Last write succeeded (false → error, shown red).
    pub ok: bool,
    /// Open but no consumer yet (e.g. TCP server with no client) → UI shows "waiting".
    pub waiting: bool,
    pub bytes_per_sec: u64,
    pub frames_out: u64,
    pub errors: u64,
}

pub struct Relay {
    pub id: String,
    pub protocol: String,
    pub target: String,
    /// The config this relay was built from — used to reuse unchanged relays across reconfigure.
    pub config: RelayConfig,
    encoder: Box<dyn Encoder>,
    sink: Box<dyn OutputSink>,
    pub bytes_out: u64,
    pub frames_out: u64,
    pub errors: u64,
    pub ok: bool,
    last_emit: Option<Instant>,
}

impl Relay {
    /// Build a live relay from config (opens the output transport — may fail if the device is missing).
    /// Async because the BLE output has to connect (scan + GATT); serial/tcp/udp are immediate.
    pub async fn build(cfg: &RelayConfig) -> Result<Self, String> {
        let encoder: Box<dyn Encoder> = match cfg.protocol.as_str() {
            "ltm" => Box::new(LtmEncoder::new()),
            "mavlink" => Box::new(MavlinkEncoder::new()),
            "crsf" => Box::new(CrsfEncoder::new()),
            "smartport" => Box::new(SmartportEncoder::new()),
            other => return Err(format!("Unsupported relay protocol: {}", other)),
        };
        let sink: Box<dyn OutputSink> = match cfg.output.kind.as_str() {
            "serial" => {
                let port = cfg.output.port.as_deref().filter(|p| !p.is_empty()).ok_or("serial relay needs a port")?;
                let baud = cfg.output.baud.unwrap_or(115200);
                Box::new(SerialSink::open(port, baud)?)
            }
            "ble" => {
                let id = cfg.output.ble_device_id.as_deref().filter(|s| !s.is_empty()).ok_or("ble relay needs a device")?;
                Box::new(BleSink::open(id).await?)
            }
            "tcp" => {
                let port = cfg.output.listen_port.ok_or("tcp relay needs a listen port")?;
                Box::new(TcpSink::open(port)?)
            }
            "udp" => {
                let host = cfg.output.host.as_deref().filter(|h| !h.is_empty()).ok_or("udp relay needs a host")?;
                let port = cfg.output.udp_port.ok_or("udp relay needs a port")?;
                if crate::transport::udp::targets_own_listener(host, port) {
                    return Err(format!(
                        "{host}:{port} is the UDP port Kite itself receives telemetry on. Choose another port or host for this relay."
                    ));
                }
                Box::new(UdpSink::open(host, port)?)
            }
            other => return Err(format!("Unsupported relay output kind: {}", other)),
        };
        let target = sink.description();
        log::info!("[RELAY {}] {} → {}", cfg.id, cfg.protocol, target);
        Ok(Self {
            id: cfg.id.clone(),
            protocol: cfg.protocol.clone(),
            target,
            config: cfg.clone(),
            encoder,
            sink,
            bytes_out: 0,
            frames_out: 0,
            errors: 0,
            ok: true,
            last_emit: None,
        })
    }

    /// Build one full frame set from the cache (called once per pacer tick) and write it out. Errors are
    /// counted, not fatal (the device may reappear).
    pub fn emit_set(&mut self, cache: &TelemetryCache) {
        let now = Instant::now();
        if self.last_emit.is_some_and(|t| now.duration_since(t) < MIN_EMIT_INTERVAL) {
            return;
        }
        self.last_emit = Some(now);
        let frames = self.encoder.frame_set(cache);
        if frames.is_empty() {
            return;
        }
        match self.sink.write(&frames) {
            Ok(()) => {
                self.bytes_out += frames.len() as u64;
                self.frames_out += 1;
                self.ok = true;
            }
            Err(e) => {
                self.errors += 1;
                self.ok = false;
                log::warn!("[RELAY {}] write failed: {}", self.id, e);
            }
        }
    }

    /// Open but waiting for a consumer (e.g. TCP server with no client connected).
    pub fn pending(&self) -> bool {
        self.sink.pending()
    }
}
