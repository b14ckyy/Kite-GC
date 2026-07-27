// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Effective low-power state, resolved from `settings.lowPower3D` (off / on / auto = only on battery).
//
// The setting started as the 3D render-rate cap (Map3D owns that side). This module exposes the same
// decision to the rest of the UI and mirrors it onto a root class, so plain CSS can opt out of
// expensive work without every component wiring up the setting + battery query itself.
//
// What that buys: the widget bars animate layout properties (`height` / `width` / `top` / `bottom`),
// and those force a reflow + repaint per frame. Idle that costs nothing — the transitions only run on
// a value change — but in flight throttle/acceleration/battery change continuously, so the transitions
// overlap into a permanent reflow at display rate for the whole flight. In low-power mode we drop the
// smoothing and let the bars snap to the current value instead.

import { readable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { settings } from './settings';

/** Root class mirroring `lowPowerActive` — CSS gates off it, e.g. `:global(html.kite-low-power)`. */
const ROOT_CLASS = 'kite-low-power';
/** AC/battery re-check interval. Matches how rarely that state changes; the query is a cheap sysfs read. */
const BATTERY_POLL_MS = 60_000;

/** True when low-power mode is in effect right now (`on`, or `auto` while running on battery). */
export const lowPowerActive = readable(false, (set) => {
  let mode: 'off' | 'on' | 'auto' = 'off';
  let onBattery = false;
  let timer: ReturnType<typeof setInterval> | undefined;

  const apply = () => {
    const active = mode === 'on' || (mode === 'auto' && onBattery);
    if (typeof document !== 'undefined') {
      document.documentElement.classList.toggle(ROOT_CLASS, active);
    }
    set(active);
  };

  const refreshBattery = async () => {
    try {
      onBattery = await invoke<boolean>('system_on_battery');
    } catch {
      onBattery = false; // detection unavailable → treat as AC, same as the 3D cap does
    }
    apply();
  };

  const unsubscribe = settings.subscribe((s) => {
    mode = s.lowPower3D ?? 'off';
    // Only `auto` depends on the battery state; poll for that mode alone.
    clearInterval(timer);
    timer = undefined;
    if (mode === 'auto') {
      void refreshBattery();
      timer = setInterval(() => void refreshBattery(), BATTERY_POLL_MS);
    } else {
      apply();
    }
  });

  return () => {
    unsubscribe();
    clearInterval(timer);
    if (typeof document !== 'undefined') document.documentElement.classList.remove(ROOT_CLASS);
  };
});
