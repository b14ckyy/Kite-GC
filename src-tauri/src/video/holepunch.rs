// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! DEV-ONLY hole-punch spike (Windows) — the P2.1 architecture probe
//! (Dev-Docs active/MOBILE_RTSP.md).
//!
//! Question to answer: can a native layer show through a transparent WebView2 in Tauri's
//! windowed hosting mode? This parks a solid-magenta child window at a given rect inside
//! the main window, either BELOW the WebView2 child in the sibling z-order (the real
//! test) or on top of everything (the control).
//!
//! The magenta is rendered through a **D3D11/DXGI flip-model swapchain**, not GDI — and
//! that is load-bearing: Tao's `transparent: true` enables DWM blur-behind with an empty
//! region, which makes the DWM honour the window surface's ALPHA channel, and GDI always
//! writes alpha 0 — a GDI-painted child composites as fully transparent (measured: the
//! first spike round was invisible even ON TOP). A swapchain presents opaque pixels, and
//! it is what the real Media Foundation sink will use anyway, so this doubles as the
//! render-layer skeleton.
//!
//! `KITE_SPIKE_AUTO=top|bottom` runs the spike unattended shortly after startup (the
//! `bottom` variant also strips the page's backgrounds and hides img/canvas via JS) so
//! the whole probe can be driven by a screenshot loop without clicking the DEV buttons.
//!
//! Throwaway scaffolding: removed once the real native video sink lands. Windows dev
//! builds only.

use tauri::Manager;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
    ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory2, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostMessageW, RegisterClassW, SetWindowPos, HWND_BOTTOM,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WINDOW_EX_STYLE, WM_CLOSE, WNDCLASSW,
    WS_CHILD, WS_VISIBLE,
};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn spike_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Create the magenta child at `x,y w×h` (PHYSICAL pixels, main-window client coords) for
/// `seconds`. `topmost=false` places it below the WebView2 sibling (the real test).
pub fn spawn(
    app: &tauri::AppHandle,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    topmost: bool,
    seconds: u64,
) -> Result<String, String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    let parent_raw = window.hwnd().map_err(|e| format!("hwnd: {e}"))?.0 as isize;

    // The child must be created on the thread that owns the parent window.
    let (tx, rx) = mpsc::channel::<Result<isize, String>>();
    app.run_on_main_thread(move || {
        let result = unsafe { create_child(parent_raw, x, y, w, h, topmost) };
        let _ = tx.send(result);
    })
    .map_err(|e| format!("main thread dispatch: {e}"))?;
    let child_raw = rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "spike: main thread did not respond".to_string())??;

    // Render + self-destruct on a worker: present magenta at ~10 Hz for the duration,
    // then WM_CLOSE (legal cross-thread; DefWindowProc destroys on the owning thread).
    let secs = seconds.clamp(1, 60);
    std::thread::spawn(move || {
        if let Err(e) = unsafe { present_magenta(child_raw, secs) } {
            log::warn!("[video] hole-punch spike: D3D render failed: {e}");
        }
        unsafe {
            let _ = PostMessageW(
                Some(HWND(child_raw as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });

    let placement = if topmost { "TOP (control)" } else { "BOTTOM (below the WebView)" };
    log::info!("[video] hole-punch spike: magenta child at {x},{y} {w}x{h}, {placement}, {secs}s");
    Ok(format!(
        "magenta child at {x},{y} {w}\u{d7}{h}, {placement}, {secs} s"
    ))
}

/// Unattended spike for the screenshot loop: `KITE_SPIKE_AUTO=top|bottom`. Waits for the
/// WebView to be up, strips the page backgrounds for the `bottom` variant (backgrounds
/// transparent + img/canvas hidden — the Leaflet tile carpet is an opaque `<img>` layer
/// that would cover any hole), then parks the child mid-window for 25 s.
pub fn auto_spike_if_requested(app: &tauri::AppHandle) {
    let Ok(mode) = std::env::var("KITE_SPIKE_AUTO") else { return };
    let topmost = mode != "bottom";
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        if !topmost {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval(
                    "(() => { const s = document.createElement('style'); \
                     s.textContent = '*, *::before, *::after { background: transparent !important; backdrop-filter: none !important; } \
                     img, canvas, video { visibility: hidden !important; }'; \
                     document.head.appendChild(s); setTimeout(() => s.remove(), 26000); })()",
                );
            }
        }
        match spawn(&app, 300, 200, 640, 400, topmost, 25) {
            Ok(msg) => log::info!("[video] auto spike ({mode}): {msg}"),
            Err(e) => log::warn!("[video] auto spike ({mode}) failed: {e}"),
        }
    });
}

unsafe fn create_child(
    parent_raw: isize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    topmost: bool,
) -> Result<isize, String> {
    unsafe {
        let class = w!("KiteHolePunchSpike");
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let instance = GetModuleHandleW(None).map_err(|e| format!("module handle: {e}"))?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(spike_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                CLASS_REGISTERED.store(false, Ordering::SeqCst);
                return Err("RegisterClassW failed".into());
            }
        }
        let child = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("spike"),
            WS_CHILD | WS_VISIBLE,
            x,
            y,
            w,
            h,
            Some(HWND(parent_raw as *mut _)),
            None,
            None,
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))?;
        let insert_after = if topmost { HWND_TOP } else { HWND_BOTTOM };
        let _ = SetWindowPos(
            child,
            Some(insert_after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        Ok(child.0 as isize)
    }
}

/// Clear-to-magenta through a flip-model swapchain on the child HWND, re-presented at
/// ~10 Hz for `secs`. Opaque alpha — which is the entire point (see module docs).
unsafe fn present_magenta(child_raw: isize, secs: u64) -> Result<(), String> {
    unsafe {
        let hwnd = HWND(child_raw as *mut _);

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
        let device = device.ok_or("no D3D11 device")?;
        let context = context.ok_or("no D3D11 context")?;

        let factory: IDXGIFactory2 =
            CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: 0,
            Height: 0,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        let swapchain = factory
            .CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)
            .map_err(|e| format!("CreateSwapChainForHwnd: {e}"))?;

        let buffer: ID3D11Texture2D = swapchain
            .GetBuffer(0)
            .map_err(|e| format!("GetBuffer: {e}"))?;
        let mut rtv: Option<ID3D11RenderTargetView> = None;
        device
            .CreateRenderTargetView(&buffer, None, Some(&mut rtv))
            .map_err(|e| format!("CreateRenderTargetView: {e}"))?;
        let rtv = rtv.ok_or("no render target view")?;

        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        while std::time::Instant::now() < deadline {
            context.ClearRenderTargetView(&rtv, &[1.0f32, 0.0, 1.0, 1.0]);
            let _ = swapchain.Present(1, DXGI_PRESENT(0));
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
