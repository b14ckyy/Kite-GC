// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/** Copy `text` to the system clipboard. The async Clipboard API first (WebView2 / WKWebView / Chromium
 *  on Android — the call happens inside a click, so the user-gesture rule is met); the legacy
 *  `execCommand('copy')` path as a fallback for a WebKitGTK build that refuses the API. */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch (e) {
    console.warn('[clipboard] navigator.clipboard failed, trying execCommand', e);
  }
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  } catch (e) {
    console.warn('[clipboard] execCommand copy failed', e);
    return false;
  }
}
