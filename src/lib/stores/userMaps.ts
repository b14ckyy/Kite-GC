// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// User online maps (Settings → Custom Online Provider, all platforms): persisted in `settings.userMapProviders`,
// each a `user:<id>` provider resolved like a built-in, tiles via the normal online path. The template may carry
// the user's API key: never log it — log the id/name, or `templateHost`.

import { get } from 'svelte/store';
import { settings } from '$lib/stores/settings';
import { setUserProviders, userProvider, type UserMapSpec } from '$lib/config/mapProviders';
import { loadTileBytes } from '$lib/cache/tileLoader';
import { fillTileTemplate, templateHost, tileAt } from '$lib/helpers/tileTemplate';

let syncedList: UserMapSpec[] | null = null;

/** Mirror the persisted list into the provider registry. Idempotent per list object, so every reader
 *  (this module's subscription, `watchMapProvider`) can call it before resolving an id without relying
 *  on the order the settings subscribers run in. */
export function syncUserMapRegistry(list: UserMapSpec[]): void {
  if (list === syncedList) return;
  syncedList = list;
  setUserProviders(list.map(userProvider));
}

// Filled synchronously at import (the settings store is loaded from localStorage at creation), so a
// map view that resolves a persisted `user:` id on mount already finds it.
settings.subscribe((s) => syncUserMapRegistry(s.userMapProviders));

/** Add a map (a new stable id) or replace the one with the same id. */
export function saveUserMap(spec: Omit<UserMapSpec, 'id'> & { id?: string }): void {
  const saved: UserMapSpec = { ...spec, id: spec.id || crypto.randomUUID() };
  settings.update((s) => {
    const list = s.userMapProviders;
    const exists = list.some((m) => m.id === saved.id);
    return {
      ...s,
      userMapProviders: exists ? list.map((m) => (m.id === saved.id ? saved : m)) : [...list, saved],
    };
  });
}

/** Remove a map. A selected one falls back like any unknown provider id (the selection is kept). */
export function deleteUserMap(id: string): void {
  settings.update((s) => ({ ...s, userMapProviders: s.userMapProviders.filter((m) => m.id !== id) }));
}

export type UserMapTestResult =
  | { ok: true; z: number; x: number; y: number; width: number; height: number }
  | { ok: false; z: number; x: number; y: number; error: string }
  | { ok: false; z: number; x: number; y: number; notImage: true };

/** Zoom of the test tile: detailed enough to need a valid key, coarse enough to exist almost anywhere. */
const TEST_ZOOM = 12;

function decodeImageSize(buf: ArrayBuffer): Promise<{ width: number; height: number }> {
  const url = URL.createObjectURL(new Blob([buf]));
  return new Promise<{ width: number; height: number }>((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => reject(new Error('not an image'));
    img.src = url;
  }).finally(() => URL.revokeObjectURL(url));
}

/** Settings → Test: fetch and decode the tile under the 2D map centre (zoom 12 or the map's max) through the
 *  normal loader, uncached. The error text never carries the URL (the key may be in it): the backend strips it,
 *  any slipped copy is replaced by the host. */
export async function testUserMapTemplate(template: string, maxZoom: number): Promise<UserMapTestResult> {
  const t = template.trim();
  const [lat, lon] = get(settings).map.center;
  const { z, x, y } = tileAt(lat, lon, Math.max(0, Math.min(TEST_ZOOM, maxZoom)));
  const url = fillTileTemplate(t, z, x, y);
  let buf: ArrayBuffer;
  try {
    buf = await loadTileBytes(url, t);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, z, x, y, error: msg.split(url).join(templateHost(t) || '…') };
  }
  try {
    const { width, height } = await decodeImageSize(buf);
    return { ok: true, z, x, y, width, height };
  } catch {
    return { ok: false, z, x, y, notImage: true };
  }
}
