// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Pure helpers for Leaflet-style tile URL templates: filling one in (3D view, user-map test) and validating a
// user's (Settings → Custom Online Provider). Placeholders: `{z}` `{x}` `{y}`; `{-y}` = TMS row from the south;
// `{s}` = subdomain (a/b/c); `{r}` = Leaflet's retina suffix (`@2x` on high-DPI 2D, empty elsewhere).

/** Every placeholder Kite fills in. Leaflet throws on any other `{name}` it cannot resolve. */
export const TILE_PLACEHOLDERS = ["z", "x", "y", "-y", "s", "r"] as const;

const PLACEHOLDER_RE = /\{([^{}]*)\}/g;

/** Fill a template for tile z/x/y. Subdomain = `(x + y + z) % n` as in 3D and CachedTileLayer._buildUrlAt —
 *  Leaflet's regular 2D path uses `(x + y) % n`, so a `{s}` tile may be cached under two subdomains. `retina`
 *  replaces `{r}`. */
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

/** Leaflet options for an online source with `tileSize` px tiles up to URL level `maxUrlZoom`. URL level = map
 *  zoom + `zoomOffset`; native limits are MAP zooms (`_clampZoom` runs before the offset): 512 px → offset −1,
 *  native to maxUrlZoom+1, minNativeZoom 1; 128 px → +1, native to maxUrlZoom−1; 256 px → none. Over-zoom to ≥ 20. */
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

/** Leaflet options for a custom map with `tileSize` px tiles and highest FILE level `maxLevel`: the same level
 *  shift as `userLayerZoom` (512 px level Z native at map zoom Z+1, 128 px at Z−1), over-zoom two map zooms past
 *  the highest native one (at least 20). */
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
