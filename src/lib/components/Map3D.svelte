<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import * as Cesium from "cesium";
  import "cesium/Build/Cesium/Widgets/widgets.css";

  // Set Cesium base URL for Workers/Assets (defined in vite.config.js)
  if (typeof window !== 'undefined') {
    (window as any).CESIUM_BASE_URL = '/cesium';
  }
  import { telemetry } from "$lib/stores/telemetry";
  import { homePosition } from "$lib/stores/home";
  import { settings } from "$lib/stores/settings";
  import { getProviderById } from "$lib/config/mapProviders";
  import type { MapProvider } from "$lib/config/mapProviders";
  import { getCachedTile, putCachedTile, initTileCache } from "$lib/cache/tileCache";
  import { isValidGpsCoordinate } from "$lib/helpers/telemetry";
  import { getNavStateColor, classifyFlightMode } from "$lib/helpers/trackColors";
  import type { TrackColorMode } from "$lib/helpers/trackColors";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import type { PlatformType } from "$lib/helpers/uavIcons";
  import { PLATFORM_MULTIROTOR } from "$lib/helpers/uavIcons";

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

  // ── State ──────────────────────────────────────────────────────────

  let cesiumContainer: HTMLDivElement;
  let viewer: Cesium.Viewer | undefined = $state(undefined);
  let uavEntity: Cesium.Entity | undefined;
  let homeEntity: Cesium.Entity | undefined;
  let trailEntity: Cesium.Entity | undefined;
  let playbackTrackEntity: Cesium.Entity | undefined;
  let playbackMarkerEntity: Cesium.Entity | undefined;
  let unsubTelemetry: (() => void) | undefined;
  let unsubHome: (() => void) | undefined;
  let unsubSettingsWatch: (() => void) | undefined;

  // Trail (live tracking)
  let trailPositions: Cesium.Cartesian3[] = [];
  let lastTrailLat = 0;
  let lastTrailLon = 0;

  // Chase camera state
  let chaseModeActive = $state(false);
  let cameraOffset = { heading: 0, pitch: -25, range: 200 };

  // Smooth chase camera interpolation state
  let chaseLerpActive = false;
  let chaseTarget = { lat: 0, lon: 0, alt: 0, heading: 0 };
  let chaseCurrent = { lat: 0, lon: 0, alt: 0, heading: 0 };
  let chaseInited = false;
  const CHASE_SMOOTHING = 0.07; // 0..1 — lower = smoother (exponential lerp factor per frame)

  // Geoid undulation offset: MSL → WGS84 ellipsoid height
  // Computed once per track from terrain sample at first valid point.
  let geoidOffset = 0;

  // 1×1 transparent canvas for tile fallback (avoids gray tiles on 404/error)
  // REMOVED: transparent tiles replace parent → gray globe visible underneath
  // Now we let errors propagate; Cesium keeps the parent tile visible for FAILED tiles.

  /**
   * Wait for Cesium World Terrain to finish loading.
   * Returns the terrain provider once ready, or null on timeout.
   */
  function waitForTerrain(v: Cesium.Viewer, timeoutMs = 15000): Promise<Cesium.TerrainProvider | null> {
    const tp = v.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      return Promise.resolve(tp);
    }
    return new Promise((resolve) => {
      const timeout = setTimeout(() => { listener(); resolve(null); }, timeoutMs);
      const listener = v.scene.terrainProviderChanged.addEventListener(() => {
        const current = v.scene.terrainProvider;
        if (current && !(current instanceof Cesium.EllipsoidTerrainProvider)) {
          clearTimeout(timeout);
          listener();
          resolve(current);
        }
      });
    });
  }

  // ── Cached Imagery Provider ────────────────────────────────────────

  /**
   * Convert a Leaflet-style URL template to Cesium-compatible format.
   * Strips Leaflet-specific {r} (retina) tag.
   */
  function leafletUrlToCesium(url: string): string {
    return url.replace('{r}', '');
  }

  /**
   * Build the actual tile URL from a template + tile coordinates.
   * Used as the IndexedDB cache key.
   */
  function buildTileUrl(template: string, x: number, y: number, z: number, subdomains: string[]): string {
    let url = template
      .replace('{x}', String(x))
      .replace('{y}', String(y))
      .replace('{z}', String(z));
    if (subdomains.length > 0) {
      url = url.replace('{s}', subdomains[(x + y + z) % subdomains.length]);
    }
    return url;
  }

  /**
   * Load a tile image — checks IndexedDB cache first, then fetches from network.
   */
  async function loadCachedImage(url: string): Promise<HTMLImageElement> {
    // Check IndexedDB cache
    const cached = await getCachedTile(url);
    if (cached) {
      return new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image();
        img.crossOrigin = '';
        img.onload = () => { URL.revokeObjectURL(cached); resolve(img); };
        img.onerror = () => {
          URL.revokeObjectURL(cached);
          // Cache entry corrupted — fall back to network
          fetchAndCacheImage(url).then(resolve, reject);
        };
        img.src = cached;
      });
    }
    // Cache miss — fetch from network
    return fetchAndCacheImage(url);
  }

  /**
   * Fetch a tile from network, store in IndexedDB cache, return as Image.
   * Throws on error (404, CORS, network) — Cesium will keep the parent tile visible.
   */
  async function fetchAndCacheImage(url: string): Promise<HTMLImageElement> {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`Tile ${resp.status}`);
    const buf = await resp.arrayBuffer();
    putCachedTile(url, buf).catch(() => {}); // fire-and-forget
    return new Promise<HTMLImageElement>((resolve, reject) => {
      const blob = new Blob([buf]);
      const blobUrl = URL.createObjectURL(blob);
      const img = new Image();
      img.crossOrigin = '';
      img.onload = () => { URL.revokeObjectURL(blobUrl); resolve(img); };
      img.onerror = () => { URL.revokeObjectURL(blobUrl); reject(new Error('Tile decode failed')); };
      img.src = blobUrl;
    });
  }

  /** Return a 1×1 transparent canvas (created once, reused). Synchronous — no async load needed. */
  // REMOVED — transparent tile approach replaced parent tiles with blank → gray globe
  // Error propagation + errorEvent handler is the correct approach.

  /**
   * Create a CesiumJS imagery provider with IndexedDB tile caching.
   * Overrides requestImage to check/fill our shared tile cache.
   */
  function createCachedImageryProvider(provider: MapProvider): Cesium.UrlTemplateImageryProvider {
    const cesiumUrl = leafletUrlToCesium(provider.url);
    const hasSubdomains = cesiumUrl.includes('{s}');
    const subdomains = hasSubdomains ? ['a', 'b', 'c'] : [];

    const imgProvider = new Cesium.UrlTemplateImageryProvider({
      url: cesiumUrl,
      subdomains: hasSubdomains ? subdomains : undefined,
      maximumLevel: provider.cesiumMaxZoom ?? provider.maxZoom,
      credit: new Cesium.Credit(provider.attribution, false),
    });

    // Override requestImage to route through our IndexedDB cache.
    // Errors (404, CORS) propagate as rejections → Cesium marks tile as FAILED
    // → parent tile remains visible (correct upsampling behavior).
    (imgProvider as any).requestImage = function (
      x: number, y: number, level: number, _request?: unknown
    ): Promise<HTMLImageElement> {
      const tileUrl = buildTileUrl(cesiumUrl, x, y, level, subdomains);
      return loadCachedImage(tileUrl);
    };

    // Silently handle tile errors — prevents "rendering has stopped" crash.
    // The parent tile stays visible for failed child tiles.
    imgProvider.errorEvent.addEventListener(() => {});

    return imgProvider;
  }

  /**
   * Apply the selected map provider (base + overlays) to the Cesium viewer.
   */
  function applyMapProvider(providerId: string) {
    if (!viewer) return;

    const provider = getProviderById(providerId);
    const layers = viewer.imageryLayers;

    // Remove all existing layers
    layers.removeAll();

    // Add base layer
    layers.addImageryProvider(createCachedImageryProvider(provider));

    // Add overlay layers (e.g. labels for hybrid)
    if (provider.overlays) {
      for (const ol of provider.overlays) {
        const olProvider = createCachedImageryProvider({
          id: '',
          label: '',
          url: ol.url,
          attribution: ol.attribution || '',
          maxZoom: ol.maxZoom,
          cesiumMaxZoom: ol.cesiumMaxZoom,
        });
        layers.addImageryProvider(olProvider);
      }
    }

    viewer.scene.requestRender();
  }



  // ── Initialization ─────────────────────────────────────────────────

  onMount(async () => {
    // Read settings once
    let ionToken = '';
    let mapProviderId = 'osm';
    let cacheMaxMB = 200;
    const unsubSettings = settings.subscribe((s) => {
      ionToken = s.cesiumIonToken || '';
      mapProviderId = s.mapProvider || 'osm';
      cacheMaxMB = s.mapCacheMaxMB || 0;
    });
    unsubSettings(); // read once, unsubscribe

    // Init tile cache (shared with 2D map)
    await initTileCache(cacheMaxMB);

    // Configure Cesium Ion token if available
    if (ionToken) {
      Cesium.Ion.defaultAccessToken = ionToken;
    }

    // Hide the credit container in a real DOM element
    const creditDiv = document.createElement('div');
    creditDiv.style.display = 'none';
    cesiumContainer.appendChild(creditDiv);

    // Build the base imagery provider from the selected map provider
    const baseProvider = getProviderById(mapProviderId);

    viewer = new Cesium.Viewer(cesiumContainer, {
      // Disable all default widgets for clean embedding
      animation: false,
      timeline: false,
      homeButton: false,
      sceneModePicker: false,
      baseLayerPicker: false,
      navigationHelpButton: false,
      geocoder: false,
      fullscreenButton: false,
      infoBox: false,
      selectionIndicator: false,
      creditContainer: creditDiv,

      // Base imagery from settings (same provider as 2D map)
      baseLayer: new Cesium.ImageryLayer(
        createCachedImageryProvider(baseProvider)
      ),

      // Terrain: use Cesium World Terrain if Ion token is available
      terrain: ionToken
        ? Cesium.Terrain.fromWorldTerrain({ requestVertexNormals: true })
        : undefined,

      // Rendering
      requestRenderMode: true,
      maximumRenderTimeChange: 0.0,
      msaaSamples: 2,
      scene3DOnly: true,
    });

    // Add overlay layers for hybrid providers (also cached)
    if (baseProvider.overlays) {
      for (const ol of baseProvider.overlays) {
        const olProvider = createCachedImageryProvider({
          id: '',
          label: '',
          url: ol.url,
          attribution: ol.attribution || '',
          maxZoom: ol.maxZoom,
          cesiumMaxZoom: ol.cesiumMaxZoom,
        });
        viewer.imageryLayers.addImageryProvider(olProvider);
      }
    }

    // Enable depth testing against terrain when terrain is loaded
    if (ionToken) {
      viewer.scene.globe.depthTestAgainstTerrain = true;
    }

    // ── Performance: limit view distance ──
    // Fog hides distant terrain gradually; far clip plane caps geometry.
    viewer.scene.fog.enabled = true;
    viewer.scene.fog.density = 2.5e-4;       // default 2e-4, slightly denser
    viewer.scene.fog.minimumBrightness = 0.1;
    // Limit tile cache to reduce RAM usage
    viewer.scene.globe.tileCacheSize = 100;   // default 100 tiles

    // Lighting
    viewer.scene.globe.enableLighting = false;

    // Set initial camera to a reasonable default
    viewer.camera.setView({
      destination: Cesium.Cartesian3.fromDegrees(11.0, 48.0, 5000),
      orientation: {
        heading: 0,
        pitch: Cesium.Math.toRadians(-45),
        roll: 0,
      },
    });

    // Subscribe to live telemetry
    unsubTelemetry = telemetry.subscribe((telem) => {
      if (!viewer) return;
      if (!isValidGpsCoordinate(telem.lat, telem.lon)) return;

      // Use MSL altitude + geoid offset for correct ellipsoid height.
      // Fall back to relative baro altitude + geoid offset.
      const altMsl = telem.altMsl ?? telem.altitude;
      const alt = Math.max(altMsl + geoidOffset, 0);
      updateUavPosition3D(telem.lat, telem.lon, alt, telem.yaw, telem.activeFlightModeFlags);
      trackFollowPosition(telem.lat, telem.lon, alt, telem.yaw);

      if (chaseModeActive) {
        updateChaseCamera(telem.lat, telem.lon, alt, telem.yaw);
      }

      viewer.scene.requestRender();
    });

    // Subscribe to home position
    unsubHome = homePosition.subscribe((home) => {
      if (!viewer || !home.set) return;
      updateHomePosition3D(home.lat, home.lon, home.alt);
      viewer.scene.requestRender();
    });

    // Watch for map provider changes in settings (live switching)
    let currentProviderId = mapProviderId;
    unsubSettingsWatch = settings.subscribe((next) => {
      if (next.mapProvider !== currentProviderId) {
        currentProviderId = next.mapProvider;
        applyMapProvider(currentProviderId);
      }
    });
  });

  onDestroy(() => {
    chaseLerpActive = false; // stop animation loop
    unsubTelemetry?.();
    unsubHome?.();
    unsubSettingsWatch?.();
    if (viewer && !viewer.isDestroyed()) {
      viewer.destroy();
    }
  });

  // ── UAV Entity ─────────────────────────────────────────────────────

  function updateUavPosition3D(lat: number, lon: number, alt: number, heading: number, flightModeFlags = 0) {
    if (!viewer) return;

    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);
    const color = getNavStateColor(flightModeFlags);
    const cesiumColor = Cesium.Color.fromCssColorString(color);

    // HPR: heading from INAV is 0=North CW, Cesium heading is from North CW — same convention
    const hpr = new Cesium.HeadingPitchRoll(
      Cesium.Math.toRadians(heading),
      0,
      0
    );
    const orientation = Cesium.Transforms.headingPitchRollQuaternion(position, hpr);

    if (!uavEntity) {
      uavEntity = viewer.entities.add({
        position,
        orientation: orientation as any,
        // 3D model placeholder — uses a colored point + label for now
        point: {
          pixelSize: 14,
          color: cesiumColor,
          outlineColor: Cesium.Color.WHITE,
          outlineWidth: 2,
          heightReference: Cesium.HeightReference.NONE,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
        label: {
          text: 'UAV',
          font: '11px monospace',
          fillColor: Cesium.Color.WHITE,
          outlineColor: Cesium.Color.BLACK,
          outlineWidth: 2,
          style: Cesium.LabelStyle.FILL_AND_OUTLINE,
          verticalOrigin: Cesium.VerticalOrigin.BOTTOM,
          pixelOffset: new Cesium.Cartesian2(0, -18),
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
        // Direction arrow billboard
        billboard: {
          image: buildUavArrowDataUri(color),
          width: 32,
          height: 32,
          rotation: -Cesium.Math.toRadians(heading),
          verticalOrigin: Cesium.VerticalOrigin.CENTER,
          horizontalOrigin: Cesium.HorizontalOrigin.CENTER,
          heightReference: Cesium.HeightReference.NONE,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      });
    } else {
      (uavEntity.position as Cesium.ConstantPositionProperty).setValue(position);
      (uavEntity.orientation as Cesium.ConstantProperty).setValue(orientation);

      if (uavEntity.point) {
        uavEntity.point.color = new Cesium.ConstantProperty(cesiumColor);
      }
      if (uavEntity.billboard) {
        uavEntity.billboard.image = new Cesium.ConstantProperty(buildUavArrowDataUri(color));
        uavEntity.billboard.rotation = new Cesium.ConstantProperty(-Cesium.Math.toRadians(heading));
      }
    }

    // Live trail
    updateTrail3D(lat, lon, alt);
  }

  // ── UAV Arrow Data URI (simple SVG) ────────────────────────────────

  function buildUavArrowDataUri(fillColor: string): string {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24">
      <path d="M12 2 L5 20 L12 16 L19 20 Z" fill="${fillColor}" stroke="white" stroke-width="1"/>
    </svg>`;
    return `data:image/svg+xml;base64,${btoa(svg)}`;
  }

  // ── Home Position ──────────────────────────────────────────────────

  function updateHomePosition3D(lat: number, lon: number, alt: number) {
    if (!viewer) return;

    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);

    if (!homeEntity) {
      homeEntity = viewer.entities.add({
        position,
        point: {
          pixelSize: 12,
          color: Cesium.Color.fromCssColorString('#27ae60'),
          outlineColor: Cesium.Color.WHITE,
          outlineWidth: 2,
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
        label: {
          text: 'H',
          font: 'bold 14px sans-serif',
          fillColor: Cesium.Color.WHITE,
          outlineColor: Cesium.Color.BLACK,
          outlineWidth: 2,
          style: Cesium.LabelStyle.FILL_AND_OUTLINE,
          verticalOrigin: Cesium.VerticalOrigin.BOTTOM,
          pixelOffset: new Cesium.Cartesian2(0, -14),
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      });
    } else {
      (homeEntity.position as Cesium.ConstantPositionProperty).setValue(position);
    }
  }

  // ── Live Trail ─────────────────────────────────────────────────────

  function updateTrail3D(lat: number, lon: number, alt: number) {
    if (!viewer) return;

    // Skip if too close to last point
    const dist = Math.sqrt((lat - lastTrailLat) ** 2 + (lon - lastTrailLon) ** 2) * 111000;
    if (dist < 1) return;

    lastTrailLat = lat;
    lastTrailLon = lon;
    trailPositions.push(Cesium.Cartesian3.fromDegrees(lon, lat, alt));

    if (!trailEntity && trailPositions.length >= 2) {
      trailEntity = viewer.entities.add({
        polyline: {
          positions: new Cesium.CallbackProperty(() => trailPositions, false),
          width: 2,
          material: new Cesium.ColorMaterialProperty(
            Cesium.Color.fromCssColorString('#37a8db').withAlpha(0.7)
          ),
          clampToGround: false,
        },
      });
    }
  }

  // ── Playback Track ─────────────────────────────────────────────────

  $effect(() => {
    if (!viewer) return;
    updatePlaybackTrack3D(playbackTrack, trackColorMode);
  });

  async function updatePlaybackTrack3D(track: TelemetryRecord[], _colorMode: TrackColorMode) {
    if (!viewer) return;

    // Remove old
    if (playbackTrackEntity) {
      viewer.entities.remove(playbackTrackEntity);
      playbackTrackEntity = undefined;
    }

    if (track.length < 2) return;

    // Find first valid GPS point to compute geoid undulation
    const firstPt = track.find(
      (p) => p.lat != null && p.lon != null && isValidGpsCoordinate(p.lat!, p.lon!) && p.alt_m != null
    );

    // Compute geoid offset: sample terrain ellipsoid height at first point,
    // compare with GPS MSL altitude → difference is the geoid undulation.
    // Must wait for Cesium World Terrain to finish loading (async).
    const terrainProvider = await waitForTerrain(viewer);
    if (firstPt && terrainProvider) {
      try {
        const refPos = Cesium.Cartographic.fromDegrees(firstPt.lon!, firstPt.lat!);
        const sampled = await Cesium.sampleTerrainMostDetailed(terrainProvider, [refPos]);
        if (sampled[0] && sampled[0].height != null) {
          // Geoid undulation N = ellipsoid_height - MSL_height (of the ground)
          // At arming, UAV is on the ground: alt_m = ground MSL, baro_alt_m ≈ 0
          const groundMsl = firstPt.alt_m ?? 0;
          geoidOffset = sampled[0].height - groundMsl;
          console.log(`[Map3D] Geoid offset: ${geoidOffset.toFixed(1)}m (terrain=${sampled[0].height.toFixed(1)}, GPS_MSL=${groundMsl.toFixed(1)})`);
        }
      } catch (e) {
        console.warn('[Map3D] Terrain sample failed, geoidOffset=0', e);
      }
    } else {
      console.warn('[Map3D] No terrain provider available, geoidOffset=0');
    }

    const positions: Cesium.Cartesian3[] = [];
    for (const pt of track) {
      if (pt.lat != null && pt.lon != null && isValidGpsCoordinate(pt.lat, pt.lon)) {
        const ellipsoidAlt = (pt.alt_m ?? 0) + geoidOffset;
        positions.push(Cesium.Cartesian3.fromDegrees(pt.lon, pt.lat, ellipsoidAlt));
      }
    }

    if (positions.length < 2) return;

    playbackTrackEntity = viewer.entities.add({
      polyline: {
        positions,
        width: 3,
        material: new Cesium.ColorMaterialProperty(
          Cesium.Color.fromCssColorString('#37a8db').withAlpha(0.8)
        ),
        clampToGround: false,
      },
    });

    // Zoom to track on first load
    viewer.flyTo(playbackTrackEntity, {
      duration: 1.5,
      offset: new Cesium.HeadingPitchRange(0, Cesium.Math.toRadians(-45), 0),
    });

    viewer.scene.requestRender();
  }

  // ── Playback Marker ────────────────────────────────────────────────

  $effect(() => {
    if (!viewer) return;
    updatePlaybackMarker3D(playbackPoint);
  });

  function updatePlaybackMarker3D(point: TelemetryRecord | null) {
    if (!viewer) return;

    if (!point || point.lat == null || point.lon == null || !isValidGpsCoordinate(point.lat, point.lon)) {
      if (playbackMarkerEntity) {
        viewer.entities.remove(playbackMarkerEntity);
        playbackMarkerEntity = undefined;
      }
      return;
    }

    const lat = point.lat;
    const lon = point.lon;
    const alt = (point.alt_m ?? 0) + geoidOffset;
    const heading = point.heading ?? 0;
    const flags = point.active_flight_mode_flags ?? 0;
    const color = classifyFlightMode(flags).color;
    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);

    if (!playbackMarkerEntity) {
      playbackMarkerEntity = viewer.entities.add({
        position,
        point: {
          pixelSize: 12,
          color: Cesium.Color.fromCssColorString(color),
          outlineColor: Cesium.Color.WHITE,
          outlineWidth: 2,
          heightReference: Cesium.HeightReference.NONE,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
        billboard: {
          image: buildUavArrowDataUri(color),
          width: 28,
          height: 28,
          rotation: -Cesium.Math.toRadians(heading),
          verticalOrigin: Cesium.VerticalOrigin.CENTER,
          horizontalOrigin: Cesium.HorizontalOrigin.CENTER,
          heightReference: Cesium.HeightReference.NONE,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      });
    } else {
      (playbackMarkerEntity.position as Cesium.ConstantPositionProperty).setValue(position);
      if (playbackMarkerEntity.point) {
        playbackMarkerEntity.point.color = new Cesium.ConstantProperty(
          Cesium.Color.fromCssColorString(color)
        );
      }
      if (playbackMarkerEntity.billboard) {
        playbackMarkerEntity.billboard.image = new Cesium.ConstantProperty(buildUavArrowDataUri(color));
        playbackMarkerEntity.billboard.rotation = new Cesium.ConstantProperty(-Cesium.Math.toRadians(heading));
      }
    }

    // In chase mode, follow the playback marker
    trackFollowPosition(lat, lon, alt, heading);
    if (chaseModeActive) {
      updateChaseCamera(lat, lon, alt, heading);
    }

    viewer.scene.requestRender();
  }

  // ── Chase Camera ───────────────────────────────────────────────────

  /** Lerp a single value. */
  function lerp(a: number, b: number, t: number): number {
    return a + (b - a) * t;
  }

  /** Shortest-path angle lerp in degrees (handles 359→1 wrap). */
  function lerpAngle(a: number, b: number, t: number): number {
    let diff = ((b - a + 540) % 360) - 180;
    return a + diff * t;
  }

  /** Chase camera animation loop — runs via requestAnimationFrame while active. */
  function chaseAnimationLoop() {
    if (!chaseLerpActive || !viewer) return;

    // Interpolate current toward target
    chaseCurrent.lat = lerp(chaseCurrent.lat, chaseTarget.lat, CHASE_SMOOTHING);
    chaseCurrent.lon = lerp(chaseCurrent.lon, chaseTarget.lon, CHASE_SMOOTHING);
    chaseCurrent.alt = lerp(chaseCurrent.alt, chaseTarget.alt, CHASE_SMOOTHING);
    chaseCurrent.heading = lerpAngle(chaseCurrent.heading, chaseTarget.heading, CHASE_SMOOTHING);

    const target = Cesium.Cartesian3.fromDegrees(
      chaseCurrent.lon, chaseCurrent.lat, Math.max(chaseCurrent.alt, 1)
    );
    const hpr = new Cesium.HeadingPitchRange(
      Cesium.Math.toRadians(chaseCurrent.heading + cameraOffset.heading),
      Cesium.Math.toRadians(cameraOffset.pitch),
      cameraOffset.range
    );

    viewer.camera.lookAt(target, hpr);
    viewer.scene.requestRender();

    requestAnimationFrame(chaseAnimationLoop);
  }

  function updateChaseCamera(lat: number, lon: number, alt: number, heading: number) {
    if (!viewer) return;

    // Set target — the lerp loop will smoothly move toward it
    chaseTarget.lat = lat;
    chaseTarget.lon = lon;
    chaseTarget.alt = alt;
    chaseTarget.heading = heading;

    // First call: snap immediately (no lerp from 0,0)
    if (!chaseInited) {
      chaseCurrent.lat = lat;
      chaseCurrent.lon = lon;
      chaseCurrent.alt = alt;
      chaseCurrent.heading = heading;
      chaseInited = true;
    }

    // Start animation loop if not running
    if (!chaseLerpActive) {
      chaseLerpActive = true;
      requestAnimationFrame(chaseAnimationLoop);
    }
  }

  // Track last known position for follow mode toggle
  let lastFollowLat = 0;
  let lastFollowLon = 0;
  let lastFollowAlt = 0;
  let lastFollowHeading = 0;

  /** Update the "last known position" for follow mode — called from telemetry + playback paths. */
  function trackFollowPosition(lat: number, lon: number, alt: number, heading: number) {
    lastFollowLat = lat;
    lastFollowLon = lon;
    lastFollowAlt = alt;
    lastFollowHeading = heading;
  }

  export function toggleChaseMode() {
    chaseModeActive = !chaseModeActive;
    if (chaseModeActive) {
      // Snap to last known position, then start lerp loop
      chaseInited = false;
      if (lastFollowLat !== 0 || lastFollowLon !== 0) {
        updateChaseCamera(lastFollowLat, lastFollowLon, lastFollowAlt, lastFollowHeading);
      }
    } else {
      // Stop lerp loop and release camera
      chaseLerpActive = false;
      chaseInited = false;
      if (viewer) {
        viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
      }
    }
    return chaseModeActive;
  }

  export function setCameraRange(range: number) {
    cameraOffset.range = Math.max(50, Math.min(5000, range));
    if (chaseModeActive) {
      updateChaseCamera(lastFollowLat, lastFollowLon, lastFollowAlt, lastFollowHeading);
    }
  }

  export function setCameraPitch(pitch: number) {
    cameraOffset.pitch = Math.max(-90, Math.min(-5, pitch));
    if (chaseModeActive) {
      updateChaseCamera(lastFollowLat, lastFollowLon, lastFollowAlt, lastFollowHeading);
    }
  }

  // ── Public API (matching Map.svelte pattern) ───────────────────────

  export function flyTo(lat: number, lon: number, alt = 500) {
    if (!viewer) return;
    viewer.camera.flyTo({
      destination: Cesium.Cartesian3.fromDegrees(lon, lat, alt + 300),
      orientation: {
        heading: 0,
        pitch: Cesium.Math.toRadians(-45),
        roll: 0,
      },
      duration: 1.5,
    });
  }

  export function resetTrail() {
    trailPositions = [];
    lastTrailLat = 0;
    lastTrailLon = 0;
    if (trailEntity && viewer) {
      viewer.entities.remove(trailEntity);
      trailEntity = undefined;
    }
  }
</script>

<div class="map3d-wrapper">
  <div class="cesium-container" bind:this={cesiumContainer}></div>

  <!-- Chase-cam toggle button -->
  <button
    class="chase-btn"
    class:active={chaseModeActive}
    onclick={() => toggleChaseMode()}
    title={chaseModeActive ? 'Free camera' : 'Chase camera (follow UAV)'}
  >
    {chaseModeActive ? '🎥 Follow' : '👁 Free'}
  </button>

  <!-- Camera controls (visible in chase mode) -->
  {#if chaseModeActive}
    <div class="chase-controls">
      <label>
        Range
        <input
          type="range"
          min="50"
          max="2000"
          step="10"
          value={cameraOffset.range}
          oninput={(e) => setCameraRange(Number((e.target as HTMLInputElement).value))}
        />
        <span>{cameraOffset.range}m</span>
      </label>
      <label>
        Pitch
        <input
          type="range"
          min="-90"
          max="-5"
          step="1"
          value={cameraOffset.pitch}
          oninput={(e) => setCameraPitch(Number((e.target as HTMLInputElement).value))}
        />
        <span>{cameraOffset.pitch}°</span>
      </label>
    </div>
  {/if}
</div>

<style>
  .map3d-wrapper {
    width: 100%;
    height: 100%;
    position: relative;
  }

  .cesium-container {
    width: 100%;
    height: 100%;
  }

  /* Override Cesium default widget styling for dark theme */
  .cesium-container :global(.cesium-viewer) {
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  .chase-btn {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 10000;
    min-width: 44px;
    height: 36px;
    padding: 0 10px;
    border: 2px solid rgba(55, 168, 219, 0.6);
    border-radius: 6px;
    background: rgba(46, 46, 46, 0.92);
    color: #fff;
    font-size: 13px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    backdrop-filter: blur(8px);
    transition: border-color 0.2s, background 0.2s;
    pointer-events: all;
  }

  .chase-btn:hover {
    border-color: #37a8db;
    background: rgba(55, 168, 219, 0.25);
  }

  .chase-btn.active {
    border-color: #37a8db;
    background: rgba(55, 168, 219, 0.35);
    box-shadow: 0 0 8px rgba(55, 168, 219, 0.4);
  }

  .chase-controls {
    position: absolute;
    top: 54px;
    right: 10px;
    z-index: 10000;
    background: rgba(46, 46, 46, 0.9);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 6px;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    backdrop-filter: blur(8px);
  }

  .chase-controls label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: #ccc;
    white-space: nowrap;
  }

  .chase-controls input[type="range"] {
    width: 100px;
    accent-color: #37a8db;
  }

  .chase-controls span {
    min-width: 45px;
    text-align: right;
    font-family: monospace;
    color: #37a8db;
  }
</style>
