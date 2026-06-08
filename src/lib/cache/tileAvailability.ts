// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Placeholder-tile detection for over-zoomed satellite imagery.
//
// Problem: ESRI World Imagery advertises zoom 1–20, but many areas only have
// real satellite tiles up to z17–19. Requesting a higher level does NOT return
// an HTTP error — the server replies 200 with a fixed "no imagery here" blank
// image. That blank is tolerable on the 2D map (zoom out), but unacceptable in
// the 3D follow camera, which can descend to UAV altitude and demand z19/z20.
//
// Detection: the blank is bit-for-bit identical everywhere, so a content hash
// that recurs across two *different* tile URLs is, with practical certainty, a
// placeholder (real imagery tiles are never byte-identical). We track, per
// coarse region, two facts:
//   • the lowest zoom confirmed to be a placeholder  (→ what to skip)
//   • the highest zoom confirmed to hold real imagery (→ what to fall back to)
// so the maps can fall back to a real ancestor tile and skip doomed requests.
//
// In-memory only (per session): ESRI adds imagery over time, so re-learning
// each run is safer than persisting a cap that could hide newly available
// detail. The cost is a few placeholder fetches per sparse region per session.

/** Only inspect over-zoom levels — normal browsing stays zero-cost. ESRI sparse
 *  areas top out at z17–19, so z≥19 covers every reported blank case. */
export const MIN_DETECT_ZOOM = 19;
/** Availability granularity: one z14 tile ≈ a town. */
const REGION_ZOOM = 14;

/** Confirmed placeholder content hashes. Provider-agnostic — the ESRI blank is
 *  the same bytes regardless of location. Learned at runtime (no hardcoded seed,
 *  so a provider changing its blank still self-calibrates). */
const placeholderHashes = new Set<number>();
/** hash → first tile URL that produced it (awaiting a second, different URL). */
const seenOnce = new Map<number, string>();
/** regionKey → lowest zoom confirmed to be a placeholder. */
const regionMinPlaceholder = new Map<string, number>();
/** regionKey → highest zoom confirmed (or assumed) to hold real imagery. */
const regionVerified = new Map<string, number>();

/** FNV-1a over the tile bytes (stride-sampled for large JPEGs), length folded in
 *  so equal-content/different-length can't collide. Fast, good distribution. */
function hashBytes(buf: ArrayBuffer): number {
  const bytes = new Uint8Array(buf);
  let h = 0x811c9dc5;
  const stride = bytes.length > 8192 ? Math.ceil(bytes.length / 8192) : 1;
  for (let i = 0; i < bytes.length; i += stride) {
    h ^= bytes[i];
    h = Math.imul(h, 0x01000193);
  }
  h ^= bytes.length;
  return h >>> 0;
}

function regionKey(providerId: string, z: number, x: number, y: number): string {
  const shift = z - REGION_ZOOM;
  const rx = shift > 0 ? x >>> shift : x;
  const ry = shift > 0 ? y >>> shift : y;
  return `${providerId}:${rx}:${ry}`;
}

/** True if `z` is at/above the lowest placeholder zoom learned for this region —
 *  the caller should fall back to the parent instead of requesting it. */
export function isKnownUnavailable(providerId: string, z: number, x: number, y: number): boolean {
  if (z < MIN_DETECT_ZOOM) return false;
  const mp = regionMinPlaceholder.get(regionKey(providerId, z, x, y));
  return mp !== undefined && z >= mp;
}

/** The zoom to fall back to for an over-zoom tile: the verified real level if
 *  known, else an optimistic (lowestPlaceholder − 1). undefined if nothing
 *  learned yet for the region. */
export function regionRenderCap(providerId: string, z: number, x: number, y: number): number | undefined {
  const key = regionKey(providerId, z, x, y);
  const v = regionVerified.get(key);
  if (v !== undefined) return v;
  const mp = regionMinPlaceholder.get(key);
  return mp !== undefined ? mp - 1 : undefined;
}

/** True if we have a placeholder for this region but haven't yet verified the
 *  real imagery depth below it — the caller should probe + repaint. */
export function regionNeedsRefine(providerId: string, z: number, x: number, y: number): boolean {
  const key = regionKey(providerId, z, x, y);
  return regionMinPlaceholder.get(key) !== undefined && regionVerified.get(key) === undefined;
}

/** Record the verified real-imagery zoom for a region (the deepest level that
 *  is not a placeholder, or the detection floor when coverage stops below it). */
export function setRegionVerified(providerId: string, z: number, x: number, y: number, value: number): void {
  regionVerified.set(regionKey(providerId, z, x, y), value);
}

/** Inspect a freshly fetched tile. Returns true if it is a placeholder — the
 *  caller must then NOT cache it. Records the region's lowest placeholder zoom
 *  on confirmation. */
export function isPlaceholderTile(
  providerId: string,
  z: number,
  x: number,
  y: number,
  buf: ArrayBuffer,
  url: string,
): boolean {
  if (z < MIN_DETECT_ZOOM) return false;

  const h = hashBytes(buf);
  let placeholder = placeholderHashes.has(h);

  if (!placeholder) {
    const prevUrl = seenOnce.get(h);
    if (prevUrl === undefined) {
      seenOnce.set(h, url);
    } else if (prevUrl !== url) {
      // Same exact bytes from two different tiles → placeholder.
      placeholderHashes.add(h);
      seenOnce.delete(h);
      placeholder = true;
      console.debug(`[tileAvail] placeholder hash confirmed: ${h} (${providerId})`);
    }
  }

  if (placeholder) {
    const key = regionKey(providerId, z, x, y);
    const prev = regionMinPlaceholder.get(key);
    if (prev === undefined || z < prev) regionMinPlaceholder.set(key, z);
  }
  return placeholder;
}
