// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared map-tile byte loader for both the 2D (Leaflet) and 3D (Cesium) tile
// pipelines. Routes fetches through the Rust backend (`fetch_tile` → reqwest:
// HTTP/2, pooled connections, concurrency-capped) instead of the OS WebView's
// HTTP client. The WebView caps connections per host and, on WebKitGTK/Linux,
// serialises requests to a single-host provider (e.g. ESRI) — which makes tiles
// crawl in. The Rust path multiplexes over HTTP/2 and behaves the same across
// OSes. Falls back to the WebView's fetch() if the backend is unavailable (e.g.
// a non-Tauri context or a command failure).

import { invoke } from "@tauri-apps/api/core";
import { isCustomTileUrl, parseCustomTileUrl } from "$lib/config/mapProviders";

/**
 * Fetch raw tile bytes via the Rust backend, falling back to the WebView's fetch().
 * `layer` is a stable per-layer key (the layer's URL template) — the backend caps
 * concurrency per layer, so a multi-layer provider (ESRI Hybrid) isn't throttled by
 * its own overlays while single-layer community providers stay conservatively capped.
 *
 * A custom map's pseudo URL (`custom://<map>/<z>/<x>/<y>`) is read from the local tile folder by
 * the backend instead — exact level only, rejecting when the tile does not exist (no network
 * fallback: there is nothing to fetch).
 */
export async function loadTileBytes(url: string, layer: string): Promise<ArrayBuffer> {
  if (isCustomTileUrl(url)) {
    const t = parseCustomTileUrl(url);
    if (!t) throw new Error(`bad custom tile URL: ${url}`);
    return await invoke<ArrayBuffer>("custom_map_tile", { map: t.map, z: t.z, x: t.x, y: t.y });
  }
  try {
    // Tauri delivers the command's `Response` bytes as an ArrayBuffer.
    return await invoke<ArrayBuffer>("fetch_tile", { url, layer });
  } catch {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return await resp.arrayBuffer();
  }
}
