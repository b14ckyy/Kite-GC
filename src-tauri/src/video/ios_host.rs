// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Sebastian Kumor
// Ported from apple_host.rs, Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! UIKit host for the iOS hole-punch video layer: the iOS twin of `apple_host`, with the same
//! functions so `apple_sink` serves both. One container UIView per surface, placed directly under
//! the WKWebView in its superview; the page is see-through there (`tauri.ios.conf.json`).

// Same reason as apple_host: the CATransform3D C helpers are flagged as renamed.
#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::sync::OnceLock;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_quartz_core::{CALayer, CATransform3D, CATransform3DConcat, CATransform3DIdentity, CATransform3DMakeRotation, CATransform3DMakeScale};
use objc2_ui_kit::{UIColor, UIView};
use tauri::{AppHandle, Manager};

/// One on-screen output: the clip container under the WebView plus the sink's display layer.
struct Output {
    webview: Retained<UIView>,
    container: Retained<UIView>,
    video: Option<Retained<CALayer>>,
    /// Last rect (physical px: x, y, w, h, cx, cy, cw, ch), re-applied when a layer attaches later.
    rect: [i32; 8],
}

thread_local! {
    static OUTPUTS: RefCell<HashMap<String, Output>> = RefCell::new(HashMap::new());
    static ORIENT: Cell<(bool, bool)> = const { Cell::new((false, false)) };
    /// The main window's WKWebView. iOS has one window, and `with_webview` is async, so it is
    /// captured once at install instead of being looked up from the main thread later.
    static WEBVIEW: RefCell<Option<Retained<UIView>>> = const { RefCell::new(None) };
}

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Remember the app handle and the WebView, and clear the scroll view, which wry leaves opaque.
pub fn install(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.with_webview(|w| {
        let wk = w.inner() as *mut AnyObject;
        // SAFETY: tauri hands out the live WKWebView (a UIView) on the main thread.
        let Some(view) = (unsafe { Retained::retain(wk as *mut UIView) }) else { return };
        unsafe {
            let clear = UIColor::clearColor();
            let scroll: *mut AnyObject = msg_send![wk, scrollView];
            if !scroll.is_null() {
                let _: () = msg_send![scroll, setBackgroundColor: &*clear];
                let _: () = msg_send![scroll, setOpaque: false];
            }
        }
        WEBVIEW.with(|v| *v.borrow_mut() = Some(view));
    });
}

fn on_main(f: impl FnOnce(&mut HashMap<String, Output>) + Send + 'static) {
    let Some(app) = APP.get() else { return };
    let _ = app.run_on_main_thread(move || {
        OUTPUTS.with(|m| f(&mut m.borrow_mut()));
    });
}

fn webview_of(label: &str) -> Result<Retained<UIView>, String> {
    if label != "main" {
        return Err(format!("no window labelled {label} on iOS"));
    }
    WEBVIEW.with(|v| v.borrow().clone()).ok_or_else(|| "WebView not captured yet".into())
}

fn create_container(label: &str) -> Result<Output, String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("not on the main thread".into());
    };
    let webview = webview_of(label)?;
    let parent = webview.superview().ok_or("WebView has no superview")?;

    let container = UIView::initWithFrame(
        mtm.alloc::<UIView>(),
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(1.0, 1.0)),
    );
    container.setUserInteractionEnabled(false);
    let layer = container.layer();
    let black = CGColor::new_srgb(0.0, 0.0, 0.0, 1.0);
    layer.setBackgroundColor(Some(&black));
    layer.setMasksToBounds(true);
    parent.insertSubview_belowSubview(&container, &webview);

    log::info!("[video] ios host: container in window {label} (scale {})", display_scale(&webview));
    Ok(Output { webview, container, video: None, rect: [0, 0, 1, 1, 0, 0, 1, 1] })
}

/// Screen scale (the page's devicePixelRatio). WKWebView's `contentScaleFactor` stays 1 on a 2x screen.
fn display_scale(view: &UIView) -> f64 {
    let traits: *mut AnyObject = unsafe { msg_send![view, traitCollection] };
    if traits.is_null() {
        return 1.0;
    }
    let scale: f64 = unsafe { msg_send![traits, displayScale] };
    scale.max(1.0)
}

/// Physical px in WebView coordinates (top-left origin, like UIKit) to points in the superview.
fn layout(out: &Output) {
    let s = display_scale(&out.webview);
    let p = |v: i32| v as f64 / s;
    let [x, y, w, h, cx, cy, cw, ch] = out.rect;
    let origin = out.webview.frame().origin;
    let (cw_pt, ch_pt) = (p(cw).max(1.0), p(ch).max(1.0));
    out.container.setFrame(CGRect::new(
        CGPoint::new(origin.x + p(cx), origin.y + p(cy)),
        CGSize::new(cw_pt, ch_pt),
    ));
    if let Some(video) = &out.video {
        let (w_pt, h_pt) = (p(w).max(1.0), p(h).max(1.0));
        video.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w_pt, h_pt)));
        video.setPosition(CGPoint::new(p(x - cx) + w_pt / 2.0, p(y - cy) + h_pt / 2.0));
        video.setContentsScale(s);
        let (mirror, rotate180) = ORIENT.with(|o| o.get());
        video.setTransform(transform(mirror, rotate180));
    }
}

fn transform(mirror: bool, rotate180: bool) -> CATransform3D {
    let mut t = unsafe { CATransform3DIdentity };
    if mirror {
        t = CATransform3DConcat(t, CATransform3DMakeScale(-1.0, 1.0, 1.0));
    }
    if rotate180 {
        t = CATransform3DConcat(t, CATransform3DMakeRotation(std::f64::consts::PI, 0.0, 0.0, 1.0));
    }
    t
}

/// Create output `key`'s display layer on the main thread and lay it out. Blocks until done.
pub fn attach(
    key: String,
    window: String,
    make: impl FnOnce() -> Retained<CALayer> + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = channel::<Result<(), String>>();
    on_main(move |outs| {
        let built = match outs.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => Ok(e.into_mut()),
            std::collections::hash_map::Entry::Vacant(e) => create_container(&window).map(|out| e.insert(out)),
        };
        let res = built.map(|out| {
            if let Some(old) = out.video.take() {
                old.removeFromSuperlayer();
            }
            let layer = make();
            out.container.layer().addSublayer(&layer);
            out.video = Some(layer);
            layout(out);
        });
        let _ = tx.send(res);
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(res) => res,
        Err(_) => Err("display layer attach timed out".into()),
    }
}

/// FULL box `x/y/w/h` for the aspect-fit layout, VISIBLE box `cx/cy/cw/ch` for the clip.
pub fn place(key: String, rect: [i32; 8]) {
    on_main(move |outs| {
        if let Some(out) = outs.get_mut(&key) {
            out.rect = rect;
            layout(out);
        }
    });
}

pub fn remove(key: String) {
    on_main(move |outs| {
        if let Some(out) = outs.remove(&key) {
            if let Some(v) = &out.video {
                v.removeFromSuperlayer();
            }
            out.container.removeFromSuperview();
        }
    });
}

pub fn remove_all() {
    on_main(|outs| {
        for (_, out) in outs.drain() {
            if let Some(v) = &out.video {
                v.removeFromSuperlayer();
            }
            out.container.removeFromSuperview();
        }
    });
}

/// `keys` arrives highest priority first and the widget must end on top, so each is re-inserted
/// directly under the WebView in order: the last one inserted is the topmost container.
pub fn restack(keys: Vec<String>) {
    on_main(move |outs| {
        for key in &keys {
            let Some(out) = outs.get(key) else { continue };
            let Some(parent) = out.webview.superview() else { continue };
            parent.insertSubview_belowSubview(&out.container, &out.webview);
        }
    });
}

pub fn set_orient(mirror: bool, rotate180: bool) {
    on_main(move |outs| {
        ORIENT.with(|o| o.set((mirror, rotate180)));
        for out in outs.values() {
            layout(out);
        }
    });
}
