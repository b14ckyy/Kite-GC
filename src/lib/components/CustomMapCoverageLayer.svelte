<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- CustomMapCoverageLayer.svelte
     Coverage frames of the active custom map on the 2D map, one per covered area (from the backend's
     coverage index), always shown while the map is active:
       * while the "finder" — the area's box ×4 per side around its centre — fits the map's viewport
         (size, not position): that finder as a dashed rectangle, so the detail of a pyramid stays
         findable on top of its own low-res overview;
       * once the finder would be larger than the viewport in either direction: the real footprint
         outline, solid — exactly where the best tiles are;
       * a finder smaller than a few pixels becomes a small ring.
     Accent colour, non-interactive. Usage: <CustomMapCoverageLayer {map} />
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import L from 'leaflet';
  import { settings } from '$lib/stores/settings';
  import {
    customMaps,
    customMapCoverage,
    customMapName,
    loadCustomMapCoverage,
    type CustomMapCoverage,
  } from '$lib/stores/customMaps';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  let providerId: string | null = null;
  let coverages: Record<string, CustomMapCoverage> = {};
  let coverage: CustomMapCoverage | null = null;
  let group: L.LayerGroup | undefined;
  let drawnZoom = -1;
  let drawnCoverage: CustomMapCoverage | null = null;
  /** One letter per area (outline / finder / ring) of the last build. */
  let drawnClasses = '';

  const COLOR = '#37a8db';
  /** Below this on-screen size (px, both sides) a finder rectangle is drawn as a ring marker instead. */
  const MIN_PX = 6;

  function clear() {
    if (group) {
      map.removeLayer(group);
      group = undefined;
    }
  }

  /** The VISIBLE map size. In heading-up the Leaflet container is an enlarged square rotated inside
   *  its wrapper (Map.svelte `applyHeadingUpSize`), so `map.getSize()` overstates what is on screen;
   *  the wrapper (the container's parent, `overflow: hidden`) is what the pilot sees. */
  function visibleSize(): { x: number; y: number } {
    const own = map.getSize();
    const host = map.getContainer().parentElement;
    if (!host || host.clientWidth === 0 || host.clientHeight === 0) return own;
    return { x: Math.min(own.x, host.clientWidth), y: Math.min(own.y, host.clientHeight) };
  }

  type AreaClass = 'outline' | 'finder' | 'ring';

  function draw() {
    const z = Math.round(map.getZoom());
    const view = visibleSize();
    // Classify first (cheap): the drawn geometry depends only on the zoom and each area's class, so
    // a resize that changes no class — e.g. a panel transition firing `resize` every frame — keeps
    // the layer as it is instead of rebuilding up to 64 × 200 segments.
    const plans: { cls: AreaClass; cx: number; cy: number; hw: number; hh: number }[] = [];
    for (const a of coverage?.areas ?? []) {
      // The finder: the tight box ×4 per side around its centre, in projected pixels at this zoom
      // (so it stays square to the map).
      const [w, s, e, n] = a.bounds;
      const p0 = map.project([n, w], z);
      const p1 = map.project([s, e], z);
      const hw = Math.abs(p1.x - p0.x) * 2;
      const hh = Math.abs(p1.y - p0.y) * 2;
      // Finder larger than the visible map (a size test, not a position one): real outline.
      const cls: AreaClass = hw * 2 > view.x || hh * 2 > view.y
        ? 'outline'
        : hw * 2 < MIN_PX && hh * 2 < MIN_PX ? 'ring' : 'finder';
      plans.push({ cls, cx: (p0.x + p1.x) / 2, cy: (p0.y + p1.y) / 2, hw, hh });
    }
    const classes = plans.map((p) => p.cls[0]).join('');
    if (coverage === drawnCoverage && z === drawnZoom && classes === drawnClasses) return;
    drawnCoverage = coverage;
    drawnZoom = z;
    drawnClasses = classes;
    clear();
    if (!coverage) return;
    const lines: L.LatLngTuple[][] = [];
    const finders: L.LatLngBounds[] = [];
    const marks: L.LatLngTuple[] = [];
    const clampLat = (v: number) => Math.max(-85.0511, Math.min(85.0511, v));
    const clampLon = (v: number) => Math.max(-180, Math.min(180, v));
    coverage.areas.forEach((a, i) => {
      const { cls, cx, cy, hw, hh } = plans[i];
      const [w, s, e, n] = a.bounds;
      if (cls === 'outline') {
        // Close enough: the real footprint outline — exactly where the best tiles are.
        for (const g of a.segments) lines.push([[g[0], g[1]], [g[2], g[3]]]);
      } else if (cls === 'ring') {
        marks.push([(s + n) / 2, (w + e) / 2]);
      } else {
        const nw = map.unproject([cx - hw, cy - hh], z);
        const se = map.unproject([cx + hw, cy + hh], z);
        finders.push(L.latLngBounds([clampLat(se.lat), clampLon(nw.lng)], [clampLat(nw.lat), clampLon(se.lng)]));
      }
    });
    if (lines.length === 0 && finders.length === 0 && marks.length === 0) return;
    group = L.layerGroup();
    const style = { color: COLOR, weight: 2, opacity: 0.9, interactive: false };
    if (lines.length) L.polyline(lines, style).addTo(group);
    for (const b of finders) L.rectangle(b, { ...style, fill: false, dashArray: '6 4' }).addTo(group);
    for (const c of marks) L.circleMarker(c, { ...style, radius: 5, fill: false }).addTo(group);
    group.addTo(map);
  }

  /** Re-resolve the active custom map and its coverage (loads it on first need), then redraw. */
  function update() {
    if (providerId === null) return;
    const name = customMapName(providerId);
    coverage = name ? (coverages[name] ?? null) : null;
    if (name && !coverage) void loadCustomMapCoverage(name);
    draw();
  }

  const unsubSettings = settings.subscribe((s) => {
    if (s.mapProvider === providerId) return; // the store also emits on every map move
    providerId = s.mapProvider;
    update();
  });
  const unsubMaps = customMaps.subscribe(() => update());
  const unsubCoverage = customMapCoverage.subscribe((c) => {
    coverages = c;
    update();
  });
  onMount(() => {
    const m = map;
    // The viewport size decides finder vs. outline, so a resize (dock, swap into the mini frame)
    // redraws too.
    m.on('zoomend resize', draw);
    return () => m.off('zoomend resize', draw);
  });

  onDestroy(() => {
    unsubSettings();
    unsubMaps();
    unsubCoverage();
    clear();
  });
</script>
