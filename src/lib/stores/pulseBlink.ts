// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Hard-blink mode for the app's looping status indicators — WebKitGTK and mobile.
//
// On WebKitGTK a looping CSS animation makes the compositor rebuild the whole window every frame. The
// cost is per frame produced, not per pixel changed, so the GCS live dot — eight pixels — measured
// ~46 % of a core, and stayed there when panned right out of the viewport. `will-change` changed
// nothing, `steps()` changed nothing (it quantises the value, not the frame production), and CSS has
// no frame-rate control at all. Measured on the same element: 60 Hz → 46 %, 5 Hz → 23 %, 2 Hz → 14 %,
// 1 Hz → 6 %. Only producing fewer frames helps.
//
// Android's WebView is Chromium and animates the dot on the compositor thread, exactly like Windows —
// but the FIXED cost per produced frame (every backdrop-filter surface re-samples its backdrop, and
// the Mali driver marshals a full composition pass) is what a mobile GPU cannot hide. Measured on the
// Teclast M11 (1920×1200): the same 8 px dot alone = ~150 % of a core (RenderThread 74 % + Mali
// driver 54 % + Viz 26 %); dot hidden = ~4 %. Same defect economics as WebKitGTK, same cure.
// iOS is gated precautionarily (user decision, unmeasured): a mobile GPU paying a fixed price per
// composited frame is the expected economics there too, and nobody has a device on hand to prove
// otherwise — better a hard blink than a hot tablet.
//
// So the indicators stop animating and follow this clock instead: one interval for the whole app,
// toggling one class on the root element. That is deliberate — a frame carries every change at once,
// so ten blinking indicators cost the same as one, and independent per-element timers would land in
// separate frames and become additive. Verified on a Debian laptop: global system load with the GCS
// dot live fell from 15-17 % to 3-4 %.
//
// A root class rather than component state because the map indicators are Leaflet divIcons built as
// HTML strings — they cannot carry a Svelte class binding, but CSS reaches them fine.
//
// Not tied to power-save on purpose: that setting also caps the 3D view at 20 fps, which is a very
// different trade (Cesium renders on the GPU). Nobody should have to accept a stuttering globe to get
// sane indicator cost. Windows and macOS never enter this mode and keep the smooth pulses.

import { isMobile, isWebKitGtk } from '../platform';

/** Set for the whole session on WebKitGTK/mobile; CSS gates `animation: none` on it. */
const MODE_CLASS = 'kite-blink-mode';
/** Toggled at BLINK_INTERVAL_MS; CSS gates the "bright" half of each indicator on it. */
const BLINK_CLASS = 'kite-blink';
/** Half-period — a full on/off cycle takes 2 s. Slow on purpose: cost scales with the flip rate, and
 *  this is where it stops mattering while still reading as a live indicator rather than a glitch. */
const BLINK_INTERVAL_MS = 1000;

let timer: ReturnType<typeof setInterval> | undefined;

/** Start the blink clock. No-op outside WebKitGTK/mobile, and idempotent. Returns a teardown fn. */
export function initPulseBlink(): () => void {
  if ((!isWebKitGtk && !isMobile) || typeof document === 'undefined' || timer) return () => {};
  const root = document.documentElement;
  root.classList.add(MODE_CLASS, BLINK_CLASS);
  let on = true;
  timer = setInterval(() => {
    on = !on;
    root.classList.toggle(BLINK_CLASS, on);
  }, BLINK_INTERVAL_MS);
  return () => {
    clearInterval(timer);
    timer = undefined;
    root.classList.remove(MODE_CLASS, BLINK_CLASS);
  };
}
