// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The recording FILES of flights: copied beside a `.kflight` (folder `<name>_video`) and back, removed
// with a deleted flight, cleaned up when no flight knows them — each only when the user says so.
// Desktop only: a share-sheet export has no folder beside it, and mobile records nothing.

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isMobile } from '$lib/platform';
import { videoState } from '$lib/stores/video';
import { formatBackupSize } from './dbBackupPrompt';
import type { DbTransferUi } from './dbTransfer';

const K = 'logbook.videoTransfer';
/** File names listed in the clean-up question before "… and N more". */
const CLEANUP_NAMES_SHOWN = 8;

interface VideoSet {
  count: number;
  bytes: number;
  /** Export only: recordings the flights are linked to that are not below the recording folder. */
  missing: number;
  /** Import only: recordings of the imported flights that were below the recording folder already. */
  present: number;
  /** Clean-up only: the file names. */
  names: string[];
}

const where = (dbPath: string) => ({ dbPath: dbPath || undefined, videoDir: get(videoState).recordDir || undefined });

/** The folder name beside `kflightPath`, as the backend builds it (`video_files::sidecar_dir`). */
function videoFolderName(kflightPath: string): string {
  const file = kflightPath.split(/[\\/]/).pop() ?? kflightPath;
  return `${file.replace(/\.[^.]*$/, '')}_video`;
}

async function copyWithProgress(ui: DbTransferUi, total: number, run: () => Promise<VideoSet>): Promise<VideoSet> {
  const title = ui.tr(`${K}.busyTitle`);
  const progress = (done: number, of: number) =>
    ui.notice({ title, message: ui.tr(`${K}.busyMsg`, { done: formatBackupSize(done), total: formatBackupSize(of) }) });
  // An event can arrive after the command's answer: it must not put the notice back up.
  let running = true;
  let unlisten = () => {};
  try {
    progress(0, total);
    unlisten = await listen<{ done: number; total: number }>('flightlog-video-copy-progress', (e) => {
      if (running) progress(e.payload.done, e.payload.total);
    });
    return await run();
  } finally {
    running = false;
    unlisten();
    ui.notice(null);
  }
}

async function showFailure(ui: DbTransferUi, title: string, error: unknown): Promise<void> {
  // `no-space:<needed>:<free>` — the marker of EXPORT_NO_SPACE_ERROR (commands/flightlog.rs).
  const space = /^no-space:(\d+):(\d+)$/.exec(String(error));
  const message = space
    ? ui.tr(`${K}.noSpace`, { need: formatBackupSize(Number(space[1])), free: formatBackupSize(Number(space[2])) })
    : ui.tr(`${K}.failed`, { error: String(error) });
  await ui.show({ title, message });
}

/** After a `.kflight` was written to `kflightPath`: offer to put the flights' recordings beside it.
 *  `flightIds` null = every flight. Returns the line for the caller's result message, '' when nothing
 *  was copied. */
export async function offerVideoExport(
  ui: DbTransferUi,
  flightIds: number[] | null,
  kflightPath: string,
  dbPath: string,
): Promise<string> {
  if (isMobile) return '';
  try {
    const args = { flightIds, ...where(dbPath) };
    const found = await invoke<VideoSet>('flightlog_export_videos_info', args);
    if (found.count === 0) return '';
    let message = ui.tr(`${K}.exportMsg`, { count: String(found.count), size: formatBackupSize(found.bytes) });
    if (found.missing > 0) message += '\n\n' + ui.tr(`${K}.exportMissing`, { count: String(found.missing) });
    const answer = await ui.show({
      title: ui.tr(`${K}.exportTitle`),
      message,
      buttons: [{ label: ui.tr(`${K}.exportCopy`), value: 'copy', primary: true }],
      cancelLabel: ui.tr(`${K}.exportSkip`),
    });
    if (answer !== 'copy') return '';
    const copied = await copyWithProgress(ui, found.bytes, () =>
      invoke<VideoSet>('flightlog_export_videos', { ...args, outputPath: kflightPath }));
    return ui.tr(`${K}.exportDone`, { count: String(copied.count), folder: videoFolderName(kflightPath) });
  } catch (e) {
    await showFailure(ui, ui.tr(`${K}.exportTitle`), e);
    return '';
  }
}

/** After a `.kflight` at `filePath` was imported: offer to copy the recordings beside it into the
 *  recording folder, and say which of the imported flights found theirs there already. Returns the lines
 *  for the caller's result message, '' when there is nothing to say. */
export async function offerVideoImport(
  ui: DbTransferUi,
  filePath: string,
  importedIds: number[],
  dbPath: string,
): Promise<string> {
  if (isMobile) return '';
  const lines: string[] = [];
  try {
    const args = { filePath, ...where(dbPath) };
    const found = await invoke<VideoSet>('flightlog_import_videos_info', { ...args, importedIds });
    if (found.present > 0) lines.push(ui.tr(`${K}.importPresent`, { count: String(found.present) }));
    if (found.count === 0) return lines.join('\n');
    const answer = await ui.show({
      title: ui.tr(`${K}.importTitle`),
      message: ui.tr(`${K}.importMsg`, { count: String(found.count), size: formatBackupSize(found.bytes) }),
      buttons: [{ label: ui.tr(`${K}.importCopy`), value: 'copy', primary: true }],
      cancelLabel: ui.tr(`${K}.importSkip`),
    });
    if (answer !== 'copy') return lines.join('\n');
    const copied = await copyWithProgress(ui, found.bytes, () => invoke<VideoSet>('flightlog_import_videos', args));
    lines.push(ui.tr(`${K}.importDone`, { count: String(copied.count) }));
  } catch (e) {
    await showFailure(ui, ui.tr(`${K}.importTitle`), e);
  }
  return lines.join('\n');
}

/** Before flights are deleted (afterwards nothing names the files): offer to remove the recordings only
 *  they are linked to. The flights are deleted either way. */
export async function offerVideoDelete(ui: DbTransferUi, flightIds: number[], dbPath: string): Promise<void> {
  if (isMobile || flightIds.length === 0) return;
  try {
    const args = { flightIds, ...where(dbPath) };
    const found = await invoke<VideoSet>('flightlog_delete_videos_info', args);
    if (found.count === 0) return;
    const answer = await ui.show({
      title: ui.tr(`${K}.deleteTitle`),
      message: ui.tr(`${K}.deleteMsg`, { count: String(found.count), size: formatBackupSize(found.bytes) }),
      buttons: [{ label: ui.tr(`${K}.deleteYes`), value: 'delete', danger: true }],
      cancelLabel: ui.tr(`${K}.deleteKeep`),
    });
    if (answer === 'delete') await invoke<VideoSet>('flightlog_delete_videos', args);
  } catch (e) {
    console.warn('[video] recordings of the deleted flights not removed:', e);
  }
}

/** Video panel → clean up: list Kite's recordings no flight is linked to and remove them on a yes. */
export async function cleanupOrphanVideos(ui: DbTransferUi, dbPath: string): Promise<void> {
  const title = ui.tr(`${K}.cleanupTitle`);
  try {
    const found = await invoke<VideoSet>('flightlog_orphan_videos_info', where(dbPath));
    if (found.count === 0) {
      await ui.show({ title, message: ui.tr(`${K}.cleanupNone`) });
      return;
    }
    const shown = found.names.slice(0, CLEANUP_NAMES_SHOWN);
    const more = found.names.length - shown.length;
    if (more > 0) shown.push(ui.tr(`${K}.cleanupMore`, { count: String(more) }));
    const answer = await ui.show({
      title,
      message: ui.tr(`${K}.cleanupMsg`, {
        count: String(found.count),
        size: formatBackupSize(found.bytes),
        names: shown.join('\n'),
      }),
      buttons: [{ label: ui.tr(`${K}.cleanupYes`), value: 'delete', danger: true }],
    });
    if (answer !== 'delete') return;
    const gone = await invoke<VideoSet>('flightlog_orphan_videos_delete', where(dbPath));
    await ui.show({ title, message: ui.tr(`${K}.cleanupDone`, { count: String(gone.count), size: formatBackupSize(gone.bytes) }) });
  } catch (e) {
    await ui.show({ title, message: String(e) });
  }
}
