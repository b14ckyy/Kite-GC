// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Reading and writing the `content://` URIs Android's file pickers hand back.
//!
//! Tauri's dialog plugin opens the Storage Access Framework on Android — `ACTION_CREATE_DOCUMENT`
//! for save, `ACTION_OPEN_DOCUMENT` for open — and returns the chosen document as a `content://`
//! URI string, not a filesystem path. Nothing in `std::fs` can open one: scoped storage means the
//! file may live in another app's provider, on a SD card, or in Drive, and is reachable only through
//! the `ContentResolver`.
//!
//! That is why every export was failing on mobile. The app could write perfectly good bytes and had
//! nowhere to put them — and since app-private storage is wiped on uninstall, "export" could not get
//! a flight log off the device at all.
//!
//! These two functions bridge the gap over JNI, using the plumbing in `android/jvm.rs`. Callers do
//! not use them directly; `user_file.rs` decides when a path is really a URI and routes accordingly.

use jni::objects::{JByteArray, JObject, JValue};

use super::jvm;

/// Chunk size for stream copies. Java arrays cross the JNI boundary as copies, so a whole flight-log
/// export in one allocation would double its size in the Java heap for the duration of the call.
const CHUNK: usize = 256 * 1024;

/// True when `path` is a SAF document URI rather than a filesystem path.
pub fn is_content_uri(path: &str) -> bool {
    path.starts_with("content://")
}

/// `context.getContentResolver()`.
fn resolver<'l>(env: &mut jni::JNIEnv<'l>) -> Result<JObject<'l>, String> {
    let ctx_ptr = ndk_context::android_context().context();
    if ctx_ptr.is_null() {
        return Err("Android context not available".to_string());
    }
    // SAFETY: `ndk_context` holds a global reference to the Activity for the process lifetime.
    let ctx = unsafe { JObject::from_raw(ctx_ptr.cast()) };
    let r = env.call_method(
        &ctx,
        "getContentResolver",
        "()Landroid/content/ContentResolver;",
        &[],
    );
    let out = jvm::check(env, r.and_then(|v| v.l()), "getContentResolver");
    std::mem::forget(ctx); // borrowed handle; dropping the wrapper must not release the global ref
    out
}

/// `Uri.parse(uri)`. `android.net.Uri` is a platform class, so the ordinary `find_class` works here —
/// the bootstrap loader is reachable from any thread. Only the app's own classes need the Activity
/// class loader (see `jvm::app_class`).
fn parse_uri<'l>(env: &mut jni::JNIEnv<'l>, uri: &str) -> Result<JObject<'l>, String> {
    let r = env.new_string(uri);
    let juri = jvm::check(env, r, "building the URI string")?;
    let r = env.find_class("android/net/Uri");
    let class = jvm::check(env, r, "finding android.net.Uri")?;
    let r = env
        .call_static_method(
            &class,
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&juri)],
        )
        .and_then(|v| v.l());
    jvm::check(env, r, "Uri.parse")
}

/// Copy the file at `src` into the document `uri`, replacing its contents.
///
/// Opened with mode `"wt"` — write + truncate. Plain `"w"` leaves any previous, longer content in
/// place beyond what we write, so re-exporting over an existing file would produce a valid prefix
/// followed by trailing garbage from the old one.
pub fn write_uri(uri: &str, src: &std::path::Path) -> Result<(), String> {
    use std::io::Read;

    let mut file = std::fs::File::open(src).map_err(|e| format!("reopening the export: {e}"))?;
    let mut env = jvm::env()?;
    let resolver = resolver(&mut env)?;
    let juri = parse_uri(&mut env, uri)?;

    let r = env.new_string("wt");
    let mode = jvm::check(&mut env, r, "building the open mode")?;
    let r = env
        .call_method(
            &resolver,
            "openOutputStream",
            "(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;",
            &[JValue::Object(&juri), JValue::Object(&mode)],
        )
        .and_then(|v| v.l());
    let stream = jvm::check(&mut env, r, "openOutputStream")?;
    if stream.is_null() {
        return Err(format!("Android returned no output stream for {uri}"));
    }

    let mut buf = vec![0u8; CHUNK];
    let mut result = Ok(());
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                result = Err(format!("reading the export: {e}"));
                break;
            }
        };
        let r = env.byte_array_from_slice(&buf[..n]);
        let chunk: JByteArray = match jvm::check(&mut env, r, "staging a chunk") {
            Ok(a) => a,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        let r = env
            .call_method(
                &stream,
                "write",
                "([BII)V",
                &[JValue::Object(&chunk), JValue::Int(0), JValue::Int(n as i32)],
            )
            .map(|_| ());
        if let Err(e) = jvm::check(&mut env, r, "OutputStream.write") {
            result = Err(e);
            break;
        }
    }

    // Always close, even on a write error: an unclosed stream leaves the SAF document locked and
    // half-written, and the next export to the same file would fail for an unrelated-looking reason.
    let r = env.call_method(&stream, "flush", "()V", &[]).map(|_| ());
    let flushed = jvm::check(&mut env, r, "OutputStream.flush");
    let r = env.call_method(&stream, "close", "()V", &[]).map(|_| ());
    let closed = jvm::check(&mut env, r, "OutputStream.close");

    result.and(flushed).and(closed)
}

/// Copy the document `uri` into the file at `dest`.
pub fn read_uri(uri: &str, dest: &std::path::Path) -> Result<(), String> {
    use std::io::Write;

    let mut env = jvm::env()?;
    let resolver = resolver(&mut env)?;
    let juri = parse_uri(&mut env, uri)?;

    let r = env
        .call_method(
            &resolver,
            "openInputStream",
            "(Landroid/net/Uri;)Ljava/io/InputStream;",
            &[JValue::Object(&juri)],
        )
        .and_then(|v| v.l());
    let stream = jvm::check(&mut env, r, "openInputStream")?;
    if stream.is_null() {
        return Err(format!("Android returned no input stream for {uri}"));
    }

    let mut file = std::fs::File::create(dest).map_err(|e| format!("creating the import copy: {e}"))?;
    let r = env.new_byte_array(CHUNK as i32);
    let jbuf: JByteArray = jvm::check(&mut env, r, "allocating a read buffer")?;
    let mut scratch = vec![0i8; CHUNK];
    let mut result = Ok(());

    loop {
        let r = env
            .call_method(
                &stream,
                "read",
                "([B)I",
                &[JValue::Object(&jbuf)],
            )
            .and_then(|v| v.i());
        let n = match jvm::check(&mut env, r, "InputStream.read") {
            Ok(n) => n,
            Err(e) => {
                result = Err(e);
                break;
            }
        };
        if n < 0 {
            break; // end of stream
        }
        let n = n as usize;
        let r = env.get_byte_array_region(&jbuf, 0, &mut scratch[..n]);
        if let Err(e) = jvm::check(&mut env, r, "copying a chunk") {
            result = Err(e);
            break;
        }
        // i8 and u8 have identical layout; only the `n` bytes just filled are read back.
        let bytes = unsafe { std::slice::from_raw_parts(scratch.as_ptr() as *const u8, n) };
        if let Err(e) = file.write_all(bytes) {
            result = Err(format!("writing the import copy: {e}"));
            break;
        }
    }

    let r = env.call_method(&stream, "close", "()V", &[]).map(|_| ());
    let closed = jvm::check(&mut env, r, "InputStream.close");
    result.and(closed)
}
