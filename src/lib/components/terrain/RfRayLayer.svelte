<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- RfRayLayer.svelte
     2D RF radio-shadow overlay: one transparent triangle per degraded fly-through, pointing from the
     ground interference point (apex — the sightline's closest-approach near-point) out to the
     measurement point (base). The uniform fill encodes the combined RF loss (green→red); only
     corridors worse than RF_RAY_DB are emitted, so the map stays uncluttered.

     Rendered on a Canvas (not SVG): hundreds of polygons pan/zoom in one draw, no per-shape reproject.
     Shown only while the Terrain Analyzer is open in Show-Map (compact) mode — the layout where the
     map and the analyzer strip are visible together.
     Usage: <RfRayLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import L from 'leaflet';
  import { terrainAnalysis, terrainRfRays } from '$lib/stores/terrainAnalysis';
  import { rfColor, type RfRay } from '$lib/helpers/rfLink';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  // Canvas renderer: many polygons redraw in a single pass on pan/zoom (SVG reprojects each path).
  const renderer = L.canvas({ padding: 0.5 });
  const group = L.layerGroup();
  let attached = false;

  function render(rays: RfRay[], visible: boolean) {
    group.clearLayers();
    if (visible) {
      for (const r of rays) {
        const colour = rfColor(r.db);
        L.polygon([r.apex, r.base[0], r.base[1]], {
          renderer,
          color: colour,
          weight: 1,
          opacity: 0.45,
          fillColor: colour,
          fillOpacity: 0.22,
          interactive: false,
        }).addTo(group);
      }
    }
    if (visible && !attached) {
      group.addTo(map);
      attached = true;
    } else if (!visible && attached) {
      map.removeLayer(group);
      attached = false;
    }
  }

  // Visible only in Show-Map (compact) mode while the analyzer is open.
  const unsubState = terrainAnalysis.subscribe((s) => {
    render(get(terrainRfRays), s.open && s.compact);
  });
  const unsubRays = terrainRfRays.subscribe((rays) => {
    const s = get(terrainAnalysis);
    render(rays, s.open && s.compact);
  });

  onDestroy(() => {
    unsubState();
    unsubRays();
    if (attached) map.removeLayer(group);
  });
</script>
