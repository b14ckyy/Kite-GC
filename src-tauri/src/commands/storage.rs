// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Storage-access commands: share sheet, folder picker, picked-file names. Android stores the picker's
// tree URI as the setting value; see `StorageAccess.kt` and `user_file::move_raw_logs`.

/// Hand a file to the platform share sheet (Android, iOS). Desktop has none and needs none — files
/// live on paths the user can reach. `save_label` names an extra "save to device" action (Android 14+).
#[tauri::command]
pub fn share_file(
    path: String,
    mime: String,
    save_label: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let _ = &app;
        return crate::android::storage::share_file(&path, &mime, save_label.as_deref().unwrap_or(""));
    }
    #[cfg(target_os = "ios")]
    {
        let _ = (&mime, &save_label);
        return share_ios(&app, path);
    }
    #[allow(unreachable_code)]
    {
        let _ = (path, mime, save_label, app);
        Err("share_file is for mobile — desktop reveals the file in its folder instead".into())
    }
}

/// `UIActivityViewController` over the main view controller. On an iPad it is a popover and needs an
/// anchor: without `sourceView` / `sourceRect` UIKit raises an exception.
#[cfg(target_os = "ios")]
fn share_ios(app: &tauri::AppHandle, path: String) -> Result<(), String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::NSString;
    use tauri::Manager;

    let window = app.get_webview_window("main").ok_or("no main window")?;
    window
        .with_webview(move |w| {
            let vc = w.view_controller() as *mut AnyObject;
            let (Some(url_class), Some(array_class), Some(sheet_class)) = (
                AnyClass::get(c"NSURL"),
                AnyClass::get(c"NSArray"),
                AnyClass::get(c"UIActivityViewController"),
            ) else {
                return;
            };
            if vc.is_null() {
                return;
            }
            // SAFETY: tauri calls this on the main thread with the live view controller; every object
            // below is created and consumed inside this call.
            unsafe {
                let ns_path = NSString::from_str(&path);
                let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*ns_path];
                if url.is_null() {
                    return;
                }
                let items: *mut AnyObject = msg_send![array_class, arrayWithObject: url];
                let sheet: *mut AnyObject = msg_send![sheet_class, alloc];
                let none: *mut AnyObject = std::ptr::null_mut();
                let sheet: *mut AnyObject =
                    msg_send![sheet, initWithActivityItems: items, applicationActivities: none];
                if sheet.is_null() {
                    return;
                }
                let popover: *mut AnyObject = msg_send![sheet, popoverPresentationController];
                if !popover.is_null() {
                    let view: *mut AnyObject = msg_send![vc, view];
                    let bounds: CGRect = msg_send![view, bounds];
                    let anchor = CGRect::new(
                        CGPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0),
                        CGSize::new(1.0, 1.0),
                    );
                    let _: () = msg_send![popover, setSourceView: view];
                    let _: () = msg_send![popover, setSourceRect: anchor];
                }
                let no_completion: Option<&block2::Block<dyn Fn()>> = None;
                let _: () = msg_send![vc, presentViewController: sheet, animated: true, completion: no_completion];
            }
        })
        .map_err(|e| format!("share sheet: {e}"))
}

/// Where a mobile export is written before it is shared: `<app cache>/exports/<name>`. The folder holds
/// one export at a time — the previous one is removed here, because the app that received it may still
/// have been reading when the share call returned.
#[tauri::command]
pub fn export_scratch_path(name: String, app: tauri::AppHandle) -> Result<String, String> {
    let dir = export_scratch_dir(&app)?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("export folder: {e}"))?;
    let file = std::path::Path::new(&name)
        .file_name()
        .ok_or_else(|| format!("not a file name: {name}"))?;
    Ok(dir.join(file).to_string_lossy().into_owned())
}

/// Remove what the last export left in the cache (called at app start).
#[tauri::command]
pub fn export_scratch_clear(app: tauri::AppHandle) {
    if let Ok(dir) = export_scratch_dir(&app) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn export_scratch_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    app.path().app_cache_dir().map(|d| d.join("exports")).map_err(|e| format!("cache folder: {e}"))
}

/// Android: move the closed raw logs into the folder `raw_path` names and delete the leftover
/// `db-mirror` folder. Skipped while a session is live: the link teardown moves them.
#[tauri::command(async)]
pub fn storage_flush_raw_logs(
    raw_path: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    crate::user_file::remove_db_mirror_snapshot();
    if state.sessions.has_live_session() {
        return Ok(());
    }
    let dir = crate::flightlog::db::resolve_raw_log_dir(&raw_path, false);
    crate::user_file::move_raw_logs(&dir, &raw_path);
    Ok(())
}

/// The shared export the user chose to save to the device from the share sheet (Android), once.
#[tauri::command]
pub fn export_take_save_request() -> Option<String> {
    #[cfg(target_os = "android")]
    match crate::android::storage::take_save_request() {
        Ok(path) => return path,
        Err(e) => log::warn!("[storage] save request not read: {e}"),
    }
    None
}

/// Copy a shared export to the place a save dialog returned (a document URI on Android).
#[tauri::command(async)]
pub fn export_copy(src: String, dest: String) -> Result<(), String> {
    crate::user_file::with_write_path(&dest, |p| {
        std::fs::copy(&src, p).map(|_| ()).map_err(|e| format!("Failed to save: {e}"))
    })
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
