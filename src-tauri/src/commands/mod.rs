// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Tauri Commands Module
// All frontend-callable commands are defined here, organized by feature area.
// Each submodule exposes Tauri commands that the Svelte frontend can invoke.

pub mod aero;
pub mod connection;
pub mod flightlog;
pub mod info;
pub mod mission;
pub mod radar;
pub mod terrain;
