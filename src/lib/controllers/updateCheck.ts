// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Startup update check (no auto-download by design — we point the user at the release notes). On launch
// we ask the backend for the relevant GitHub release (stable, or incl. pre-releases per the user's
// channel choice), compare it to the running version, and — if it's newer and not a version the user
// chose to skip — surface a one-shot prompt. The only persisted state is the skipped version, which we
// honour until a *higher* version appears. A failed check is logged and ignored; it never disrupts use.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { settings } from '$lib/stores/settings';
import { APP_VERSION } from '$lib/buildInfo';
import { compareVersions } from '$lib/utils/version';

/** Matches the Rust `UpdateInfo` (commands/update_check.rs). */
interface UpdateInfo {
  version: string;
  tag: string;
  url: string;
  name: string;
  prerelease: boolean;
}

/** A newer release the user should know about, or null when there's nothing to show. Drives UpdateDialog. */
export const pendingUpdate = writable<UpdateInfo | null>(null);

/** The running app version (for the dialog's "you have …" line). */
export const currentVersion = APP_VERSION;

/** Run once on startup. No-op when the check is disabled, the fetch fails, the latest isn't newer, or the
 *  user already skipped this (or an equal/higher) version. */
export async function runUpdateCheck(): Promise<void> {
  const cfg = get(settings).updateCheck;
  if (cfg.mode === 'disabled') return;

  let info: UpdateInfo | null;
  try {
    info = await invoke<UpdateInfo | null>('check_for_update', {
      includePrerelease: cfg.mode === 'prerelease',
    });
  } catch (e) {
    console.warn('[update] check failed:', e);
    return;
  }
  if (!info) return;

  // Only newer than what we run.
  if (compareVersions(info.version, APP_VERSION) <= 0) return;
  // Respect a skipped version — but resurface once something higher than it ships.
  if (cfg.skippedVersion && compareVersions(info.version, cfg.skippedVersion) <= 0) return;

  pendingUpdate.set(info);
}

/** "Open Release Page" — hand the release URL to the system browser, then dismiss. */
export async function openReleasePage(): Promise<void> {
  const info = get(pendingUpdate);
  if (info) {
    try { await openUrl(info.url); } catch (e) { console.warn('[update] open failed:', e); }
  }
  pendingUpdate.set(null);
}

/** "Remind me later" — dismiss without persisting; it'll prompt again next launch. */
export function remindLater(): void {
  pendingUpdate.set(null);
}

/** "Skip this version" — persist the version so it stays hidden until a higher one appears. */
export function skipVersion(): void {
  const info = get(pendingUpdate);
  if (info) {
    const cur = get(settings).updateCheck;
    settings.patch({ updateCheck: { ...cur, skippedVersion: info.version } });
  }
  pendingUpdate.set(null);
}
