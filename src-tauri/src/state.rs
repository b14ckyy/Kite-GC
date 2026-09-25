// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use crate::aero::AeroCache;
use crate::flightlog::recorder::{ActiveTempPathHandle, FlightRecorderHandle, PendingSessionHandle};
use crate::mavlink_proto::MavlinkHandle;
use crate::msp::FcInfo;
use crate::passive_telemetry::PassiveHandle;
use crate::radar::source::SourceUpdate;
use crate::radar::RadarManager;
use crate::scheduler::rc_tx::{RcTxHandle, RcTxState};
use crate::scheduler::{MspRequester, SchedulerHandle};

/// Which protocol is currently active
pub enum ActiveProtocol {
    Msp(SchedulerHandle),
    Mavlink(MavlinkHandle),
    /// Passive, listen-only telemetry (FrSkyX/CRSF/LTM/MAVLink-passive), protocol auto-detected.
    PassiveTelemetry(PassiveHandle),
}

/// Error of every INAV/MSP command on a link without MSP (plain MAVLink, passive telemetry).
pub(crate) const NO_MSP_ERR: &str = "FC is not running MSP (INAV)";

/// Run `f` against the connection's MSP scheduler: a direct MSP link, or the MSP-over-MAVLink tunnel
/// scheduler of a MAVLink link to INAV 10.0+ (`MavlinkHandle.msp`). This is THE backend answer to "does
/// this link have MSP" — every INAV one-shot command (mission, safehome, geozone, settings, craft name,
/// stats, RC config) resolves its scheduler here, so it works the same over both links.
///
/// The protocol lock is only held to clone the request handle; `f` runs without it (a tunnel mission
/// transfer is N × RTT and must not block `disconnect`). Transactions of one connection still run one
/// at a time (`MspRequester::begin_transaction`). A disconnect mid-transaction makes the next request
/// fail fast ("Scheduler thread gone"). Never call `with_msp` from inside `f`.
pub(crate) fn with_msp<T>(
    state: &AppState,
    f: impl FnOnce(&MspRequester) -> Result<T, String>,
) -> Result<T, String> {
    let msp = {
        let proto = state.protocol.lock().map_err(|e| e.to_string())?;
        match proto.as_ref() {
            Some(p) => p.msp_requester().ok_or_else(|| NO_MSP_ERR.to_string())?,
            None => return Err("Not connected".into()),
        }
    };
    let _txn = msp.begin_transaction();
    f(&msp)
}

/// `with_msp` on a blocking worker thread, for the long multi-request commands (mission transfers,
/// safehome / geozone batches): Tauri runs `async` commands on the async runtime's workers, and a tunnel
/// transaction of tens of seconds would park one of them (on a 4-core device `disconnect` then queues
/// behind it). `f` gets the app handle to reach its own managed state (`app.state::<…>()`).
pub(crate) async fn with_msp_blocking<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppHandle, &MspRequester) -> Result<T, String> + Send + 'static,
{
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        with_msp(&state, |h| f(&app, h))
    })
    .await
    .map_err(|e| format!("MSP worker failed: {e}"))?
}

impl ActiveProtocol {
    /// The MSP request handle of this link — direct MSP, or the MSP-over-MAVLink tunnel — if it has one.
    pub(crate) fn msp_requester(&self) -> Option<MspRequester> {
        match self {
            ActiveProtocol::Msp(h) => Some(h.requester()),
            ActiveProtocol::Mavlink(m) => m.msp.as_ref().map(|h| h.requester()),
            ActiveProtocol::PassiveTelemetry(_) => None,
        }
    }
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
    /// GCS RC-injection state (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c). Written by the rc_stream_*
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
    /// Temp `.ktmp` the connected recorder is writing right now (`None` while not recording). The
    /// orphan scan and the discard sweeps leave it alone — it is a live session, not a leftover.
    pub active_temp_path: ActiveTempPathHandle,
    /// Flight recorder of the active connection (`None` while disconnected or with logging off). Lets
    /// the command layer reach the recorder protocol-independently — the live platform-type override.
    pub recorder: Mutex<Option<FlightRecorderHandle>>,
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
            active_temp_path: Arc::new(Mutex::new(None)),
            recorder: Mutex::new(None),
        }
    }
}
