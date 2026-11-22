// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Mini view: a small always-on-top window while the main window is minimized (desktop), picture-in-picture
//! on Android. The main window publishes the values; this module only creates, places, shows and hides it.

use tauri::AppHandle;

#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};

/// Window label of the mini view. The frontend's `MINI_LABEL` must match.
#[cfg(desktop)]
pub const MINI_LABEL: &str = "mini";
/// Emitted to the main window when the mini window is shown (`true`) or hidden (`false`).
#[cfg(desktop)]
const VISIBLE_EVENT: &str = "mini-view-visible";
/// Logical width and the gap to the work area's bottom-right corner; the height follows the monitor's aspect.
#[cfg(desktop)]
const MINI_W: f64 = 300.0;
#[cfg(desktop)]
const MINI_MARGIN: f64 = 12.0;

#[cfg(desktop)]
static ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(desktop)]
static SHOWN: AtomicBool = AtomicBool::new(false);

/// The Settings toggle. Desktop: builds the hidden window up front (building one from the event loop
/// deadlocks on Windows) or destroys it. Android: tells the activity whether leaving enters PiP.
#[tauri::command(async)]
pub fn mini_view_set_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(desktop)]
    let result = {
        ENABLED.store(enabled, Ordering::SeqCst);
        if enabled {
            ensure_window(&app)
        } else {
            destroy_window(&app);
            Ok(())
        }
    };
    #[cfg(target_os = "android")]
    let result = {
        let _ = &app;
        crate::android::pip::set_enabled(enabled).inspect_err(|e| log::warn!("[mini-view] PiP switch failed: {e}"))
    };
    #[cfg(target_os = "ios")]
    let result = {
        let _ = (&app, enabled);
        Ok(())
    };
    result
}

/// A click on the mini window: bring the main window back. The mini window hides with it.
#[tauri::command]
pub fn mini_view_restore(app: AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.unminimize();
            let _ = main.show();
            let _ = main.set_focus();
        }
        hide(&app);
    }
    #[cfg(mobile)]
    let _ = app;
    Ok(())
}

/// Main-window events from the run loop: minimized → show, anything else → hide, gone → destroy.
/// No dedicated minimize event exists, so every resize and focus change re-reads the state.
#[cfg(desktop)]
pub fn on_main_window_event(app: &AppHandle, event: &tauri::WindowEvent) {
    use tauri::{Manager, WindowEvent};
    match event {
        WindowEvent::Resized(_) | WindowEvent::Focused(_) => {}
        WindowEvent::Destroyed => {
            destroy_window(app);
            return;
        }
        _ => return,
    }
    let Some(main) = app.get_webview_window("main") else { return };
    if ENABLED.load(Ordering::SeqCst) && main.is_minimized().unwrap_or(false) {
        show(app, &main);
    } else {
        hide(app);
    }
}

#[cfg(desktop)]
fn ensure_window(app: &AppHandle) -> Result<(), String> {
    use tauri::{window::Color, Manager, WebviewUrl, WebviewWindowBuilder};

    if app.get_webview_window(MINI_LABEL).is_some() {
        return Ok(());
    }
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(app, MINI_LABEL, WebviewUrl::App("mini".into()))
        .title("Kite Ground Control — Mini view")
        .decorations(false)
        // An undecorated window's shadow brings a white top border on Windows 10.
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .background_color(Color(0x2e, 0x2e, 0x2e, 0xff))
        .inner_size(MINI_W, MINI_W * 9.0 / 16.0);
    // All WebView2s of a process share one environment and must carry the same browser arguments.
    #[cfg(windows)]
    if let Some(args) = app
        .config()
        .app
        .windows
        .first()
        .and_then(|w| w.additional_browser_args.clone())
    {
        builder = builder.additional_browser_args(&args);
    }
    builder.build().map(|_| ()).map_err(|e| {
        log::warn!("[mini-view] window could not be created: {e}");
        format!("mini view window: {e}")
    })
}

#[cfg(desktop)]
fn destroy_window(app: &AppHandle) {
    use tauri::{Emitter, Manager};
    if SHOWN.swap(false, Ordering::SeqCst) {
        let _ = app.emit_to("main", VISIBLE_EVENT, false);
    }
    if let Some(win) = app.get_webview_window(MINI_LABEL) {
        let _ = win.destroy();
    }
}

/// Bottom-right of the work area of the monitor the main window is on (Windows reports a minimized
/// window's monitor from its restored rectangle).
#[cfg(desktop)]
fn show(app: &AppHandle, main: &tauri::WebviewWindow) {
    use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};
    let Some(mini) = app.get_webview_window(MINI_LABEL) else { return };
    if SHOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let monitor = main
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| main.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let area = m.work_area();
        // Ultra-wide and tall screens clamped to Android's PiP range, so the layout stays the same.
        let aspect = (m.size().width as f64 / m.size().height.max(1) as f64).clamp(1.0 / 2.39, 2.39);
        let w = (MINI_W * scale).round() as i32;
        let h = (MINI_W * scale / aspect).round() as i32;
        let gap = (MINI_MARGIN * scale).round() as i32;
        let x = area.position.x + area.size.width as i32 - w - gap;
        let y = area.position.y + area.size.height as i32 - h - gap;
        // Position first: moving onto a monitor with another scale resizes the window on the way.
        let _ = mini.set_position(PhysicalPosition::new(x, y));
        let _ = mini.set_size(PhysicalSize::new(w as u32, h as u32));
    }
    let _ = mini.show();
    let _ = mini.set_always_on_top(true);
    let _ = app.emit_to("main", VISIBLE_EVENT, true);
}

#[cfg(desktop)]
fn hide(app: &AppHandle) {
    use tauri::{Emitter, Manager};
    if !SHOWN.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some(mini) = app.get_webview_window(MINI_LABEL) {
        let _ = mini.hide();
    }
    let _ = app.emit_to("main", VISIBLE_EVENT, false);
}
