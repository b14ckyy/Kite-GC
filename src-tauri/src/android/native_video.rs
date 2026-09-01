// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI bridge to the Kotlin `NativeVideo` view host — the Android hole-punch layer
//! (MOBILE_RTSP.md P2.2): a SurfaceView below the transparent WebView that the MediaCodec
//! sink (video/android_sink.rs) decodes onto. Callers are Rust worker threads; the Kotlin
//! side hops every view mutation to the UI thread itself, so these calls are cheap.

use jni::objects::JValue;

use super::jvm;

const CLASS: &str = "com.kitegc.app.NativeVideo";

/// Create (or re-layout) the layer at a window rect (physical pixels — the same convention
/// as the Windows sink). The sink starts this at 1×1; the frontend pushes the real rect.
pub fn show(x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env.call_static_method(
        &class,
        "show",
        "(IIII)V",
        &[JValue::Int(x), JValue::Int(y), JValue::Int(w), JValue::Int(h)],
    );
    jvm::check(&mut env, r, "NativeVideo.show").map(|_| ())
}

/// Move/resize the layer: full box `x/y/w/h` (the Kotlin side aspect-fits the video into
/// it) plus the visible part `cx/cy/cw/ch` it clips the video to (scrolled containers).
#[allow(clippy::too_many_arguments)]
pub fn set_rect(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    cx: i32,
    cy: i32,
    cw: i32,
    ch: i32,
) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env.call_static_method(
        &class,
        "setRect",
        "(IIIIIIII)V",
        &[
            JValue::Int(x),
            JValue::Int(y),
            JValue::Int(w),
            JValue::Int(h),
            JValue::Int(cx),
            JValue::Int(cy),
            JValue::Int(cw),
            JValue::Int(ch),
        ],
    );
    jvm::check(&mut env, r, "NativeVideo.setRect").map(|_| ())
}

/// Tell the view host the decoded video's pixel size (drives the aspect-fit).
pub fn set_video_size(w: i32, h: i32) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env.call_static_method(
        &class,
        "setVideoSize",
        "(II)V",
        &[JValue::Int(w), JValue::Int(h)],
    );
    jvm::check(&mut env, r, "NativeVideo.setVideoSize").map(|_| ())
}

/// Show/hide the layer (hidden = shifted off-screen, so the surface survives).
pub fn set_visible(visible: bool) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env.call_static_method(
        &class,
        "setVisible",
        "(Z)V",
        &[JValue::Bool(visible as u8)],
    );
    jvm::check(&mut env, r, "NativeVideo.setVisible").map(|_| ())
}

/// Remove the layer and restore the WebView's opaque background (sink stopped).
pub fn destroy() -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env.call_static_method(&class, "destroy", "()V", &[]);
    jvm::check(&mut env, r, "NativeVideo.destroy").map(|_| ())
}

/// The surface generation counter — bumped by Kotlin on every surfaceCreated/surfaceDestroyed.
/// The decode loop keys its codec lifetime to this (NOT to activity pause — PiP keeps a
/// paused activity visible and playing).
pub fn generation() -> Result<i32, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env
        .call_static_method(&class, "generation", "()I", &[])
        .and_then(|v| v.i());
    jvm::check(&mut env, r, "NativeVideo.generation")
}

/// Acquire the layer's live output surface as an `ANativeWindow` for MediaCodec, or `None`
/// while no surface exists (view not created yet, or destroyed by the OS).
pub fn acquire_native_window() -> Result<Option<ndk::native_window::NativeWindow>, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, CLASS)?;
    let r = env
        .call_static_method(&class, "surface", "()Landroid/view/Surface;", &[])
        .and_then(|v| v.l());
    let surface = jvm::check(&mut env, r, "NativeVideo.surface")?;
    if surface.is_null() {
        return Ok(None);
    }
    // SAFETY: `env` is the valid JNIEnv of this attached thread, and `surface` is a live
    // local reference to an android.view.Surface. ANativeWindow_fromSurface acquires its own
    // reference; the returned NativeWindow releases it on drop.
    let window = unsafe {
        ndk::native_window::NativeWindow::from_surface(env.get_raw().cast(), surface.as_raw())
    };
    Ok(window)
}
