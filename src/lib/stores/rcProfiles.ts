// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC control profiles — user-managed, shareable config files (Documents/KiteGC/HID-Profiles/*.json,
// see src-tauri/src/hid/profiles.rs). A profile bundles the channel assignments/methods/behaviour and
// is NOT auto-linked to any device or FC — the user picks the active profile and the matching FC
// config themselves. This store mirrors the files and exposes load/save/delete + the working channel
// map currently being edited. See docs/active/RC_CONTROL.md §7.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

/** One HID-control → RC-channel binding. (Filled by the mapping UI in the next phase.) */
export interface RcBinding {
  /** Backend-stable code of the source control (axis/button/hat). */
  code: number;
  source: 'axis' | 'button' | 'hat';
  /** Invert the axis direction (axes only). */
  invert: boolean;
  /** Centre deadband, 0..1 (axes only). */
  deadband: number;
  /** Exponential curve strength, 0..1 (axes only); 0 = linear. */
  expo: number;
}

/** RC channel number (1..32) → its binding. */
export type RcChannelMap = Record<number, RcBinding>;

export interface RcProfile {
  /** Display name (also the basis of the on-disk filename). */
  name: string;
  /** Device the profile was built for — metadata only, never auto-applied. */
  deviceUuid: string | null;
  deviceName: string | null;
  /** Channel assignments/methods/behaviour. */
  channels: RcChannelMap;
}

/** All profiles found on disk (sorted by name). */
export const rcProfiles = writable<RcProfile[]>([]);
/** The channel config currently being edited — loaded from / saved to a profile; survives delete. */
export const currentChannels = writable<RcChannelMap>({});

/** (Re)load the profile list from disk. */
export async function loadProfiles(): Promise<void> {
  try {
    const raw = await invoke<string[]>('hid_profile_list');
    const parsed = raw
      .map((t) => {
        try {
          return JSON.parse(t) as RcProfile;
        } catch {
          return null;
        }
      })
      .filter((p): p is RcProfile => !!p && typeof p.name === 'string')
      .sort((a, b) => a.name.localeCompare(b.name));
    rcProfiles.set(parsed);
  } catch (e) {
    console.warn('[rc] loadProfiles failed', e);
  }
}

/** Save (create or overwrite) a profile file, then refresh the list. */
export async function saveProfile(profile: RcProfile): Promise<void> {
  await invoke('hid_profile_save', { name: profile.name, json: JSON.stringify(profile, null, 2) });
  await loadProfiles();
}

/** Delete a profile file by name, then refresh the list. */
export async function deleteProfile(name: string): Promise<void> {
  await invoke('hid_profile_delete', { name });
  await loadProfiles();
}

/** Absolute path of the profiles directory (for display). */
export async function profilesDir(): Promise<string> {
  try {
    return await invoke<string>('hid_profiles_dir');
  } catch {
    return '';
  }
}
