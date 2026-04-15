<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import L from "leaflet";
  import "leaflet/dist/leaflet.css";
  import { settings } from "$lib/stores/settings";
  import { telemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";

  let mapContainer: HTMLDivElement;
  let map: L.Map | undefined;
  let uavMarker: L.Marker | undefined;
  let unsubTelemetry: (() => void) | undefined;

  // Simple arrow SVG icon for the UAV — rotated by heading
  function createUavIcon(heading: number): L.DivIcon {
    return L.divIcon({
      className: "uav-icon",
      html: `<div style="transform: rotate(${heading}deg); width: 28px; height: 28px;">
        <svg viewBox="0 0 24 24" width="28" height="28">
          <path d="M12 2 L5 20 L12 16 L19 20 Z" fill="#37a8db" stroke="#1a1a1a" stroke-width="1.5" stroke-linejoin="round"/>
        </svg>
      </div>`,
      iconSize: [28, 28],
      iconAnchor: [14, 14],
    });
  }

  function updateUavPosition(lat: number, lon: number, heading: number) {
    if (!map) return;
    if (lat === 0 && lon === 0) return; // no valid GPS data yet

    const pos: L.LatLngExpression = [lat, lon];

    if (!uavMarker) {
      uavMarker = L.marker(pos, { icon: createUavIcon(heading), zIndexOffset: 1000 }).addTo(map);
    } else {
      uavMarker.setLatLng(pos);
      uavMarker.setIcon(createUavIcon(heading));
    }
  }

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

    // Subscribe to telemetry for UAV position updates
    unsubTelemetry = telemetry.subscribe((t) => {
      if (t.lastUpdate > 0) {
        updateUavPosition(t.lat, t.lon, t.yaw);
      }
    });

    return () => window.removeEventListener("resize", onResize);
  });

  onDestroy(() => {
    if (unsubTelemetry) unsubTelemetry();
    if (map) {
      map.off("moveend", saveMapState);
      map.off("zoomend", saveMapState);
      if (uavMarker) map.removeLayer(uavMarker);
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
