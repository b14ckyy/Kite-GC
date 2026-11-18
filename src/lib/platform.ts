// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Lightweight host-OS detection for platform-specific UI, from the WebView user-agent rather than
// @tauri-apps/plugin-os: the string is stable per platform (WKWebView "Macintosh", WebView2 "Windows",
// WebKitGTK "Linux") and a sync value needs no dependency or async round-trip.

const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';

// iPadOS reports a desktop "Macintosh" user-agent (has done since iPadOS 13), so the UA alone cannot
// tell an iPad from a Mac. A multi-touch pointer does: Macs have no touchscreen (maxTouchPoints 0),
// iPads/iPhones report > 1. We use that to disambiguate.
const hasTouch = typeof navigator !== 'undefined' && navigator.maxTouchPoints > 1;

/** True on iOS / iPadOS (iPhone/iPod UA, or a touch-capable "Macintosh" which is really an iPad - this
 *  also matches the iOS Simulator, whose WKWebView reports a "Macintosh" UA with a multi-touch pointer). */
export const isIOS = /iPad|iPhone|iPod/i.test(ua) || (/Macintosh|Mac OS X/i.test(ua) && hasTouch);

/** True in the Android WebView ("Mozilla/5.0 (Linux; Android 14; …)" — which is why the Linux flag
 *  below excludes Android explicitly). */
export const isAndroid = /Android/i.test(ua);

/** True on any mobile build (iOS/iPadOS or Android). FORM-FACTOR tier only (touch layout, safe areas,
 *  mobile defaults) — never gate a capability on it: use the `has*` flags (Android has USB serial, iOS none). */
export const isMobile = isIOS || isAndroid;

/** Known ground-station hardware, from the `KiteDevice/<name>` token the Android app module appends
 *  to the WebView user-agent (DeviceProfile.kt — the identification rule lives there, in one place).
 *  `null` on every device it does not recognise, which is every stock phone and tablet. */
export const deviceProfile: string | null = (() => {
  const m = /KiteDevice\/([A-Za-z0-9_-]+)/.exec(ua);
  return m ? m[1] : null;
})();

/** RadioMaster AX12: its density setting puts it on either side of Chromium's 600 dp "Mobile"
 *  threshold, so the profile forces the phone chrome on this handheld. */
export const isAx12 = deviceProfile === 'ax12';

/** True on a phone: iPhone/iPod, Android with the "Mobile" UA token (absent on tablets), or a known
 *  handheld transmitter. UA-based, so orientation-independent. Gates phone-only layout that must
 *  NOT affect tablets. */
export const isPhone = /iPhone|iPod/i.test(ua) || (isAndroid && /Mobile/i.test(ua)) || isAx12;

/** True on a tablet (iPad, Android tablets): a mobile build that is not a phone. */
export const isTablet = isMobile && !isPhone;

// ── Capabilities — what the platform can do, independent of its form factor ───────────────────

/** Serial ports exist: every desktop OS, and Android through the USB host API (OTG). iOS is the one
 *  platform where the serial transport is genuinely absent — not hidden, absent. */
export const hasSerialPorts = !isIOS;

/** Video sources exist. Not on iPhone: on iOS video is iPad-only (RTSP_APPLE.md). */
export const hasVideo = !(isIOS && isPhone);

/** "Compact Database" (full VACUUM): desktop only. On Android SQLite keeps the VACUUM's working copy in memory,
 *  which a large DB on a small tablet may not survive, and a mobile DB holds no log blobs anyway. The backend
 *  refuses it on mobile too. */
export const hasDbCompact = !isMobile;

/** Custom (offline) maps from a local tile folder: desktop only. On Android and iOS the app cannot
 *  freely read a user-chosen folder tree (scoped storage / sandbox), so the selector entries, the
 *  Settings row and the coverage frames are hidden there; the backend refuses the commands too. */
export const hasCustomMaps = !isMobile;

/** RC control (the RC tool and its Settings switch) is desktop only: no joystick on a phone or tablet,
 *  touch sticks are too easy to hit by accident, and the channels cannot be configured there. */
export const hasRcControl = !isMobile;

/** True when running inside the macOS WebView (WKWebView) — used to mirror native window-control
 *  placement and drive the native-capture backend (AVFoundation). Excludes iPadOS, which shares the
 *  "Macintosh" user-agent. */
export const isMacOS = /Macintosh|Mac OS X/i.test(ua) && !isIOS;

/** True on the Windows WebView (WebView2) — drives the native-capture backend (DirectShow). */
export const isWindows = /Windows/i.test(ua);

/** True on the Linux WebView (WebKitGTK; excludes Android) — drives the native-capture backend (V4L2). */
export const isLinux = /Linux/i.test(ua) && !/Android/i.test(ua);

/** True on WebKitGTK. Drives the hard-blink indicator mode: there a looping CSS animation makes the compositor
 *  rebuild the whole window every frame (~46 % of a core for one 6-px dot, regardless of size, will-change,
 *  steps() or visibility); only fewer frames help (1 Hz → 6 %). WebView2 and macOS keep smooth animations. */
export const isWebKitGtk = isLinux;

/** True on any WebKit WebView (WebKitGTK, WKWebView on macOS/iOS — one shared resource loader). Drives the
 *  MJPEG reader's `?raw=1` request: WebKit handles `multipart/x-mixed-replace` in that loader and the first
 *  `fetch` read fails at zero bytes; the same bytes under another content type stream fine. */
export const isWebKit = isLinux || isMacOS || isIOS;

// Tag the document root on mobile so global CSS can add bottom breathing room (RC sticks, map buttons,
// attribution and the home indicator share the strip). `--safe-bottom` exposes the safe area (needs
// viewport-fit=cover).
if (typeof document !== 'undefined' && isMobile) {
  document.documentElement.classList.add('is-mobile');
  // Separate iPhone tag so iPhone-only rules (that must not touch iPad) can key off it in any orientation.
  if (isPhone) document.documentElement.classList.add('is-phone');
  // Separate tablet tag (iPad, future Android tablets) for tablet-only rules that must not touch the phone.
  else document.documentElement.classList.add('is-tablet');
  // Known hardware (`data-device="ax12"`) for device-specific rules; absent on stock devices.
  if (deviceProfile) document.documentElement.dataset.device = deviceProfile;
  document.documentElement.style.setProperty('--safe-bottom', 'env(safe-area-inset-bottom, 0px)');
  // `--safe-top` covers the status bar / notch strip so the toolbar and top-anchored map layers
  // are not overlaid by the iOS status bar (clock, battery). Same viewport-fit=cover requirement.
  document.documentElement.style.setProperty('--safe-top', 'env(safe-area-inset-top, 0px)');
  // In landscape the notch / Dynamic Island sits on a side edge: `--safe-left` / `--safe-right` move the
  // rail and panels clear of it. Ignored on Android, which reports a small punch-hole as a full left
  // inset (52 css px); the hidden-bar insets top/right stay.
  document.documentElement.style.setProperty('--safe-left', isAndroid ? '0px' : 'env(safe-area-inset-left, 0px)');
  document.documentElement.style.setProperty('--safe-right', 'env(safe-area-inset-right, 0px)');
}

// Replay player width, read by LogPlayer, the stick overlay and +page (a phone too narrow for it slides
// the widget column aside). Narrow on the phone (~760 css px map), wider on the desktop.
export const logPlayerWidth = isPhone ? 480 : 640;
if (typeof document !== 'undefined') {
  document.documentElement.style.setProperty('--log-player-w', `${logPlayerWidth}px`);
}
