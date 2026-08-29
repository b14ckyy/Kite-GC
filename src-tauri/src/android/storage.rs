// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! JNI shim in front of `StorageAccess.kt` — user-chosen storage folders.
//!
//! Same boundary contract as the USB-serial shim: primitives and Strings across JNI, the Kotlin side
//! owns all state, and a null result's reason is fetched afterwards via `getLastError`.
//!
//! The model is scoped storage: the user grants ONE folder through the system tree picker, the app
//! holds a persistable grant on it, and no storage permission exists anywhere. Because a SAF grant
//! provides no POSIX path, nothing *lives* in that folder — the session-end mirror
//! (`user_file::mirror_session`) copies artefacts into it through the ContentResolver.

use jni::objects::JString;

use super::jvm;

const BRIDGE: &str = "com.kitegc.app.StorageAccess";

/// Open the system folder picker and block until the user answers. Returns the granted **tree URI**
/// (`content://…`) to store as the setting value; `Ok(None)` = cancelled.
pub fn pick_folder() -> Result<Option<String>, String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let r = env
        .call_static_method(&class, "pickFolder", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l());
    let value = jvm::check(&mut env, r, "StorageAccess.pickFolder")?;
    if value.is_null() {
        // Cancelled, or failed — getLastError tells the two apart.
        let r = env
            .call_static_method(&class, "getLastError", "()Ljava/lang/String;", &[])
            .and_then(|v| v.l());
        let err = jvm::check(&mut env, r, "StorageAccess.getLastError")?;
        if err.is_null() {
            return Ok(None);
        }
        let msg = env
            .get_string(&JString::from(err))
            .map(String::from)
            .map_err(|e| format!("reading the picker error: {e}"))?;
        return Err(msg);
    }
    let path = env
        .get_string(&JString::from(value))
        .map(String::from)
        .map_err(|e| format!("reading the picked path: {e}"))?;
    Ok(Some(path))
}

/// Hand a file to the system share sheet (mail, messengers, a text editor…). Fire-and-forget —
/// the sheet is the user's from there on.
pub fn share_file(path: &str, mime: &str) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let j_path = env
        .new_string(path)
        .map_err(|e| format!("building the path string: {e}"))?;
    let j_mime = env
        .new_string(mime)
        .map_err(|e| format!("building the mime string: {e}"))?;
    let r = env
        .call_static_method(
            &class,
            "shareFile",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[(&j_path).into(), (&j_mime).into()],
        )
        .and_then(|v| v.l());
    let err = jvm::check(&mut env, r, "StorageAccess.shareFile")?;
    if err.is_null() {
        return Ok(());
    }
    let msg = env
        .get_string(&JString::from(err))
        .map(String::from)
        .map_err(|e| format!("reading the share error: {e}"))?;
    Err(msg)
}

/// Mirror every regular file in `src_dir` into the granted tree at `tree_uri` — files already
/// present with the same size are skipped, everything else is created or rewritten through the
/// ContentResolver. Flat (no subdirectories), which matches the raw-log staging dir and the DB
/// snapshot dir.
pub fn sync_dir_to_tree(tree_uri: &str, src_dir: &str) -> Result<(), String> {
    let mut env = jvm::env()?;
    let class = jvm::app_class(&mut env, BRIDGE)?;
    let j_uri = env
        .new_string(tree_uri)
        .map_err(|e| format!("building the tree-uri string: {e}"))?;
    let j_dir = env
        .new_string(src_dir)
        .map_err(|e| format!("building the src-dir string: {e}"))?;
    let r = env
        .call_static_method(
            &class,
            "syncDirToTree",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[(&j_uri).into(), (&j_dir).into()],
        )
        .and_then(|v| v.l());
    let err = jvm::check(&mut env, r, "StorageAccess.syncDirToTree")?;
    if err.is_null() {
        return Ok(());
    }
    let msg = env
        .get_string(&JString::from(err))
        .map(String::from)
        .map_err(|e| format!("reading the sync error: {e}"))?;
    Err(msg)
}
