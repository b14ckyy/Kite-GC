// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI shim in front of `MiniView.kt` — whether leaving the app enters picture-in-picture. Same boundary
//! contract as the other bridges (a primitive across, the Kotlin side owns the activity).

use jni::objects::JValue;

use super::jvm;

const BRIDGE: &str = "com.kitegc.app.MiniView";

pub fn set_enabled(on: bool) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env
        .call_static_method(&class, "setEnabled", "(Z)V", &[JValue::Bool(on as u8)])
        .map(|_| ());
    jvm::check(&mut env, r, "MiniView.setEnabled")
}
