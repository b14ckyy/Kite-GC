// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Storage-access commands: share sheet, folder picker, picked-file names. Android stores the picker's
// tree URI as the setting value; see `StorageAccess.kt` and `user_file::mirror_session`.

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

/// The file name of something a file dialog returned. Desktop: the last path component. Android: the
/// provider's display name — a document URI often ends in an id, not in the name.
#[tauri::command]
pub fn file_display_name(path: String) -> String {
    #[cfg(target_os = "android")]
    if crate::android::content::is_content_uri(&path) {
        match crate::android::content::display_name(&path) {
            Ok(Some(name)) => return name,
            Ok(None) => {}
            Err(e) => log::warn!("[storage] no display name for a picked document: {e}"),
        }
    }
    std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(path)
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
