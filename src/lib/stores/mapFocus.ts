// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// "Jump somewhere" requests for the map views (custom map "Show map area", map search); both maps subscribe
// (`onMapFocus`) and carry them out. A jump takes the camera from a follow mode first, or the next follow frame
// undoes it.

import { writable, get } from 'svelte/store';
import type { GeoBounds } from '$lib/stores/customMaps';

export interface MapFocusRequest {
  /** Who asked: the views treat them alike except where noted on the views' handlers. */
  source: 'customMap' | 'search';
  /** The area to fit, `[west, south, east, north]`; west > east = it crosses 180°. Null = a point. */
  bounds: GeoBounds | null;
  /** The target point: centre for a point request, and where the search's short-lived pin goes. */
  lat: number;
  lon: number;
}

const mapFocus = writable<(MapFocusRequest & { seq: number }) | null>(null);
let focusSeq = 0;

/** Ask the map views to jump. */
export function requestMapFocus(req: MapFocusRequest): void {
  mapFocus.set({ ...req, seq: ++focusSeq });
}

/** Subscribe to NEW jump requests only — a view mounted after a request (2D↔3D switch) must not
 *  replay the last one. */
export function onMapFocus(fn: (req: MapFocusRequest) => void): () => void {
  let seen = get(mapFocus)?.seq ?? 0;
  return mapFocus.subscribe((f) => {
    if (!f || f.seq <= seen) return;
    seen = f.seq;
    fn(f);
  });
}
