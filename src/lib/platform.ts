// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Lightweight host-OS detection for platform-specific UI (e.g. macOS traffic-light window controls on
// the LEFT vs the Windows/Linux control cluster on the RIGHT). We read the WebView user-agent rather
// than pulling in @tauri-apps/plugin-os: the string is stable per platform (WKWebView reports
// "Macintosh", WebView2 "Windows", WebKitGTK "Linux") and this only drives cosmetic layout, so a sync
// value with no extra dependency / async round-trip is preferable.

const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';

// iPadOS reports a desktop "Macintosh" user-agent (has done since iPadOS 13), so the UA alone cannot
// tell an iPad from a Mac. A multi-touch pointer does: Macs have no touchscreen (maxTouchPoints 0),
// iPads/iPhones report > 1. We use that to disambiguate.
const hasTouch = typeof navigator !== 'undefined' && navigator.maxTouchPoints > 1;

/** True on iOS / iPadOS (iPhone/iPod UA, or a touch-capable "Macintosh" which is really an iPad - this
 *  also matches the iOS Simulator, whose WKWebView reports a "Macintosh" UA with a multi-touch pointer). */
export const isIOS = /iPad|iPhone|iPod/i.test(ua) || (/Macintosh|Mac OS X/i.test(ua) && hasTouch);

/** True on any mobile/tablet build. Currently iOS/iPadOS only; Android would extend this later. */
export const isMobile = isIOS;

/** True when running inside the macOS WebView (WKWebView) — used to mirror native window-control
 *  placement and drive the native-capture backend (AVFoundation). Excludes iPadOS, which shares the
 *  "Macintosh" user-agent. */
export const isMacOS = /Macintosh|Mac OS X/i.test(ua) && !isIOS;

/** True on the Windows WebView (WebView2) — drives the native-capture backend (DirectShow). */
export const isWindows = /Windows/i.test(ua);

/** True on the Linux WebView (WebKitGTK; excludes Android) — drives the native-capture backend (V4L2). */
export const isLinux = /Linux/i.test(ua) && !/Android/i.test(ua);

// Tag the document root on mobile so global CSS can add bottom breathing room. The bottom edge is
// crowded there: the on-screen RC sticks, the map zoom/compass buttons, the Leaflet attribution label
// and the iPad home indicator all fight for the same strip. `--safe-bottom` exposes the device safe
// area (needs viewport-fit=cover in the viewport meta) for those bottom-anchored overlays to lift by.
if (typeof document !== 'undefined' && isMobile) {
  document.documentElement.classList.add('is-mobile');
  document.documentElement.style.setProperty('--safe-bottom', 'env(safe-area-inset-bottom, 0px)');
  // `--safe-top` covers the status bar / notch strip so the toolbar and top-anchored map layers
  // are not overlaid by the iOS status bar (clock, battery). Same viewport-fit=cover requirement.
  document.documentElement.style.setProperty('--safe-top', 'env(safe-area-inset-top, 0px)');
}
