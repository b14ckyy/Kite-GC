// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Map search jump: how close the views go and the short-lived pin at the target (Map.svelte,
// Map3D.svelte; the request comes through stores/mapFocus).

/** 2D zoom for a bare point (coordinates, "to my location", a hit without a bounding box). */
export const SEARCH_POINT_ZOOM_2D = 16;
/** 2D: a fitted bounding box never zooms in further than this (a single building's box). */
export const SEARCH_MAX_ZOOM_2D = 17;
/** 2D: margin around a fitted bounding box (px). */
export const SEARCH_FIT_PAD_PX = 40;

/** 3D: radius (m) of the sphere framed around a bare point — the camera ends up ~1 km away. */
export const SEARCH_POINT_RADIUS_3D_M = 400;
/** 3D: a bounding box smaller than this radius (m) is framed as if it were this large. */
export const SEARCH_MIN_RADIUS_3D_M = 250;
/** 3D: camera pitch of the jump (looking down at the target). */
export const SEARCH_PITCH_3D_DEG = -50;
/** 3D: how long the jump waits for the terrain height at the target before flying anyway (ms). */
export const SEARCH_TERRAIN_WAIT_MS = 2000;

/** The pin stays fully visible this long, then fades out over SEARCH_PIN_FADE_MS. */
export const SEARCH_PIN_SHOW_MS = 4000;
export const SEARCH_PIN_FADE_MS = 1000;

/** Teardrop pin in the accent colour, tip at the bottom centre (26 × 36). */
export const SEARCH_PIN_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="26" height="36" viewBox="0 0 26 36">' +
  '<path d="M13 1.5C6.6 1.5 1.5 6.6 1.5 12.9c0 8.6 11.5 21.6 11.5 21.6s11.5-13 11.5-21.6C24.5 6.6 19.4 1.5 13 1.5z" ' +
  'fill="#37a8db" stroke="#fff" stroke-width="2"/>' +
  '<circle cx="13" cy="13" r="4.2" fill="#fff"/></svg>';
