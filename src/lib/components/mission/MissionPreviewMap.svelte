<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- MissionPreviewMap.svelte
     A small, non-interactive Leaflet preview of a library mission on the current map provider:
     the mission path as a theme-accent polyline, fit to its bounds. Used in the Mission Manager
     detail. Read-only (all interaction disabled).
-->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import L from 'leaflet';
  import 'leaflet/dist/leaflet.css';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import { getProviderById } from '$lib/config/mapProviders';
  import { cachedTileLayer } from '$lib/cache/CachedTileLayer';
  import { hasLocation, toDeg, type Waypoint } from '$lib/stores/mission';

  let { waypointsJson }: { waypointsJson: string } = $props();

  let container: HTMLDivElement;
  let map: L.Map | undefined;
  let overlay: L.LayerGroup | undefined;
  let bounds: L.LatLngBounds | undefined;
  let ro: ResizeObserver | undefined;
  let refitTimer: ReturnType<typeof setTimeout> | undefined;

  function geoLatLngs(json: string): [number, number][] {
    let wps: Waypoint[];
    try { wps = JSON.parse(json); } catch { return []; }
    return wps
      .filter((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0))
      .map((w) => [toDeg(w.lat), toDeg(w.lon)] as [number, number]);
  }

  function draw(json: string) {
    if (!map) return;
    if (overlay) { map.removeLayer(overlay); overlay = undefined; }
    const pts = geoLatLngs(json);
    if (pts.length === 0) return;
    const g = L.layerGroup();
    const pl = L.polyline(pts, { color: '#37a8db', weight: 2.5, opacity: 0.95 });
    g.addLayer(pl);
    g.addLayer(L.circleMarker(pts[0], { radius: 3, color: '#fff', fillColor: '#37a8db', fillOpacity: 1, weight: 1 }));
    g.addTo(map);
    overlay = g;
    const b = pl.getBounds();
    bounds = b.isValid() ? b : undefined;
    if (bounds) map.fitBounds(bounds, { padding: [6, 6], maxZoom: 18 });
  }

  // Resize handler: the panel animates its width (transition) the first time a mission is
  // selected, so the container grows after the map was first fit. invalidateSize alone keeps
  // the old zoom → the mission looks too small; refit to the bounds so it fills the new size.
  function refit() {
    if (!map) return;
    map.invalidateSize();
    if (bounds) map.fitBounds(bounds, { padding: [6, 6], maxZoom: 18 });
  }

  // Debounce: the width transition fires the ResizeObserver many times with intermediate
  // sizes; fitting on a half-animated size yields a bad zoom that sticks. Wait until the
  // size settles, then do one clean refit at the final dimensions (also covers window resize).
  function scheduleRefit() {
    if (refitTimer) clearTimeout(refitTimer);
    refitTimer = setTimeout(() => refit(), 80);
  }

  onMount(() => {
    const s = get(settings);
    map = L.map(container, {
      zoomControl: false, attributionControl: false,
      dragging: false, scrollWheelZoom: false, doubleClickZoom: false,
      boxZoom: false, keyboard: false, touchZoom: false,
    });
    map.setView([0, 0], 2);
    const provider = getProviderById(s.mapProvider);
    cachedTileLayer(provider.url, {
      attribution: '',
      maxZoom: provider.maxZoom,
      providerId: provider.detectPlaceholders ? provider.id : undefined,
    }).addTo(map);
    draw(waypointsJson);
    // The detail box lays out after mount (and the panel may still be animating its width
    // wider on the first selection) → fix the size + refit on every resize.
    ro = new ResizeObserver(() => scheduleRefit());
    ro.observe(container);
    scheduleRefit();
  });

  // Redraw when the mission changes (draw() only mutates Leaflet layers → no reactive loop).
  $effect(() => {
    void waypointsJson;
    if (map) draw(waypointsJson);
  });

  onDestroy(() => {
    if (refitTimer) clearTimeout(refitTimer);
    ro?.disconnect();
    if (map) { map.remove(); map = undefined; }
  });
</script>

<div class="preview-map" bind:this={container}></div>

<style>
  .preview-map { width: 100%; height: 100%; border: 1px solid #333; border-radius: 4px; background: #1a1a1a; overflow: hidden; }
  .preview-map :global(.leaflet-container) { background: #1a1a1a; cursor: default; font-family: inherit; }
</style>
