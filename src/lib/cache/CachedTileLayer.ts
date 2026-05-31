// Custom Leaflet TileLayer that checks IndexedDB cache before fetching from network.
// Falls back to normal network fetch on cache miss and stores the result.
//
// For flagged providers (ESRI satellite) it also detects over-zoom "placeholder"
// tiles (a fixed blank the server returns above the available zoom) and fills
// those tiles with the scaled-up parent imagery — mirroring how Cesium keeps the
// parent tile visible — instead of leaving a blank.

import L from "leaflet";
import { getCachedTile, putCachedTile } from "$lib/cache/tileCache";
import {
  isKnownUnavailable,
  isPlaceholderTile,
  regionRenderCap,
  regionNeedsRefine,
  setRegionVerified,
  MIN_DETECT_ZOOM,
} from "$lib/cache/tileAvailability";

export const CachedTileLayer = L.TileLayer.extend({
  createTile(coords: L.Coords, done: L.DoneCallback): HTMLElement {
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
    // Try to fetch and cache, but always display the tile even if caching fails
    fetch(url)
      .then((resp) => {
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        return resp.arrayBuffer();
      })
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
    const dz = Math.max(1, coords.z - cap);
    const scale = 1 << dz;
    const px = coords.x >> dz;
    const py = coords.y >> dz;
    const qx = coords.x & (scale - 1);
    const qy = coords.y & (scale - 1);
    const ts = this.getTileSize();
    const purl = this._buildUrlAt(px, py, coords.z - dz);

    // 1px bleed on every side so the clipped image always fully covers the tile
    // box (no sub-pixel hairline gap at the seams during movement).
    const BLEED = 1;
    const img = document.createElement("img");
    img.alt = "";
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
          const resp = await fetch(purl);
          if (resp.ok) buf = await resp.arrayBuffer();
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
  options?: L.TileLayerOptions & { providerId?: string },
): L.TileLayer {
  return new (CachedTileLayer as any)(url, options);
}
