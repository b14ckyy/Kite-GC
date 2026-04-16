// IndexedDB-backed map tile cache with configurable size limit and LRU eviction.
// Stores fetched tile images as ArrayBuffers keyed by URL.
//
// Performance design:
// - getCachedTile uses READONLY transactions so all reads run in parallel
// - LRU lastAccess updates are fire-and-forget (no await)
// - Stats are tracked in-memory, only synced from DB on init/clear
// - Eviction is debounced — runs at most once per second, not per tile

import { writable } from "svelte/store";

const DB_NAME = "inav-gcs-tile-cache";
const DB_VERSION = 1;
const STORE_NAME = "tiles";

interface CachedTile {
  url: string;
  data: ArrayBuffer;
  size: number;
  lastAccess: number;
}

export interface TileCacheStats {
  usedBytes: number;
  maxBytes: number;
  tileCount: number;
}

/** Reactive store exposing current cache fill level */
export const tileCacheStats = writable<TileCacheStats>({
  usedBytes: 0,
  maxBytes: 0,
  tileCount: 0,
});

let dbInstance: IDBDatabase | null = null;
let currentMaxBytes = 0;

// In-memory stat tracking — avoids iterating DB on every put
let memUsedBytes = 0;
let memTileCount = 0;

function publishStats(): void {
  tileCacheStats.set({ usedBytes: memUsedBytes, maxBytes: currentMaxBytes, tileCount: memTileCount });
}

function openDb(): Promise<IDBDatabase> {
  if (dbInstance) return Promise.resolve(dbInstance);
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = (e) => {
      const db = (e.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        const store = db.createObjectStore(STORE_NAME, { keyPath: "url" });
        store.createIndex("lastAccess", "lastAccess", { unique: false });
      }
    };
    req.onsuccess = (e) => {
      dbInstance = (e.target as IDBOpenDBRequest).result;
      resolve(dbInstance);
    };
    req.onerror = () => reject(req.error);
  });
}

/** Full DB scan to sync in-memory stats — only on init */
async function syncStatsFromDb(): Promise<void> {
  try {
    const db = await openDb();
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);

    // Use count + cursor for size (count is fast, size needs iteration)
    const countReq = store.count();
    const count = await new Promise<number>((resolve, reject) => {
      countReq.onsuccess = () => resolve(countReq.result);
      countReq.onerror = () => reject(countReq.error);
    });

    if (count === 0) {
      memUsedBytes = 0;
      memTileCount = 0;
      publishStats();
      return;
    }

    let usedBytes = 0;
    const cursor = store.openCursor();
    await new Promise<void>((resolve, reject) => {
      cursor.onsuccess = (e) => {
        const c = (e.target as IDBRequest<IDBCursorWithValue>).result;
        if (c) {
          usedBytes += (c.value as CachedTile).size;
          c.continue();
        } else {
          resolve();
        }
      };
      cursor.onerror = () => reject(cursor.error);
    });

    memUsedBytes = usedBytes;
    memTileCount = count;
    publishStats();
  } catch {
    // DB not available — leave stats at zero
  }
}

/** Configure the maximum cache size. 0 = disabled. */
export async function setCacheMaxMB(mb: number): Promise<void> {
  currentMaxBytes = mb * 1024 * 1024;
  if (mb === 0) {
    await clearCache();
  } else {
    await evictIfNeeded();
  }
  publishStats();
}

/** Try to get a tile from the cache. Returns a blob URL or null. */
export async function getCachedTile(url: string): Promise<string | null> {
  if (currentMaxBytes === 0) return null;
  try {
    const db = await openDb();

    // READONLY transaction — allows parallel reads (critical for performance)
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);

    const tile: CachedTile | undefined = await new Promise((resolve, reject) => {
      const req = store.get(url);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });

    if (!tile) return null;

    // Update lastAccess fire-and-forget — do NOT await
    try {
      const txW = db.transaction(STORE_NAME, "readwrite");
      txW.objectStore(STORE_NAME).put({ ...tile, data: tile.data, lastAccess: Date.now() });
    } catch {
      // non-critical
    }

    const blob = new Blob([tile.data]);
    return URL.createObjectURL(blob);
  } catch {
    return null;
  }
}

/** Store a fetched tile in the cache. */
export async function putCachedTile(url: string, data: ArrayBuffer): Promise<void> {
  if (currentMaxBytes === 0) return;
  try {
    const db = await openDb();

    // Check if tile already exists (update vs insert for stats tracking)
    const txR = db.transaction(STORE_NAME, "readonly");
    const existing: CachedTile | undefined = await new Promise((resolve, reject) => {
      const req = txR.objectStore(STORE_NAME).get(url);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });

    const tile: CachedTile = {
      url,
      data,
      size: data.byteLength,
      lastAccess: Date.now(),
    };

    const txW = db.transaction(STORE_NAME, "readwrite");
    txW.objectStore(STORE_NAME).put(tile);
    await new Promise<void>((resolve, reject) => {
      txW.oncomplete = () => resolve();
      txW.onerror = () => reject(txW.error);
    });

    // Update in-memory stats
    if (existing) {
      memUsedBytes += tile.size - existing.size;
    } else {
      memUsedBytes += tile.size;
      memTileCount++;
    }
    publishStats();

    // Debounced eviction — only when over limit
    if (memUsedBytes > currentMaxBytes) {
      scheduleEviction();
    }
  } catch {
    // Cache write failure is non-critical
  }
}

let evictionTimer: ReturnType<typeof setTimeout> | null = null;

/** Schedule eviction to run at most once per second */
function scheduleEviction(): void {
  if (evictionTimer) return;
  evictionTimer = setTimeout(async () => {
    evictionTimer = null;
    await evictIfNeeded();
    publishStats();
  }, 1000);
}

/** Evict oldest-accessed tiles until we're under the size limit */
async function evictIfNeeded(): Promise<void> {
  if (currentMaxBytes === 0) return;
  if (memUsedBytes <= currentMaxBytes) return;
  try {
    const db = await openDb();

    // Use the lastAccess index to iterate oldest-first without loading all data
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const index = store.index("lastAccess");

    let bytesToFree = memUsedBytes - currentMaxBytes;
    let deletedCount = 0;

    const cursor = index.openCursor(); // ascending = oldest first
    await new Promise<void>((resolve, reject) => {
      cursor.onsuccess = (e) => {
        if (bytesToFree <= 0) { resolve(); return; }
        const c = (e.target as IDBRequest<IDBCursorWithValue>).result;
        if (c) {
          const tile = c.value as CachedTile;
          bytesToFree -= tile.size;
          memUsedBytes -= tile.size;
          deletedCount++;
          c.delete();
          c.continue();
        } else {
          resolve();
        }
      };
      cursor.onerror = () => reject(cursor.error);
    });

    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });

    memTileCount -= deletedCount;
  } catch {
    // Eviction failure is non-critical — resync stats
    await syncStatsFromDb();
  }
}

/** Clear the entire tile cache */
export async function clearCache(): Promise<void> {
  try {
    const db = await openDb();
    const tx = db.transaction(STORE_NAME, "readwrite");
    tx.objectStore(STORE_NAME).clear();
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
    memUsedBytes = 0;
    memTileCount = 0;
    tileCacheStats.set({ usedBytes: 0, maxBytes: currentMaxBytes, tileCount: 0 });
  } catch {
    // Clear failure is non-critical
  }
}

/** Initialize the cache on app startup — call once after settings are loaded */
export async function initTileCache(maxMB: number): Promise<void> {
  currentMaxBytes = maxMB * 1024 * 1024;
  await syncStatsFromDb();
}
