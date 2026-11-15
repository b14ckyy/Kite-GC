// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Native INAV Blackbox decoder (Dev-Docs active/NATIVE_BLACKBOX_DECODER.md). The core is a verbatim
//! copy of Kite Analyzer's `src-tauri/src/logs/inav/` (source revision and deviations under each
//! file's SPDX header); `telemetry.rs` is Kite's own frame sink for the Blackbox importer.

mod encodings;
// The copied core stays verbatim, so the parts only Kite Analyzer needs are silenced here instead
// of deleted (frames.rs `FrameKind::ALL` / `marker`, schema.rs's table-layer items).
#[allow(dead_code)]
mod frames;
mod header;
mod predictors;
#[allow(dead_code)]
mod schema;
mod stream;
mod telemetry;

pub(super) use telemetry::{decode_rows, select_log, Compat};

/// Kite Analyzer's `logs::table::DType` (the column types `schema.rs` names), minus its serde
/// derive: Kite has no table layer, only the schema's own tests compare these.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DType {
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

#[cfg(test)]
mod compare_test;
#[cfg(test)]
mod coverage_test;
#[cfg(test)]
mod import_test;
