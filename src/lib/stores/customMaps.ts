// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Custom (offline) maps from a local tile folder — desktop only (`hasCustomMaps`).
//
// The backend lists the maps under the configured folder (`settings.customMapsPath`, '' = default
// `<app data>/custom_maps`); each becomes a provider `custom:<folder>` in the map-provider registry
// (`config/mapProviders.ts`), so every map view resolves it through `getProviderById` like a built-in
// one. The coverage (the covered areas behind the frames and "Show map area") is loaded per map on demand; the
// backend keeps it as an index file next to the tiles.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { settings } from '$lib/stores/settings';
import { hasCustomMaps } from '$lib/platform';
import {
  customProvider,
  getProviderById,
  isCustomProviderId,
  isUserProviderId,
  normalizeTileSize,
  setCustomProviders,
  type MapProvider,
  type UserMapSpec,
} from '$lib/config/mapProviders';
// Imported for its side effect too: it mirrors the persisted user online maps into the provider
// registry at load, before any map view (all of which import this module) resolves an id.
import { syncUserMapRegistry } from '$lib/stores/userMaps';

/** `[west, south, east, north]` in degrees. */
export type GeoBounds = [number, number, number, number];

export interface CustomMapInfo {
  name: string;
  minZoom: number;
  maxZoom: number;
  /** Detected tile extension (Debug Monitor). */
  ext: string | null;
  /** Tile pixel size of the whole map (128 / 256 / 512; 256 when no tile header was readable). */
  tileSize: number;
  /** Real pixel size `[w, h]` of the sample tile behind `tileSize` (Debug Monitor). */
  tilePx: [number, number] | null;
}

export interface CustomMapsState {
  /** The first listing has finished (with or without success). Until then a persisted custom
   *  provider id cannot be resolved, so the map views wait instead of flashing the fallback. */
  loaded: boolean;
  scanning: boolean;
  /** The folder the last listing scanned ('' before the first one) — Debug Monitor. */
  root: string;
  /** The default folder (shown when the setting is empty). */
  defaultRoot: string;
  maps: CustomMapInfo[];
  providers: MapProvider[];
  /** Backend error of the last listing ('' = fine). English, from the backend. */
  error: string;
  /** Bumped by every successful listing — a Rescan re-creates the tile layers of a custom map. */
  revision: number;
}

export interface CustomMapArea {
  /** Highest tile level in the area. */
  top: number;
  /** Tight box. */
  bounds: GeoBounds;
  /** Footprint outline, segments `[lat0, lon0, lat1, lon1]`. */
  segments: [number, number, number, number][];
}

export interface CustomMapCoverage {
  minZoom: number | null;
  maxZoom: number | null;
  /** Covered areas, highest `top` first: always framed on the 2D map (a ×4 finder box while that fits
   *  the view, the real outline beyond), and the targets of "Show map area". */
  areas: CustomMapArea[];
  /** Debug Monitor from here on. */
  /** True when the backend's area limit cut further areas off. */
  areasTruncated: boolean;
  tileCount: number;
  /** Per tile level: tile count, footprint grid level, merged rectangle count. */
  levels: { z: number; tiles: number; grid: number; rects: number }[];
  /** Duration of the scan that built this coverage (ms). */
  scanMs: number;
  /** Index file read or written (null: could not be stored). */
  indexPath: string | null;
  /** True when the stored index was still valid (no scan ran). */
  indexHit: boolean;
}

/** One resolved custom-map tile, for the Debug Monitor (dev builds only). z/x/y and `srcZ` are FILE
 *  levels (the map zoom differs by the tile-size shift on a 512 / 128 px map). */
export interface CustomTileDebugEntry {
  z: number;
  x: number;
  y: number;
  /** `exact` = on disk; `ancestor` = drawn from level `srcZ`; `children` = from level `srcZ`;
   *  `none` = nothing to draw; `below` = more than one level below the lowest level (no lookup). */
  kind: 'exact' | 'ancestor' | 'children' | 'none' | 'below';
  srcZ: number | null;
}

/** Last resolved custom-map tiles, newest first (dev builds only — empty in production). */
export const customTileDebug = writable<CustomTileDebugEntry[]>([]);
const TILE_DEBUG_KEEP = 24;

/** Record a resolved tile for the Debug Monitor. No-op (and tree-shaken) in production. */
export function recordCustomTile(entry: CustomTileDebugEntry): void {
  if (!import.meta.env.DEV) return;
  customTileDebug.update((l) => [entry, ...l].slice(0, TILE_DEBUG_KEEP));
}

export const customMaps = writable<CustomMapsState>({
  loaded: !hasCustomMaps,
  scanning: false,
  root: '',
  defaultRoot: '',
  maps: [],
  providers: [],
  error: '',
  revision: 0,
});

/** Coverage per map name, filled on demand by `loadCustomMapCoverage`. Cleared by every listing. */
export const customMapCoverage = writable<Record<string, CustomMapCoverage>>({});

/** "Jump to map" request for the map views (2D fits the bounds, 3D flies to them). */
export const customMapFocus = writable<{ bounds: GeoBounds; seq: number } | null>(null);

interface Listing {
  root: string;
  defaultRoot: string;
  maps: CustomMapInfo[];
}

/** Bumped by every listing (success or failure). A coverage request started under an older listing
 *  is discarded: it may belong to a same-named map of the previous folder. */
let listingGen = 0;
const coverageInflight = new Map<string, Promise<CustomMapCoverage | null>>();

function resetCoverage(): void {
  listingGen++;
  coverageInflight.clear();
  customMapCoverage.set({});
}

async function doRefresh(): Promise<void> {
  customMaps.update((s) => ({ ...s, scanning: true }));
  const path = get(settings).customMapsPath ?? '';
  try {
    const res = await invoke<Listing>('custom_maps_list', { root: path });
    const providers = res.maps.map((m) =>
      customProvider({ name: m.name, minZoom: m.minZoom, maxZoom: m.maxZoom, tileSize: normalizeTileSize(m.tileSize) }),
    );
    setCustomProviders(providers);
    resetCoverage();
    customMaps.update((s) => ({
      loaded: true,
      scanning: false,
      root: res.root,
      defaultRoot: res.defaultRoot,
      maps: res.maps,
      providers,
      error: '',
      revision: s.revision + 1,
    }));
  } catch (e) {
    console.warn('[custom-maps] listing failed:', e);
    setCustomProviders([]);
    resetCoverage();
    customMaps.update((s) => ({
      ...s,
      loaded: true,
      scanning: false,
      root: path,
      maps: [],
      providers: [],
      error: e instanceof Error ? e.message : String(e),
    }));
  }
}

// Listings run one after another: the backend keeps ONE registered set, so the last listing started
// must also be the last one to finish.
let chain: Promise<void> = Promise.resolve();
let firstLoad: Promise<void> | null = null;

/** Re-list the maps of the configured folder. */
export function refreshCustomMaps(): Promise<void> {
  if (!hasCustomMaps) return Promise.resolve();
  chain = chain.then(doRefresh, doRefresh);
  return chain;
}

/** List the maps once (no-op once listed). */
export function ensureCustomMapsLoaded(): Promise<void> {
  if (!hasCustomMaps || get(customMaps).loaded) return Promise.resolve();
  firstLoad ??= refreshCustomMaps();
  return firstLoad;
}

/** Change detection key of a resolved provider: a custom map is re-applied after every listing (its
 *  tiles may have changed on disk), a user online map whenever it is edited (same id, new template),
 *  a built-in one only when it changes. */
function providerKey(p: MapProvider, revision: number): string {
  if (p.custom) return `${p.id}|${p.custom.minZoom}-${p.custom.maxZoom}|${p.custom.tileSize}|${revision}`;
  return isUserProviderId(p.id) ? `${p.id}|${p.maxZoom}|${p.tileSize}|${p.attribution}|${p.url}` : p.id;
}

/**
 * Follow the provider the map views should show (`settings.mapProvider`, resolved against the
 * custom-map and user-map registry). Calls `apply` immediately for a built-in provider and whenever the
 * resolved provider changes — NOT on the many other settings writes. A persisted custom id waits for
 * the first listing; a custom or user map that no longer exists resolves to the built-in fallback.
 */
export function watchMapProvider(apply: (p: MapProvider) => void): () => void {
  let id: string | null = null;
  let userList: UserMapSpec[] | null = null;
  let state = get(customMaps);
  let lastKey = '';
  const check = () => {
    if (id === null) return;
    if (hasCustomMaps && isCustomProviderId(id) && !state.loaded) {
      void ensureCustomMapsLoaded();
      return;
    }
    const p = getProviderById(id);
    const key = providerKey(p, state.revision);
    if (key === lastKey) return;
    lastKey = key;
    apply(p);
  };
  const unsubMaps = customMaps.subscribe((s) => { state = s; check(); });
  const unsubSettings = settings.subscribe((s) => {
    if (s.mapProvider === id && s.userMapProviders === userList) return;
    id = s.mapProvider;
    userList = s.userMapProviders;
    syncUserMapRegistry(userList); // before resolving, whatever order the subscribers run in
    check();
  });
  return () => { unsubMaps(); unsubSettings(); };
}

/** The custom map behind a provider id, if it is one that is currently listed. */
export function customMapName(providerId: string): string | null {
  if (!isCustomProviderId(providerId)) return null;
  const p = getProviderById(providerId);
  return p.custom ? p.custom.name : null;
}

/** Coverage of a map — cached; `force` rescans the folder (Settings → Rescan). `null` when the scan
 *  failed (or the folder was re-listed meanwhile). */
export function loadCustomMapCoverage(name: string, force = false): Promise<CustomMapCoverage | null> {
  if (!hasCustomMaps) return Promise.resolve(null);
  if (!force) {
    const have = get(customMapCoverage)[name];
    if (have) return Promise.resolve(have);
    const pending = coverageInflight.get(name);
    if (pending) return pending;
  }
  const gen = listingGen;
  const p = invoke<CustomMapCoverage>('custom_map_coverage', { map: name, force })
    .then((c) => {
      // Re-listed while this ran: it may describe a same-named map of the previous folder.
      if (gen !== listingGen) return null;
      customMapCoverage.update((m) => ({ ...m, [name]: c }));
      return c;
    })
    .catch((e) => {
      console.warn(`[custom-maps] coverage of '${name}' failed:`, e);
      return null;
    })
    .finally(() => {
      if (coverageInflight.get(name) === p) coverageInflight.delete(name);
    });
  coverageInflight.set(name, p);
  return p;
}

/** Settings → Rescan: re-list the folder and rebuild the selected map's coverage index. */
export async function rescanCustomMaps(): Promise<void> {
  await refreshCustomMaps();
  const name = customMapName(get(settings).mapProvider);
  if (name) await loadCustomMapCoverage(name, true);
}

/** Settings → Choose: pick the folder holding the maps, persist it, re-list. */
export async function chooseCustomMapsFolder(): Promise<void> {
  const current = get(settings).customMapsPath || get(customMaps).defaultRoot || undefined;
  try {
    const selected = await open({ directory: true, multiple: false, defaultPath: current });
    if (typeof selected !== 'string' || selected.length === 0) return;
    settings.patch({ customMapsPath: selected });
    await refreshCustomMaps();
  } catch (e) {
    console.error('[custom-maps] choosing the folder failed', e);
  }
}

/** Settings → Use default: back to `<app data>/custom_maps`, re-list. */
export async function resetCustomMapsFolder(): Promise<void> {
  settings.patch({ customMapsPath: '' });
  await refreshCustomMaps();
}

/** Squared equirectangular distance (deg²) from a point to the nearest point of a bounds box. */
function distToBounds(lat: number, lon: number, b: GeoBounds): number {
  const dLon = lon < b[0] ? b[0] - lon : lon > b[2] ? lon - b[2] : 0;
  const dLat = lat < b[1] ? b[1] - lat : lat > b[3] ? lat - b[3] : 0;
  const k = Math.cos((lat * Math.PI) / 180);
  return (dLon * k) ** 2 + dLat ** 2;
}

let focusSeq = 0;

/** "Show map area": ask the map views to fit the covered area nearest to the current 2D view
 *  centre. `empty` = the map has no tiles; `error` = its coverage could not be read; `cancelled` =
 *  the folder was re-listed meanwhile (the answer may belong to the previous folder — nothing to say). */
export async function focusCustomMap(name: string): Promise<'ok' | 'empty' | 'error' | 'cancelled'> {
  const gen = listingGen;
  const cov = await loadCustomMapCoverage(name);
  if (gen !== listingGen) return 'cancelled';
  if (!cov) return 'error';
  if (cov.areas.length === 0) return 'empty';
  const [lat, lon] = get(settings).map.center;
  let best = cov.areas[0].bounds;
  let bestD = Infinity;
  for (const a of cov.areas) {
    const d = distToBounds(lat, lon, a.bounds);
    if (d < bestD) { bestD = d; best = a.bounds; }
  }
  customMapFocus.set({ bounds: best, seq: ++focusSeq });
  return 'ok';
}

/** Subscribe to NEW "jump to map" requests only — a view mounted after a request (2D↔3D switch) must
 *  not replay the last one. */
export function onCustomMapFocus(fn: (bounds: GeoBounds) => void): () => void {
  let seen = get(customMapFocus)?.seq ?? 0;
  return customMapFocus.subscribe((f) => {
    if (!f || f.seq <= seen) return;
    seen = f.seq;
    fn(f.bounds);
  });
}
