// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Custom Leaflet TileLayer that checks IndexedDB cache before fetching from network.
// Falls back to normal network fetch on cache miss and stores the result.
//
// For flagged providers (ESRI satellite) it also detects over-zoom "placeholder"
// tiles (a fixed blank the server returns above the available zoom) and fills
// those tiles with the scaled-up parent imagery — mirroring how Cesium keeps the
// parent tile visible — instead of leaving a blank.
//
// Custom (local tile folder) maps take their own path (`_createCustomTile`): no IndexedDB cache, and
// every tile is a <canvas> — either the tile itself, or composed from its nearest existing ancestor
// (scaled up) or, one level below the lowest level of an area, from its children (scaled down).

import L from "leaflet";
import { invoke } from "@tauri-apps/api/core";
import { getCachedTile, putCachedTile } from "$lib/cache/tileCache";
import { loadTileBytes } from "$lib/cache/tileLoader";
import {
  isKnownUnavailable,
  isPlaceholderTile,
  regionRenderCap,
  regionNeedsRefine,
  setRegionVerified,
  MIN_DETECT_ZOOM,
} from "$lib/cache/tileAvailability";
import type { CustomMapSpec, MapProvider } from "$lib/config/mapProviders";
import { recordCustomTile } from "$lib/stores/customMaps";

/** Backend answer for a custom-map tile that is not on disk (see `custom_maps::resolve_tile`). */
interface CustomTilePlan {
  kind: "tile" | "children" | "none";
  z: number;
  children: [boolean, boolean, boolean, boolean];
}

/** Where tile `coords` sits inside its ancestor at zoom `ancestorZ`: the ancestor's x/y, the scale
 *  (2^levels) and this tile's quadrant inside it. Shared by the ESRI placeholder fill and the
 *  custom-map fallback. */
function ancestorQuadrant(coords: L.Coords, ancestorZ: number) {
  const dz = Math.max(1, coords.z - ancestorZ);
  const scale = 1 << dz;
  return {
    z: coords.z - dz,
    scale,
    px: coords.x >> dz,
    py: coords.y >> dz,
    qx: coords.x & (scale - 1),
    qy: coords.y & (scale - 1),
  };
}

/** Decode tile bytes into an image, off the paint path where the engine supports it. */
async function decodeTileBytes(buf: ArrayBuffer): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(new Blob([buf]));
  try {
    const img = new Image();
    img.decoding = "async";
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("tile decode failed"));
      img.src = url;
    });
    // Force the decode now (on the raster workers) rather than inside the first drawImage.
    await img.decode().catch(() => {});
    return img;
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Size a custom-map canvas to the image and draw it whole (Leaflet scales it to the grid). */
function drawWhole(canvas: HTMLCanvasElement, img: HTMLImageElement): void {
  canvas.width = img.naturalWidth || 256;
  canvas.height = img.naturalHeight || 256;
  canvas.getContext("2d")?.drawImage(img, 0, 0);
}

/** Release a removed custom-map canvas's pixels right away — WebKit keeps a canvas backing store
 *  until the element is garbage-collected, which during a long pan is many tiles' worth. */
function freeCanvasTile(e: L.TileEvent): void {
  const el: HTMLElement = e.tile;
  if (el instanceof HTMLCanvasElement) {
    el.width = 0;
    el.height = 0;
  }
}

export const CachedTileLayer = L.TileLayer.extend({
  createTile(coords: L.Coords, done: L.DoneCallback): HTMLElement {
    if (this.options.customMap) return this._createCustomTile(coords, done);

    const providerId: string | undefined = this.options.providerId;

    // Track gesture state once, so a learned-cap redraw can wait for idle
    // instead of flashing the whole grid mid-pan.
    if (this._map && !this._moveHooked) {
      this._moveHooked = true;
      this._map.on("movestart zoomstart", () => { this._isMapMoving = true; });
      this._map.on("moveend zoomend", () => { this._isMapMoving = false; });
    }

    // Already known to be an over-zoom placeholder for this region → don't
    // fetch; fill with the scaled parent imagery (real ancestor) instead.
    if (providerId && isKnownUnavailable(providerId, coords.z, coords.x, coords.y)) {
      return this._createFallbackTile(coords, done);
    }

    const tile = document.createElement("img");
    tile.alt = "";
    tile.setAttribute("role", "presentation");
    // Decode off the paint path: after a zoom, 20-40 fresh tiles land at once, and without this the
    // engine may decode them synchronously in the frame that first paints them - which on a weak SoC
    // is 3-8 ms per tile and reads as the whole interface stalling right after the gesture. With it,
    // decoding happens on the raster workers and each tile appears when its pixels are ready.
    tile.decoding = "async";
    const url = this.getTileUrl(coords);

    getCachedTile(url).then((blobUrl) => {
      if (blobUrl) {
        // Cache hit — load from blob URL
        tile.onload = () => {
          URL.revokeObjectURL(blobUrl);
          done(undefined, tile);
        };
        tile.onerror = () => {
          URL.revokeObjectURL(blobUrl);
          // Cache entry corrupted — fall back to network
          this._fetchAndCache(tile, url, done, coords);
        };
        tile.src = blobUrl;
      } else {
        // Cache miss — fetch from network
        this._fetchAndCache(tile, url, done, coords);
      }
    }).catch(() => {
      // Cache read failed — fall through to network
      this._fetchAndCache(tile, url, done, coords);
    });

    return tile;
  },

  _fetchAndCache(tile: HTMLImageElement, url: string, done: L.DoneCallback, coords?: L.Coords) {
    const providerId: string | undefined = this.options.providerId;
    // Try to fetch and cache, but always display the tile even if caching fails.
    // this._url is the layer's URL template — the backend caps concurrency per layer.
    loadTileBytes(url, this._url)
      .then((buf) => {
        // Over-zoom placeholder? Don't cache it. Once confirmed, fail the tile
        // and redraw — the redraw re-creates it as a parent-filled fallback.
        if (providerId && coords && isPlaceholderTile(providerId, coords.z, coords.x, coords.y, buf, url)) {
          this._scheduleRedraw();
          done(new Error("placeholder tile (over-zoom)"), tile);
          return;
        }

        // Store in cache (fire-and-forget)
        putCachedTile(url, buf).catch(() => {});

        const blob = new Blob([buf]);
        const blobUrl = URL.createObjectURL(blob);
        tile.onload = () => {
          URL.revokeObjectURL(blobUrl);
          done(undefined, tile);
        };
        tile.onerror = () => {
          URL.revokeObjectURL(blobUrl);
          done(new Error("Tile load failed"), tile);
        };
        tile.src = blobUrl;
      })
      .catch((err) => {
        // Network fetch failed — try loading directly via img src as last resort
        tile.onload = () => done(undefined, tile);
        tile.onerror = () => done(err, tile);
        tile.crossOrigin = "";
        tile.src = url;
      });
  },

  /**
   * Build an over-zoom tile as a clipping <div> holding a scaled child <img> of
   * the real ancestor tile (resolved through the IndexedDB cache, then network).
   * The div's overflow:hidden crops the oversized image to the correct quadrant
   * — reliable across engines (CSS background on an <img> is not).
   */
  _createFallbackTile(coords: L.Coords, done: L.DoneCallback): HTMLElement {
    const providerId: string = this.options.providerId;
    const z = coords.z;
    const cap = regionRenderCap(providerId, z, coords.x, coords.y);
    const target = cap !== undefined ? cap : z - 1;

    const div = document.createElement("div");
    div.setAttribute("role", "presentation");
    div.style.overflow = "hidden";
    // Composite the clipped content once and only transform it during pan
    // (otherwise the clip edge re-rasters every frame → flickering seam grid).
    div.style.willChange = "transform";

    this._paintAncestor(div, coords, target, done);

    // Optimistic target may itself be a placeholder where coverage stops lower —
    // probe + repaint the whole layer at the verified level.
    if (regionNeedsRefine(providerId, z, coords.x, coords.y)) {
      this._refineCap(coords);
    }
    return div;
  },

  /** Fill `div` with ancestor zoom `cap`, scaled and offset to this tile's
   *  quadrant. Loads the ancestor from the IndexedDB cache, else the network. */
  _paintAncestor(div: HTMLElement, coords: L.Coords, cap: number, done: L.DoneCallback) {
    const { z: pz, scale, px, py, qx, qy } = ancestorQuadrant(coords, cap);
    const ts = this.getTileSize();
    const purl = this._buildUrlAt(px, py, pz);

    // 1px bleed on every side so the clipped image always fully covers the tile
    // box (no sub-pixel hairline gap at the seams during movement).
    const BLEED = 1;
    const img = document.createElement("img");
    img.alt = "";
    // Same off-the-paint-path decode as the regular tiles in createTile - the ancestor image is
    // scale-many times larger on screen, so a synchronous decode here would stall even harder.
    img.decoding = "async";
    img.style.position = "absolute";
    img.style.width = `${ts.x * scale + 2 * BLEED}px`;
    img.style.height = `${ts.y * scale + 2 * BLEED}px`;
    img.style.left = `${-qx * ts.x - BLEED}px`;
    img.style.top = `${-qy * ts.y - BLEED}px`;
    // Own GPU layer → composited (not re-rasterised) when the tile pane pans.
    img.style.transform = "translateZ(0)";
    img.style.backfaceVisibility = "hidden";
    div.appendChild(img);

    let settled = false;
    const finish = () => { if (!settled) { settled = true; done(undefined, div); } };

    getCachedTile(purl).then((blobUrl) => {
      if (blobUrl) {
        img.onload = () => { URL.revokeObjectURL(blobUrl); finish(); };
        img.onerror = () => { URL.revokeObjectURL(blobUrl); img.onload = finish; img.onerror = finish; img.src = purl; };
        img.src = blobUrl;
      } else {
        img.onload = finish;
        img.onerror = finish;
        img.src = purl;
      }
    }).catch(() => {
      img.onload = finish;
      img.onerror = finish;
      img.src = purl;
    });
  },

  /** Probe ancestor levels (down to the detection floor) to find the real
   *  imagery depth for this region, then mark it verified and redraw once.
   *  Deduped per ancestor so a pan doesn't trigger a probe storm. */
  async _refineCap(coords: L.Coords) {
    const providerId: string = this.options.providerId;
    const z = coords.z;
    if (!this._refining) this._refining = new Set<string>();
    const guard = `${z}:${coords.x >> 1}:${coords.y >> 1}`;
    if (this._refining.has(guard)) return;
    this._refining.add(guard);
    try {
      for (let pz = z - 1; pz >= MIN_DETECT_ZOOM; pz--) {
        const dz = z - pz;
        const px = coords.x >> dz;
        const py = coords.y >> dz;
        const purl = this._buildUrlAt(px, py, pz);
        let buf: ArrayBuffer | null = null;
        try {
          buf = await loadTileBytes(purl, this._url);
        } catch { return; /* network issue — keep the optimistic paint */ }
        if (!buf) return;
        if (!isPlaceholderTile(providerId, pz, px, py, buf, purl)) {
          // Real imagery at pz — verify, cache it, repaint the layer at it.
          setRegionVerified(providerId, z, coords.x, coords.y, pz);
          putCachedTile(purl, buf).catch(() => {});
          this._scheduleRedraw();
          return;
        }
        // placeholder → isPlaceholderTile lowered the floor; keep going deeper
      }
      // Everything probed (≥ detection floor) was a placeholder → coverage stops
      // below where we can detect; assume the first un-probed level is real.
      setRegionVerified(providerId, z, coords.x, coords.y, MIN_DETECT_ZOOM - 1);
      this._scheduleRedraw();
    } finally {
      this._refining.delete(guard);
    }
  },

  /** Coalesced layer redraw — re-creates tiles so newly-learned region caps take
   *  effect (each tile re-runs createTile with the updated availability). */
  _scheduleRedraw() {
    if (this._redrawTimer) clearTimeout(this._redrawTimer);
    this._redrawTimer = setTimeout(() => {
      this._redrawTimer = null;
      if (this._isMapMoving) {
        // Don't redraw mid-gesture (flashes the grid) — do it once on idle.
        if (!this._redrawOnIdle && this._map) {
          this._redrawOnIdle = true;
          this._map.once("moveend", () => { this._redrawOnIdle = false; this.redraw(); });
        }
      } else {
        this.redraw();
      }
    }, 250);
  },

  /**
   * Custom-map tile: a <canvas> (the element must exist before we know what to draw). The tile on
   * disk is drawn at its own resolution and the canvas is scaled to the 256 px grid by Leaflet's
   * inline tile size — so 100, 256 or 512 px tiles all fit the grid. A missing tile is composed from
   * what the backend resolves: the nearest existing ancestor (scaled up, any number of levels), or,
   * one level below the lowest level of an area, the up-to-four children (scaled down). Nothing is
   * drawn further below. Never touches the IndexedDB tile cache.
   */
  _createCustomTile(coords: L.Coords, done: L.DoneCallback): HTMLElement {
    const spec: CustomMapSpec = this.options.customMap;
    const canvas = document.createElement("canvas");
    canvas.setAttribute("role", "presentation");
    canvas.width = 1;
    canvas.height = 1;
    // Leaflet requires `done` to run asynchronously (it registers the tile after createTile returns,
    // and appends it to the level in the same synchronous `_update`, so it is connected by then).
    // A canvas that is no longer in the document was aborted or unloaded: reporting it would let
    // `_tileReady` (which looks the tile up by its KEY) mark a newer tile at the same coords loaded.
    // `complete` mirrors an <img>: Leaflet's _abortLoading drops every other-level tile that is not
    // complete on a zoom change — without it, the drawn canvases of the previous level would vanish
    // at once instead of staying as the backdrop until the new level has loaded.
    const finish = () => queueMicrotask(() => {
      if (!canvas.isConnected) return;
      (canvas as HTMLCanvasElement & { complete?: boolean }).complete = true;
      done(undefined, canvas);
    });
    const { x, y, z } = coords;
    if (z < spec.minZoom - 1) {
      // More than one level below the lowest level: nothing to show, no backend round-trip.
      recordCustomTile({ z, x, y, kind: "below", srcZ: null });
      finish();
      return canvas;
    }
    loadTileBytes(this._buildUrlAt(x, y, z), this._url)
      .then(async (buf) => {
        const img = await decodeTileBytes(buf);
        if (canvas.isConnected) drawWhole(canvas, img);
        recordCustomTile({ z, x, y, kind: "exact", srcZ: z });
        finish();
      })
      .catch(() => this._composeCustomTile(canvas, coords, spec).then(() => finish(), () => finish()));
    return canvas;
  },

  /** Fill `canvas` for a custom-map tile that is not on disk at its own level (see _createCustomTile). */
  async _composeCustomTile(canvas: HTMLCanvasElement, coords: L.Coords, spec: CustomMapSpec): Promise<void> {
    const { x, y, z } = coords;
    if (!canvas.isConnected) return; // aborted meanwhile — skip the lookup
    const plan = await invoke<CustomTilePlan>("custom_map_resolve", { map: spec.name, z, x, y });
    recordCustomTile({
      z, x, y,
      kind: plan.kind === "none" ? "none" : plan.kind === "children" ? "children" : plan.z === z ? "exact" : "ancestor",
      srcZ: plan.kind === "none" ? null : plan.z,
    });
    if (plan.kind === "none") return;
    const load = (lx: number, ly: number, lz: number) =>
      loadTileBytes(this._buildUrlAt(lx, ly, lz), this._url).then(decodeTileBytes);

    if (plan.kind === "tile" && plan.z === z) {
      // Appeared on disk since the first read — draw it as it is.
      const img = await load(x, y, z);
      if (canvas.isConnected) drawWhole(canvas, img);
      return;
    }
    // Don't allocate a backing store for a canvas Leaflet has already dropped.
    if (!canvas.isConnected) return;

    // Composite at the screen's pixel density so an up-scaled ancestor is not doubly blurred.
    const ts = this.getTileSize();
    const dpr = Math.min(3, Math.max(1, window.devicePixelRatio || 1));
    const w = Math.round(ts.x * dpr);
    const h = Math.round(ts.y * dpr);
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.imageSmoothingQuality = "high";

    if (plan.kind === "tile") {
      // Nearest existing ancestor, scaled up: crop this tile's quadrant out of it.
      const q = ancestorQuadrant(coords, plan.z);
      const img = await load(q.px, q.py, q.z);
      const sw = img.naturalWidth / q.scale;
      const sh = img.naturalHeight / q.scale;
      ctx.drawImage(img, q.qx * sw, q.qy * sh, sw, sh, 0, 0, w, h);
      return;
    }

    // One level below the lowest level: the (up to) four children, scaled down into the quadrants.
    await Promise.all(plan.children.map(async (has, i) => {
      if (!has) return;
      const dx = i & 1;
      const dy = i >> 1;
      try {
        const img = await load(coords.x * 2 + dx, coords.y * 2 + dy, plan.z);
        ctx.drawImage(img, (dx * w) / 2, (dy * h) / 2, w / 2, h / 2);
      } catch { /* a child that vanished meanwhile just stays empty */ }
    }));
  },

  /** Build a tile URL at explicit x/y/z from the layer template (Leaflet's
   *  getTileUrl always uses the current map zoom, so we fill it manually). */
  _buildUrlAt(x: number, y: number, z: number): string {
    const subs: string | string[] = this.options.subdomains ?? "abc";
    const sub = subs[(x + y + z) % subs.length];
    return this._url
      .replace("{s}", sub)
      .replace("{z}", String(z))
      .replace("{x}", String(x))
      .replace("{y}", String(y))
      .replace("{r}", "");
  },
});

export function cachedTileLayer(
  url: string,
  options?: L.TileLayerOptions & { providerId?: string; customMap?: CustomMapSpec },
): L.TileLayer {
  const layer: L.TileLayer = new (CachedTileLayer as any)(url, options);
  // Custom-map tiles are canvases: free their pixels as soon as Leaflet drops them.
  if (options?.customMap) layer.on("tileunload tileabort", freeCanvasTile as L.LeafletEventHandlerFn);
  return layer;
}

/** Extra layer options for a provider's base layer: a custom map gets its local-tile path and
 *  `maxNativeZoom` = its highest level (Leaflet scales the top level above it — over-zoom), no
 *  attribution. Built-in providers get nothing. Spread AFTER the caller's own options. */
export function providerLayerOptions(p: MapProvider): Partial<L.TileLayerOptions> & { customMap?: CustomMapSpec } {
  if (!p.custom) return {};
  return { customMap: p.custom, maxNativeZoom: p.custom.maxZoom, attribution: "" };
}
