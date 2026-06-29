// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Passive telemetry handler — a dedicated, strictly listen-only thread that owns the ByteTransport.
//
// It reads incoming bytes, captures them to file (Phase B), feeds the protocol detector and reports
// framing stats to the Debug Monitor via the `debug-telemetry-stats` event. It NEVER writes to the
// transport.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::transport::{ByteTransport, TransportError};

use super::capture::Capture;
use super::decoders::crsf::CrsfDecoder;
use super::decoders::frsky::FrskyDecoder;
use super::decoders::ltm::LtmDecoder;
use super::detector::{Detector, Protocol};
use super::msp_probe::{MspProbe, MspProbeStats};

/// Bytes of the live stream kept for the Debug Monitor hex tail.
const HEX_TAIL_BYTES: usize = 192;
/// Debug snapshot emit throttle.
const EMIT_INTERVAL: Duration = Duration::from_millis(100);
/// Capture flush cadence (so a crash keeps most of the data).
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// FC link is considered lost after this long without a fresh FC-origin frame. The receiver/TX keeps
/// sending housekeeping (RSSI, link-stats) after the FC drops, so "any data" never goes stale — this
/// tracks FC-origin frames specifically.
const FC_STALE_MS: u128 = 5000;
/// Experimental MSP-over-SmartPort probe — the one deliberate exception to the listen-only rule. Tests
/// whether the radio's BLE bridge forwards an inbound S.Port frame into the FC uplink (see `msp_probe`).
/// Dev-only (release never writes to the transport).
///
/// STATUS (2026-06-17): kept ARMED but not yet working. On the FrSky X20RS the Ethos Bluetooth only does
/// telemetry *forwarding* (downlink mirror) — injected writes to every writable characteristic
/// (0xFFF3/0xFFF6) × all frame variants produced no FC reply, while the passive MSP reassembler decoded
/// the radio's own polling cleanly. Pending the Ethos dev confirming whether a bidirectional/serial BT
/// mode can forward the uplink; once it does, this fires automatically on reconnect. This whole feature
/// is an isolated commit and can be reverted wholesale if the idea is abandoned.
const MSP_PROBE_ENABLED: bool = cfg!(debug_assertions);
/// How often to fire the MSP_API_VERSION probe request.
const MSP_PROBE_INTERVAL: Duration = Duration::from_secs(1);

pub enum PassiveCommand {
    Stop,
}

pub struct PassiveHandle {
    cmd_tx: mpsc::Sender<PassiveCommand>,
    thread: Option<thread::JoinHandle<Option<Box<dyn ByteTransport>>>>,
}

impl PassiveHandle {
    /// Stop the handler and return the transport for cleanup.
    pub fn stop(mut self) -> Option<Box<dyn ByteTransport>> {
        let _ = self.cmd_tx.send(PassiveCommand::Stop);
        self.thread.take().and_then(|t| t.join().ok()).flatten()
    }
}

/// Start the passive telemetry handler on a dedicated thread.
pub fn start(
    transport: Box<dyn ByteTransport>,
    app_handle: AppHandle,
    recorder: Option<FlightRecorderHandle>,
) -> PassiveHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<PassiveCommand>();
    let thread = thread::spawn(move || handler_loop(transport, app_handle, cmd_rx, recorder));
    PassiveHandle {
        cmd_tx,
        thread: Some(thread),
    }
}

/// Per-protocol hit count for the Debug Monitor.
#[derive(Clone, Serialize)]
struct ProtoHit {
    name: String,
    count: u32,
}

/// Altitude-reference signal (`telemetry-alt-ref`): tells the frontend whether this protocol's GPS
/// altitude is true MSL (`msl: true`) or only relative-to-arming (`msl: false`). Relative-only protocols
/// (LTM, CRSF) carry no MSL, so the frontend must anchor them to a ground reference for correct AGL/3D.
#[derive(Clone, Serialize)]
struct AltRef {
    msl: bool,
}

/// Active-protocol signal (`telemetry-protocol`) for the connection status box: the locked primary
/// protocol + an optional secondary (a higher-level protocol tunneled inside it, e.g. ArduPilot
/// passthrough → "MAVLink").
#[derive(Clone, Serialize, PartialEq)]
struct ProtoInfo {
    primary: String,
    secondary: Option<String>,
}

/// FC-link liveness signal (`telemetry-fc-link`) for the status box: false once FC-origin frames have
/// gone quiet for `FC_STALE_MS`, even while the receiver/TX keeps sending housekeeping.
#[derive(Clone, Serialize)]
struct FcLink {
    alive: bool,
}

/// Display name of the locked carrier protocol for the status box ("SmartPort", not "FrSkyX/SmartPort").
fn primary_name(p: Protocol) -> &'static str {
    match p {
        Protocol::Frsky => "SmartPort",
        Protocol::Crsf => "CRSF",
        Protocol::Ltm => "LTM",
        Protocol::Mavlink => "MAVLink",
    }
}

/// Debug snapshot emitted as `debug-telemetry-stats`.
#[derive(Clone, Serialize)]
struct TelemSnapshot {
    /// Locked protocol name, or "" while still searching.
    locked: String,
    /// Best current guess (highest hits), or "" if nothing matched yet.
    best_guess: String,
    total_bytes: u64,
    bytes_per_sec: u64,
    chunk_count: u64,
    /// Last bytes of the stream, space-separated hex.
    hex_tail: String,
    /// Absolute path of the .bin capture (so the user can find/hand off the files).
    capture_file: String,
    /// Decoded CRSF frame count (0 unless CRSF is locked) — confirms live decoding into the dump.
    crsf_frames: u64,
    /// Decoded LTM frame count (0 unless LTM is locked).
    ltm_frames: u64,
    /// MSP-over-SmartPort probe diagnostics (only present in dev builds once SmartPort is locked).
    msp_probe: Option<MspProbeStats>,
    hits: Vec<ProtoHit>,
}

fn handler_loop(
    mut transport: Box<dyn ByteTransport>,
    app_handle: AppHandle,
    cmd_rx: mpsc::Receiver<PassiveCommand>,
    recorder: Option<FlightRecorderHandle>,
) -> Option<Box<dyn ByteTransport>> {
    log::info!(
        "Passive telemetry handler started (listen-only) via {}",
        transport.description()
    );

    // Capture files live under the raw-log dir (Documents/KiteGC by default), in a radiotelem/ subfolder.
    let raw_dir = crate::flightlog::db::resolve_raw_log_dir("", crate::is_portable());
    let mut capture = match Capture::new(&raw_dir) {
        Ok(c) => {
            log::info!("Radio telemetry capture → {}", c.bin_path());
            Some(c)
        }
        Err(e) => {
            log::error!("Failed to open radio telemetry capture: {}", e);
            None
        }
    };
    let capture_file = capture.as_ref().map(|c| c.bin_path()).unwrap_or_default();

    let mut detector = Detector::new();
    // Active decoder, created once the detector locks a protocol we can decode (FrSky first).
    let mut frsky: Option<FrskyDecoder> = None;
    // CRSF decoder (Phase E2): on lock, decodes frames into the radiotelem_<ts>.crsf.txt dump for
    // offline scaling validation. Does NOT yet publish unified events (that is E4).
    let mut crsf: Option<CrsfDecoder> = None;
    // LTM decoder: on lock, publishes unified events + feeds the recorder, and writes a
    // radiotelem_<ts>.ltm.txt decoded dump for validation.
    let mut ltm: Option<LtmDecoder> = None;
    // Experimental MSP-over-SmartPort probe (dev-only): created once SmartPort is locked.
    let mut msp_probe: Option<MspProbe> = None;
    let mut last_probe = Instant::now() - MSP_PROBE_INTERVAL;
    // Refine the recorder's protocol label once the sub-protocol is locked (set only once).
    let mut protocol_labeled = false;
    // Last protocol info pushed to the status box (re-emitted only on change, e.g. AP passthrough start).
    let mut last_proto: Option<ProtoInfo> = None;
    // Last FC-link-alive state pushed to the status box (re-emitted only on change).
    let mut last_fc_alive: Option<bool> = None;
    let mut hex_tail: VecDeque<u8> = VecDeque::with_capacity(HEX_TAIL_BYTES);
    let mut buf = [0u8; 1024];

    // Throughput window.
    let mut win_start = Instant::now();
    let mut win_bytes: u64 = 0;
    let mut last_bytes_per_sec: u64 = 0;

    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut last_flush = Instant::now();

    loop {
        // 1. Commands (non-blocking).
        match cmd_rx.try_recv() {
            Ok(PassiveCommand::Stop) => {
                log::info!("Passive telemetry handler stopping");
                if let Some(c) = capture.as_mut() { c.flush(); }
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                return Some(transport);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(c) = capture.as_mut() { c.flush(); }
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                return Some(transport);
            }
        }

        // 2. Read incoming bytes — listen-only, never write.
        match transport.read_bytes(&mut buf) {
            Ok(0) => {
                // Idle (timeout). Small sleep so we never busy-spin if a transport returns immediately.
                thread::sleep(Duration::from_millis(1));
            }
            Ok(n) => {
                let chunk = &buf[..n];
                if let Some(c) = capture.as_mut() {
                    c.write(chunk);
                }
                detector.push(chunk);
                // On first lock, refine the recorder's protocol label to the detected sub-protocol.
                if !protocol_labeled {
                    if let Some(p) = detector.locked() {
                        let label = match p {
                            Protocol::Frsky => "Telemetry (SmartPort)",
                            Protocol::Crsf => "Telemetry (CRSF)",
                            Protocol::Ltm => "Telemetry (LTM)",
                            Protocol::Mavlink => "Telemetry (MAVLink)",
                        };
                        if let Some(ref rec) = recorder {
                            if let Ok(mut r) = rec.lock() { r.set_protocol(label); }
                        }
                        // Tell the frontend whether this protocol delivers true MSL altitude. LTM/CRSF
                        // send only arming-relative altitude → the frontend must anchor it to a ground
                        // reference (captured at the arm edge) for correct AGL/3D.
                        let msl = !matches!(p, Protocol::Ltm | Protocol::Crsf);
                        let _ = app_handle.emit("telemetry-alt-ref", AltRef { msl });
                        log::info!("Passive telemetry locked: {} (alt MSL: {})", label, msl);
                        protocol_labeled = true;
                    }
                }
                // Once FrSky is locked, decode the stream into unified telemetry events.
                if detector.locked() == Some(Protocol::Frsky) {
                    frsky.get_or_insert_with(FrskyDecoder::new).push_bytes(chunk);
                    // Experimental MSP probe: watch the same stream for MSP replies (primID 0x32).
                    if MSP_PROBE_ENABLED {
                        msp_probe.get_or_insert_with(MspProbe::new).push_bytes(chunk);
                    }
                }
                // Once CRSF is locked, decode frames into the analysis dump (E2; no widget feed yet).
                if detector.locked() == Some(Protocol::Crsf) {
                    crsf.get_or_insert_with(|| {
                        CrsfDecoder::new(capture.as_ref().map(|c| c.sibling_path("crsf.txt")))
                    })
                    .push_bytes(chunk);
                }
                // Once LTM is locked, decode into unified events (+ dump for validation).
                if detector.locked() == Some(Protocol::Ltm) {
                    ltm.get_or_insert_with(|| {
                        LtmDecoder::new(capture.as_ref().map(|c| c.sibling_path("ltm.txt")))
                    })
                    .push_bytes(chunk);
                }
                win_bytes += n as u64;
                for &b in chunk {
                    if hex_tail.len() == HEX_TAIL_BYTES {
                        hex_tail.pop_front();
                    }
                    hex_tail.push_back(b);
                }
            }
            Err(TransportError::Timeout) => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(TransportError::Disconnected) => {
                log::warn!("Passive telemetry transport disconnected");
                if let Some(c) = capture.as_mut() { c.flush(); }
                if let Some(ref rec) = recorder {
                    if let Ok(mut r) = rec.lock() { r.shutdown(); }
                }
                let _ = app_handle.emit("telemetry-disconnected", ());
                return Some(transport);
            }
            Err(e) => {
                log::warn!("Passive telemetry read error: {}", e);
            }
        }

        // 2b. Fire the experimental MSP probe (dev-only). Writes one MSP_API_VERSION request frame
        //     into the transport every MSP_PROBE_INTERVAL — the deliberate exception to listen-only.
        if let Some(probe) = msp_probe.as_mut() {
            if last_probe.elapsed() >= MSP_PROBE_INTERVAL {
                let frame = probe.next_tx();
                match transport.write_bytes(&frame) {
                    Ok(()) => log::debug!("[MSP-PROBE] tx {} bytes", frame.len()),
                    Err(e) => log::warn!("[MSP-PROBE] tx failed: {}", e),
                }
                last_probe = Instant::now();
            }
        }

        // 3. Roll the throughput window.
        let elapsed = win_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            last_bytes_per_sec = (win_bytes as f64 / elapsed) as u64;
            win_bytes = 0;
            win_start = Instant::now();
        }

        // 4. Periodic capture + dump flush.
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            if let Some(c) = capture.as_mut() {
                c.flush();
            }
            if let Some(c) = crsf.as_mut() {
                c.flush();
            }
            if let Some(c) = ltm.as_mut() {
                c.flush();
            }
            last_flush = Instant::now();
        }

        // 5. Emit debug stats (throttled).
        if last_emit.elapsed() >= EMIT_INTERVAL {
            let hex_tail_str = hex_tail
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");
            let snapshot = TelemSnapshot {
                locked: detector.locked().map(|p| p.name().to_string()).unwrap_or_default(),
                best_guess: detector.best_guess().map(|p| p.name().to_string()).unwrap_or_default(),
                total_bytes: detector.total_bytes(),
                bytes_per_sec: last_bytes_per_sec,
                chunk_count: capture.as_ref().map(|c| c.chunks()).unwrap_or(0),
                hex_tail: hex_tail_str,
                capture_file: capture_file.clone(),
                crsf_frames: crsf.as_ref().map(|c| c.frames()).unwrap_or(0),
                ltm_frames: ltm.as_ref().map(|c| c.frames()).unwrap_or(0),
                msp_probe: msp_probe.as_ref().map(|p| p.stats().clone()),
                hits: detector
                    .hit_table()
                    .into_iter()
                    .map(|(name, count)| ProtoHit { name: name.to_string(), count })
                    .collect(),
            };
            let _ = app_handle.emit("debug-telemetry-stats", &snapshot);

            // Push decoded telemetry to the widgets/map (unified events) + the flight recorder.
            if let Some(dec) = frsky.as_mut() {
                dec.publish(&app_handle, recorder.as_ref());
            }
            if let Some(dec) = crsf.as_mut() {
                dec.publish(&app_handle, recorder.as_ref());
            }
            if let Some(dec) = ltm.as_mut() {
                dec.publish(&app_handle, recorder.as_ref());
            }

            // Connection status-box protocol signal (primary + optional secondary), change-detected so
            // it fires on lock and again when ArduPilot passthrough turns up as a secondary protocol.
            if let Some(p) = detector.locked() {
                let secondary = if frsky.as_ref().is_some_and(|d| d.ap_active())
                    || crsf.as_ref().is_some_and(|d| d.ap_active())
                {
                    Some("MAVLink".to_string())
                } else {
                    None
                };
                let info = ProtoInfo { primary: primary_name(p).to_string(), secondary };
                if last_proto.as_ref() != Some(&info) {
                    let _ = app_handle.emit("telemetry-protocol", &info);
                    last_proto = Some(info);
                }

                // FC-link liveness: based on fresh FC-origin frames (not RX/TX housekeeping).
                let fc_age = match p {
                    Protocol::Frsky => frsky.as_ref().and_then(|d| d.fc_age_ms()),
                    Protocol::Crsf => crsf.as_ref().and_then(|d| d.fc_age_ms()),
                    Protocol::Ltm => ltm.as_ref().and_then(|d| d.fc_age_ms()),
                    Protocol::Mavlink => None,
                };
                let alive = fc_age.is_none_or(|ms| ms < FC_STALE_MS);
                if last_fc_alive != Some(alive) {
                    let _ = app_handle.emit("telemetry-fc-link", FcLink { alive });
                    last_fc_alive = Some(alive);
                }
            }
            last_emit = Instant::now();
        }
    }
}
