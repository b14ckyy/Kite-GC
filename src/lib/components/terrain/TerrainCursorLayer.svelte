<!-- TerrainCursorLayer.svelte
     Mirrors the Terrain Analysis chart cursor onto the 2D map:
     • a persistent pin where the user clicked (survives panel close)
     • a transient dot following the mouse over the profile
     Usage: <TerrainCursorLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import { terrainCursor, type LatLng } from '$lib/stores/terrainAnalysis';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  let placedMarker: L.Marker | undefined;
  let hoverMarker: L.CircleMarker | undefined;

  function renderPlaced(p: LatLng | null) {
    if (!p) {
      if (placedMarker) {
        map.removeLayer(placedMarker);
        placedMarker = undefined;
      }
      return;
    }
    if (!placedMarker) {
      placedMarker = L.marker([p.lat, p.lon], {
        interactive: false,
        keyboard: false,
        icon: L.divIcon({
          className: 'terrain-cursor-pin',
          html:
            '<div style="background:#37a8db;width:18px;height:18px;border-radius:50% 50% 50% 0;transform:rotate(-45deg);border:2px solid #fff;box-shadow:0 0 4px rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;">' +
            '<span style="width:5px;height:5px;border-radius:50%;background:#fff;transform:rotate(45deg);"></span></div>',
          iconSize: [20, 20],
          iconAnchor: [10, 20],
        }),
      }).addTo(map);
    } else {
      placedMarker.setLatLng([p.lat, p.lon]);
    }
  }

  function renderHover(p: LatLng | null) {
    if (!p) {
      if (hoverMarker) {
        map.removeLayer(hoverMarker);
        hoverMarker = undefined;
      }
      return;
    }
    if (!hoverMarker) {
      hoverMarker = L.circleMarker([p.lat, p.lon], {
        radius: 6,
        color: '#ffffff',
        weight: 2,
        fillColor: '#37a8db',
        fillOpacity: 0.9,
        interactive: false,
      }).addTo(map);
    } else {
      hoverMarker.setLatLng([p.lat, p.lon]);
    }
  }

  const unsub = terrainCursor.subscribe((c) => {
    renderPlaced(c.placed);
    renderHover(c.hover);
  });

  onDestroy(() => {
    unsub();
    if (placedMarker) map.removeLayer(placedMarker);
    if (hoverMarker) map.removeLayer(hoverMarker);
  });
</script>
