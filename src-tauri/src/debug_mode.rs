// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Runtime debug-mode flag.
//!
//! Historically the in-app Debug Monitor + verbose diagnostics were gated at COMPILE time
//! (`#[cfg(debug_assertions)]` / `import.meta.env.DEV`) and stripped from release builds entirely
//! (ADR-008). To let a shipped RELEASE build expose those on demand, this flag is set once at startup
//! from the `--debug` CLI argument; the debug-stat trackers check it at runtime instead.
//!
//! Default: `true` in debug builds (so `tauri dev` behaves exactly as before, no flag needed), `false`
//! in release (until `--debug` flips it on). The value never changes after startup.
//!
//! Mobile has no command line, so a release APK could never reach `--debug`. For local test
//! builds pushed over ADB, compiling with the environment variable `KITE_DEBUG_UI` set (any
//! value) bakes debug mode in: `KITE_DEBUG_UI=1 tauri android build`. CI/release builds don't
//! set it, so tester/sideload builds keep the release default.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool =
    AtomicBool::new(cfg!(debug_assertions) || option_env!("KITE_DEBUG_UI").is_some());

/// Enable debug mode (called once at startup when `--debug` is present). Idempotent.
pub fn set(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// True when the Debug Monitor + verbose diagnostics should be active (debug build, or a release
/// started with `--debug`). A single relaxed atomic load — cheap enough for hot-path tracker calls.
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// True when `--debug` was passed on the command line. Used at startup to decide both the debug flag
/// and the initial log level.
pub fn debug_flag_present() -> bool {
    std::env::args().any(|a| a == "--debug")
}
