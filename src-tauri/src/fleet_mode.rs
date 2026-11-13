// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Multi-vehicle ("fleet") feature gate — a launch flag, modelled on `debug_mode` — plus the separate
//! switch for group-flight RECORDING.
//!
//! **Fleet features.** Off, Kite is the single-vehicle GCS: a second system id on a link never becomes a
//! vehicle (no discovery, no fleet markers or STATUSTEXT tags), a second link is refused, commands only
//! reach a link's primary vehicle, and the frontend hides the link manager, Fleet tab and group commands
//! (Dev-Docs active/GROUP_FLIGHTS.md §5e, Marc 2026-10-02: no setting, a launch flag).
//!
//! Default: ON in a debug build (`tauri dev` needs no flag), OFF in a release build unless the launch
//! asks for it — the `--fleet` CLI argument or the `KITE_FLEET` environment variable (any value, so the
//! portable build can use it too). `--debug` on a release build does NOT turn it on: a tester collecting
//! diagnostics must not get a half-finished feature. Mobile has no command line: compiling with
//! `KITE_FLEET` set bakes it in (`KITE_FLEET=1 tauri android build`), like `KITE_DEBUG_UI` for debug
//! mode — release or not. In a debug build the Debug Monitor can flip it at runtime
//! (`set_fleet_enabled`); a release build refuses that.
//!
//! **Group recording** (temporary — until the group-flight end screen exists, GROUP_FLIGHTS.md steps
//! 7–8): with the fleet features on, a group flight's member recordings could not be stored yet. So by
//! default only the FIRST link's primary vehicle records, as an ordinary single flight: no recorder for
//! a vehicle discovered beside it or for the primary of a further link, and the group coordinator never
//! forms a group (no `.kgrp`, no member files, no `group_id`). Nobody can wreck a flight database by
//! trying `--fleet`. A developer opts in with the `KITE_FLEET_RECORDING` environment variable (any value;
//! no CLI flag, no setting, no UI toggle) — and only together with the fleet features.
//!
//! Both flags are shared `AtomicBool`s; the group coordinator gets the recording one as an `Arc`
//! (`group_recording_handle`) so its tests can give each coordinator a flag of its own.

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
