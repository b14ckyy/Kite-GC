// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Settings → Data: the whole database out to a file and back in, and every flight into one .kflight.
// All three go through the plain save / open dialog — files this large are not for the share sheet.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';
import { isMobile } from '$lib/platform';
import { formatBackupSize } from './dbBackupPrompt';
import { dbFolderState, flightCount as count, type DbFolderState } from './dbFolderSwitch';

export interface DbTransferUi {
  show: (opts: DialogOptions) => Promise<string | null>;
  tr: (key: string, values?: Record<string, string>) => string;
  /** A blocking notice while a transfer runs; null takes it down. */
  notice: (n: { title: string; message: string } | null) => void;
}

const K = 'settings.dbTransfer';
const stamp = () => new Date().toISOString().slice(0, 10);

/** `true` = written; `false` = cancelled or failed (the error is shown here). */
export async function exportDatabase(dbPath: string, ui: DbTransferUi): Promise<boolean> {
  const { show, tr, notice } = ui;
  const outputPath = await save({
    title: tr(`${K}.exportDb`),
    defaultPath: `KiteGC_database_${stamp()}.db`,
    filters: [{ name: tr(`${K}.dbFilter`), extensions: ['db'] }],
  });
  if (!outputPath) return false;
  notice({ title: tr(`${K}.exportDbBusyTitle`), message: tr(`${K}.busyMsg`) });
  try {
    await invoke('flightlog_export_database', { outputPath, dbPath });
    notice(null);
    await show({ title: tr(`${K}.exportDb`), message: tr(`${K}.exportDbDone`) });
    return true;
  } catch (e) {
    notice(null);
    await show({ title: tr(`${K}.exportDb`), message: String(e) });
    return false;
  }
}

/** Replace the database with a file the user picks. `true` = replaced: the caller reloads everything
 *  that came from the old one and runs a pending upgrade. */
export async function importDatabase(dbPath: string, ui: DbTransferUi): Promise<boolean> {
  const { show, tr, notice } = ui;
  // A mobile picker greys out what its filter does not know, and `.db` has no MIME type.
  const filePath = await open({
    multiple: false,
    filters: isMobile ? undefined : [{ name: tr(`${K}.dbFilter`), extensions: ['db'] }],
  });
  if (typeof filePath !== 'string' || !filePath) return false;
  notice({ title: tr(`${K}.importBusyTitle`), message: tr(`${K}.busyMsg`) });
  try {
    const [staged, current] = await Promise.all([
      invoke<DbFolderState>('flightlog_import_database_stage', { filePath, dbPath }),
      dbFolderState(dbPath),
    ]);
    notice(null);
    const older = staged.schema_version < staged.current_version;
    const answer = await show({
      title: tr(`${K}.importTitle`),
      message:
        tr(`${K}.importMsg`, { flights: count(staged), size: formatBackupSize(staged.size_bytes), curFlights: current.exists ? count(current) : '0' }) +
        (older ? '\n\n' + tr(`${K}.importOlder`) : ''),
      buttons: [{ label: tr(`${K}.importYes`), value: 'replace', danger: true }],
    });
    const replace = answer === 'replace';
    if (replace) notice({ title: tr(`${K}.importBusyTitle`), message: tr(`${K}.busyMsg`) });
    await invoke('flightlog_import_database_finish', { replace, dbPath });
    notice(null);
    return replace;
  } catch (e) {
    notice(null);
    await invoke('flightlog_import_database_finish', { replace: false, dbPath }).catch(() => {});
    await show({ title: tr(`${K}.importDb`), message: tr(`${K}.importFailed`, { error: String(e) }) });
    return false;
  }
}

/** Ask, then delete every original log file from the database (mobile: they came with a desktop
 *  database and only HI-RES replay uses them). */
export async function removeOriginalLogs(dbPath: string, files: number, sizeBytes: number, ui: DbTransferUi): Promise<void> {
  const { show, tr } = ui;
  const answer = await show({
    title: tr(`${K}.originalLogsTitle`),
    message: tr(`${K}.originalLogsMsg`, { count: String(files), size: formatBackupSize(sizeBytes) }),
    buttons: [{ label: tr(`${K}.originalLogsRemove`), value: 'remove', danger: true }],
  });
  if (answer !== 'remove') return;
  try {
    await invoke('flightlog_original_logs_remove', { dbPath });
  } catch (e) {
    await show({ title: tr(`${K}.originalLogsTitle`), message: String(e) });
  }
}

/** Runs a `.kflight` export under a blocking notice that counts the flights written. */
export async function withExportProgress<T>(
  ui: Pick<DbTransferUi, 'tr' | 'notice'>,
  title: string,
  total: number,
  run: () => Promise<T>,
): Promise<T> {
  const progress = (done: number, of: number) =>
    ui.notice({ title, message: ui.tr(`${K}.flightsBusyMsg`, { done: String(done), total: String(of) }) });
  progress(0, total);
  // An event can arrive after the command's answer: it must not put the notice back up.
  let running = true;
  const unlisten = await listen<{ exported: number; total: number }>('flightlog-export-progress', (e) => {
    if (running) progress(e.payload.exported, e.payload.total);
  });
  try {
    return await run();
  } finally {
    running = false;
    unlisten();
    ui.notice(null);
  }
}

/** Every flight into one `.kflight`, with the count shown while it runs. */
export async function exportAllFlights(dbPath: string, ui: DbTransferUi): Promise<void> {
  const { show, tr } = ui;
  const state = await dbFolderState(dbPath);
  if (state.flights === 0 || !state.exists) {
    await show({ title: tr(`${K}.exportFlights`), message: tr(`${K}.flightsNone`) });
    return;
  }
  const outputPath = await save({
    title: tr(`${K}.exportFlights`),
    defaultPath: `KiteGC_all_flights_${stamp()}.kflight`,
    filters: [{ name: tr('logbook.kflightFileFilter'), extensions: ['kflight'] }],
  });
  if (!outputPath) return;
  try {
    const result = await withExportProgress(ui, tr(`${K}.flightsBusyTitle`), state.flights ?? 0, () =>
      invoke<{ exported: number; total: number }>('flightlog_export_all', { outputPath, dbPath }));
    await show({
      title: tr(`${K}.exportFlights`),
      message: tr(`${K}.flightsDone`, { exported: String(result.exported), total: String(result.total) }),
    });
  } catch (e) {
    // `no-space:<needed>:<free>` — the marker of EXPORT_NO_SPACE_ERROR (commands/flightlog.rs).
    const space = /^no-space:(\d+):(\d+)$/.exec(String(e));
    const message = space
      ? tr(`${K}.noSpace`, { need: formatBackupSize(Number(space[1])), free: formatBackupSize(Number(space[2])) })
      : String(e);
    await show({ title: tr(`${K}.exportFlights`), message });
  }
}
