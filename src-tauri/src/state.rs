// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::aero::AeroCache;
use crate::flightlog::recorder::PendingSessionHandle;
use crate::mavlink_proto::MavlinkHandle;
use crate::msp::FcInfo;
use crate::passive_telemetry::PassiveHandle;
use crate::radar::source::SourceUpdate;
use crate::radar::RadarManager;
use crate::scheduler::rc_tx::{RcTxHandle, RcTxState};
use crate::scheduler::SchedulerHandle;

/// Which protocol is currently active
pub enum ActiveProtocol {
    Msp(SchedulerHandle),
    Mavlink(MavlinkHandle),
    /// Passive, listen-only telemetry (FrSkyX/CRSF/LTM/MAVLink-passive), protocol auto-detected.
    PassiveTelemetry(PassiveHandle),
}

/// Global application state managed by Tauri
pub struct AppState {
    /// Active protocol handler (None when disconnected)
    pub protocol: Mutex<Option<ActiveProtocol>>,
    /// Flight controller info from last successful handshake
    pub fc_info: Mutex<Option<FcInfo>>,
    /// Radar (foreign-vehicle tracking) subsystem — fully independent of `protocol`.
    pub radar: Mutex<RadarManager>,
    /// Bridge for scheduler-fed radar sources (ADS-B via MSP): the radar aggregator's ingest channel
    /// (Some while radar runs) and a runtime on/off flag the MSP scheduler polls.
    pub radar_ingest: Arc<Mutex<Option<std::sync::mpsc::Sender<SourceUpdate>>>>,
    pub radar_msp_enabled: Arc<AtomicBool>,
    /// GCS RC-injection state (docs/active/RC_CONTROL.md §10 Phase 4c). Written by the rc_stream_*
    /// commands, read+streamed by the MSP scheduler thread. Independent of `protocol` lifecycle.
    pub rc_tx: RcTxHandle,
    /// Stop handle for the live BLE scan session (Some while scanning). Dropping/replacing the
    /// sender ends the session — see `commands::connection::ble_scan_start` / `ble_scan_stop`.
    pub ble_scan_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Airspace Manager (aeronautical data) — last fetched region cached in RAM, or None.
    pub aero: Mutex<Option<AeroCache>>,
    /// Pending live-recording session awaiting commit/discard (deferred commit, ADR-041). Set by the
    /// recorder on disarm; resolved by the Save/Discard commands or the recorder's grace-arm path.
    /// Lives here (not in the recorder) so it survives a disconnect while the End-Flight dialog is open.
    pub pending_session: PendingSessionHandle,
    /// A recovered orphan session the user chose to **continue on reconnect** (ADR-042). The next
    /// recorder consults it on its first polled status: armed → resume the same `.ktmp`; disarmed →
    /// finalize it into `pending_session` + the End-Flight dialog.
    pub resume_pending: PendingSessionHandle,
}

impl AppState {
    pub fn new() -> Self {
        let radar = RadarManager::new();
        let radar_ingest = radar.ingest_handle();
        Self {
            protocol: Mutex::new(None),
            fc_info: Mutex::new(None),
            radar: Mutex::new(radar),
            radar_ingest,
            radar_msp_enabled: Arc::new(AtomicBool::new(false)),
            rc_tx: Arc::new(Mutex::new(RcTxState::default())),
            ble_scan_stop: Mutex::new(None),
            aero: Mutex::new(None),
            pending_session: Arc::new(Mutex::new(None)),
            resume_pending: Arc::new(Mutex::new(None)),
        }
    }
}
