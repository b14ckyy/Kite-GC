// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC control profiles — user-managed, shareable config files (Documents/KiteGC/HID-Profiles/*.json,
// see src-tauri/src/hid/profiles.rs). A profile bundles the channel assignments/methods/behaviour and
// is NOT auto-linked to any device or FC — the user picks the active profile and the matching FC
// config themselves. This store mirrors the files and exposes load/save/delete + the working channel
// map currently being edited. See docs/archive/MSP_RC_CONTROL.md §7.

import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import type { RcChannelMap } from '$lib/helpers/rcMethods';
import type { ManualMap } from './rcManual';

/** Which mapping model a profile holds. `channel` = INAV/ArduPilot channel methods (cross-compatible);
 *  `manual` = PX4 MANUAL_CONTROL raw mapping. The two are NOT interchangeable, so the profile dropdown
 *  only shows the kind matching the active platform. */
export type RcProfileKind = 'channel' | 'manual';

export interface RcProfile {
  /** Display name (also the basis of the on-disk filename). */
  name: string;
  /** Which platform group this profile targets — drives the dropdown filter. Older profiles lack it;
   *  `profileKind()` infers it from the payload for those. */
  kind?: RcProfileKind;
  /** Device the profile was built for — metadata only, never auto-applied. */
  deviceUuid: string | null;
  deviceName: string | null;
  /** Channel assignments/methods/behaviour (INAV / ArduPilot channel platforms). Set for `channel`. */
  channels?: RcChannelMap;
  /** PX4 MANUAL_CONTROL mapping (4 sticks + aux + buttons). Set for `manual`. */
  manual?: ManualMap;
}

/** A profile's kind — explicit when present, otherwise inferred for legacy files: a populated channel
 *  map means a channel (INAV/ArduPilot) profile; otherwise a stored manual map means a PX4 profile. */
export function profileKind(p: RcProfile): RcProfileKind {
  if (p.kind === 'channel' || p.kind === 'manual') return p.kind;
  const hasChannels = p.channels && Object.keys(p.channels).length > 0;
  if (hasChannels) return 'channel';
  return p.manual ? 'manual' : 'channel';
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
