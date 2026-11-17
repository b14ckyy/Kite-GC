// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Import of a `.kmission` file into the mission library. Identity and statistics come from the waypoints,
// never from the file. The extra data is stored as the text the backend read: it may be a newer Kite's.

import { missionFileRead, missionDbFindByHash, missionDbSave, missionDbSetExtra } from '$lib/stores/flightlog';
import { missionLibraryRevision } from '$lib/stores/missionManager';
import type { LibraryMissionInput } from '$lib/stores/flightlogTypes';
import type { Waypoint } from '$lib/stores/mission';
import type { ArduWaypoint } from '$lib/stores/missionArdupilot';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';
import { buildMissionInput } from '$lib/helpers/missionLibrary';
import { buildArduMissionInput } from '$lib/helpers/missionLibraryArdu';

export interface MissionFileImport {
  id: number;
  /** false = the mission was already in the library (matched by its waypoints). */
  added: boolean;
}

const isInt = (v: unknown): v is number => Number.isInteger(v);
const isNumOrNull = (v: unknown): boolean => v === null || (typeof v === 'number' && Number.isFinite(v));

/** The fields the editor and the backend need on every waypoint, per system — a list that fails here
 *  would sit in the library and never load. */
function checkWaypoints(list: unknown, mav: boolean): void {
  const ok = Array.isArray(list) && list.length > 0 && list.every((w) => {
    if (typeof w !== 'object' || w === null) return false;
    const r = w as Record<string, unknown>;
    return mav
      ? [r.command, r.frame, r.lat, r.lon].every(isInt) && typeof r.autocontinue === 'boolean'
        && [r.param1, r.param2, r.param3, r.param4, r.alt].every(isNumOrNull)
      : [r.number, r.action, r.lat, r.lon, r.altitude, r.p1, r.p2, r.p3, r.flag].every(isInt);
  });
  if (!ok) throw new Error('malformed waypoint list in .kmission file');
}

/** Key order and spacing of the stored text do not count. */
function canonical(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(canonical);
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    return Object.fromEntries(Object.keys(o).sort().map((k) => [k, canonical(o[k])]));
  }
  return v;
}
function sameJson(a: string | null, b: string | null): boolean {
  if (a == null || b == null) return a == b;
  try {
    return JSON.stringify(canonical(JSON.parse(a))) === JSON.stringify(canonical(JSON.parse(b)));
  } catch {
    return a === b;
  }
}

/** Import `path` into the library of `dbPath`. A mission that is already there gets the file's `extra`
 *  when it has none; when both carry one, `show` asks. Returns null when the user cancelled. */
export async function importMissionFile(
  path: string,
  dbPath: string,
  show: (opts: DialogOptions) => Promise<string | null>,
  tr: (key: string) => string,
): Promise<MissionFileImport | null> {
  const { file, extra_json: extra } = await missionFileRead(path);
  const body = file.mission;
  if (!['inav', 'ardupilot', 'px4'].includes(body.format)) throw new Error(`unknown mission system "${body.format}"`);
  const mav = body.format !== 'inav';
  checkWaypoints(body.waypoints, mav);

  const built: LibraryMissionInput = mav
    ? await buildArduMissionInput(body.waypoints as ArduWaypoint[], {
        name: body.name,
        notes: body.notes ?? null,
        format: body.format,
        vehicleClass: body.vehicle_class ?? null,
      })
    : await buildMissionInput(body.waypoints as Waypoint[], {
        name: body.name,
        notes: body.notes ?? null,
        sourceXml: body.source_xml ?? null,
        home: body.home_lat != null && body.home_lon != null ? { lat: body.home_lat, lng: body.home_lon } : null,
      });
  const input: LibraryMissionInput = {
    ...built,
    extra_json: extra,
    created_at: body.created_at ?? null,
    location_name: body.location_name ?? null,
  };

  const existing = await missionDbFindByHash(input.content_hash, dbPath);
  if (!existing) {
    const id = await missionDbSave(input, dbPath);
    missionLibraryRevision.update((n) => n + 1);
    return { id, added: true };
  }

  if (extra != null && !sameJson(existing.extra_json, extra)) {
    if (existing.extra_json != null) {
      const answer = await show({
        title: tr('missionMgr.extraConflictTitle'),
        message: tr('missionMgr.extraConflictMsg'),
        buttons: [
          { label: tr('missionMgr.extraConflictKeep'), value: 'keep' },
          { label: tr('missionMgr.extraConflictOverwrite'), value: 'overwrite', danger: true },
        ],
      });
      if (answer == null) return null;
      if (answer === 'keep') return { id: existing.id, added: false };
    }
    await missionDbSetExtra(existing.id, extra, dbPath);
  }
  return { id: existing.id, added: false };
}
