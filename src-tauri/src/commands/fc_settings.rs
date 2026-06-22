// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared INAV setting read/write by name (MSP2_COMMON_SETTING / MSP2_COMMON_SET_SETTING). The FC
// returns/accepts each setting in its native byte width; reads interpret whatever length comes back,
// writes preserve that width. Used by commands/rc.rs and commands/safehome.rs. Runtime-only — callers
// persist explicitly via MSP_EEPROM_WRITE when needed.

use crate::msp::{MSP2_COMMON_SETTING, MSP2_COMMON_SET_SETTING};
use crate::scheduler::SchedulerHandle;

/// Read a setting's raw value bytes by name (null-terminated name → value bytes).
pub(crate) fn read_setting(handle: &SchedulerHandle, name: &str) -> Result<Vec<u8>, String> {
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    handle.msp_request(MSP2_COMMON_SETTING, &payload)
}

/// Write a setting's raw value bytes by name (runtime only).
pub(crate) fn set_setting(handle: &SchedulerHandle, name: &str, value: &[u8]) -> Result<(), String> {
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    payload.extend_from_slice(value);
    handle.msp_request(MSP2_COMMON_SET_SETTING, &payload).map(|_| ())
}

/// Read a setting as an unsigned integer, interpreting the FC's native width (1/2/4 bytes, LE).
/// `None` if the setting is absent (older/limited firmware) or the response is empty.
pub(crate) fn read_uint_setting(handle: &SchedulerHandle, name: &str) -> Option<u64> {
    let b = read_setting(handle, name).ok()?;
    match b.len() {
        1 => Some(b[0] as u64),
        2 => Some(u16::from_le_bytes([b[0], b[1]]) as u64),
        l if l >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64),
        _ => None,
    }
}

/// Write an unsigned integer to a setting at the FC's native width: read it once to learn the width,
/// then write the value truncated to that width. Errors if the setting can't be read (unknown name).
pub(crate) fn set_uint_setting(handle: &SchedulerHandle, name: &str, value: u64) -> Result<(), String> {
    let width = read_setting(handle, name)?.len().clamp(1, 8);
    let bytes = value.to_le_bytes();
    set_setting(handle, name, &bytes[..width])
}
