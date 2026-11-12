// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Map tile provider definitions for Leaflet
// All providers listed here are free and require no API key; the user's own (keyed) sources are the
// user online maps further down.

export interface MapProvider {
  id: string;
  label: string;
  url: string;
  attribution: string;
  maxZoom: number;
  /** Max zoom for CesiumJS 3D view — lower than maxZoom for providers with sparse high-zoom coverage */
  cesiumMaxZoom?: number;
  /** Optional overlay layer URLs rendered on top of the base layer (e.g. labels on satellite) */
  overlays?: { url: string; attribution: string; maxZoom: number; cesiumMaxZoom?: number }[];
  /** True when the base imagery is dark (satellite, dark theme) — used for UI contrast */
  dark?: boolean;
  /** Run over-zoom placeholder detection on this base layer (ESRI satellite
   *  returns a fixed blank tile instead of a 404 above the available zoom). */
  detectPlaceholders?: boolean;
  /** Set on a custom (local tile folder) map — see `src/lib/stores/customMaps.ts`. */
  custom?: CustomMapSpec;
  /** Tile pixel size, set on a user online map only (others are 256). `maxZoom` is then the highest
   *  URL level, not the highest map zoom. */
  tileSize?: UserTileSize;
}

/** A custom map: one folder under the custom-map root, holding XYZ tiles from `minZoom` to `maxZoom`. */
export interface CustomMapSpec {
  /** Folder name (= label and the part after `custom:` in the provider id). */
  name: string;
  minZoom: number;
  maxZoom: number;
}

export const MAP_PROVIDERS: MapProvider[] = [
  {
    id: "osm",
    label: "OpenStreetMap",
    url: "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png",
    attribution:
      '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    maxZoom: 19,
  },
  {
    id: "esri-street",
    label: "ESRI World Street Map",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, DeLorme, NAVTEQ',
    maxZoom: 20,
    cesiumMaxZoom: 17,
  },
  {
    id: "esri-satellite",
    label: "ESRI World Imagery",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, Maxar, Earthstar Geographics',
    maxZoom: 20,
    // Raised from 17 → 20 now that placeholder detection falls back per-region.
    cesiumMaxZoom: 20,
    dark: true,
    detectPlaceholders: true,
  },
  {
    id: "esri-hybrid",
    label: "ESRI Hybrid",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, Maxar, Earthstar Geographics',
    maxZoom: 20,
    // Raised from 17 → 20 now that placeholder detection falls back per-region.
    cesiumMaxZoom: 20,
    dark: true,
    detectPlaceholders: true,
    overlays: [
      {
        url: "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Boundaries_and_Places/MapServer/tile/{z}/{y}/{x}",
        attribution: "",
        maxZoom: 20,
        cesiumMaxZoom: 17,
      },
      {
        url: "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Transportation/MapServer/tile/{z}/{y}/{x}",
        attribution: "",
        maxZoom: 20,
        cesiumMaxZoom: 17,
      },
    ],
  },
  {
    id: "opentopomap",
    label: "OpenTopoMap",
    url: "https://{s}.tile.opentopomap.org/{z}/{x}/{y}.png",
    attribution:
      '&copy; <a href="https://opentopomap.org">OpenTopoMap</a> (<a href="https://creativecommons.org/licenses/by-sa/3.0/">CC-BY-SA</a>)',
    maxZoom: 17,
  },
  {
    id: "cartodb-dark",
    label: "CartoDB Dark Matter",
    url: "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
    attribution:
      '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors &copy; <a href="https://carto.com/">CARTO</a>',
    maxZoom: 20,
    dark: true,
  },
];

export const DEFAULT_PROVIDER_ID = "osm";

// ── Custom maps (dynamic entries, desktop only) ─────────────────────────────────────────────────
// The static list above never changes; the custom maps found in the custom-map folder are kept in
// this registry (filled by `stores/customMaps.ts` after each scan) and resolved by the same lookup.

/** Provider-id prefix of a custom map: `custom:<folder name>`. */
export const CUSTOM_PROVIDER_PREFIX = "custom:";
/** Pseudo URL scheme the tile loader routes to the local-file command instead of the network. */
export const CUSTOM_TILE_SCHEME = "custom://";

let customProviders: MapProvider[] = [];

export function isCustomProviderId(id: string): boolean {
  return id.startsWith(CUSTOM_PROVIDER_PREFIX);
}

/** Pseudo URL template of a custom map's tiles (the folder name is URI-encoded, so a `{`, `/` or `%`
 *  in it cannot collide with the placeholders or the path). */
export function customTileTemplate(name: string): string {
  return `${CUSTOM_TILE_SCHEME}${encodeURIComponent(name)}/{z}/{x}/{y}`;
}

/** True for a custom map's (template or filled-in) tile URL — local, never cached in IndexedDB. */
export function isCustomTileUrl(url: string): boolean {
  return url.startsWith(CUSTOM_TILE_SCHEME);
}

/** Parse a filled-in custom tile URL (`custom://<map>/<z>/<x>/<y>`); `null` if it is not one. */
export function parseCustomTileUrl(url: string): { map: string; z: number; x: number; y: number } | null {
  if (!url.startsWith(CUSTOM_TILE_SCHEME)) return null;
  const parts = url.slice(CUSTOM_TILE_SCHEME.length).split("/");
  if (parts.length !== 4) return null;
  const [z, x, y] = parts.slice(1).map((p) => (/^\d+$/.test(p) ? Number(p) : NaN));
  if (!Number.isInteger(z) || !Number.isInteger(x) || !Number.isInteger(y)) return null;
  let map: string;
  try { map = decodeURIComponent(parts[0]); } catch { return null; }
  return map ? { map, z, x, y } : null;
}

/** Build the provider entry of a custom map. No attribution (the user's own tiles). The 2D zoom range
 *  reaches the higher of 20 (like the built-in providers) and two levels above the map's highest level;
 *  above that level Leaflet scales the top level up (`maxNativeZoom`). The 3D view stops at the highest
 *  level that exists. */
export function customProvider(spec: CustomMapSpec): MapProvider {
  return {
    id: `${CUSTOM_PROVIDER_PREFIX}${spec.name}`,
    label: spec.name,
    url: customTileTemplate(spec.name),
    attribution: "",
    maxZoom: Math.max(20, spec.maxZoom + 2),
    cesiumMaxZoom: spec.maxZoom,
    custom: spec,
  };
}

/** Replace the registered custom maps (called by the custom-map store after a scan). */
export function setCustomProviders(list: MapProvider[]): void {
  customProviders = list;
}

/** The registered custom maps, in folder-name order. */
export function getCustomProviders(): MapProvider[] {
  return customProviders;
}

// ── User online maps (dynamic entries, all platforms) ───────────────────────────────────────────
// Online tile sources the user added by URL template (Settings → Custom Online Provider), persisted in
// `settings.userMapProviders` and mirrored into this registry by `stores/userMaps.ts`. Their tiles take
// the normal online path (backend `fetch_tile` + IndexedDB cache) like the built-in providers.

/** Provider-id prefix of a user online map: `user:<stable id>` (the id survives a rename). */
export const USER_PROVIDER_PREFIX = "user:";

/** A user online map as persisted in the settings. `url` is a Leaflet-style template; an API key is
 *  simply part of it — never log it (log the provider id / name instead). */
export interface UserMapSpec {
  /** Stable id, generated on creation (the part after `user:` in the provider id). */
  id: string;
  name: string;
  url: string;
  maxZoom: number;
  /** Plain text (escaped before it reaches the attribution control). '' = none. */
  attribution: string;
  /** Pixel size of one tile as the service delivers it. `maxZoom` stays the highest `{z}` in the URL;
   *  the 2D layer shifts its levels to fit (see `providerLayerOptions`). Older entries load as 256. */
  tileSize: UserTileSize;
}

/** Tile pixel sizes a user online map can declare. */
export const USER_TILE_SIZES = [128, 256, 512] as const;
export type UserTileSize = (typeof USER_TILE_SIZES)[number];

/** The supported tile size nearest to `n` on a log scale (100 → 128, 300 → 256, 400 → 512); 256 for
 *  anything that is not a positive number (an entry stored before the field existed). */
export function normalizeTileSize(n: unknown): UserTileSize {
  if (typeof n !== "number" || !Number.isFinite(n) || n <= 0) return 256;
  const exp = Math.max(7, Math.min(9, Math.round(Math.log2(n))));
  return (1 << exp) as UserTileSize;
}

let userProviders: MapProvider[] = [];

export function isUserProviderId(id: string): boolean {
  return id.startsWith(USER_PROVIDER_PREFIX);
}

export function userProviderId(spec: UserMapSpec): string {
  return `${USER_PROVIDER_PREFIX}${spec.id}`;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Build the provider entry of a user online map. The attribution is the user's plain text — Leaflet
 *  and Cesium render credits as HTML, so it is escaped. */
export function userProvider(spec: UserMapSpec): MapProvider {
  return {
    id: userProviderId(spec),
    label: spec.name,
    url: spec.url,
    attribution: escapeHtml(spec.attribution),
    maxZoom: spec.maxZoom,
    tileSize: spec.tileSize,
  };
}

/** Replace the registered user online maps (called by the user-map store on every list change). */
export function setUserProviders(list: MapProvider[]): void {
  userProviders = list;
}

/** Resolve a provider id — built-in, user online map or custom. An unknown id (e.g. a custom map whose
 *  folder is gone, or a deleted user map) falls back to the first built-in provider. */
export function getProviderById(id: string): MapProvider {
  return (
    MAP_PROVIDERS.find((p) => p.id === id) ??
    userProviders.find((p) => p.id === id) ??
    customProviders.find((p) => p.id === id) ??
    MAP_PROVIDERS[0]
  );
}
