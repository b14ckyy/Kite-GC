// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Native INAV Blackbox decoder (Dev-Docs active/NATIVE_BLACKBOX_DECODER.md). The core — header,
//! frame parser with the encodings and predictors INAV writes, validation/resync as blackbox-tools'
//! `parser.c`, and the per-version schema — is a copy of Kite Analyzer's `src-tauri/src/logs/inav/`
//! (each copied file names its source revision and every intentional deviation under its SPDX
//! header; both projects keep their own copy on purpose). `telemetry.rs` is Kite's own frame sink:
//! one log → the rows the Blackbox importer stores.

mod encodings;
// The copied core stays verbatim, so the parts Kite Analyzer needs and Kite does not are silenced
// here instead of deleted: `FrameKind::ALL` / `marker` (frames.rs) and the schema's table-layer
// items — `PayloadKind::dtype`, `EventAttr::column` / `unit`, `InavSchema::fields` /
// `dtype_overrides`, `field_unit` / `field_dtype` (schema.rs).
#[allow(dead_code)]
mod frames;
mod header;
mod predictors;
#[allow(dead_code)]
mod schema;
mod stream;
mod telemetry;

pub(super) use telemetry::{decode_rows, Compat};

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
