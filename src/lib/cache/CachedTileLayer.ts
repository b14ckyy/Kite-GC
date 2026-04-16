// Custom Leaflet TileLayer that checks IndexedDB cache before fetching from network.
// Falls back to normal network fetch on cache miss and stores the result.

import L from "leaflet";
import { getCachedTile, putCachedTile } from "$lib/cache/tileCache";

export const CachedTileLayer = L.TileLayer.extend({
  createTile(coords: L.Coords, done: L.DoneCallback): HTMLImageElement {
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
          this._fetchAndCache(tile, url, done);
        };
        tile.src = blobUrl;
      } else {
        // Cache miss — fetch from network
        this._fetchAndCache(tile, url, done);
      }
    }).catch(() => {
      // Cache read failed — fall through to network
      this._fetchAndCache(tile, url, done);
    });

    return tile;
  },

  _fetchAndCache(tile: HTMLImageElement, url: string, done: L.DoneCallback) {
    // Try to fetch and cache, but always display the tile even if caching fails
    fetch(url)
      .then((resp) => {
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        return resp.arrayBuffer();
      })
      .then((buf) => {
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
});

export function cachedTileLayer(
  url: string,
  options?: L.TileLayerOptions,
): L.TileLayer {
  return new (CachedTileLayer as any)(url, options);
}
