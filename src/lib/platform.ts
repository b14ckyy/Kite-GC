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

/** True on iPhone/iPod specifically (NOT iPad). iPad reports a "Macintosh" UA, so it never matches.
 *  Used to gate iPhone-only layout that must NOT affect iPad, in any orientation (unlike a width media
 *  query, which can't tell a landscape iPhone from a tablet). */
export const isPhone = /iPhone|iPod/i.test(ua);

/** True on a tablet (iPad, and any future Android tablet): mobile build that is not a phone. UA-based,
 *  so it is orientation-independent — a landscape iPhone (wide viewport) never counts as a tablet. Used
 *  to gate tablet-only layout that must NOT affect the phone. */
export const isTablet = isMobile && !isPhone;

/** True when running inside the macOS WebView (WKWebView) — used to mirror native window-control
 *  placement and drive the native-capture backend (AVFoundation). Excludes iPadOS, which shares the
 *  "Macintosh" user-agent. */
export const isMacOS = /Macintosh|Mac OS X/i.test(ua) && !isIOS;

/** True on the Windows WebView (WebView2) — drives the native-capture backend (DirectShow). */
export const isWindows = /Windows/i.test(ua);

/** True on the Linux WebView (WebKitGTK; excludes Android) — drives the native-capture backend (V4L2). */
export const isLinux = /Linux/i.test(ua) && !/Android/i.test(ua);

/** True on WebKitGTK specifically — the Linux WebView, as opposed to macOS's WKWebView (a different
 *  WebKit port on Core Animation) or Chromium-based WebView2.
 *
 *  Drives the hard-blink indicator mode. On WebKitGTK a looping CSS animation makes the compositor
 *  rebuild the entire window every frame: the cost is per frame produced, not per pixel changed, so a
 *  single 6-pixel dot measured ~46 % of a core. It is unaffected by the element's size, by
 *  `will-change`, by `steps()` (which quantises the value, not the frame production), and even by
 *  whether the element is on screen at all — a marker panned to the other side of the world costs
 *  exactly the same. Only producing fewer frames helps: 5 Hz → 23 %, 2 Hz → 14 %, 1 Hz → 6 %.
 *
 *  Neither WebView2 nor macOS shows this, so both keep the smooth animations. */
export const isWebKitGtk = isLinux;

/** True on any WebKit-based WebView — WebKitGTK on Linux and WKWebView on macOS, which are different
 *  ports of the same WebCore and so share its resource loader.
 *
 *  Drives the `?raw=1` request of the off-thread MJPEG reader. WebKit handles
 *  `multipart/x-mixed-replace` inside that loader and never exposes it to `fetch`: measured on
 *  WebKitGTK 2.52.5 from a `tauri://localhost` page, the response headers arrive and the first
 *  `reader.read()` fails with `Load failed` at zero bytes — main thread and worker alike, which is
 *  what silently pushed Linux back onto the `<img>` sink. The identical bytes under another content
 *  type stream perfectly. WebView2 reads multipart directly and is deliberately left untouched. */
export const isWebKit = isLinux || isMacOS;

// Tag the document root on mobile so global CSS can add bottom breathing room. The bottom edge is
// crowded there: the on-screen RC sticks, the map zoom/compass buttons, the Leaflet attribution label
// and the iPad home indicator all fight for the same strip. `--safe-bottom` exposes the device safe
// area (needs viewport-fit=cover in the viewport meta) for those bottom-anchored overlays to lift by.
if (typeof document !== 'undefined' && isMobile) {
  document.documentElement.classList.add('is-mobile');
  // Separate iPhone tag so iPhone-only rules (that must not touch iPad) can key off it in any orientation.
  if (isPhone) document.documentElement.classList.add('is-phone');
  // Separate tablet tag (iPad, future Android tablets) for tablet-only rules that must not touch the phone.
  else document.documentElement.classList.add('is-tablet');
  document.documentElement.style.setProperty('--safe-bottom', 'env(safe-area-inset-bottom, 0px)');
  // `--safe-top` covers the status bar / notch strip so the toolbar and top-anchored map layers
  // are not overlaid by the iOS status bar (clock, battery). Same viewport-fit=cover requirement.
  document.documentElement.style.setProperty('--safe-top', 'env(safe-area-inset-top, 0px)');
  // In landscape the notch / Dynamic Island sits on a side edge, over the left-anchored nav rail and
  // panels. `--safe-left` / `--safe-right` let those shift clear of it (both 0 in portrait).
  document.documentElement.style.setProperty('--safe-left', 'env(safe-area-inset-left, 0px)');
  document.documentElement.style.setProperty('--safe-right', 'env(safe-area-inset-right, 0px)');
}
