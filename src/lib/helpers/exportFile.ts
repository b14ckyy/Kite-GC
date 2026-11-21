// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// One way out for every exported file. Desktop: the save dialog. Mobile: the file is written to the app
// cache and handed to the system share sheet — an export there is mostly a hand-over to another device
// or person. Dev-Docs ROADMAP "Mobile: every export goes through the OS share sheet".

import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { isAndroid, isIOS } from '$lib/platform';

export interface ExportFilter {
  name: string;
  extensions: string[];
}

export interface ExportRequest {
  title?: string;
  /** Proposed file name with extension. On mobile it IS the name: no dialog lets the user change it. */
  name: string;
  /** Desktop: the first filter's first extension must be the one of `name`. */
  filters: ExportFilter[];
  /** Writes the export to the path it is given (a real path on every platform). */
  write: (path: string) => Promise<unknown>;
}

/** The iOS share sheet is written but has not run on a device: off until it has. */
const IOS_SHARE_READY = false;

export const sharesExports = isAndroid || (isIOS && IOS_SHARE_READY);

/** Android sorts the share targets by MIME type; Kite's own formats have none, the generic one offers
 *  the widest list (file managers, nearby share, cloud drives, mail). */
const MIME: Record<string, string> = {
  csv: 'text/csv',
  gpx: 'application/gpx+xml',
  kml: 'application/vnd.google-earth.kml+xml',
  kmz: 'application/vnd.google-earth.kmz',
  txt: 'text/plain',
};
function mimeFor(name: string): string {
  const ext = /\.([a-z0-9]+)$/i.exec(name)?.[1].toLowerCase() ?? '';
  return MIME[ext] ?? 'application/octet-stream';
}

/** Characters no file system takes, out of a name the user typed. */
export function fileSafeName(name: string, fallback = 'export'): string {
  // eslint-disable-next-line no-control-regex
  return name.replace(/[\\/:*?"<>|\x00-\x1f]+/g, ' ').replace(/\s+/g, ' ').trim() || fallback;
}

/** Export through the save dialog (desktop) or the share sheet (mobile). Resolves to the path written,
 *  or null when the user cancelled the dialog. */
export async function exportFile(req: ExportRequest): Promise<string | null> {
  if (!sharesExports) {
    const path = await save({ title: req.title, defaultPath: req.name, filters: req.filters });
    if (!path) return null;
    await req.write(path);
    return path;
  }
  const path = await invoke<string>('export_scratch_path', { name: req.name });
  await req.write(path);
  await invoke('share_file', {
    path,
    mime: mimeFor(req.name),
    saveLabel: isAndroid ? get(t)('export.saveToDevice') : null,
  });
  if (isAndroid) watchSaveToDevice(req);
  return path;
}

let watch: ReturnType<typeof setInterval> | null = null;

/** Where a failure after the share sheet goes: by then the export call has long returned. */
let reportError: (message: string) => void = (m) => console.warn('[export]', m);
export function setExportErrorReporter(report: (message: string) => void): void {
  reportError = report;
}

/** The share sheet's own "save to device" action (Android 14+) only reports back when asked: poll for
 *  it while the sheet can still be open, then run the ordinary save dialog and copy the file. */
function watchSaveToDevice(req: ExportRequest): void {
  if (watch) clearInterval(watch);
  const until = Date.now() + 3 * 60 * 1000;
  const timer = setInterval(() => {
    void (async () => {
      if (Date.now() > until) {
        clearInterval(timer);
        return;
      }
      const src = await invoke<string | null>('export_take_save_request').catch(() => null);
      if (!src) return;
      clearInterval(timer);
      try {
        const dest = await save({ title: req.title, defaultPath: req.name, filters: req.filters });
        if (dest) await invoke('export_copy', { src, dest });
      } catch (e) {
        // The system has created the document already: without a word the user keeps an empty file.
        reportError(String(e));
      }
    })();
  }, 700);
  watch = timer;
}

/** Empty the export cache; the app that received the last export has long finished reading. */
export function clearExportCache(): void {
  if (sharesExports) void invoke('export_scratch_clear').catch(() => {});
}

/** File name stem for an export of ONE flight: vehicle name and location where known, else `Flight_<id>`,
 *  always followed by the flight's local date and time. */
export function flightFileStem(f: {
  id: number;
  craft_name?: string | null;
  location_name?: string | null;
  start_time?: string | null;
  utc_offset_min?: number | null;
}): string {
  // Of the location only the place: a geocoded name goes on with district and country.
  const parts = [f.craft_name ?? '', (f.location_name ?? '').replace(/,.*$/, '')]
    .map((p) => fileSafeName(p, ''))
    .filter((p) => p.length > 0);
  if (parts.length === 0) parts.push(`Flight_${f.id}`);
  const start = f.start_time ? new Date(f.start_time) : null;
  if (start && !Number.isNaN(start.getTime())) {
    // An unknown offset counts as 0, as in the logbook list: the name must show the time the list shows.
    const local = new Date(start.getTime() + (f.utc_offset_min ?? 0) * 60000).toISOString();
    parts.push(`${local.slice(0, 10)}_${local.slice(11, 13)}${local.slice(14, 16)}`);
  }
  return parts.join('_').replace(/\s+/g, '_');
}
