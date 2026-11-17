// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// File-dialog filters and the format of a picked mission file. Linux pickers match extensions
// case-sensitively (GTK `GPatternSpec`, portal globs, no case-fold), hence `anyCase`.

import { isAndroid } from '$lib/platform';

/** Expand extensions to their lower- and uppercase spellings, preserving order and deduplicating. */
export function anyCase(extensions: string[]): string[] {
  const out: string[] = [];
  for (const ext of extensions) {
    for (const variant of [ext.toLowerCase(), ext.toUpperCase()]) {
      if (!out.includes(variant)) out.push(variant);
    }
  }
  return out;
}

/** Filters for an OPEN dialog. Android's picker filters by MIME type and Kite's own extensions have
 *  none: as soon as one extension of a list is known to Android (`txt`), every other file is greyed
 *  out. There the dialog gets no filter and the caller sorts out what was picked. */
export function openFilters(name: string, extensions: string[]): { name: string; extensions: string[] }[] | undefined {
  return isAndroid ? undefined : [{ name, extensions: anyCase(extensions) }];
}

export type MissionFileKind = 'kmission' | 'plan' | 'waypoints' | 'mission';

/** Which mission format a picked file is: by the extension of its `name`, and by content when the name has
 *  none (an Android provider need not give one). A foreign extension is refused unread — without a
 *  filter the Android picker also offers videos. Null = none of them. */
export async function missionFileKind(name: string, readText: () => Promise<string>): Promise<MissionFileKind | null> {
  const ext = /\.([a-z0-9]+)$/i.exec(name)?.[1].toLowerCase();
  if (ext === 'kmission' || ext === 'plan' || ext === 'waypoints' || ext === 'mission') return ext;
  if (ext === 'txt') return 'waypoints';
  if (ext !== undefined && ext !== 'json' && ext !== 'xml') return null;
  const head = (await readText()).trimStart().slice(0, 4096);
  if (head.startsWith('QGC WPL')) return 'waypoints';
  if (head.startsWith('<')) return 'mission';
  if (head.startsWith('{')) {
    if (/"format"\s*:\s*"kmission"/.test(head)) return 'kmission';
    if (/"fileType"\s*:\s*"Plan"/.test(head)) return 'plan';
  }
  return null;
}
