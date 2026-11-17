// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight-database upgrade (a blocking notice while it runs) and the question about its backup.
// A backup is identified by folder and modification time, so the next upgrade's backup is asked about
// again; Kite removes one only when the user chooses it.

import { invoke } from '@tauri-apps/api/core';
import { get } from 'svelte/store';
import { settings } from '$lib/stores/settings';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';

export interface DbBackupInfo {
  path: string;
  size_bytes: number;
  modified_ms: number;
}

/** An existing database whose format is not this build's: `from_version` below `to_version` can be
 *  upgraded, above it the file was written by a newer Kite and cannot be used. */
export interface DbSchemaState {
  from_version: number;
  to_version: number;
  size_bytes: number;
}

export type DbUpgradeOutcome = 'none' | 'done' | 'newer';

const REMIND_AFTER_MS = 14 * 24 * 60 * 60 * 1000;

export function formatBackupSize(bytes: number): string {
  return bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(2)} GB`
    : `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
}

/** Read-only look at the database of `dbPath`; null = none there, or already this build's format. */
export function probeDbSchema(dbPath: string): Promise<DbSchemaState | null> {
  return invoke<DbSchemaState | null>('flightlog_schema_state', { dbPath }).catch(() => null);
}

/** Run a pending upgrade now, with `notice` set while it runs — before anything else opens the database
 *  and upgrades it unseen. Rejects when the upgrade fails. */
export async function runPendingDbUpgrade(
  dbPath: string,
  notice: (pending: DbSchemaState | null) => void,
): Promise<DbUpgradeOutcome> {
  const state = await probeDbSchema(dbPath);
  if (!state) return 'none';
  if (state.from_version > state.to_version) return 'newer';
  notice(state);
  try {
    await invoke('flightlog_upgrade_run', { dbPath });
    return 'done';
  } finally {
    notice(null);
  }
}

/** Backups already asked about in this run: a dismissed dialog comes back at the next start. */
const asked = new Set<string>();

/** Ask about the backup next to the database of `dbPath`, unless that was answered or is not yet due. */
export async function checkDbBackupPrompt(
  dbPath: string,
  show: (opts: DialogOptions) => Promise<string | null>,
  tr: (key: string, values?: Record<string, string>) => string,
): Promise<void> {
  const info = await invoke<DbBackupInfo | null>('flightlog_backup_info', { dbPath }).catch(() => null);
  if (!info) return;
  const id = `${info.path}@${info.modified_ms}`;
  if (asked.has(id)) return;
  asked.add(id);
  const ack = get(settings).dbBackupAck?.[info.path];
  if (ack && ack.modifiedMs === info.modified_ms && (ack.remindAt === 0 || Date.now() < ack.remindAt)) return;

  const answer = await show({
    title: tr('settings.dbBackupPromptTitle'),
    message: tr('settings.dbBackupPromptMsg', { size: formatBackupSize(info.size_bytes), path: info.path }),
    buttons: [
      { label: tr('settings.dbBackupPromptDelete'), value: 'delete', danger: true },
      { label: tr('settings.dbBackupPromptKeep'), value: 'keep' },
      { label: tr('settings.dbBackupPromptRemind'), value: 'remind', primary: true },
    ],
  });
  const acks = { ...(get(settings).dbBackupAck ?? {}) };
  if (answer === 'delete') {
    try {
      await invoke('flightlog_backup_delete', { dbPath });
      delete acks[info.path];
    } catch (e) {
      await show({ title: tr('settings.dbBackupPromptTitle'), message: String(e) });
      return;
    }
  } else if (answer === 'keep') {
    acks[info.path] = { modifiedMs: info.modified_ms, remindAt: 0 };
  } else if (answer === 'remind') {
    acks[info.path] = { modifiedMs: info.modified_ms, remindAt: Date.now() + REMIND_AFTER_MS };
  } else {
    return;
  }
  settings.patch({ dbBackupAck: acks });
}
