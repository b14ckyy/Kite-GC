// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// User online maps: tile sources the user added by URL template (Settings → Custom Online Provider), on every
// platform. Persisted as `settings.userMapProviders`; each becomes a provider `user:<id>` in the
// map-provider registry (`config/mapProviders.ts`), so every map view resolves it through
// `getProviderById` like a built-in one, and its tiles take the same online path (backend `fetch_tile`,
// IndexedDB cache — the per-layer concurrency key is the template).
//
// The URL template may carry the user's API key: it is never logged. Log the provider id or name, and
// for a host `templateHost` (no path, no query).

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

/**
 * Settings → Test: fetch the tile under the current 2D map centre (at zoom 12, or the map's max zoom
 * if lower) through the normal tile loader and decode it. Not cached. The error text never carries the
 * URL (the key may be in it): the backend strips it, and any copy that slips through is replaced by
 * the host.
 */
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
