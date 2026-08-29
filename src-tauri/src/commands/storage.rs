// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Storage-access commands — the platform side of the custom database / raw-log folder settings.
//
// Desktop needs none of this: the dialog plugin's folder picker returns a real path there. Android
// owns the difference: the system tree picker grants ONE folder (scoped storage, no permission),
// the setting stores that grant's tree URI, and the session-end mirror copies artefacts into it —
// see `StorageAccess.kt` and `user_file::mirror_session` for the two halves.

/// Hand a file to the platform share sheet (Android). Desktop has no share sheet and no need for
/// one — files live on paths the user can reach, and the frontend offers "open folder" there
/// instead — so this answers with that pointer rather than a dead end.
#[tauri::command]
pub fn share_file(path: String, mime: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        return crate::android::storage::share_file(&path, &mime);
    }
    #[allow(unreachable_code)]
    {
        let _ = (path, mime);
        Err("share_file is Android-only — desktop reveals the file in its folder instead".into())
    }
}

/// Open the platform folder picker. On Android this returns the granted **tree URI** (`content://…`)
/// to store as the setting value (`Ok(None)` = cancelled); on desktop the frontend uses the dialog
/// plugin directly, so this answers with a pointer rather than a dead end.
#[tauri::command]
pub async fn storage_pick_folder() -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        // The Kotlin side blocks on the picker (minutes, potentially) — keep that off the async
        // runtime's worker threads.
        return tauri::async_runtime::spawn_blocking(crate::android::storage::pick_folder)
            .await
            .map_err(|e| format!("folder picker task failed: {e}"))?;
    }
    #[allow(unreachable_code)]
    Err("storage_pick_folder is Android-only — desktop uses the dialog plugin's folder picker".into())
}
