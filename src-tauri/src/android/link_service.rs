// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI shim in front of `LinkService.kt` — the foreground service that keeps the process alive
//! while a link is up, and its notification (Dev-Docs active/BACKGROUND_TELEMETRY.md). Same
//! boundary contract as the other bridges: primitives across, the Kotlin side owns the Android
//! objects and hops to the main thread itself.
//!
//! `start` also spawns the 1 Hz ticker that re-renders the notification from `link_status`
//! whenever its text changed; `stop` ends the ticker first, then the service — so a notification
//! can never be refreshed after the service was told to go.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use jni::objects::JValue;

use super::jvm;

const BRIDGE: &str = "com.kitegc.app.LinkService";

/// Notification refresh cadence. The values change at telemetry rate; the notification does not
/// need to — and every update is a Binder call.
const TICK: Duration = Duration::from_secs(1);

static TICKER: Mutex<Option<(Arc<AtomicBool>, JoinHandle<()>)>> = Mutex::new(None);

fn call(method: &str, sig: &str, args: &[JValue]) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.call_static_method(&class, method, sig, args).map(|_| ());
    jvm::check(&mut env, r, &format!("LinkService.{method}"))
}

fn call_texts(method: &str, title: &str, text: &str) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env.new_string(title);
    let jtitle = jvm::check(&mut env, r, "building the notification title")?;
    let r = env.new_string(text);
    let jtext = jvm::check(&mut env, r, "building the notification text")?;
    let r = env
        .call_static_method(
            &class,
            method,
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&jtitle), JValue::Object(&jtext)],
        )
        .map(|_| ());
    jvm::check(&mut env, r, &format!("LinkService.{method}"))
}

/// Start the foreground service with the current notification and begin refreshing it.
pub fn start() -> Result<(), String> {
    let (title, text) = crate::link_status::notification();
    call_texts("start", &title, &text)?;

    let mut slot = TICKER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((stop, handle)) = slot.take() {
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::Builder::new()
        .name("link-notification".into())
        .spawn(move || {
            let mut last = (title, text);
            while !flag.load(Ordering::SeqCst) {
                std::thread::sleep(TICK);
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let now = crate::link_status::notification();
                if now != last {
                    if let Err(e) = call_texts("update", &now.0, &now.1) {
                        log::debug!("[link-service] notification update failed: {e}");
                    }
                    last = now;
                }
            }
        })
        .map_err(|e| format!("spawning the notification ticker: {e}"))?;
    *slot = Some((stop, handle));
    Ok(())
}

/// End the refresh ticker, then the service (its notification goes with it).
pub fn stop() -> Result<(), String> {
    let taken = TICKER.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((stop, handle)) = taken {
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
    }
    call("stop", "()V", &[])
}
