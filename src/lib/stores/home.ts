// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Home position store — set on arm + GPS fix, read by widgets and map

import { writable } from 'svelte/store';

export interface HomePosition {
  lat: number;
  lon: number;
  alt: number;
  set: boolean;
}

export const homePosition = writable<HomePosition>({
  lat: 0,
  lon: 0,
  alt: 0,
  set: false,
});
