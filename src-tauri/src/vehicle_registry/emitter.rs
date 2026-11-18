// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! `VehicleEmitter` — an `AppHandle` stand-in that stamps every emitted payload with the vehicle it
//! belongs to.
//!
//! The protocol handlers (MAVLink handler, MSP scheduler, passive decoders) take this in place of the
//! raw `AppHandle`. Because it exposes an inherent `emit` with the same shape as `tauri::Emitter::emit`
//! and derefs to `AppHandle`, the existing `app_handle.emit("telemetry-…", &data)` calls compile
//! unchanged and now carry `vehicleId` / `linkId`, while anything needing a real `&AppHandle`
//! (debug trackers, link stats) still works through deref coercion.
//!
//! Payload rules (see design §2.3):
//! - JSON object  → `vehicleId` and `linkId` fields are inserted (existing keys are never overwritten)
//! - `()` / null  → `{ "vehicleId", "linkId" }`
//! - anything else (array, scalar) → `{ "vehicleId", "linkId", "value": <payload> }`

use std::ops::Deref;

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter};

use super::{LinkId, VehicleId};

pub const FIELD_VEHICLE: &str = "vehicleId";
pub const FIELD_LINK: &str = "linkId";
pub const FIELD_VALUE: &str = "value";

#[derive(Clone)]
pub struct VehicleEmitter {
    app: AppHandle,
    id: VehicleId,
    key: String,
}

impl VehicleEmitter {
    pub fn new(app: AppHandle, id: VehicleId) -> Self {
        let key = id.to_key();
        Self { app, id, key }
    }

    /// Same link, different system id — for vehicles discovered on a shared MAVLink link.
    pub fn for_sysid(&self, sysid: u8) -> Self {
        Self::new(self.app.clone(), VehicleId::new(self.id.link, sysid))
    }

    /// The wire key (`"L1:S1"`).
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn link_id(&self) -> LinkId {
        self.id.link
    }

    /// The app handle, for the few handler paths that touch managed state directly.
    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    /// Emit `event` app-wide with the payload stamped with this vehicle's ids.
    pub fn emit<S: Serialize>(&self, event: &str, payload: S) -> tauri::Result<()> {
        let value = serde_json::to_value(payload)?;
        self.app.emit(event, stamp(value, &self.key, self.id.link))
    }
}

impl Deref for VehicleEmitter {
    type Target = AppHandle;
    fn deref(&self) -> &AppHandle {
        &self.app
    }
}

/// Apply the payload rules. Pure so it can be unit-tested without a Tauri runtime.
fn stamp(value: Value, key: &str, link: LinkId) -> Value {
    let mut map = match value {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert(FIELD_VALUE.into(), other);
            m
        }
    };
    map.entry(FIELD_VEHICLE).or_insert_with(|| Value::String(key.to_string()));
    map.entry(FIELD_LINK).or_insert_with(|| Value::from(link));
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn object_gets_ids_inserted() {
        let out = stamp(json!({"roll": 1.5, "pitch": -2.0}), "L1:S1", 1);
        assert_eq!(out, json!({"roll": 1.5, "pitch": -2.0, "vehicleId": "L1:S1", "linkId": 1}));
    }

    #[test]
    fn existing_ids_are_not_overwritten() {
        let out = stamp(json!({"vehicleId": "L9:S9", "x": 1}), "L1:S1", 1);
        assert_eq!(out["vehicleId"], "L9:S9");
        assert_eq!(out["linkId"], 1);
    }

    #[test]
    fn unit_payload_becomes_id_object() {
        let out = stamp(Value::Null, "L2:S0", 2);
        assert_eq!(out, json!({"vehicleId": "L2:S0", "linkId": 2}));
    }

    #[test]
    fn array_payload_is_wrapped_in_value() {
        let out = stamp(json!([1, 2, 3]), "L1:S7", 1);
        assert_eq!(out, json!({"value": [1, 2, 3], "vehicleId": "L1:S7", "linkId": 1}));
    }

    #[test]
    fn scalar_payload_is_wrapped_in_value() {
        let out = stamp(json!(true), "L1:S1", 1);
        assert_eq!(out, json!({"value": true, "vehicleId": "L1:S1", "linkId": 1}));
    }
}
