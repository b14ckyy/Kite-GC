// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// AGL waypoints of the INAV mission whose terrain elevation is not available. INAV stores REL / AMSL
// only, so such a waypoint cannot be converted for the FC or a `.mission` file: the planner marks it
// and locks upload and export (the backend refuses them as well).

import { derived, get, writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import {
  mission, activeMissionIndex, missionCount, allMissionWaypoints, isModifier, hasLocation, toDeg, ALT_MODE_AGL,
} from './mission';

export interface AglTerrainGap {
  /** 1-based mission slot. */
  mission: number;
  /** 1-based position in the slot's waypoint list. */
  wpNumber: number;
  /** The number the planner shows (Jump, RTH and Set Heading are not counted). */
  shown: number;
}

export const aglTerrainGaps = writable<AglTerrainGap[]>([]);

/** `wpNumber`s of the gaps in the active mission. */
export const aglTerrainGapsActive = derived([aglTerrainGaps, activeMissionIndex], ([$gaps, $idx]) =>
  new Set($gaps.filter((g) => g.mission === $idx).map((g) => g.wpNumber)));

let run = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

async function check(): Promise<void> {
  const my = ++run;
  const asks: (AglTerrainGap & { lat: number; lon: number })[] = [];
  allMissionWaypoints().forEach((wps, m) => {
    let shown = 0;
    wps.forEach((w, k) => {
      if (!isModifier(w.action)) shown++;
      if ((w.alt_mode ?? 0) === ALT_MODE_AGL && hasLocation(w.action)) {
        asks.push({ mission: m + 1, wpNumber: k + 1, shown, lat: toDeg(w.lat), lon: toDeg(w.lon) });
      }
    });
  });
  if (asks.length === 0) {
    if (get(aglTerrainGaps).length > 0) aglTerrainGaps.set([]);
    return;
  }
  let elevations: (number | null)[];
  try {
    elevations = await invoke<(number | null)[]>('terrain_elevations', { points: asks.map((a) => [a.lat, a.lon]) });
  } catch {
    return;
  }
  if (my !== run) return;
  aglTerrainGaps.set(asks
    .filter((_, i) => elevations[i] == null)
    .map(({ mission: m, wpNumber, shown }) => ({ mission: m, wpNumber, shown })));
}

// A dragged waypoint writes the store many times a second.
function schedule(): void {
  clearTimeout(timer);
  timer = setTimeout(() => void check(), 300);
}

mission.subscribe(schedule);
missionCount.subscribe(schedule);
activeMissionIndex.subscribe(schedule);
