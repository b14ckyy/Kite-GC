<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import L from "leaflet";
  import "leaflet/dist/leaflet.css";
  import { settings } from "$lib/stores/settings";
  import { get } from "svelte/store";

  let mapContainer: HTMLDivElement;
  let map: L.Map | undefined;

  function saveMapState() {
    if (!map) return;
    const c = map.getCenter();
    settings.patch({
      map: { center: [c.lat, c.lng], zoom: map.getZoom() },
    });
  }

  onMount(() => {
    const s = get(settings);

    map = L.map(mapContainer, {
      center: s.map.center,
      zoom: s.map.zoom,
      zoomControl: false,
      attributionControl: true,
    });

    // Place zoom controls top-right so they don't collide with the nav panel
    L.control.zoom({ position: "topright" }).addTo(map);

    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
      maxZoom: 19,
    }).addTo(map);

    map.on("moveend", saveMapState);
    map.on("zoomend", saveMapState);

    // Invalidate size when container resizes (e.g. side panel toggle)
    const onResize = () => map?.invalidateSize();
    window.addEventListener("resize", onResize);

    // Fix tile rendering on initial load
    setTimeout(() => map?.invalidateSize(), 100);

    return () => window.removeEventListener("resize", onResize);
  });

  onDestroy(() => {
    if (map) {
      map.off("moveend", saveMapState);
      map.off("zoomend", saveMapState);
      map.remove();
    }
  });
</script>

<div bind:this={mapContainer} class="map"></div>

<style>
  .map {
    width: 100%;
    height: 100%;
  }

  /* Fix Leaflet icon paths broken by bundlers */
  :global(.leaflet-default-icon-path) {
    background-image: url("leaflet/dist/images/marker-icon.png");
  }
</style>
