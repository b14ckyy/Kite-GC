<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- MissionPreviewMap — small non-interactive Leaflet preview of a library mission (fit to bounds) for the
     Mission Manager detail. A mission set shows all missions, one colour each, or one of them (pager). -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from 'svelte-i18n';
  import L from 'leaflet';
  import 'leaflet/dist/leaflet.css';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import { getProviderById, isCustomProviderId } from '$lib/config/mapProviders';
  import { cachedTileLayer, providerLayerOptions } from '$lib/cache/CachedTileLayer';
  import { ensureCustomMapsLoaded } from '$lib/stores/customMaps';
  import { hasLocation, toDeg, splitMultiMission, type Waypoint } from '$lib/stores/mission';
  import { arduHasLocation, type ArduWaypoint } from '$lib/stores/missionArdupilot';

  let { waypointsJson, format = 'inav' }: { waypointsJson: string; format?: string } = $props();

  // The preview box aspect mirrors the mission's bbox aspect (set by the Manager), so a tight, even
  // ~10px border fills it nicely. maxZoom is generous so small missions still fill to the border
  // instead of sitting tiny in the middle (the bbox fits at whatever zoom reaches the 10px inset).
  const FIT_OPTS: L.FitBoundsOptions = { padding: [10, 10], maxZoom: 21 };

  let container: HTMLDivElement;
  let map: L.Map | undefined;
  let overlay: L.LayerGroup | undefined;
  let bounds: L.LatLngBounds | undefined;
  let ro: ResizeObserver | undefined;
  let refitTimer: ReturnType<typeof setTimeout> | undefined;

  // One colour per mission slot (INAV holds up to nine); a plain mission uses the first, the accent.
  const COLORS = ['#37a8db', '#f5a623', '#7ed321', '#ff5c93', '#b388ff', '#ffe14d', '#26e0c8', '#ff7043', '#e0e0e0'];
  let missionCount = $state(1);
  /** 0 = all missions, n = only mission n. */
  let page = $state(0);
  function step(d: number) { page = (page + d + missionCount + 1) % (missionCount + 1); }

  /** One path per mission (index = mission slot, empty when it has no position): an INAV entry can be a
   *  set, drawn without a line between its missions. */
  function geoPaths(json: string): [number, number][][] {
    if (format === 'ardupilot' || format === 'px4') {
      let wps: ArduWaypoint[];
      try { wps = JSON.parse(json); } catch { return []; }
      const path = wps
        .filter((w) => arduHasLocation(w.command) && !(w.lat === 0 && w.lon === 0))
        .map((w) => [w.lat / 1e7, w.lon / 1e7] as [number, number]);
      return [path];
    }
    let wps: Waypoint[];
    try { wps = JSON.parse(json); } catch { return []; }
    return splitMultiMission(wps)
      .map((seg) => seg
        .filter((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0))
        .map((w) => [toDeg(w.lat), toDeg(w.lon)] as [number, number]));
  }

  function draw(json: string, only: number) {
    if (!map) return;
    if (overlay) { map.removeLayer(overlay); overlay = undefined; }
    const paths = geoPaths(json);
    missionCount = Math.max(1, paths.length);
    const g = L.layerGroup();
    const shown = L.latLngBounds([]);
    paths.forEach((path, i) => {
      if (path.length === 0 || (only > 0 && only !== i + 1)) return;
      const color = COLORS[i % COLORS.length];
      g.addLayer(L.polyline(path, { color, weight: 2.5, opacity: 0.95 }));
      g.addLayer(L.circleMarker(path[0], { radius: 3, color: '#fff', fillColor: color, fillOpacity: 1, weight: 1 }));
      shown.extend(path);
    });
    g.addTo(map);
    overlay = g;
    bounds = shown.isValid() ? shown : undefined;
    if (bounds) map.fitBounds(bounds, FIT_OPTS);
  }

  // Resize handler: the panel animates its width (transition) the first time a mission is
  // selected, so the container grows after the map was first fit. invalidateSize alone keeps
  // the old zoom → the mission looks too small; refit to the bounds so it fills the new size.
  function refit() {
    if (!map) return;
    map.invalidateSize();
    if (bounds) map.fitBounds(bounds, FIT_OPTS);
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
    const addBase = () => {
      if (!map) return;
      const provider = getProviderById(s.mapProvider);
      // maxNativeZoom caps tile *requests* at the provider's real max, but maxZoom lets the map (and
      // thus fitBounds) zoom further by upscaling those tiles — so a small mission still fills the
      // preview to the ~10px border instead of being capped tiny at the provider's native zoom.
      cachedTileLayer(provider.url, {
        attribution: '',
        maxNativeZoom: provider.maxNativeZoom ?? provider.maxZoom,
        providerId: provider.detectPlaceholders ? provider.id : undefined,
        ...providerLayerOptions(provider),
        // After the spread: a user online map's over-zoom limit (20) must not lower the preview's 21.
        maxZoom: 21,
      }).addTo(map);
    };
    // A custom map resolves only once the custom-map folder is listed.
    if (isCustomProviderId(s.mapProvider)) void ensureCustomMapsLoaded().then(addBase);
    else addBase();
    draw(waypointsJson, page);
    // The detail box lays out after mount (and the panel may still be animating its width
    // wider on the first selection) → fix the size + refit on every resize.
    ro = new ResizeObserver(() => scheduleRefit());
    ro.observe(container);
    scheduleRefit();
  });

  // Redraw when the mission changes (draw() only mutates Leaflet layers → no reactive loop).
  $effect(() => {
    void waypointsJson;
    void format;
    if (map) draw(waypointsJson, page);
  });

  onDestroy(() => {
    if (refitTimer) clearTimeout(refitTimer);
    ro?.disconnect();
    if (map) { map.remove(); map = undefined; }
  });
</script>

<div class="preview-frame">
  <div class="preview-map" bind:this={container}></div>
  {#if missionCount > 1}
    <div class="pager">
      <button onclick={() => step(-1)} aria-label={$t('missionMgr.previewPrev')}>‹</button>
      <span class="pager-label" style:color={page > 0 ? COLORS[(page - 1) % COLORS.length] : undefined}>
        {page === 0 ? $t('missionMgr.previewAll') : `${page} / ${missionCount}`}
      </span>
      <button onclick={() => step(1)} aria-label={$t('missionMgr.previewNext')}>›</button>
    </div>
  {/if}
</div>

<style>
  .preview-frame { position: relative; width: 100%; height: 100%; }
  /* Above the Leaflet panes (z-index up to 700 inside the map container). */
  .pager { position: absolute; left: 50%; bottom: 8px; transform: translateX(-50%); z-index: 800; display: flex; align-items: center; gap: 4px; padding: 2px 4px; background: rgba(30, 30, 30, 0.82); border: 1px solid #444; border-radius: 999px; }
  .pager button { width: 34px; height: 30px; padding: 0; font-size: 22px; line-height: 1; color: #e0e0e0; background: none; border: none; border-radius: 999px; cursor: pointer; }
  .pager button:hover { background: rgba(55, 168, 219, 0.25); }
  .pager-label { min-width: 56px; text-align: center; font-size: 14px; font-weight: 600; color: #e0e0e0; font-variant-numeric: tabular-nums; }
  .preview-map { width: 100%; height: 100%; border: 1px solid #333; border-radius: 4px; background: #1a1a1a; overflow: hidden; }
  .preview-map :global(.leaflet-container) { background: #1a1a1a; cursor: default; font-family: inherit; }
</style>
