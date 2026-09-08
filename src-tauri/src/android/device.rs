// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI bridge to the Kotlin `DeviceProfile`: which known ground-station hardware this is, and the
//! serial ports built into it.
//!
//! The identification rule itself lives in `DeviceProfile.kt` (one place, see its docs); this side
//! only asks, caches the answer and logs it. Today the one profile is the RadioMaster AX12, whose
//! internal ELRS UART is a raw `/dev/ttyS1` — the reason the Android serial transport grew a tty
//! backend next to the USB-host one.

use std::sync::OnceLock;

use super::jvm;
use crate::transport::PortInfo;

const BRIDGE: &str = "com.kitegc.app.DeviceProfile";

/// Cached on the first successful query only — a JNI failure (the VM not up yet) must not pin an
/// empty answer for the life of the process.
static PROFILE: OnceLock<String> = OnceLock::new();

/// The matched profile name, "" on a device Kite does not recognise.
///
/// The first successful call logs the build fields the decision was made from — at the default log
/// level on purpose: the AX12 support is a blind implementation, and the one thing a tester's log
/// must answer is whether the device was recognised at all.
pub fn profile() -> String {
    if let Some(p) = PROFILE.get() {
        return p.clone();
    }
    match query_profile() {
        Ok((profile, summary)) => {
            log::warn!("[android] device profile={profile:?} {summary}");
            PROFILE.get_or_init(|| profile).clone()
        }
        Err(e) => {
            log::debug!("[android] device profile not available yet: {e}");
            String::new()
        }
    }
}

fn query_profile() -> Result<(String, String), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let profile = string_call(&mut env, &class, "profile")?;
    let summary = string_call(&mut env, &class, "buildSummary")?;
    Ok((profile, summary))
}

/// Serial ports built into the device (the AX12's `/dev/ttyS1`), in the shape the port picker
/// already understands — `port_type` is `internal-uart`. Empty on an unknown device and on any
/// failure: enumeration runs on a timer and must never surface an error.
pub fn internal_serial_ports() -> Vec<PortInfo> {
    let json = match internal_ports_json() {
        Ok(json) => json,
        Err(e) => {
            log::debug!("[android] internal serial ports not available: {e}");
            return Vec::new();
        }
    };

    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        label: String,
        #[serde(rename = "type")]
        port_type: String,
    }

    match serde_json::from_str::<Vec<Entry>>(&json) {
        Ok(entries) => entries
            .into_iter()
            .map(|e| PortInfo {
                path: e.path,
                label: e.label,
                port_type: e.port_type,
            })
            .collect(),
        Err(e) => {
            log::warn!("[android] could not parse the internal port list ({e}): {json}");
            Vec::new()
        }
    }
}

fn internal_ports_json() -> Result<String, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    string_call(&mut env, &class, "internalSerialPorts")
}

/// Call a static `() -> String` method of the bridge class.
fn string_call(
    env: &mut jni::JNIEnv<'_>,
    class: &jni::objects::JClass<'_>,
    method: &str,
) -> Result<String, String> {
    let r = env
        .call_static_method(class, method, "()Ljava/lang/String;", &[])
        .and_then(|v| v.l());
    let value = jvm::check(env, r, &format!("DeviceProfile.{method}"))?;
    let r = env.get_string(&value.into()).map(String::from);
    jvm::check(env, r, &format!("reading DeviceProfile.{method}"))
}
