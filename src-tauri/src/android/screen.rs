// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI shim in front of `ScreenLock.kt` — the display-on flag while a link is active. Same boundary
//! contract as the other bridges (a primitive across, the Kotlin side owns the window).

use jni::objects::JValue;

use super::jvm;

const BRIDGE: &str = "com.kitegc.app.ScreenLock";

/// Set or clear the keep-screen-on window flag. Fire-and-forget; the flag change hops to the main
/// thread on the Kotlin side.
pub fn keep_on(on: bool) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env
        .call_static_method(&class, "keepOn", "(Z)V", &[JValue::Bool(on as u8)])
        .map(|_| ());
    jvm::check(&mut env, r, "ScreenLock.keepOn")
}
