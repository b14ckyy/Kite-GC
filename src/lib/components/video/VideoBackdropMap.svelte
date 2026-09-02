<!-- SPDX-License-Identifier: GPL-3.0-or-later
     Copyright (C) 2026 Marc Hoffmann (b14ckyy) -->

<!-- Blurred map backdrop for the unobstructed fullscreen video mode: a second, bare Leaflet
     instance (base tile layer only — no markers, no UAV icon, no overlays) that follows the
     UAV at a slow tick and fills the area around the aspect-exact video box with a thematic
     ground picture instead of flat grey. Cheap by construction:

       * the map renders at HALF resolution (52% box scaled ×2, with a 2% bleed so the blur
         never shows a faded edge) — tile decode, raster and the blur pass all run on a
         quarter of the pixels,
       * position updates are discrete (1 Hz, no pan animation) — the blur re-rasterises only
         on those ticks, not per frame (the fullscreen-video-blur idea died on exactly that
         60 Hz fixed cost),
       * pointer-events are off; Leaflet runs without any interaction handlers.

     The wrapper carries `data-nv-clip`, so while the native decode sink is live the surface
     router cuts its hole into this layer too (same opt-in the main `.layer-map` uses —
     without it the backdrop would paint OVER the hardware video below the WebView).

     Attribution: hidden here — the backdrop shows the SAME provider the visible map instance
     (mini frame) is already attributing on screen, and the imagery is decorative/blurred.

     Follow source, in priority order: the blackbox-replay position while a replay plays (a
     replayed model is a valid UAV position — the backdrop follows it like the mini map
     does), else the live UAV while there is a GPS fix, else the GCS location, else the
     component shows nothing (the app background stays). Fixed zoom — detail is irrelevant
     under the blur, and a lower zoom keeps the follow ticks from pulling fresh tiles
     constantly. -->

<script lang="ts">
  import { onMount } from 'svelte';
  import L from 'leaflet';
  import { get } from 'svelte/store';
  import { cachedTileLayer } from '$lib/cache/CachedTileLayer';
  import { getProviderById } from '$lib/config/mapProviders';
  import { settings } from '$lib/stores/settings';
  import { telemetry } from '$lib/stores/telemetry';
  import { gcsLocation } from '$lib/stores/gcsLocation';

  /** Blackbox-replay position (passed by the page while a replay plays) — wins over live. */
  let { replayPos = null }: { replayPos?: { lat: number; lon: number } | null } = $props();

  /** Fixed backdrop zoom: wide enough that 1 Hz follow ticks rarely need new tiles. */
  const ZOOM = 13;
  /** Follow tick (ms) — each applied move re-rasterises the blur, so keep it slow. */
  const TICK_MS = 1000;
  /** Ignore sub-jitter moves (deg, ≈ 10 m) so a hovering UAV doesn't re-render at all. */
  const MIN_MOVE_DEG = 1e-4;

  let mapEl = $state<HTMLDivElement | null>(null);
  let wrapEl = $state<HTMLDivElement | null>(null);
  let map: L.Map | null = null;
  let base: L.TileLayer | null = null;
  let last: { lat: number; lon: number } | null = null;
  /** Provider id the current base layer was built for — rebuilding tears the whole layer
   *  down (visible grey flash + fade-in), so it must happen ONLY on a real change. The
   *  settings store emits on every map interaction (the main map persists its view state
   *  through it), not just on provider switches. */
  let appliedProviderId = '';

  function target(): { lat: number; lon: number } | null {
    if (replayPos) return replayPos;
    const t = get(telemetry);
    if (t.fixType >= 2 && (t.lat !== 0 || t.lon !== 0)) return { lat: t.lat, lon: t.lon };
    return get(gcsLocation);
  }

  function applyBase(providerId: string): void {
    if (!map) return;
    if (base) map.removeLayer(base);
    const p = getProviderById(providerId);
    appliedProviderId = providerId;
    // Base imagery only — label/hybrid overlays are unreadable under the blur and would
    // only double the tile traffic.
    base = cachedTileLayer(p.url, { maxZoom: p.maxZoom }).addTo(map);
  }

  function tick(): void {
    const pos = target();
    if (!pos || !mapEl) return;
    if (!map) {
      map = L.map(mapEl, {
        zoomControl: false,
        attributionControl: false,
        dragging: false,
        scrollWheelZoom: false,
        doubleClickZoom: false,
        boxZoom: false,
        keyboard: false,
        touchZoom: false,
        zoomAnimation: false,
        markerZoomAnimation: false,
      }).setView([pos.lat, pos.lon], ZOOM);
      applyBase(get(settings).mapProvider);
      last = pos;
      return;
    }
    if (last && Math.abs(pos.lat - last.lat) < MIN_MOVE_DEG && Math.abs(pos.lon - last.lon) < MIN_MOVE_DEG) return;
    map.setView([pos.lat, pos.lon], ZOOM, { animate: false });
    last = pos;
  }

  onMount(() => {
    tick();
    const timer = setInterval(tick, TICK_MS);
    const unsubProvider = settings.subscribe((s) => {
      if (map && s.mapProvider && s.mapProvider !== appliedProviderId) applyBase(s.mapProvider);
    });
    // Leaflet sizes itself from the container once — dock resizes / window resizes change
    // the wrapper, so re-measure (cheap; pan:false keeps the centre).
    const ro = new ResizeObserver(() => map?.invalidateSize({ pan: false }));
    if (wrapEl) ro.observe(wrapEl);
    return () => {
      clearInterval(timer);
      unsubProvider();
      ro.disconnect();
      map?.remove();
      map = null;
      base = null;
    };
  });
</script>

<div class="backdrop-wrap" bind:this={wrapEl} data-nv-clip aria-hidden="true">
  <div class="backdrop-map" bind:this={mapEl}></div>
</div>

<style>
  .backdrop-wrap {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
  }
  /* Half-resolution + upscale + blur (see the header comment). The 2% bleed hides the
     blur's faded edge outside the visible area; brightness pulls the imagery slightly
     back so the video box stays the visual subject. */
  .backdrop-map {
    position: absolute;
    left: -2%;
    top: -2%;
    width: 52%;
    height: 52%;
    transform: scale(2);
    transform-origin: 0 0;
    /* Gentle out-of-focus look (2px pre-scale ≈ 4px visually after the ×2 upscale). */
    filter: blur(2px) brightness(0.8);
    background: transparent;
  }
  /* Leaflet's container default is a LIGHT grey (#ddd) — on the dark UI any tile
     load/redraw would flash bright. Transparent lets the app ground show instead. */
  .backdrop-map :global(.leaflet-container) {
    background: transparent;
  }
</style>
