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
