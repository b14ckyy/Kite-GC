// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI bridge to the Kotlin `NetInfo` lookup: is the device's active network on Wi-Fi?
//! Drives the pause of continuous GCS location updates while an RTSP stream runs — fused
//! location's periodic Wi-Fi scans take the radio off-channel and burst-drop RTP (see
//! NetInfo.kt for the measurement).

use super::jvm;

pub fn active_net_is_wifi() -> Result<bool, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, "com.kitegc.app.NetInfo")?;
    let r = env
        .call_static_method(&class, "activeNetIsWifi", "()Z", &[])
        .and_then(|v| v.z());
    jvm::check(&mut env, r, "NetInfo.activeNetIsWifi")
}
