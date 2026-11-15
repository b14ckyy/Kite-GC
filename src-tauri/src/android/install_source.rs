// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI bridge to the Kotlin `InstallSource` lookup: was this copy installed by the Google Play Store?

use super::jvm;

pub fn is_play_store() -> Result<bool, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, "com.kitegc.app.InstallSource")?;
    let r = env
        .call_static_method(&class, "isPlayStore", "()Z", &[])
        .and_then(|v| v.z());
    jvm::check(&mut env, r, "InstallSource.isPlayStore")
}
