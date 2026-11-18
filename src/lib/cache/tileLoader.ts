// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared tile byte loader for the 2D and 3D pipelines, through the Rust backend (`fetch_tile`: HTTP/2, pooled,
// concurrency-capped) instead of the WebView's client, which caps per-host connections and on WebKitGTK
// serialises single-host providers (tiles crawl). Falls back to the WebView's fetch() without the backend.

import { invoke } from "@tauri-apps/api/core";
import { isCustomTileUrl, parseCustomTileUrl } from "$lib/config/mapProviders";

/** WebView-fallback timeout — matches the backend client's (`commands/tiles.rs`). */
const FALLBACK_TIMEOUT_MS = 15_000;

/** Fetch raw tile bytes via the backend, else the WebView's fetch(). `layer` (the URL template) is the backend's
 *  per-layer concurrency key, so a provider's overlays don't throttle its base. A custom map's
 *  `custom://<map>/<z>/<x>/<y>` is read from the local folder (exact level, rejects a missing tile, no network). */
export async function loadTileBytes(url: string, layer: string): Promise<ArrayBuffer> {
  if (isCustomTileUrl(url)) {
    const t = parseCustomTileUrl(url);
    if (!t) throw new Error(`bad custom tile URL: ${url}`);
    return await invoke<ArrayBuffer>("custom_map_tile", { map: t.map, z: t.z, x: t.x, y: t.y });
  }
  try {
    // Tauri delivers the command's `Response` bytes as an ArrayBuffer.
    return await invoke<ArrayBuffer>("fetch_tile", { url, layer });
  } catch (backendErr) {
    // Same 15 s cap as the backend client: a host that drops packets must not hold the tile (or
    // Settings → Test) for minutes.
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), FALLBACK_TIMEOUT_MS);
    try {
      const resp = await fetch(url, { signal: ctrl.signal }).catch(() => null);
      if (resp?.ok) {
        // A body cut off by the timeout also ends in the backend's error below.
        const buf = await resp.arrayBuffer().catch(() => null);
        if (buf) return buf;
      }
    } finally {
      clearTimeout(timer);
    }
    // Both failed: report the backend's reason (HTTP status / network error, never the URL) — the
    // WebView's own failure is mostly an opaque CORS "failed to fetch". Settings → Test shows it.
    throw backendErr instanceof Error ? backendErr : new Error(String(backendErr));
  }
}
