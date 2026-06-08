// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Application State
// Holds the shared state for the Tauri application, including the active connection.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::mavlink_proto::MavlinkHandle;
use crate::msp::FcInfo;
use crate::radar::source::SourceUpdate;
use crate::radar::RadarManager;
use crate::scheduler::SchedulerHandle;

/// Which protocol is currently active
pub enum ActiveProtocol {
    Msp(SchedulerHandle),
    Mavlink(MavlinkHandle),
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
    /// Stop handle for the live BLE scan session (Some while scanning). Dropping/replacing the
    /// sender ends the session — see `commands::connection::ble_scan_start` / `ble_scan_stop`.
    pub ble_scan_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
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
            ble_scan_stop: Mutex::new(None),
        }
    }
}
