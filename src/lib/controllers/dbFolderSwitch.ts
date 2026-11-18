// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Questions when the database folder changes. Only the database and its backup move; sessions and
// caches stay in the app's data folder (flightlog/db_move.rs).

import { invoke } from '@tauri-apps/api/core';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';
import { formatBackupSize } from './dbBackupPrompt';

export interface DbFolderState {
  path: string;
  exists: boolean;
  schema_version: number;
  current_version: number;
  size_bytes: number;
  flights: number | null;
}

/** `same` = both settings name one database; `use` = point Kite at the folder (a new database, or the
 *  one there); `move` = take the current one along. */
export type DbFolderPlan =
  | { action: 'cancel' }
  | { action: 'same' }
  | { action: 'use'; from: DbFolderState; to: DbFolderState }
  | { action: 'move'; overwrite: boolean };

type Show = (opts: DialogOptions) => Promise<string | null>;
type Tr = (key: string, values?: Record<string, string>) => string;

export function dbFolderState(dbPath: string): Promise<DbFolderState> {
  return invoke<DbFolderState>('flightlog_folder_state', { dbPath });
}

const count = (s: DbFolderState) => (s.flights == null ? '?' : String(s.flights));
const size = (s: DbFolderState) => formatBackupSize(s.size_bytes);

/** Ask what the change from folder `prev` to folder `next` should do. Nothing is written here. */
export async function planDbFolderSwitch(prev: string, next: string, show: Show, tr: Tr): Promise<DbFolderPlan> {
  const [from, to] = await Promise.all([dbFolderState(prev), dbFolderState(next)]);
  if (from.path === to.path) return { action: 'same' };
  const use: DbFolderPlan = { action: 'use', from, to };

  if (!to.exists) {
    if (!from.exists) return use;
    const answer = await show({
      title: tr('settings.dbMove.moveTitle'),
      message: tr('settings.dbMove.moveMsg', { flights: count(from), size: size(from) }),
      buttons: [
        { label: tr('settings.dbMove.startNew'), value: 'new' },
        { label: tr('settings.dbMove.move'), value: 'move', primary: true },
      ],
    });
    if (answer === 'move') return { action: 'move', overwrite: false };
    return answer === 'new' ? use : { action: 'cancel' };
  }

  const older = to.schema_version > 0 && to.schema_version < to.current_version;
  const newer = to.schema_version > to.current_version;
  // Not a database, damaged or locked: opening it would fail on every access.
  const unreadable = to.flights == null;
  if (!from.exists && !older && !newer && !unreadable) return use;

  const note = unreadable ? 'existsUnreadable' : newer ? 'existsNewer' : older ? 'existsOlder' : null;
  const buttons: DialogOptions['buttons'] = [];
  if (from.exists) buttons.push({ label: tr('settings.dbMove.overwrite'), value: 'overwrite', danger: true });
  if (!newer && !unreadable) {
    buttons.push({ label: tr(older ? 'settings.dbMove.upgradeOpen' : 'settings.dbMove.open'), value: 'open', primary: true });
  }
  const answer = await show({
    title: tr('settings.dbMove.existsTitle'),
    message:
      tr('settings.dbMove.existsMsg', { flights: count(to), size: size(to) }) +
      (from.exists ? '\n' + tr('settings.dbMove.existsCurrent', { flights: count(from), size: size(from) }) : '') +
      (note ? '\n\n' + tr(`settings.dbMove.${note}`) : '') +
      (from.exists ? '\n\n' + tr('settings.dbMove.existsOverwriteHint') : ''),
    buttons,
  });
  if (answer === 'open') return use;
  if (answer !== 'overwrite') return { action: 'cancel' };

  const sure = await show({
    title: tr('settings.dbMove.overwriteTitle'),
    message: tr('settings.dbMove.overwriteMsg', { flights: count(to), curFlights: count(from) }),
    buttons: [{ label: tr('settings.dbMove.overwrite'), value: 'overwrite', danger: true }],
  });
  return sure === 'overwrite' ? { action: 'move', overwrite: true } : { action: 'cancel' };
}

/** After another database was opened: back the previous one up into the new folder, keep it or delete it.
 *  `busy` is set while a backup is written. Dismissing the dialog keeps it. */
export async function askAboutPreviousDb(
  prev: string,
  next: string,
  from: DbFolderState,
  prevFolder: string,
  show: Show,
  tr: Tr,
  busy: (on: boolean) => void,
): Promise<void> {
  const answer = await show({
    title: tr('settings.dbMove.oldTitle'),
    message: tr('settings.dbMove.oldMsg', { flights: count(from), size: size(from), path: prevFolder }),
    buttons: [
      { label: tr('settings.dbMove.oldDelete'), value: 'delete', danger: true },
      { label: tr('settings.dbMove.oldBackup'), value: 'backup' },
    ],
    cancelLabel: tr('settings.dbMove.oldKeep'),
  });
  if (answer !== 'delete' && answer !== 'backup') return;
  if (answer === 'backup') busy(true);
  try {
    const copy = await invoke<string | null>('flightlog_db_retire', { old: prev, new: next, keepBackup: answer === 'backup' });
    busy(false);
    if (copy) await show({ title: tr('settings.dbMove.oldTitle'), message: tr('settings.dbMove.backupDone', { path: copy }) });
  } catch (e) {
    busy(false);
    await show({ title: tr('settings.dbMove.oldTitle'), message: tr('settings.dbMove.oldFailed', { error: String(e) }) });
  }
}
