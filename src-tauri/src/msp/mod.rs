// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MSP Protocol Module
// Handles MSP (MultiWii Serial Protocol) encoding, decoding, and message definitions.

pub mod codec;
pub mod features;
pub mod parser;
pub mod transport;
pub mod types;

pub use codec::MspCodec;
pub use features::{FeatureSet, InavVersion};
pub use parser::MspParser;
pub use transport::MspTransport;
pub use types::*;
