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
