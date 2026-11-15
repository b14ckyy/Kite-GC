// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Multi-vehicle ("fleet") feature gate — a launch flag (`--fleet` / `KITE_FLEET`), not a setting; on in
//! debug builds, and `--debug` must NOT turn it on. Separately, group-flight RECORDING (temporary): only
//! with `KITE_FLEET_RECORDING`, else just the first link's primary records, so `--fleet` cannot wreck a DB.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

static ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions) || option_env!("KITE_FLEET").is_some());

static GROUP_RECORDING: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(false)));

/// Switch the fleet features on or off (startup flag; the Debug Monitor's runtime toggle in a debug
/// build). Group recording follows: off with the features, back on only with the developer opt-in.
pub fn set(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    GROUP_RECORDING.store(group_recording_wanted(on, recording_opt_in_present()), Ordering::Relaxed);
}

/// True when the multi-vehicle features are active. A single relaxed atomic load.
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// True when the launch asks for the fleet features: `--fleet` on the command line, or the runtime
/// environment variable `KITE_FLEET` set.
pub fn flag_present() -> bool {
    std::env::args().any(|a| a == "--fleet") || std::env::var_os("KITE_FLEET").is_some()
}

/// The gate decision for a multi-vehicle action — a command or selection aimed at a vehicle, or a
/// connect: allowed when it is single-vehicle use (the link's primary vehicle, the first link) or the
/// fleet features are on. `allows(single, enabled())`.
pub fn allows(single_vehicle_use: bool, fleet_on: bool) -> bool {
    single_vehicle_use || fleet_on
}

/// True when more than the first link's primary vehicle records flights and group flights form — the
/// fleet features plus the `KITE_FLEET_RECORDING` developer opt-in.
#[inline]
pub fn group_recording() -> bool {
    GROUP_RECORDING.load(Ordering::Relaxed)
}

/// The group-recording flag itself (the group coordinator reads it on every arm).
pub fn group_recording_handle() -> Arc<AtomicBool> {
    GROUP_RECORDING.clone()
}

/// Settle the group-recording switch from the fleet flag and the environment (startup, after `set`).
pub fn apply_group_recording() {
    GROUP_RECORDING.store(group_recording_wanted(enabled(), recording_opt_in_present()), Ordering::Relaxed);
}

/// The developer opt-in: the `KITE_FLEET_RECORDING` environment variable, any value.
fn recording_opt_in_present() -> bool {
    std::env::var_os("KITE_FLEET_RECORDING").is_some()
}

/// Group recording needs both: the fleet features and the developer opt-in.
fn group_recording_wanted(fleet: bool, opt_in: bool) -> bool {
    fleet && opt_in
}

#[cfg(test)]
mod tests {
    use super::{allows, group_recording_wanted};

    /// Tests run with `debug_assertions` (fleet on by default), so the OFF branch is exercised here.
    #[test]
    fn gate_allows_only_single_vehicle_use_with_fleet_off() {
        assert!(allows(true, false), "the primary vehicle / the first link always");
        assert!(!allows(false, false), "a non-primary vehicle / a second link: refused with fleet off");
        assert!(allows(false, true));
        assert!(allows(true, true));
    }

    #[test]
    fn group_recording_needs_fleet_and_the_opt_in() {
        assert!(!group_recording_wanted(false, false));
        assert!(!group_recording_wanted(true, false), "--fleet alone records the first vehicle only");
        assert!(!group_recording_wanted(false, true), "the opt-in without fleet features does nothing");
        assert!(group_recording_wanted(true, true));
    }
}
