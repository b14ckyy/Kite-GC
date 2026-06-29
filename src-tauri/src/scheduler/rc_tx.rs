// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-injection shared state (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c + docs/active/MAVLINK_RC_CONTROL.md).
// The frontend (rcEngine) writes the latest channel frame here as a heartbeat; the owning protocol thread
// reads it and streams RC to the FC. The transport differs by platform but the gate (enabled + fresh
// heartbeat) and rate are shared:
//
//   INAV / MSP (scheduler thread):
//   • MSP_SET_RAW_RC  — fire-and-forget at a fixed rate (highest priority, never blocks on an ACK so the
//     RAW_RC cadence stays jitter-free);
//   • MSP2_INAV_SET_AUX_RC — on change only, also fire-and-forget, woven in at ~5 Hz and re-sent every
//     cycle until the FC's ACK (the echoed 0x2230) is seen, then it stops until the next change. Works on
//     an asymmetric link (FC receives, no downlink): the change still applies, we just keep re-sending.
//
//   ArduPilot / MAVLink (handler thread):
//   • RC_CHANNELS_OVERRIDE (#70) — one positional frame (CH1..18) at the same selectable rate. There is
//     no override-mode gate and no AUX split on the wire; `mav_override_us` carries the whole frame.
//
// No RC leaves the GCS unless `enabled` is true AND the frontend heartbeat is fresh (deadman).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::msp::rc_encode::{encode_aux_rc, AuxResolution};

/// Default RAW_RC send interval (10 Hz) — conservative for slow OTA links. User-selectable up to 25 Hz.
pub const RC_RAW_DEFAULT_INTERVAL: Duration = Duration::from_millis(100);

/// Frontend-heartbeat deadman: no fresh RC frame for this long → stop streaming (the FC then failsafes).
/// Shared by both the MSP scheduler and the MAVLink handler so the gate is identical across platforms.
pub const RC_DEADMAN: Duration = Duration::from_millis(500);

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
    /// ArduPilot/MAVLink override frame: positional µs per channel, index 0 = CH1, `0` = uncontrolled
    /// (the MAVLink handler maps it to the per-band "ignore" sentinel). Empty = nothing to stream.
    /// Independent of `raw`/`aux_pending` (those are MSP-only); only one protocol thread reads each.
    pub mav_override_us: Vec<u16>,
    /// PX4/MAVLink manual-control state (`MANUAL_CONTROL` #69). `None` = nothing to stream. The PX4 path
    /// uses this instead of `mav_override_us` (normalised 4 axes + buttons, not per-channel µs).
    pub mav_manual: Option<ManualControl>,
}

/// One `MANUAL_CONTROL` setpoint (PX4 manual flying over MAVLink). Axes are normalised to [-1000, 1000]
/// (z thrust: -1000 = 0 %, 0 = mid, +1000 = full). `buttons`/`buttons2` are the 1→pressed bitfields for
/// joystick buttons 1–16 / 17–32 (PX4 maps each to an action FC-side). `aux[0..6]` are the optional
/// continuous extension axes (aux1–6); `ext` is the `enabled_extensions` bitmask flagging which are valid.
#[derive(Clone, Copy)]
pub struct ManualControl {
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub r: i16,
    pub buttons: u16,
    pub buttons2: u16,
    pub aux: [i16; 6],
    pub ext: u8,
}

impl Default for RcTxState {
    fn default() -> Self {
        Self {
            enabled: false,
            raw: Vec::new(),
            last_update: Instant::now(),
            aux_pending: BTreeMap::new(),
            raw_interval: RC_RAW_DEFAULT_INTERVAL,
            mav_override_us: Vec::new(),
            mav_manual: None,
        }
    }
}

/// Number of channels in an RC_CHANNELS_OVERRIDE frame (chan1_raw..chan18_raw).
pub const MAV_OVERRIDE_CHANNELS: usize = 18;

/// The MAVLink "ignore this channel" sentinel differs by band (verified against the `mavlink` crate's
/// RC_CHANNELS_OVERRIDE field docs): CH1–8 use UINT16_MAX, CH9–18 use 0.
#[inline]
fn ignore_sentinel(idx0: usize) -> u16 {
    if idx0 < 8 { u16::MAX } else { 0 }
}

/// Build the 18-channel RC_CHANNELS_OVERRIDE wire frame from positional µs values. `us[i]` is the
/// override for CH(i+1); `0` (or any channel past `us.len()`) means "leave this channel to the real RX"
/// → the per-band ignore sentinel. Controlled values are clamped to a sane PWM window.
///
/// Note: we never emit the per-band *release* sentinel (CH1–8 = 0, CH9–18 = 65534). With the GCS as the
/// sole RC source an explicit release fires ArduPilot's RC failsafe instantly; on disengage we just stop
/// streaming and let the FC's RC_OVERRIDE_TIME grace window run (see the MAVLink handler).
pub fn override_channels(us: &[u16]) -> [u16; MAV_OVERRIDE_CHANNELS] {
    let mut ch = [0u16; MAV_OVERRIDE_CHANNELS];
    for (i, slot) in ch.iter_mut().enumerate() {
        let v = us.get(i).copied().unwrap_or(0);
        *slot = if v == 0 { ignore_sentinel(i) } else { v.clamp(800, 2200) };
    }
    ch
}

/// The MAVLink "release this channel back to the RC radio" sentinel — distinct from the *ignore*
/// sentinel: CH1–8 use 0, CH9–18 use 65534 (verified against the RC_CHANNELS_OVERRIDE field docs).
#[inline]
fn release_sentinel(idx0: usize) -> u16 {
    if idx0 < 8 { 0 } else { 65534 }
}

/// Build an 18-channel RC_CHANNELS_OVERRIDE frame that *releases* every channel we were controlling
/// (non-zero in `us`) back to the FC's RC radio, leaving channels we never touched ignored. Sent a few
/// times on a **deliberate** disengage so ArduPilot hands control back immediately (revert to a real RX,
/// or RC failsafe if the GCS was the sole RC source) instead of waiting out `RC_OVERRIDE_TIME`. This is
/// the documented MAVLink convention (0 = release for CH1–8), as used by RC_CHANNELS_OVERRIDE-based GCS
/// like Mission Planner. Involuntary loss (USB/link/deadman) still just stops streaming — see the handler.
pub fn release_channels(us: &[u16]) -> [u16; MAV_OVERRIDE_CHANNELS] {
    let mut ch = [0u16; MAV_OVERRIDE_CHANNELS];
    for (i, slot) in ch.iter_mut().enumerate() {
        let was_controlled = us.get(i).copied().unwrap_or(0) != 0;
        *slot = if was_controlled { release_sentinel(i) } else { ignore_sentinel(i) };
    }
    ch
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
