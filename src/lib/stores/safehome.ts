// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Safehome + fixed-wing autoland config (INAV). See docs/active/AUTOLAND_SAFEHOME.md.
//
// Mirrors the Rust `SafeHomeConfig` (commands/safehome.rs). `loaded` is the last snapshot read from the
// FC (rendered on the map); `working` is the editable copy in the Safe Home Manager. Edits are NOT sent
// live — "Save to FC" (`saveSafehomeConfig`) sends the whole working copy as one batch + EEPROM write,
// then re-reads. Loading is always-on at INAV connect; editing/saving is a ≥7.1 path (gated in the UI).

import { writable, derived, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

/** One safehome point (lat/lon in degrees × 1e7). */
export interface SafeHome {
  index: number;
  enabled: boolean;
  lat: number;
  lon: number;
}

/** Per-site fixed-wing approach config. Headings: positive = bidirectional, negative = exclusive, 0 = off.
 *  approach_direction: 0 = left turns, 1 = right turns. */
export interface FwApproach {
  index: number;
  approach_alt_cm: number;
  land_alt_cm: number;
  approach_direction: number;
  heading1: number;
  heading2: number;
  sea_level_ref: boolean;
}

/** Global autoland / safehome settings (null when the firmware lacks the setting). */
export interface AutolandSettings {
  approach_length_cm: number | null;
  pitch2throttle_mod: number | null;
  glide_alt_cm: number | null;
  flare_alt_cm: number | null;
  glide_pitch_deg: number | null;
  flare_pitch_deg: number | null;
  max_tailwind_cms: number | null;
  safehome_usage_mode: number | null;
  rth_allow_landing: number | null;
}

export interface SafeHomeConfig {
  safehomes: SafeHome[];
  approaches: FwApproach[];
  /** `safehome_max_distance` in cm (green ring; ÷100 for metres). */
  safehome_max_distance_cm: number | null;
  /** `nav_fw_loiter_radius` in cm (yellow ring; ÷100 for metres). */
  loiter_radius_cm: number | null;
  autoland: AutolandSettings;
  has_autoland: boolean;
}

/** A safehome slot is "empty" (unused) when its coordinates are 0/0 — even if stale approach data still
 *  sits in the matching fwapproach slot. Empty slots are hidden on the map and shown as addable in the
 *  editor. */
export function isSafehomeEmpty(sh: SafeHome): boolean {
  return sh.lat === 0 && sh.lon === 0;
}

/** Last snapshot read from the FC — drives the map overlay. Null until first read / when not INAV. */
export const safehomeConfig = writable<SafeHomeConfig | null>(null);

/** Editable working copy for the Safe Home Manager (deep clone of `loaded`; null until loaded). */
export const safehomeWorking = writable<SafeHomeConfig | null>(null);

/** Safe Home Manager open in the (INAV) mission slim panel — mirrors `missionManagerOpen`. */
export const safeHomeManagerOpen = writable<boolean>(false);

/** True when the working copy differs from the loaded snapshot (enables "Save to FC"). */
export const safehomeDirty = derived(
  [safehomeConfig, safehomeWorking],
  ([$loaded, $working]) =>
    !!$loaded && !!$working && JSON.stringify($loaded) !== JSON.stringify($working),
);

/** Read the full config from the FC (INAV only). Always called on connect (download always-on). On
 *  failure / non-INAV, clears to null. Resets the working copy to the fresh snapshot. */
export async function loadSafehomeConfig(): Promise<void> {
  try {
    const cfg = await invoke<SafeHomeConfig>('safehome_read_all');
    safehomeConfig.set(cfg);
    safehomeWorking.set(structuredClone(cfg));
  } catch (e) {
    console.warn('[safehome] loadSafehomeConfig failed', e);
    safehomeConfig.set(null);
    safehomeWorking.set(null);
  }
}

/** "Save to FC": send the working copy as one batch + EEPROM, then re-read so loaded == FC truth. */
export async function saveSafehomeConfig(): Promise<void> {
  const cfg = get(safehomeWorking);
  if (!cfg) return;
  await invoke('safehome_write_all', { config: cfg });
  await loadSafehomeConfig();
}

/** Discard pending edits — reset the working copy to the loaded snapshot. */
export function revertSafehomeWorking(): void {
  const loaded = get(safehomeConfig);
  safehomeWorking.set(loaded ? structuredClone(loaded) : null);
}

/** Clear everything (on disconnect). */
export function clearSafehome(): void {
  safehomeConfig.set(null);
  safehomeWorking.set(null);
  safeHomeManagerOpen.set(false);
}
