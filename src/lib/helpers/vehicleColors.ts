// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Per-vehicle accent colours for the multi-vehicle map: each known vehicle gets a stable colour by its
// position in the (link, sysid)-sorted vehicle list, so the marker, its guided-target line and the
// vehicle list entry all read as one thing. Distinct from the active vehicle's nav-state colouring and
// from the guided-target green (#59aa29) and radar palettes.

const PALETTE = [
  '#f5a623', // amber
  '#bd10e0', // violet
  '#50e3c2', // teal
  '#ff6b6b', // coral
  '#4a90e2', // blue
  '#b8e986', // lime
  '#f8e71c', // yellow
  '#ff9ff3', // pink
];

export function vehicleColor(index: number): string {
  return PALETTE[Math.abs(index) % PALETTE.length];
}
