// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Startup update check. On launch we ask the backend for the release relevant to the user's channel
// (stable, patch-only for the running minor, or incl. pre-releases), compare it to the running version,
// and — if it's newer and not a version the user chose to skip — surface a one-shot prompt with the
// release notes. Nothing is downloaded until the user picks "Update and Restart"; that hands the accepted
// release to the backend (commands/updater.rs), which installs it and relaunches. The only persisted
// state is the skipped version, honoured until a *higher* version appears. A failed check is logged and
// ignored; it never disrupts use.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { settings } from '$lib/stores/settings';
import { APP_VERSION, APP_STORE_URL, PLAY_STORE_URL } from '$lib/buildInfo';
import { isAndroid, isIOS } from '$lib/platform';
import { compareVersions } from '$lib/utils/version';

/** Matches the Rust `UpdateInfo` (commands/update_check.rs). */
export interface UpdateInfo {
  version: string;
  tag: string;
  url: string;
  name: string;
  prerelease: boolean;
  /** Release notes as written on GitHub (markdown). */
  body: string;
}

/** How the running copy can be updated (Rust `UpdateKind`): installed package, portable binary, or a
 *  mobile build that only links to its store. */
export type UpdateKind = 'installer' | 'portable' | 'mobile';

/** Rust `UpdateProgress`, emitted as `update-progress` during the install. */
interface UpdateProgress {
  downloaded: number;
  total: number | null;
  phase: 'download' | 'install';
}

/** Where the install stands after "Update and Restart" was pressed. `idle` until then; `failed` keeps the
 *  dialog open with the error and the release page as the way out. */
export interface InstallState {
  phase: 'idle' | 'download' | 'install' | 'failed';
  /** 0–100 while downloading with a known size; null when the size is unknown. */
  percent: number | null;
  error: string;
}

/** A newer release the user should know about, or null when there's nothing to show. Drives UpdateDialog. */
export const pendingUpdate = writable<UpdateInfo | null>(null);

/** The running copy's update kind; resolved once at the first check. */
export const updateKind = writable<UpdateKind>('installer');

export const installState = writable<InstallState>({ phase: 'idle', percent: null, error: '' });

/** The running app version (for the dialog's "you have …" line). */
export const currentVersion = APP_VERSION;

/** Run once on startup. No-op when the check is disabled, the fetch fails, the latest isn't newer, or the
 *  user already skipped this (or an equal/higher) version. */
export async function runUpdateCheck(): Promise<void> {
  const cfg = get(settings).updateCheck;
  if (cfg.mode === 'disabled') return;

  let info: UpdateInfo | null;
  try {
    info = await invoke<UpdateInfo | null>('check_for_update', { channel: cfg.mode });
  } catch (e) {
    console.warn('[update] check failed:', e);
    return;
  }
  if (!info) return;

  // Only newer than what we run.
  if (compareVersions(info.version, APP_VERSION) <= 0) return;
  // Respect a skipped version — but resurface once something higher than it ships.
  if (cfg.skippedVersion && compareVersions(info.version, cfg.skippedVersion) <= 0) return;

  try {
    updateKind.set(await invoke<UpdateKind>('update_kind'));
  } catch (e) {
    console.warn('[update] kind query failed:', e);
  }
  installState.set({ phase: 'idle', percent: null, error: '' });
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

/** The store listing for this mobile build, or null when none exists yet (the dialog then offers the
 *  release page instead). */
export function storeUrl(): string | null {
  if (isAndroid && PLAY_STORE_URL) return PLAY_STORE_URL;
  if (isIOS && APP_STORE_URL) return APP_STORE_URL;
  return null;
}

/** "Open in Play Store / App Store" — mobile's replacement for the in-app install. */
export async function openStore(): Promise<void> {
  const url = storeUrl();
  if (url) {
    try { await openUrl(url); } catch (e) { console.warn('[update] store open failed:', e); }
    pendingUpdate.set(null);
  } else {
    await openReleasePage();
  }
}

/** "Update and Restart" — download + install the pending release and relaunch. Resolves only on failure:
 *  on success the backend restarts the app. */
export async function installUpdate(): Promise<void> {
  const info = get(pendingUpdate);
  if (!info || get(installState).phase === 'download' || get(installState).phase === 'install') return;
  installState.set({ phase: 'download', percent: null, error: '' });

  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await listen<UpdateProgress>('update-progress', (ev) => {
      const p = ev.payload;
      const percent = p.total && p.total > 0 ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null;
      installState.set({ phase: p.phase, percent, error: '' });
    });
    await invoke('install_update', { tag: info.tag, version: info.version });
    // Only reached if the backend returned instead of restarting — treat like a failure so the user
    // is not left staring at a frozen dialog.
    installState.set({ phase: 'failed', percent: null, error: 'The installer did not restart the app' });
  } catch (e) {
    console.warn('[update] install failed:', e);
    installState.set({ phase: 'failed', percent: null, error: String(e) });
  } finally {
    unlisten?.();
  }
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
