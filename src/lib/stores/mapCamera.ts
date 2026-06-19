// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Map camera command bus. Decouples the map components (which own the camera + view mode) from the
 * places that load a mission. A mission *load* should frame the mission once; a mission *edit* should
 * not — the maps can't tell those apart from the store alone, so load sites fire `frameMissionOnMap()`
 * explicitly. The maps act on it only when in free pan/look and not during a replay (the replay frames
 * its own track). "Go to UAV on connect" is handled inside the maps directly (they watch the
 * connection + the first GPS fix), so it needs no signal here.
 */

import { writable } from 'svelte/store';

const _frameMission = writable(0);

/** Monotonic counter; maps frame the current mission when it increments (ignoring the initial 0). */
export const frameMissionSignal = { subscribe: _frameMission.subscribe };

/** Fire after loading a mission onto the map (file / FC download / standalone library load / INAV
 *  multi-mission switch) — NOT when a mission is loaded as a replay-linked attachment. */
export function frameMissionOnMap(): void {
  _frameMission.update((n) => n + 1);
}
