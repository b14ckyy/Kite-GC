// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use std::sync::mpsc;
use tauri::{AppHandle, Manager};

use crate::aero::AeroCache;
use crate::flightlog::recorder::{FlightRecorderHandle, SessionSlotsHandle};
use crate::mavlink_proto::handler::MavlinkCommand;
use crate::mavlink_proto::MavlinkHandle;
use crate::msp::FcInfo;
use crate::passive_telemetry::PassiveHandle;
use crate::radar::source::SourceUpdate;
use crate::radar::RadarManager;
use crate::scheduler::rc_tx::{RcTxHandle, RcTxState};
use crate::scheduler::{MspRequester, SchedulerHandle};
use crate::vehicle_registry::LinkRegistry;

/// Which protocol is currently active
pub enum ActiveProtocol {
    Msp(SchedulerHandle),
    Mavlink(MavlinkHandle),
    /// Passive, listen-only telemetry (FrSkyX/CRSF/LTM/MAVLink-passive), protocol auto-detected.
    PassiveTelemetry(PassiveHandle),
}

/// A resolved MAVLink command target: the link's handler channel plus the system id to address.
/// Returned by [`AppState::mav_target`] so command handlers never hold the registry lock while they
/// wait on the vehicle.
pub struct MavTarget {
    pub cmd_tx: mpsc::Sender<MavlinkCommand>,
    pub sysid: u8,
    /// FC variant of the link's primary vehicle ("ArduPlane"/"ArduCopter"/"PX4"/…).
    pub fc_variant: String,
}

/// Error of every INAV/MSP command on a link without MSP (plain MAVLink, passive telemetry).
pub(crate) const NO_MSP_ERR: &str = "FC is not running MSP (INAV)";

/// Run `f` against the connection's MSP scheduler: a direct MSP link, or the MSP-over-MAVLink tunnel
/// scheduler of a MAVLink link to INAV 10.0+ (`MavlinkHandle.msp`). This is THE backend answer to "does
/// this link have MSP" — every INAV one-shot command (mission, safehome, geozone, settings, craft name,
/// stats, RC config) resolves its scheduler here, so it works the same over both links.
///
/// The link-registry lock is only held to clone the request handle; `f` runs without it (a tunnel mission
/// transfer is N × RTT and must not block `disconnect`). Transactions of one connection still run one
/// at a time (`MspRequester::begin_transaction`). A disconnect mid-transaction makes the next request
/// fail fast ("Scheduler thread gone"). Never call `with_msp` from inside `f`.
pub(crate) fn with_msp<T>(
    state: &AppState,
    f: impl FnOnce(&MspRequester) -> Result<T, String>,
) -> Result<T, String> {
    // Multi-vehicle: the ACTIVE vehicle's link. The MSP-over-MAVLink tunnel talks to the link's
    // handshake (primary) vehicle only, so a secondary vehicle on a shared MAVLink link has no MSP.
    let msp = {
        let reg = state.links.lock().map_err(|e| e.to_string())?;
        match reg.active_entry() {
            Some(e) if reg.active() == Some(&e.primary) => {
                e.protocol.msp_requester().ok_or_else(|| NO_MSP_ERR.to_string())?
            }
            // A secondary vehicle: say why when the link does have a tunnel (to its primary).
            Some(e) => {
                return Err(match &e.protocol {
                    ActiveProtocol::Mavlink(m) if m.msp.is_some() => {
                        "MSP over MAVLink is available for the link's primary vehicle only".into()
                    }
                    _ => NO_MSP_ERR.into(),
                })
            }
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
    /// Every open link (protocol handler + handshake info) and the active-vehicle selection. Replaces
    /// the former single `protocol` / `fc_info` slots — see `vehicle_registry`.
    pub links: Mutex<LinkRegistry>,
    /// Radar (foreign-vehicle tracking) subsystem — fully independent of the open links.
    pub radar: Mutex<RadarManager>,
    /// Bridge for scheduler-fed radar sources (ADS-B via MSP): the radar aggregator's ingest channel
    /// (Some while radar runs) and a runtime on/off flag the MSP scheduler polls.
    pub radar_ingest: Arc<Mutex<Option<std::sync::mpsc::Sender<SourceUpdate>>>>,
    pub radar_msp_enabled: Arc<AtomicBool>,
    /// GCS RC-injection state (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c). Written by the rc_stream_*
    /// commands, read+streamed by the MSP scheduler thread. Independent of the link lifecycle.
    pub rc_tx: RcTxHandle,
    /// Stop handle for the live BLE scan session (Some while scanning). Dropping/replacing the
    /// sender ends the session — see `commands::connection::ble_scan_start` / `ble_scan_stop`.
    pub ble_scan_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Airspace Manager (aeronautical data) — last fetched region cached in RAM, or None.
    pub aero: Mutex<Option<AeroCache>>,
    /// Per-vehicle recording slots shared by every recorder: the pending sessions awaiting
    /// commit/discard (deferred commit, ADR-041 — set on disarm, resolved by the Save/Discard commands
    /// or the recorder's grace-arm path; kept here so they survive a disconnect while the End-Flight
    /// dialog is open), the continue-on-reconnect queue (ADR-042) and the live-path registry the orphan
    /// scan and the discard sweeps leave alone. See `recorder::SessionSlots`.
    pub sessions: SessionSlotsHandle,
    /// Flight recorder of every recorded vehicle, by vehicle key (`"L1:S1"`) — link primaries from the
    /// connect paths, secondaries from the MAVLink handler. Lets the command layer reach the ACTIVE
    /// vehicle's recorder protocol-independently (the live platform-type override).
    pub recorders: Mutex<HashMap<String, FlightRecorderHandle>>,
}

impl AppState {
    pub fn new() -> Self {
        let radar = RadarManager::new();
        let radar_ingest = radar.ingest_handle();
        Self {
            links: Mutex::new(LinkRegistry::new()),
            radar: Mutex::new(radar),
            radar_ingest,
            radar_msp_enabled: Arc::new(AtomicBool::new(false)),
            rc_tx: Arc::new(Mutex::new(RcTxState::default())),
            ble_scan_stop: Mutex::new(None),
            aero: Mutex::new(None),
            sessions: Arc::default(),
            recorders: Mutex::new(HashMap::new()),
        }
    }
}

impl AppState {
    /// Resolve a MAVLink command target. `vehicle_id` is the frontend's `"L1:S1"` key, or `None` for the
    /// active vehicle. Errors keep the legacy texts ("Not connected", "FC is not running MAVLink").
    pub fn mav_target(&self, vehicle_id: Option<&str>) -> Result<MavTarget, String> {
        let reg = self.links.lock().map_err(|e| e.to_string())?;
        let (entry, sysid) = reg.resolve(vehicle_id)?;
        match &entry.protocol {
            ActiveProtocol::Mavlink(h) => Ok(MavTarget {
                cmd_tx: h.cmd_tx_clone(),
                sysid,
                fc_variant: h.fc_variant.clone(),
            }),
            _ => Err("FC is not running MAVLink".into()),
        }
    }

    /// Like `mav_target` but `Ok(None)` for non-MAVLink / disconnected — for the fence/rally readers
    /// that answer with an empty config instead of an error.
    pub fn mav_target_opt(&self, vehicle_id: Option<&str>) -> Result<Option<MavTarget>, String> {
        match self.mav_target(vehicle_id) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e == crate::vehicle_registry::ERR_NOT_CONNECTED || e == "FC is not running MAVLink" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Point the MAVLink RC-injection stream at `vehicle` (the new active vehicle) and DISENGAGE it.
    /// Re-pointing a live stream would hand the sticks to a different aircraft without the operator
    /// noticing; they re-engage on the new vehicle explicitly (the frontend mirrors this). The PX4 /
    /// ArduPilot choice follows the link's handshake variant (vehicles sharing one link share it).
    pub fn retarget_rc(&self, vehicle: Option<&crate::vehicle_registry::VehicleId>) {
        let target = vehicle.and_then(|v| {
            let reg = self.links.lock().ok()?;
            let entry = reg.get(v.link)?;
            Some(crate::scheduler::rc_tx::RcTarget {
                link: v.link,
                sysid: v.sysid,
                px4: entry.fc_info.fc_variant.eq_ignore_ascii_case("px4"),
            })
        });
        if let Ok(mut rc) = self.rc_tx.lock() {
            if rc.mav_target != target || target.is_none() {
                rc.enabled = false;
                rc.mav_override_us.clear();
                rc.mav_manual = None;
            }
            rc.mav_target = target;
        }
    }

    /// Register (or, with `None`, drop) the recorder of vehicle `key`.
    pub fn set_recorder(&self, key: &str, recorder: Option<&FlightRecorderHandle>) {
        if let Ok(mut map) = self.recorders.lock() {
            match recorder {
                Some(r) => {
                    map.insert(key.to_string(), r.clone());
                }
                None => {
                    map.remove(key);
                }
            }
        }
    }

    /// Drop the recorders of every vehicle on `link` (the link closed).
    pub fn drop_link_recorders(&self, link: crate::vehicle_registry::LinkId) {
        if let Ok(mut map) = self.recorders.lock() {
            map.retain(|key, _| crate::vehicle_registry::VehicleId::parse(key).is_none_or(|v| v.link != link));
        }
    }

    /// The active link's handshake info, if connected.
    #[allow(dead_code)] // Phase C (link status / relay per-vehicle)
    pub fn active_fc_info(&self) -> Option<FcInfo> {
        self.links.lock().ok().and_then(|r| r.active_fc_info().cloned())
    }
}

