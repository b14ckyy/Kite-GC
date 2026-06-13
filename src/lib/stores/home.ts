// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Home position store — read by the Home widget and the map's home/launch reference marker.
//
// `source` distinguishes an authoritative FC home (from MSP_WP 0 at connect, or the arm transition
// while connected) from a user-placed manual reference and a replay's start point:
//  - 'fc'     → locked: the reference marker is a green, non-draggable "H" pinned to home.
//  - 'manual' → the orange, draggable launch marker IS the home reference (the widget points to it).
//  - 'replay' → the replay flight's start position (shown as "H" during playback).

import { writable, derived } from 'svelte/store';

export type HomeSource = 'fc' | 'manual' | 'replay';

export interface HomePosition {
  lat: number;
  lon: number;
  alt: number;
  set: boolean;
  source: HomeSource;
}

export const homePosition = writable<HomePosition>({
  lat: 0,
  lon: 0,
  alt: 0,
  set: false,
  source: 'manual',
});

/** Authoritative FC home → the reference marker is the locked green "H" (launch "L" is hidden). */
export const homeLocked = derived(homePosition, ($h) => $h.set && $h.source === 'fc');

/** Whether to draw the dedicated green "H" home marker (FC home or replay; NOT a manual reference,
 *  which is represented by the draggable launch marker instead). */
export const homeMarkerShown = derived(
  homePosition,
  ($h) => $h.set && ($h.source === 'fc' || $h.source === 'replay'),
);
