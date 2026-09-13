// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Phone widget grid geometry (Dev-Docs active/PHONE_UI.md D2/D7). ONE place for the raster: the
// panel sizes its slots from `ROWS`, the packer (phase 2) lays widgets out on ROWS × MAX_COLS per
// page. Marc: the raster may end up 5 × 2 (smaller blocked area AND more widgets) — that is a change
// of these two numbers, nothing else may hard-code them.

/** Slot rows per page; one slot = usable panel height / ROWS. */
export const PHONE_GRID_ROWS = 4;
/** Widest the panel gets (it auto-narrows to 1 column when only S widgets are active and fit). */
export const PHONE_GRID_MAX_COLS = 2;
/** Widget pages the panel scrolls through (vertical swipe, snap). */
export const PHONE_GRID_PAGES = 2;

/** Padding (css px) between the column's glass edge and the tile area — the column panel lays
 *  the tiles out with it, and +page clips the swapped-in mini map to the same box. */
export const PHONE_GRID_PAD = 4;

// ── Bottom slots (Dev-Docs active/PHONE_BOTTOM_WIDGETS.md B1/B4) ──
/** Side slots: square, this fraction of the map viewport height. */
export const PHONE_BOTTOM_SIDE_FRAC = 0.2;
/** Centre slot: this fraction of the map viewport height; square or 2:1 wide. */
export const PHONE_BOTTOM_CENTRE_FRAC = 0.3;
/** Gap (css px) between two bottom tiles. */
export const PHONE_BOTTOM_GAP = 6;
/** Distance (css px) the bottom tiles keep from the arming pill and the map corner controls. */
export const PHONE_BOTTOM_CLEARANCE = 8;
