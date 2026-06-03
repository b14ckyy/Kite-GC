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
  let ro: ResizeObserver | undefined;

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
    if (b.isValid()) map.fitBounds(b, { padding: [6, 6], maxZoom: 18 });
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
    // The detail box lays out after mount → fix the size + refit.
    ro = new ResizeObserver(() => map?.invalidateSize());
    ro.observe(container);
    setTimeout(() => { map?.invalidateSize(); draw(waypointsJson); }, 60);
  });

  // Redraw when the mission changes (draw() only mutates Leaflet layers → no reactive loop).
  $effect(() => {
    void waypointsJson;
    if (map) draw(waypointsJson);
  });

  onDestroy(() => {
    ro?.disconnect();
    if (map) { map.remove(); map = undefined; }
  });
</script>

<div class="preview-map" bind:this={container}></div>

<style>
  .preview-map { width: 100%; height: 100%; border: 1px solid #333; border-radius: 4px; background: #1a1a1a; overflow: hidden; }
  .preview-map :global(.leaflet-container) { background: #1a1a1a; cursor: default; font-family: inherit; }
</style>
