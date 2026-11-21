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

const _frameFleet = writable(0);

/** Monotonic counter; maps frame every connected vehicle (or the selected ones, when 2+ are selected)
 *  once when it increments. Acted on in free pan/look and in the fleet camera mode. */
export const frameFleetSignal = { subscribe: _frameFleet.subscribe };

/** Fire from the fleet panel's "Frame fleet" button. */
export function frameFleetOnMap(): void {
  _frameFleet.update((n) => n + 1);
}

/** A waypoint the pilot picked: from the mission list (centre it, or frame the mission when none of it
 *  is in view) or on its map marker (a follow camera stays on it, a free camera does not move). */
export interface WaypointFocus {
  lat: number;
  lon: number;
  source: 'list' | 'marker';
}

const _focusWaypoint = writable<(WaypointFocus & { seq: number }) | null>(null);
let focusSeq = 0;

export function focusWaypointOnMap(lat: number, lon: number, source: WaypointFocus['source']): void {
  _focusWaypoint.set({ lat, lon, source, seq: ++focusSeq });
}

/** New requests only: a view mounted later must not replay the last one. */
export function onWaypointFocus(fn: (f: WaypointFocus) => void): () => void {
  let seen = focusSeq;
  return _focusWaypoint.subscribe((f) => {
    if (!f || f.seq <= seen) return;
    seen = f.seq;
    fn(f);
  });
}
