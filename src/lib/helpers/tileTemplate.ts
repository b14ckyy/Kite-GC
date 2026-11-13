// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Pure helpers for Leaflet-style tile URL templates: filling one in (the 3D view and the user-map test
// build their URLs here; the 2D view lets Leaflet's own `getTileUrl` do it) and validating a template a
// user typed (Settings → Custom Online Provider).
//
// Placeholders: `{z}` `{x}` `{y}`; `{-y}` = the row counted from the south (TMS); `{s}` = a subdomain
// (a/b/c); `{r}` = Leaflet's retina suffix (`@2x` on a high-DPI screen in 2D, empty elsewhere).

/** Every placeholder Kite fills in. Leaflet throws on any other `{name}` it cannot resolve. */
export const TILE_PLACEHOLDERS = ["z", "x", "y", "-y", "s", "r"] as const;

const PLACEHOLDER_RE = /\{([^{}]*)\}/g;

/** Fill a tile URL template for tile z/x/y. The subdomain is picked from `subdomains` by
 *  `(x + y + z) % n`, as the 3D view and the 2D map's own fallback path (`CachedTileLayer._buildUrlAt`)
 *  always did — Leaflet's regular 2D path uses `(x + y) % n`, so a `{s}` tile may be cached under two
 *  subdomains. `retina` replaces `{r}`. */
export function fillTileTemplate(
  template: string,
  z: number,
  x: number,
  y: number,
  subdomains: string | readonly string[] = "abc",
  retina = "",
): string {
  const sub = subdomains.length > 0 ? subdomains[(x + y + z) % subdomains.length] : "";
  // split/join = replace every occurrence, without needing String.replaceAll in an older WebView.
  const values: [string, string][] = [
    ["{z}", String(z)],
    ["{x}", String(x)],
    ["{-y}", String((1 << z) - 1 - y)],
    ["{y}", String(y)],
    ["{s}", sub],
    ["{r}", retina],
  ];
  return values.reduce((url, [key, value]) => url.split(key).join(value), template);
}

/** Why a user-typed template is unusable (`null` = fine). `unknown` carries the offending placeholder. */
export type TemplateProblem =
  | { kind: "url" }
  | { kind: "placeholders" }
  | { kind: "unknown"; placeholder: string };

/** Check a user-typed template: an http(s) URL that has `{z}`, `{x}` and `{y}` (or `{-y}`) and no
 *  placeholder Kite does not fill in (the 2D map would throw on it).
 *  CAUTION: settings `load()` also drops every STORED user map that fails this check — tightening it
 *  later silently deletes users' maps (with their keys) on the next start. */
export function validateTileTemplate(template: string): TemplateProblem | null {
  const t = template.trim();
  for (const m of t.matchAll(PLACEHOLDER_RE)) {
    if (!(TILE_PLACEHOLDERS as readonly string[]).includes(m[1])) return { kind: "unknown", placeholder: m[0] };
  }
  let parsed: URL;
  try {
    parsed = new URL(fillTileTemplate(t, 1, 0, 0));
  } catch {
    return { kind: "url" };
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return { kind: "url" };
  if (!t.includes("{z}") || !t.includes("{x}") || !(t.includes("{y}") || t.includes("{-y}"))) {
    return { kind: "placeholders" };
  }
  return null;
}

/** Stand-in for `{s}` while a template is parsed (a brace is not allowed in a host name). */
const SUBDOMAIN_MARK = "kite-subdomain-mark";

/** Host of a template, with `{s}` left as it is (e.g. `{s}.tile.example.com`) — what may be shown or
 *  logged; the path and query, where an API key usually sits, are left out. '' when it does not parse. */
export function templateHost(template: string): string {
  try {
    return new URL(fillTileTemplate(template.trim(), 1, 0, 0, [SUBDOMAIN_MARK])).host
      .split(SUBDOMAIN_MARK)
      .join("{s}");
  } catch {
    return "";
  }
}

/** Leaflet layer options for an online source whose tiles are `tileSize` px and whose URL goes up to
 *  level `maxUrlZoom`. Leaflet requests URL level = map zoom + `zoomOffset`, and `maxNativeZoom` /
 *  `minNativeZoom` are MAP zooms (`GridLayer._clampZoom` compares them with the map zoom before the
 *  offset is added in `TileLayer._getZoomForUrl`):
 *  - 512 px: offset −1 (a 512 px tile at map zoom Z covers what four 256 px tiles do, i.e. one URL
 *    level lower). Native up to map zoom maxUrlZoom+1; map zoom 0 would need URL level −1, so
 *    `minNativeZoom` 1 (Leaflet scales level 1 down there).
 *  - 128 px: offset +1, native up to map zoom maxUrlZoom−1.
 *  - 256 px: no shift.
 *  Above the native limit Leaflet enlarges the top level, up to `maxZoom` ≥ 20. */
export function userLayerZoom(
  maxUrlZoom: number,
  tileSize: 128 | 256 | 512,
): {
  tileSize: number;
  zoomOffset: number;
  maxNativeZoom: number;
  minNativeZoom?: number;
  maxZoom: number;
  className: string;
} {
  const zoomOffset = Math.round(Math.log2(256 / tileSize)); // 512 → −1, 256 → 0, 128 → +1
  const maxNativeZoom = Math.max(0, maxUrlZoom - zoomOffset);
  return {
    tileSize,
    // Leaflet puts it on the layer container: Map.svelte's seam rules (tile-overlap / tile-snap)
    // force a tile size and need the real one for 512 / 128 px tiles.
    className: `kite-tiles-${tileSize}`,
    zoomOffset,
    maxNativeZoom,
    ...(zoomOffset < 0 ? { minNativeZoom: -zoomOffset } : {}),
    maxZoom: Math.max(20, maxNativeZoom),
  };
}

/** Leaflet layer options for a custom (offline folder) map whose tiles are `tileSize` px and whose
 *  highest FILE level is `maxLevel`: the same level shift as an online source (`userLayerZoom` — file
 *  level = map zoom + `zoomOffset`, so a 512 px map's level Z is native at map zoom Z+1, a 128 px one's
 *  at Z−1), and over-zoom up to two map zooms past the highest native one (at least 20). */
export function customLayerZoom(maxLevel: number, tileSize: 128 | 256 | 512): ReturnType<typeof userLayerZoom> {
  const zoom = userLayerZoom(maxLevel, tileSize);
  return { ...zoom, maxZoom: Math.max(20, zoom.maxNativeZoom + 2) };
}

/** The Web Mercator (XYZ) tile containing a position at zoom `z`. */
export function tileAt(lat: number, lon: number, z: number): { z: number; x: number; y: number } {
  const n = 1 << z;
  const clampedLat = Math.max(-85.0511, Math.min(85.0511, lat));
  const latRad = (clampedLat * Math.PI) / 180;
  const x = Math.floor(((lon + 180) / 360) * n);
  const y = Math.floor(((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n);
  return { z, x: Math.max(0, Math.min(n - 1, x)), y: Math.max(0, Math.min(n - 1, y)) };
}
