// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Convert a waypoint's altitude (cm) between REL / AMSL / AGL using terrain + the launch point as the
// home reference. null when a needed ground reference is missing; the callers keep the number.

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { launchPoint, toDeg, ALT_MODE_REL, ALT_MODE_AGL, type Waypoint } from '$lib/stores/mission';

/** Terrain elevation (m, ≈ MSL) at lat/lon via backend, or null. */
async function terrainElev(lat: number, lon: number): Promise<number | null> {
  try {
    return await invoke<number | null>('terrain_elevation', { lat, lon });
  } catch {
    return null;
  }
}

/** Whether the ground elevation under the waypoint is known — the condition for an AGL altitude. */
export async function hasTerrain(wp: Waypoint): Promise<boolean> {
  return (await terrainElev(toDeg(wp.lat), toDeg(wp.lon))) != null;
}

/** The mode after `cur` in the REL → AMSL → AGL cycle. AGL is left out for a waypoint without terrain
 *  data: it could not be converted for the FC. */
export async function nextAltMode(wp: Waypoint, cur: number): Promise<number> {
  const next = (cur + 1) % 3;
  if (next !== ALT_MODE_AGL) return next;
  return (await hasTerrain(wp)) ? next : ALT_MODE_REL;
}

export async function convertAltCm(wp: Waypoint, fromMode: number, toMode: number): Promise<number | null> {
  if (fromMode === toMode) return wp.altitude;
  const valM = wp.altitude / 100;
  const lp = get(launchPoint);
  const needWpGround = fromMode === ALT_MODE_AGL || toMode === ALT_MODE_AGL;
  const needLaunch = fromMode === ALT_MODE_REL || toMode === ALT_MODE_REL;
  const wpGround = needWpGround ? await terrainElev(toDeg(wp.lat), toDeg(wp.lon)) : 0;
  const launchGround = needLaunch ? (lp ? await terrainElev(lp.lat, lp.lng) : null) : 0;
  if ((needWpGround && wpGround == null) || (needLaunch && launchGround == null)) {
    return null;
  }
  // to absolute MSL
  let absM: number;
  if (fromMode === ALT_MODE_REL) absM = (launchGround as number) + valM;
  else if (fromMode === ALT_MODE_AGL) absM = (wpGround as number) + valM;
  else absM = valM;
  // to target mode
  let outM: number;
  if (toMode === ALT_MODE_REL) outM = absM - (launchGround as number);
  else if (toMode === ALT_MODE_AGL) outM = absM - (wpGround as number);
  else outM = absM;
  return Math.round(outM * 100);
}
