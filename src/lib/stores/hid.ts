// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// HID / joystick input store — mirrors the Rust `crate::hid` backend (docs/archive/MSP_RC_CONTROL.md).
// The backend thread streams the connected-device list (`hid-devices`) and the live raw axis/button
// state of the selected device (`hid-input`). This store wires those events into reactive state and
// exposes start/stop/select. Phase 1 carries RAW values only — channel mapping comes later.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** A connected input device (matches the Rust `HidDevice`). */
export interface HidDevice {
  id: number;
  name: string;
  /** SDL-style UUID hex — stable physical identity for persisted mappings. */
  uuid: string;
  axes: number;
  buttons: number;
}

/** One raw axis reading (`code` = stable platform event code, `value` = −1..1). */
export interface HidAxis {
  code: number;
  value: number;
}

/** One raw button reading (`value` = 0..1; analog triggers ramp, switches are 0/1). */
export interface HidButton {
  code: number;
  pressed: boolean;
  value: number;
}

/** One hat / POV switch (`x`/`y` ∈ −1, 0, 1; +y = up). */
export interface HidHat {
  code: number;
  x: number;
  y: number;
}

/** Live raw state of the selected device (matches the Rust `HidSnapshot`). */
export interface HidSnapshot {
  id: number;
  axes: HidAxis[];
  buttons: HidButton[];
  hats: HidHat[];
}

/** Connected devices, refreshed on hotplug. */
export const hidDevices = writable<HidDevice[]>([]);
/** Live raw state of the selected device, or null until the first frame arrives. */
export const hidSnapshot = writable<HidSnapshot | null>(null);
/** Whether the input thread is currently running. */
export const hidActive = writable<boolean>(false);

let unlistenDevices: UnlistenFn | null = null;
let unlistenInput: UnlistenFn | null = null;

/** Start the HID input thread and begin receiving device/input events. Safe to call repeatedly. */
export async function startHid(): Promise<void> {
  if (unlistenDevices) return; // already wired
  unlistenDevices = await listen<HidDevice[]>('hid-devices', (e) => hidDevices.set(e.payload));
  unlistenInput = await listen<HidSnapshot>('hid-input', (e) => hidSnapshot.set(e.payload));
  await invoke('hid_start');
  hidActive.set(true);
}

/** Stop the HID input thread and tear down listeners. */
export async function stopHid(): Promise<void> {
  await invoke('hid_stop');
  unlistenDevices?.();
  unlistenInput?.();
  unlistenDevices = null;
  unlistenInput = null;
  hidActive.set(false);
  hidSnapshot.set(null);
  hidDevices.set([]);
}

/** Choose which connected device to stream on `hid-input`. */
export async function selectHidDevice(id: number): Promise<void> {
  await invoke('hid_select_device', { id });
}
