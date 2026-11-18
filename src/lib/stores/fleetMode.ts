// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Multi-vehicle ("fleet") feature gate (backend `fleet_mode.rs`): on in debug builds, else only with `--fleet` /
// `KITE_FLEET` (Dev-Docs active/GROUP_FLIGHTS.md §5e). Loaded once (`loadFleetMode`); switchable per session in
// a debug build. Without KITE_FLEET_RECORDING only the first vehicle is recorded (controllers/fleetRecordingNotice).

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

export const fleetEnabled = writable<boolean>(false);

/** Every vehicle of a fleet session is recorded (the backend's KITE_FLEET_RECORDING opt-in). */
export const fleetGroupRecording = writable<boolean>(false);

/** Read the launch flags from the backend (startup). */
export async function loadFleetMode(): Promise<void> {
  try {
    fleetEnabled.set(await invoke<boolean>('is_fleet_mode'));
    fleetGroupRecording.set(await invoke<boolean>('is_fleet_group_recording'));
  } catch (e) {
    console.warn('[fleet] is_fleet_mode failed:', e);
  }
}

/** Debug Monitor: switch the gate for this session (the backend refuses in a release build). */
export async function setFleetMode(on: boolean): Promise<void> {
  await invoke('set_fleet_enabled', { on });
  fleetEnabled.set(on);
  fleetGroupRecording.set(await invoke<boolean>('is_fleet_group_recording'));
}
