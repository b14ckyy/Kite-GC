// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Dev-only widget gallery. The DEV branch is dropped from a production build, fixtures and all; there the route is a 404.
import { error } from '@sveltejs/kit';
import type { Component } from 'svelte';

export async function load(): Promise<{ Shots: Component }> {
  if (import.meta.env.DEV) {
    // The IPC mock must be in place before the widget modules load: some call the backend on import.
    (await import('$lib/dev/widgetFixtures')).installWidgetMocks();
    return { Shots: (await import('$lib/dev/WidgetShots.svelte')).default };
  }
  error(404, 'Not found');
}
