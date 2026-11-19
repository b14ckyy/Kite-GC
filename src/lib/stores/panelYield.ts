// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// A panel action that continues on the map (place a point, drag a marker, edit a zone) asks the panel
// to get out of the way. The page owns the geometry and decides; the panel stays on standby and comes
// back with a tap on its rail button, never on its own.

/** Share of the map area's width a panel may cover before such an action hides it. A tuning value. */
export const PANEL_YIELD_MAP_FRACTION = 0.3;

let handler: (() => void) | null = null;

/** The page registers how a request is answered; returns the unregister function. */
export function onMapAccessRequested(h: () => void): () => void {
  handler = h;
  return () => { if (handler === h) handler = null; };
}

export function requestMapAccess(): void {
  handler?.();
}
