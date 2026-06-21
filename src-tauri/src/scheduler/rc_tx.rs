// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-injection shared state (docs/active/RC_CONTROL.md §10 Phase 4c). The frontend (rcEngine) writes the
// latest channel frame here as a heartbeat; the scheduler thread reads it and streams the RC to the FC:
//   • MSP_SET_RAW_RC  — fire-and-forget at a fixed rate (highest priority, never blocks on an ACK so the
//     RAW_RC cadence stays jitter-free);
//   • MSP2_INAV_SET_AUX_RC — on change only, also fire-and-forget, woven in at ~5 Hz and re-sent every
//     cycle until the FC's ACK (the echoed 0x2230) is seen, then it stops until the next change. Works on
//     an asymmetric link (FC receives, no downlink): the change still applies, we just keep re-sending.
// No RC leaves the GCS unless `enabled` is true AND the frontend heartbeat is fresh (deadman).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::msp::rc_encode::{encode_aux_rc, AuxResolution};

/// Default RAW_RC send interval (10 Hz) — conservative for slow OTA links. User-selectable up to 25 Hz.
pub const RC_RAW_DEFAULT_INTERVAL: Duration = Duration::from_millis(100);

/// Shared RC-injection state.
pub struct RcTxState {
    /// Master enable — set on engage, cleared on disengage. False = absolutely nothing is sent.
    pub enabled: bool,
    /// Latest encoded MSP_SET_RAW_RC payload (u16-LE CH1..CHmax). Empty = nothing to stream yet.
    pub raw: Vec<u8>,
    /// Wall-clock of the last frontend update — drives the deadman (frontend gone → stop streaming).
    pub last_update: Instant,
    /// Accumulated AUX-RC changes awaiting delivery: 0-based channel → target µs. The frontend pushes
    /// only changed channels here; the scheduler packs them into the minimal 16-bit run on send and
    /// clears the map once the FC ACK (0x2230) is seen. Accumulating (vs a single payload) means a change
    /// is never lost when the frontend updates faster than the 5 Hz send.
    pub aux_pending: BTreeMap<u8, u16>,
    /// RAW_RC send interval (user-selectable 10/15/20/25 Hz). AUX stays fixed at 5 Hz.
    pub raw_interval: Duration,
}

impl Default for RcTxState {
    fn default() -> Self {
        Self {
            enabled: false,
            raw: Vec::new(),
            last_update: Instant::now(),
            aux_pending: BTreeMap::new(),
            raw_interval: RC_RAW_DEFAULT_INTERVAL,
        }
    }
}

/// Pack the pending AUX changes into one 16-bit MSP2_INAV_SET_AUX_RC payload covering the minimal run
/// from the lowest to the highest changed channel — channels in between that aren't pending are sent as
/// 0 (skip, untouched). Full µs fidelity, smallest range. `None` if empty or the run is invalid.
pub fn aux_payload(pending: &BTreeMap<u8, u16>) -> Option<Vec<u8>> {
    let min = *pending.keys().next()?;
    let max = *pending.keys().next_back()?;
    let values: Vec<u16> = (min..=max).map(|c| pending.get(&c).copied().unwrap_or(0)).collect();
    encode_aux_rc(min, AuxResolution::Bits16, &values).ok()
}

/// Shared handle into the RC-injection state (AppState ↔ scheduler thread).
pub type RcTxHandle = Arc<Mutex<RcTxState>>;
