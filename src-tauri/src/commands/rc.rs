// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-control FC-side commands — read the INAV config that decides whether/which GCS-injected RC
// channels actually take effect (docs/archive/MSP_RC_CONTROL.md §3/§4). One-shot MSP reads through the
// scheduler. The local HID side lives in commands/hid.rs; the byte encoders in msp/rc_encode.rs.

use serde::Serialize;
use tauri::State;

use crate::msp::rc_encode::encode_raw_rc;
use crate::msp::{MSP2_COMMON_SETTING, MSP2_COMMON_SET_SETTING, MSP_MODE_RANGES, MSP_RC};
use crate::scheduler::SchedulerHandle;
use crate::state::{ActiveProtocol, AppState};

/// One configured mode-activation range (a box assigned to an RC channel window). Only non-empty
/// ranges are returned. Used for mode labels + safety locks (docs/archive/MSP_RC_CONTROL.md §-safety).
#[derive(Serialize)]
pub struct ModeRange {
    /// INAV permanent box ID (e.g. ARM=0, NAV RTH=10, FAILSAFE=27).
    pub permanent_id: u8,
    /// 1-based RC channel the mode is assigned to (AUX1 = CH5).
    pub channel: u8,
    /// Activation µs window (900..2100).
    pub range_min: u16,
    pub range_max: u16,
}

/// INAV settings relevant to GCS RC injection.
#[derive(Serialize)]
pub struct RcFcConfig {
    /// `receiver_type`: 0 = NONE, 1 = SERIAL, 2 = MSP.
    pub receiver_type: u8,
    /// `msp_override_channels` bitmask (CH1 = bit 0). `None` if the FC firmware lacks the setting
    /// (compiled without `USE_MSP_RC_OVERRIDE`).
    pub msp_override_channels: Option<u32>,
    /// Configured mode ranges (which channel triggers which box).
    pub mode_ranges: Vec<ModeRange>,
}

/// Parse an MSP_MODE_RANGES response: N × (permanentId, auxChannelIndex, startStep, endStep). Each
/// step = 25 µs from 900. Empty ranges (start == end) are unused slots and skipped. Note: unused slots
/// also carry permanentId 0 (same as ARM) — the empty-range filter disambiguates them.
fn parse_mode_ranges(payload: &[u8]) -> Vec<ModeRange> {
    payload
        .chunks_exact(4)
        .filter(|c| c[2] != c[3]) // non-empty range
        .map(|c| ModeRange {
            permanent_id: c[0],
            channel: c[1] + 5, // AUX1 (index 0) = CH5
            range_min: 900 + c[2] as u16 * 25,
            range_max: 900 + c[3] as u16 * 25,
        })
        .collect()
}

/// Read a setting by name via MSP2_COMMON_SETTING (null-terminated name → raw value bytes).
fn read_setting(handle: &SchedulerHandle, name: &str) -> Result<Vec<u8>, String> {
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    handle.msp_request(MSP2_COMMON_SETTING, &payload)
}

#[tauri::command(async)]
pub fn rc_read_fc_config(state: State<'_, AppState>) -> Result<RcFcConfig, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(_) => return Err("FC is not running MSP (INAV)".into()),
        None => return Err("Not connected".into()),
    };

    let rx = read_setting(handle, "receiver_type")?;
    let receiver_type = *rx.first().ok_or("empty receiver_type response")?;

    // Present only when the firmware compiles USE_MSP_RC_OVERRIDE; otherwise the read errors → None.
    let msp_override_channels = match read_setting(handle, "msp_override_channels") {
        Ok(b) if b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    };

    let mode_ranges = match handle.msp_request(MSP_MODE_RANGES, &[]) {
        Ok(b) => parse_mode_ranges(&b),
        Err(e) => {
            eprintln!("[RC] MSP_MODE_RANGES read failed: {e}");
            Vec::new()
        }
    };

    eprintln!(
        "[RC] FC config: receiver_type={receiver_type} msp_override_channels={msp_override_channels:?} mode_ranges={}",
        mode_ranges.len()
    );
    Ok(RcFcConfig { receiver_type, msp_override_channels, mode_ranges })
}

/// Set `msp_override_channels` to the given bitmask **at runtime only** (MSP2_COMMON_SET_SETTING; no
/// EEPROM save) so the configured RAW_RC channels can actually be overridden. Reverts on FC reboot by
/// design — we never persist FC settings. CH1 = bit 0.
#[tauri::command(async)]
pub fn rc_set_override_bitmask(mask: u32, state: State<'_, AppState>) -> Result<(), String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(_) => return Err("FC is not running MSP (INAV)".into()),
        None => return Err("Not connected".into()),
    };

    let mut payload = b"msp_override_channels".to_vec();
    payload.push(0); // null-terminated name
    payload.extend_from_slice(&mask.to_le_bytes()); // u32 LE value
    handle.msp_request(MSP2_COMMON_SET_SETTING, &payload)?;
    eprintln!("[RC] set msp_override_channels = 0x{mask:x} (runtime only)");
    Ok(())
}

/// Read the FC's current RC channel values (MSP_RC) as µs per channel (CH1..). The handover mirror
/// polls this (~0.5 Hz) so our internal state can track what the FC currently has — no jump on engage.
#[tauri::command(async)]
pub fn rc_read_channels(state: State<'_, AppState>) -> Result<Vec<u16>, String> {
    let proto = state.protocol.lock().map_err(|e| e.to_string())?;
    let handle = match proto.as_ref() {
        Some(ActiveProtocol::Msp(h)) => h,
        Some(_) => return Err("FC is not running MSP (INAV)".into()),
        None => return Err("Not connected".into()),
    };
    let raw = handle.msp_request(MSP_RC, &[])?;
    Ok(raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

// ── RC injection stream (docs/archive/MSP_RC_CONTROL.md §10 Phase 4c) ──────────────────────────────────
// The frontend (rcEngine) drives these; the MSP scheduler thread does the actual sending from the
// shared RcTxState. `rc_stream_update` doubles as the deadman heartbeat (it bumps `last_update`).

/// Push the latest RAW-RC frame (µs per channel, CH1..rawMax) to the injection stream + refresh the
/// deadman. Encoded here so the scheduler hot path only writes bytes. No-op effect until enabled.
#[tauri::command]
pub fn rc_stream_update(channels: Vec<u16>, state: State<'_, AppState>) -> Result<(), String> {
    let mut rc = state.rc_tx.lock().map_err(|e| e.to_string())?;
    rc.raw = encode_raw_rc(&channels);
    rc.last_update = std::time::Instant::now();
    Ok(())
}

/// Push AUX-RC channel changes (latched overlay). `channels` are 0-based indices (≥16 / CH17+ by Kite
/// policy), `values` the matching target µs. They're accumulated into the pending map; the scheduler
/// packs the minimal 16-bit run and re-sends at 5 Hz until the FC ACKs. Call only with channels that
/// actually changed — sending one channel costs one channel, not the whole AUX block.
#[tauri::command]
pub fn rc_stream_set_aux(channels: Vec<u8>, values: Vec<u16>, state: State<'_, AppState>) -> Result<(), String> {
    let mut rc = state.rc_tx.lock().map_err(|e| e.to_string())?;
    for (c, v) in channels.iter().zip(values.iter()) {
        rc.aux_pending.insert(*c, *v);
    }
    Ok(())
}

/// Enable/disable the RC injection stream (engage/disengage). Disabling stops all sending immediately;
/// enabling refreshes the deadman so a first frame can flow right away.
#[tauri::command]
pub fn rc_stream_enable(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    let mut rc = state.rc_tx.lock().map_err(|e| e.to_string())?;
    rc.enabled = enabled;
    if enabled {
        rc.last_update = std::time::Instant::now();
    } else {
        rc.aux_pending.clear();
    }
    Ok(())
}

/// Set the RAW_RC send rate (Hz). Clamped to 5..=50; the UI offers 10/15/20/25 (default 10 for slow
/// links). AUX stays fixed at 5 Hz regardless.
#[tauri::command]
pub fn rc_stream_set_rate(hz: u16, state: State<'_, AppState>) -> Result<(), String> {
    let hz = hz.clamp(5, 50);
    let mut rc = state.rc_tx.lock().map_err(|e| e.to_string())?;
    rc.raw_interval = std::time::Duration::from_millis((1000 / hz as u64).max(1));
    Ok(())
}
