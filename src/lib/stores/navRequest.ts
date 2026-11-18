// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Nav-rail tab requests from components that don't own the page state (the fleet panel's "edit this
// vehicle's mission", the auto-switch to the Fleet tab on a multi-selection). +page.svelte subscribes
// and routes each request through its own `selectTab`, so the tab logic stays in one place.
// See docs/02-design/features/fleet-mission.design.md §2.2.

import { writable } from 'svelte/store';

export interface NavTabRequest {
  tabId: string;
  /** Monotonic, so the same tab can be requested twice in a row. */
  n: number;
}

const _req = writable<NavTabRequest | null>(null);
let counter = 0;

export const navTabRequest = { subscribe: _req.subscribe };

export function requestNavTab(tabId: string): void {
  _req.set({ tabId, n: ++counter });
}
