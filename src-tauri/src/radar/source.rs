// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — source abstraction
//
// Each source runs independently (its own thread/loop) and pushes batches of vehicles into the
// manager's aggregator channel. The aggregator does the per-system merge + TTL + event emission.
// See docs/active/RADAR_TRACKING_CORE.md §5.

use std::sync::mpsc;

use super::vehicle::{TrackedVehicle, VehicleSource, VehicleSystem};

/// A batch of vehicles reported by one source at one instant.
pub struct SourceUpdate {
    /// Originating feed — used for per-source status tracking from Phase 1 on.
    #[allow(dead_code)]
    pub source: VehicleSource,
    pub vehicles: Vec<TrackedVehicle>,
}

/// Returned by a started source; dropping/calling `stop` tears the source down.
pub struct SourceHandle {
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl SourceHandle {
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self { stop: Some(Box::new(stop)) }
    }

    /// Explicit teardown (also runs on drop). Kept for callers that stop a source eagerly.
    #[allow(dead_code)]
    pub fn stop(mut self) {
        if let Some(f) = self.stop.take() {
            f();
        }
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        if let Some(f) = self.stop.take() {
            f();
        }
    }
}

/// A radar data source. `start` spawns the worker; it pushes `SourceUpdate`s into `tx`.
/// `system`/`source` are used by the manager for status/registry from Phase 1 on.
#[allow(dead_code)]
pub trait RadarSource: Send {
    fn system(&self) -> VehicleSystem;
    fn source(&self) -> VehicleSource;
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle;
}
