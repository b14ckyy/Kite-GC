<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import L from "leaflet";
  import "leaflet/dist/leaflet.css";
  import { settings } from "$lib/stores/settings";
  import { telemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";
  import { MAP_PROVIDERS, getProviderById, type MapProvider } from "$lib/config/mapProviders";
  import { cachedTileLayer } from "$lib/cache/CachedTileLayer";
  import { initTileCache } from "$lib/cache/tileCache";
  import { t } from 'svelte-i18n';
  import { homePosition } from "$lib/stores/home";
  import MissionLayer from "./MissionLayer.svelte";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import {
    segmentTrackByFlightMode,
    segmentTrackByAltitude,
    segmentTrackBySpeed,
    segmentTrackBySignal,
    getNavStateColor,
    classifyFlightMode,
    type TrackColorMode,
    type TrackSegment,
  } from "$lib/helpers/trackColors";
  import { createUavIcon, PLATFORM_MULTIROTOR, type PlatformType } from "$lib/helpers/uavIcons";

  let {
    playbackTrack = [],
    playbackPoint = null,
    trackColorMode = 'flightmode' as TrackColorMode,
    platformType = PLATFORM_MULTIROTOR as PlatformType,
  }: {
    playbackTrack?: TelemetryRecord[];
    playbackPoint?: TelemetryRecord | null;
    trackColorMode?: TrackColorMode;
    platformType?: PlatformType;
  } = $props();

  const ARMING_FLAG_ARMED = 2;
  const MIN_TRAIL_DIST = 1; // meters — don't add trail point if moved less

  function isValidGpsCoordinate(lat: number, lon: number): boolean {
    return Number.isFinite(lat)
      && Number.isFinite(lon)
      && lat >= -90
      && lat <= 90
      && lon >= -180
      && lon <= 180
      && !(lat === 0 && lon === 0);
  }

  let mapContainer: HTMLDivElement;
  let map = $state<L.Map | undefined>(undefined);
  let uavMarker: L.Marker | undefined;
  let unsubTelemetry: (() => void) | undefined;
  let unsubSettings: (() => void) | undefined;

  // Active tile layers (base + overlays)
  let currentBase: L.TileLayer | undefined;
  let currentOverlays: L.TileLayer[] = [];

  // Flight trail (colored segments by flight mode)
  let trailSegments: L.Polyline[] = [];
  let trailCurrentColor = '';
  let trailCurrentPositions: L.LatLng[] = [];
  let activeTrailLine: L.Polyline | undefined;
  let playbackLayerGroup: L.LayerGroup | undefined;
  let playbackMarker: L.Marker | undefined;
  let lastPlaybackTrackKey = '';

  // Home position
  let homeMarker: L.Marker | undefined;
  let wasArmed = false;

  // Map view mode: north-up (default) or heading-up (rotates map with UAV heading)
  let viewMode = $state<'north-up' | 'heading-up'>('north-up');
  let mapHeading = 0;

  function updateUavPosition(lat: number, lon: number, heading: number, navState = 0) {
    if (!map) return;
    if (!isValidGpsCoordinate(lat, lon)) return;

    const pos: L.LatLngExpression = [lat, lon];
    const color = getNavStateColor(navState);
    const icon = createUavIcon({ heading, fillColor: color, platformType });

    if (!uavMarker) {
      uavMarker = L.marker(pos, { icon, zIndexOffset: 1000 }).addTo(map);
    } else {
      uavMarker.setLatLng(pos);
      uavMarker.setIcon(icon);
    }
  }

  function createHomeIcon(): L.DivIcon {
    return L.divIcon({
      className: "home-icon",
      html: `<div style="width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
                         background: rgba(39, 174, 96, 0.85); border: 2px solid #fff; border-radius: 50%;
                         font-size: 12px; font-weight: bold; color: white; box-shadow: 0 0 6px rgba(0,0,0,0.4);">H</div>`,
      iconSize: [24, 24],
      iconAnchor: [12, 12],
    });
  }

  function updateHomeMarker(lat: number, lon: number) {
    if (!map) return;
    const pos: L.LatLngExpression = [lat, lon];
    if (homeMarker) {
      homeMarker.setLatLng(pos);
    } else {
      homeMarker = L.marker(pos, { icon: createHomeIcon(), zIndexOffset: 500 }).addTo(map);
    }
  }

  function updateTrail(lat: number, lon: number, flightModeFlags: number) {
    if (!map) return;
    const pos = L.latLng(lat, lon);

    // Only add if moved enough from last point
    if (trailCurrentPositions.length > 0 &&
        pos.distanceTo(trailCurrentPositions[trailCurrentPositions.length - 1]) < MIN_TRAIL_DIST) {
      return;
    }

    const color = classifyFlightMode(flightModeFlags).color;

    // Color changed → finalize the active segment and start a new one
    if (color !== trailCurrentColor && trailCurrentPositions.length >= 2) {
      if (activeTrailLine) {
        trailSegments.push(activeTrailLine);
        activeTrailLine = undefined;
      }
      // Start new segment from last point for continuity
      trailCurrentPositions = [trailCurrentPositions[trailCurrentPositions.length - 1]];
    }

    trailCurrentColor = color;
    trailCurrentPositions.push(pos);

    // Update or create the active (in-progress) polyline
    if (trailCurrentPositions.length >= 2) {
      if (activeTrailLine) {
        activeTrailLine.setLatLngs(trailCurrentPositions);
        activeTrailLine.setStyle({ color });
      } else {
        activeTrailLine = L.polyline(trailCurrentPositions, { color, weight: 2, opacity: 0.7 }).addTo(map);
      }
    }
  }

  function updatePlaybackTrack(track: TelemetryRecord[], colorMode: TrackColorMode) {
    if (!map) return;

    // Remove old layer group
    if (playbackLayerGroup) {
      map.removeLayer(playbackLayerGroup);
      playbackLayerGroup = undefined;
    }

    const validTrack = track.filter(
      (point) => point.lat != null && point.lon != null && isValidGpsCoordinate(point.lat!, point.lon!)
    );

    if (validTrack.length === 0) {
      lastPlaybackTrackKey = '';
      return;
    }

    playbackLayerGroup = L.layerGroup().addTo(map);

    if (colorMode === 'flightmode') {
      const segments = segmentTrackByFlightMode(validTrack);
      for (const seg of segments) {
        if (seg.points.length >= 2) {
          L.polyline(seg.points, { color: seg.color, weight: 3, opacity: 0.9 }).addTo(playbackLayerGroup);
        }
      }
    } else if (colorMode === 'altitude' || colorMode === 'speed' || colorMode === 'signal') {
      const warnAlt = get(settings).warnAltitudeM ?? 120;
      const result =
        colorMode === 'altitude' ? segmentTrackByAltitude(validTrack, warnAlt) :
        colorMode === 'speed'    ? segmentTrackBySpeed(validTrack) :
                                   segmentTrackBySignal(validTrack);
      for (const seg of result.segments) {
        if (seg.points.length >= 2) {
          L.polyline(seg.points, { color: seg.color, weight: 3, opacity: 0.9 }).addTo(playbackLayerGroup);
        }
      }
    } else {
      // 'none' or other modes — single orange line
      const positions = validTrack.map((p) => L.latLng(p.lat!, p.lon!));
      L.polyline(positions, { color: "#f5a623", weight: 3, opacity: 0.9, dashArray: "6 5" }).addTo(playbackLayerGroup);
    }

    const positions = validTrack.map((p) => L.latLng(p.lat!, p.lon!));
    const nextKey = `${positions[0].lat}:${positions[0].lng}:${positions.length}:${colorMode}`;
    if (nextKey !== lastPlaybackTrackKey) {
      map.fitBounds(L.latLngBounds(positions), { padding: [36, 36] });
      lastPlaybackTrackKey = nextKey;
    }
  }

  function updatePlaybackMarker(point: TelemetryRecord | null) {
    if (!map) return;

    if (!point || point.lat == null || point.lon == null || !isValidGpsCoordinate(point.lat, point.lon)) {
      if (playbackMarker) {
        map.removeLayer(playbackMarker);
        playbackMarker = undefined;
      }
      return;
    }

    const pos: L.LatLngExpression = [point.lat, point.lon];
    const heading = point.heading ?? 0;
    const color = getNavStateColor(point.nav_state ?? 0);
    const icon = createUavIcon({ heading, fillColor: color, platformType });

    if (playbackMarker) {
      playbackMarker.setLatLng(pos);
      playbackMarker.setIcon(icon);
    } else {
      playbackMarker = L.marker(pos, { icon, zIndexOffset: 900 }).addTo(map);
    }
  }

  function applyProvider(provider: MapProvider) {
    if (!map) return;

    // Remove existing layers
    if (currentBase) map.removeLayer(currentBase);
    for (const ol of currentOverlays) map.removeLayer(ol);
    currentOverlays = [];

    // Add base layer
    currentBase = cachedTileLayer(provider.url, {
      attribution: provider.attribution,
      maxZoom: provider.maxZoom,
    }).addTo(map);

    // Add overlay layers (e.g. labels for hybrid)
    if (provider.overlays) {
      for (const ol of provider.overlays) {
        const layer = cachedTileLayer(ol.url, {
          attribution: ol.attribution,
          maxZoom: ol.maxZoom,
          pane: "overlayPane",
        }).addTo(map);
        currentOverlays.push(layer);
      }
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

    // Initialize tile cache with persisted size limit
    initTileCache(s.mapCacheMaxMB);

    // Apply the persisted (or default) map provider
    applyProvider(getProviderById(s.mapProvider));

    map.on("moveend", saveMapState);
    map.on("zoomend", saveMapState);

    // Invalidate size when container resizes (e.g. side panel toggle)
    const onResize = () => {
      if (viewMode === 'heading-up') applyHeadingUpSize(true);
      map?.invalidateSize();
    };
    window.addEventListener("resize", onResize);

    // Fix tile rendering on initial load
    setTimeout(() => map?.invalidateSize(), 100);

    // Subscribe to telemetry for UAV position, flight trail, and home detection
    unsubTelemetry = telemetry.subscribe((t) => {
      if (t.lastUpdate > 0) {
        updateUavPosition(t.lat, t.lon, t.yaw, t.navState);

        // Heading-up mode: center on UAV and rotate map
        if (viewMode === 'heading-up' && isValidGpsCoordinate(t.lat, t.lon)) {
          map?.setView([t.lat, t.lon], map.getZoom(), { animate: false });
          mapHeading = t.yaw;
          mapContainer?.style.setProperty('--map-rotation', `${-mapHeading}deg`);
        }

        // Flight trail (colored by flight mode)
        if (isValidGpsCoordinate(t.lat, t.lon)) {
          updateTrail(t.lat, t.lon, t.activeFlightModeFlags);
        }

        // Home position: set on arm transition when GPS has fix
        const armed = (t.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
        if (armed && !wasArmed && t.fixType >= 2 && t.lat !== 0) {
          homePosition.set({ lat: t.lat, lon: t.lon, alt: t.altitude, set: true });
          updateHomeMarker(t.lat, t.lon);
          // Clear trail for new flight
          for (const seg of trailSegments) { map?.removeLayer(seg); }
          trailSegments = [];
          if (activeTrailLine) { map?.removeLayer(activeTrailLine); activeTrailLine = undefined; }
          trailCurrentPositions = [];
          trailCurrentColor = '';
        }
        wasArmed = armed;
      }
    });

    // React to provider changes from settings
    let currentProviderId = s.mapProvider;
    unsubSettings = settings.subscribe((next) => {
      if (next.mapProvider !== currentProviderId) {
        currentProviderId = next.mapProvider;
        applyProvider(getProviderById(currentProviderId));
      }
    });

    return () => window.removeEventListener("resize", onResize);
  });

  function applyHeadingUpSize(enable: boolean) {
    if (!mapContainer) return;
    const wrapper = mapContainer.parentElement;
    if (enable && wrapper) {
      // Make container a square with side = diagonal of the wrapper.
      // A rotated square with side = diagonal always fully covers the
      // original rectangle, no matter the rotation angle.
      const w = wrapper.clientWidth;
      const h = wrapper.clientHeight;
      const diag = Math.ceil(Math.sqrt(w * w + h * h));
      const offX = Math.round((diag - w) / 2);
      const offY = Math.round((diag - h) / 2);
      mapContainer.style.width = `${diag}px`;
      mapContainer.style.height = `${diag}px`;
      mapContainer.style.position = 'absolute';
      mapContainer.style.top = `-${offY}px`;
      mapContainer.style.left = `-${offX}px`;
      mapContainer.classList.add('heading-up');
    } else {
      mapContainer.style.width = '';
      mapContainer.style.height = '';
      mapContainer.style.position = '';
      mapContainer.style.top = '';
      mapContainer.style.left = '';
      mapContainer.classList.remove('heading-up');
    }
    // Leaflet must recalculate container size
    setTimeout(() => map?.invalidateSize(), 50);
  }

  function toggleViewMode() {
    viewMode = viewMode === 'north-up' ? 'heading-up' : 'north-up';
    if (viewMode === 'north-up') {
      mapHeading = 0;
      mapContainer?.style.setProperty('--map-rotation', '0deg');
    }
    applyHeadingUpSize(viewMode === 'heading-up');
  }

  $effect(() => {
    if (!map) return;
    updatePlaybackTrack(playbackTrack, trackColorMode);
  });

  $effect(() => {
    if (!map) return;
    updatePlaybackMarker(playbackPoint);
  });

  onDestroy(() => {
    if (unsubTelemetry) unsubTelemetry();
    if (unsubSettings) unsubSettings();
    if (map) {
      map.off("moveend", saveMapState);
      map.off("zoomend", saveMapState);
      if (uavMarker) map.removeLayer(uavMarker);
      for (const seg of trailSegments) map.removeLayer(seg);
      if (activeTrailLine) map.removeLayer(activeTrailLine);
      if (playbackLayerGroup) map.removeLayer(playbackLayerGroup);
      if (playbackMarker) map.removeLayer(playbackMarker);
      if (homeMarker) map.removeLayer(homeMarker);
      map.remove();
    }
  });
</script>

<div class="map-wrapper">
  <div bind:this={mapContainer} class="map" style="--map-rotation: 0deg"></div>

  {#if map}
    <MissionLayer {map} />
  {/if}

  <button class="map-view-btn"
          class:active={viewMode === 'heading-up'}
          onclick={toggleViewMode}
          title={viewMode === 'north-up' ? $t('map.headingUp') : $t('map.northUp')}>
    <svg viewBox="0 0 24 24" width="18" height="18">
      {#if viewMode === 'north-up'}
        <!-- North arrow up -->
        <polygon points="12,3 8,15 12,12 16,15" fill="#ccc" />
        <text x="12" y="21" text-anchor="middle" fill="#ccc" font-size="8" font-weight="bold">N</text>
      {:else}
        <!-- Heading arrow up, highlighted -->
        <polygon points="12,3 8,15 12,12 16,15" fill="#37a8db" />
        <text x="12" y="21" text-anchor="middle" fill="#37a8db" font-size="8" font-weight="bold">H</text>
      {/if}
    </svg>
  </button>
</div>

<style>
  .map-wrapper {
    width: 100%;
    height: 100%;
    overflow: hidden;
    position: relative;
  }

  .map {
    width: 100%;
    height: 100%;
    transition: none;
  }

  /* Heading-up: container size set via inline styles (JS),
     CSS handles only rotation. */
  :global(.map.heading-up) {
    transform: rotate(var(--map-rotation, 0deg));
    transform-origin: center center;
  }

  /* Counter-rotate Leaflet controls so they stay readable */
  :global(.map.heading-up .leaflet-control-zoom),
  :global(.map.heading-up .leaflet-control-attribution) {
    transform: rotate(calc(-1 * var(--map-rotation, 0deg)));
  }

  /* Map view toggle button */
  .map-view-btn {
    position: absolute;
    top: 80px;
    right: 10px;
    z-index: 1000;
    width: 30px;
    height: 30px;
    background: #fff;
    border: 2px solid rgba(0, 0, 0, 0.2);
    border-radius: 4px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 1px 5px rgba(0, 0, 0, 0.4);
  }

  .map-view-btn:hover {
    background: #f4f4f4;
  }

  .map-view-btn.active {
    background: #1a1a1a;
    border-color: #37a8db;
  }

  /* Fix Leaflet icon paths broken by bundlers */
  :global(.leaflet-default-icon-path) {
    background-image: url("leaflet/dist/images/marker-icon.png");
  }
</style>
