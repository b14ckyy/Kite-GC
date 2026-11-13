// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Multi-vehicle ("fleet") feature gate — the backend's `fleet_mode.rs`: on in a debug build, off in a
// release build unless launched with `--fleet` / `KITE_FLEET` (Dev-Docs active/GROUP_FLIGHTS.md §5e: no
// setting, a launch flag). Off: the single-vehicle UI — no link manager, Fleet tab, group commands, fleet
// map layers or per-vehicle STATUSTEXT tags. Loaded once at startup (`loadFleetMode`); the Debug Monitor
// can switch it for the running session in a debug build (`setFleetMode`).
//
// Group recording (temporary, until the group end screen exists): without the KITE_FLEET_RECORDING
// developer opt-in a fleet session records only the first vehicle's flights — said once per app run
// when a second vehicle appears (controllers/fleetRecordingNotice).

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
