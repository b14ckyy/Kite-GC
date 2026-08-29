// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Minimal JNI plumbing: reach the app's Kotlin classes from a Rust thread.
//!
//! `tao` publishes the `JavaVM` and the Activity through `ndk_context` when the WebView starts, which
//! is the only handle Rust gets on the Java side. Two details make this less obvious than it looks:
//!
//! 1. **The class loader.** `JNIEnv::find_class` on a thread Rust created resolves against the
//!    *system* class loader, which knows the platform classes and nothing of the app — so
//!    `find_class("com/kitegc/app/UsbSerial")` fails with `NoClassDefFoundError` even though the class
//!    is right there in the APK. The fix is to go through the Activity's own loader, which is what
//!    [`app_class`] does, caching it for the life of the process.
//!
//! 2. **Attachment.** Every thread that touches JNI must be attached to the VM. [`env`] attaches as a
//!    daemon, which is a no-op on an already-attached thread (so it is cheap to call per operation)
//!    and detaches automatically when the thread exits — a plain attach would keep a finished Rust
//!    worker thread alive from the VM's point of view.

use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};

static VM: OnceLock<JavaVM> = OnceLock::new();
static CLASS_LOADER: OnceLock<GlobalRef> = OnceLock::new();

/// The process-wide `JavaVM`, from the context `tao` published on startup.
fn java_vm() -> Result<&'static JavaVM, String> {
    if let Some(vm) = VM.get() {
        return Ok(vm);
    }
    let ptr = ndk_context::android_context().vm();
    if ptr.is_null() {
        return Err("Android JavaVM not available yet (the WebView has not started)".to_string());
    }
    // SAFETY: the pointer comes from `ndk_context`, which tao populates with the real `JavaVM*` before
    // any of our code can run; it is valid for the life of the process.
    let vm = unsafe { JavaVM::from_raw(ptr.cast()) }
        .map_err(|e| format!("could not adopt the Android JavaVM: {e}"))?;
    Ok(VM.get_or_init(|| vm))
}

/// A `JNIEnv` for the calling thread, attaching it to the VM if it is not already.
pub fn env() -> Result<JNIEnv<'static>, String> {
    java_vm()?
        .attach_current_thread_as_daemon()
        .map_err(|e| format!("could not attach the thread to the Android VM: {e}"))
}

/// Look up one of the app's own classes by its dotted binary name (`com.kitegc.app.UsbSerial`).
///
/// Goes through the Activity's class loader — see the module docs for why `find_class` cannot be used
/// here. The loader is resolved once and held as a global reference; the class itself is looked up per
/// call, which is a hash lookup inside the loader and not worth caching around the lifetime rules.
pub fn app_class<'local>(
    env: &mut JNIEnv<'local>,
    binary_name: &str,
) -> Result<JClass<'local>, String> {
    let loader = class_loader(env)?;
    let r = env.new_string(binary_name);
    let name = check(env, r, "building the class name string")?;
    let r = env
        .call_method(
            loader.as_obj(),
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )
        .and_then(|v| v.l());
    let class = check(env, r, &format!("loading {binary_name}"))?;
    Ok(JClass::from(class))
}

/// The Activity's class loader (`activity.getClass().getClassLoader()`), cached as a global ref.
fn class_loader(env: &mut JNIEnv) -> Result<&'static GlobalRef, String> {
    if let Some(loader) = CLASS_LOADER.get() {
        return Ok(loader);
    }

    let activity_ptr = ndk_context::android_context().context();
    if activity_ptr.is_null() {
        return Err("Android Activity not available yet".to_string());
    }
    // SAFETY: `ndk_context` holds a global reference to the Activity for the life of the process, so
    // borrowing it here without taking ownership is sound.
    let activity = unsafe { JObject::from_raw(activity_ptr.cast()) };

    let r = env.get_object_class(&activity);
    let activity_class = check(env, r, "reading the Activity class")?;
    let r = env
        .call_method(
            &activity_class,
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )
        .and_then(|v| v.l());
    let loader = check(env, r, "reading the app class loader")?;
    let r = env.new_global_ref(loader);
    let loader = check(env, r, "pinning the app class loader")?;

    // `activity` needs no cleanup and no `mem::forget`. In jni 0.21 `JObject` is a plain wrapper over
    // the raw handle with no `Drop` impl — releasing a reference is explicit (`AutoLocal`,
    // `delete_local_ref`, a local frame), so simply letting it fall out of scope releases nothing.
    // Which is what we want: the handle belongs to `ndk_context`, which keeps the Activity alive as a
    // global ref for the life of the process, and freeing it here would be a double-free of someone
    // else's reference.

    Ok(CLASS_LOADER.get_or_init(|| loader))
}

/// Convert a JNI result into a plain `Result<T, String>`, **clearing any pending Java exception**.
///
/// This is not a convenience wrapper — it is required for correctness. The `jni` crate reports a Java
/// exception as `Error::JavaException` but deliberately leaves it *pending* on the thread, and every
/// JNI function other than the `Exception*` family is undefined behaviour while one is. Since our
/// threads are long-lived (the transport read loop attaches once and stays attached), an uncleared
/// exception would not fail the current call — it would corrupt the next one, on a code path with no
/// obvious connection to the original fault.
///
/// So: every JNI call in this crate goes through here. Compute the result first, then hand it over —
/// two statements rather than one, because both borrow the env mutably:
///
/// ```ignore
/// let r = env.call_static_method(&class, "open", "(Ljava/lang/String;I)I", &args);
/// let handle = jvm::check(&mut env, r.and_then(|v| v.i()), "UsbSerial.open")?;
/// ```
pub fn check<T>(
    env: &mut JNIEnv,
    result: jni::errors::Result<T>,
    what: &str,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(e) => {
            let mut message = format!("{what} failed: {e}");
            if let Ok(true) = env.exception_check() {
                // describe() prints the Java stack trace to logcat — the only place the actual cause
                // is visible, since JNI gives us no way to read it back as a string cheaply.
                let _ = env.exception_describe();
                let _ = env.exception_clear();
                message.push_str(" (Java exception; stack trace in logcat)");
            }
            Err(message)
        }
    }
}
