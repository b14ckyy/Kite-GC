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
  import { getNavStateColor, classifyMode } from "$lib/helpers/trackColors";
  import type { TrackColorMode } from "$lib/helpers/trackColors";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import type { PlatformType } from "$lib/helpers/uavIcons";
  import { PLATFORM_MULTIROTOR } from "$lib/helpers/uavIcons";

  let {
    playbackTrack = [],
    playbackPoint = null,
    trackColorMode = 'flightmode' as TrackColorMode,
    platformType = PLATFORM_MULTIROTOR as PlatformType,
    fcVariant = 'INAV',
    mapViewMode = '3d' as '2d' | '3d',
    onToggleMapView,
  }: {
    playbackTrack?: TelemetryRecord[];
    playbackPoint?: TelemetryRecord | null;
    trackColorMode?: TrackColorMode;
    platformType?: PlatformType;
    fcVariant?: string;
    mapViewMode?: '2d' | '3d';
    onToggleMapView?: () => void;
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

  // Camera mode: free (no lock) | follow (smooth chase) | orbit (locked target, free orbit)
  type Camera3DMode = 'free' | 'follow' | 'orbit';
  let cameraMode = $state<Camera3DMode>('free');

  // Range (meters to target) for follow and orbit modes. Updated by zoom buttons and
  // mouse-wheel zoom. Separate from free mode which uses Cesium's native zoom.
  let lockRange = 200;

  // Follow cam pitch: user-adjustable, clamped to 0 (horizon) … -π/2 (top-down).
  // Mouse up/down drag changes camera.pitch which we read and clamp each frame.
  let followPitch = -25 * (Math.PI / 180);

  // Orbit cam: tracks the lerped point the camera orbits around
  let orbitCenter = new Cesium.Cartesian3();
  let orbitLerpActive = false;
  let orbitInited = false;
  let orbitCurrentPos = { lat: 0, lon: 0, alt: 0 };
  let orbitTargetPos = { lat: 0, lon: 0, alt: 0 };

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

      if (cameraMode === 'follow') {
        updateChaseCamera(telem.lat, telem.lon, alt, telem.yaw);
      } else if (cameraMode === 'orbit') {
        updateOrbitCamera(telem.lat, telem.lon, alt);
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
    chaseLerpActive = false;
    orbitLerpActive = false;
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
    const color = classifyMode(flags, fcVariant).color;
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

    trackFollowPosition(lat, lon, alt, heading);
    if (cameraMode === 'follow') {
      updateChaseCamera(lat, lon, alt, heading);
    } else if (cameraMode === 'orbit') {
      updateOrbitCamera(lat, lon, alt);
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
    const diff = ((b - a + 540) % 360) - 180;
    return a + diff * t;
  }

  /** Chase/follow camera animation loop — yaw-locked behind UAV, pitch user-adjustable. */
  function chaseAnimationLoop() {
    if (!chaseLerpActive || !viewer) return;

    // Smooth-lerp position and heading toward the live UAV target
    chaseCurrent.lat     = lerp(chaseCurrent.lat,     chaseTarget.lat,     CHASE_SMOOTHING);
    chaseCurrent.lon     = lerp(chaseCurrent.lon,     chaseTarget.lon,     CHASE_SMOOTHING);
    chaseCurrent.alt     = lerp(chaseCurrent.alt,     chaseTarget.alt,     CHASE_SMOOTHING);
    chaseCurrent.heading = lerpAngle(chaseCurrent.heading, chaseTarget.heading, CHASE_SMOOTHING);

    const target = Cesium.Cartesian3.fromDegrees(
      chaseCurrent.lon, chaseCurrent.lat, Math.max(chaseCurrent.alt, 1)
    );

    // Pick up pitch changes from user mouse drag; clamp to 0 (horizon) … -π/2 (top-down).
    // Heading is NOT read from camera — it is always locked to match the UAV.
    const rawPitch = viewer.camera.pitch;
    if (rawPitch >= -Math.PI / 2 && rawPitch <= 0.05) followPitch = rawPitch;

    // Sync lockRange from the live camera distance so mouse-wheel zoom sticks
    // (otherwise our lookAt below would snap the camera back every frame).
    const currentRange = Cesium.Cartesian3.distance(viewer.camera.positionWC, target);
    if (currentRange > 0.01) {
      lockRange = Math.max(LOCK_ZOOM_MIN, Math.min(LOCK_ZOOM_MAX, currentRange));
    }

    // HPR.heading = the camera's LOOK direction. Setting it to UAV heading means
    // the camera looks the same way as the UAV and is therefore positioned BEHIND it.
    const behindHeading = chaseCurrent.heading * (Math.PI / 180);

    viewer.camera.lookAt(target, new Cesium.HeadingPitchRange(behindHeading, followPitch, lockRange));
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

  // ── Orbit Camera ───────────────────────────────────────────────────

  /** Orbit camera animation loop — same CHASE_SMOOTHING as follow cam, free heading/pitch. */
  function orbitAnimationLoop() {
    if (!orbitLerpActive || !viewer) return;

    orbitCurrentPos.lat = lerp(orbitCurrentPos.lat, orbitTargetPos.lat, CHASE_SMOOTHING);
    orbitCurrentPos.lon = lerp(orbitCurrentPos.lon, orbitTargetPos.lon, CHASE_SMOOTHING);
    orbitCurrentPos.alt = lerp(orbitCurrentPos.alt, orbitTargetPos.alt, CHASE_SMOOTHING);

    const h = viewer.camera.heading;
    const p = viewer.camera.pitch;

    const newCenter = Cesium.Cartesian3.fromDegrees(
      orbitCurrentPos.lon, orbitCurrentPos.lat, Math.max(orbitCurrentPos.alt, 1)
    );
    orbitCenter = newCenter;

    // Sync lockRange from the live camera distance so mouse-wheel zoom sticks.
    const currentRange = Cesium.Cartesian3.distance(viewer.camera.positionWC, newCenter);
    if (currentRange > 0.01) {
      lockRange = Math.max(LOCK_ZOOM_MIN, Math.min(LOCK_ZOOM_MAX, currentRange));
    }

    viewer.camera.lookAt(newCenter, new Cesium.HeadingPitchRange(h, p, lockRange));
    viewer.scene.requestRender();

    requestAnimationFrame(orbitAnimationLoop);
  }

  /** Feed a new UAV position into the orbit lerp loop. */
  function updateOrbitCamera(lat: number, lon: number, alt: number) {
    if (!viewer) return;
    orbitTargetPos = { lat, lon, alt };
    if (!orbitInited) {
      orbitCurrentPos = { lat, lon, alt };
      orbitCenter = Cesium.Cartesian3.fromDegrees(lon, lat, Math.max(alt, 1));
      orbitInited = true;
    }
    if (!orbitLerpActive) {
      orbitLerpActive = true;
      requestAnimationFrame(orbitAnimationLoop);
    }
  }

  // ── Camera Mode Cycling ────────────────────────────────────────────

  function cycleCameraMode() {
    if (cameraMode === 'free') {
      cameraMode = 'follow';
      lockRange = 200;
      followPitch = -25 * (Math.PI / 180);
      chaseInited = false;
      if (lastFollowLat !== 0 || lastFollowLon !== 0) {
        // Initial snap: HPR.heading = camera look direction = UAV heading → camera behind UAV
        const initTarget = Cesium.Cartesian3.fromDegrees(
          lastFollowLon, lastFollowLat, Math.max(lastFollowAlt, 1)
        );
        viewer?.camera.lookAt(initTarget, new Cesium.HeadingPitchRange(
          lastFollowHeading * (Math.PI / 180),
          followPitch,
          lockRange
        ));
        updateChaseCamera(lastFollowLat, lastFollowLon, lastFollowAlt, lastFollowHeading);
      }
    } else if (cameraMode === 'follow') {
      cameraMode = 'orbit';
      chaseLerpActive = false;
      chaseInited = false;
      orbitInited = false;
      lockRange = 200;
      if (lastFollowLat !== 0 || lastFollowLon !== 0) {
        // Initial snap, then let orbitAnimationLoop take over
        orbitCenter = Cesium.Cartesian3.fromDegrees(
          lastFollowLon, lastFollowLat, Math.max(lastFollowAlt, 1)
        );
        viewer?.camera.lookAt(orbitCenter, new Cesium.HeadingPitchRange(
          0, -30 * (Math.PI / 180), lockRange
        ));
        orbitCurrentPos = { lat: lastFollowLat, lon: lastFollowLon, alt: lastFollowAlt };
        orbitTargetPos  = { ...orbitCurrentPos };
        orbitInited = true;
        orbitLerpActive = true;
        requestAnimationFrame(orbitAnimationLoop);
        viewer?.scene.requestRender();
      }
    } else {
      cameraMode = 'free';
      chaseLerpActive = false;
      chaseInited = false;
      orbitLerpActive = false;
      orbitInited = false;
      viewer?.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
    }
  }

  // ── Zoom ───────────────────────────────────────────────────────────

  // Zoom limits for follow / orbit modes
  const LOCK_ZOOM_MIN = 20;
  const LOCK_ZOOM_MAX = 500;

  function zoom3D(dir: 1 | -1) {
    if (!viewer) return;
    if (cameraMode === 'free') {
      if (dir > 0) viewer.camera.zoomIn(80);
      else viewer.camera.zoomOut(80);
      viewer.scene.requestRender();
      return;
    }
    lockRange = Math.max(LOCK_ZOOM_MIN, Math.min(LOCK_ZOOM_MAX, lockRange * (dir > 0 ? 0.75 : 1.35)));
    // Apply directly so zoom works even when no telemetry is driving the animation loops.
    const center = cameraMode === 'orbit'
      ? orbitCenter
      : Cesium.Cartesian3.fromDegrees(chaseCurrent.lon, chaseCurrent.lat, Math.max(chaseCurrent.alt, 1));
    if (Cesium.Cartesian3.magnitudeSquared(center) > 1) {
      viewer.camera.lookAt(
        center,
        new Cesium.HeadingPitchRange(viewer.camera.heading, viewer.camera.pitch, lockRange)
      );
      viewer.scene.requestRender();
    }
  }

  // ── Public API ─────────────────────────────────────────────────────

  export function flyTo(lat: number, lon: number, alt = 500) {
    if (!viewer) return;
    viewer.camera.flyTo({
      destination: Cesium.Cartesian3.fromDegrees(lon, lat, alt + 300),
      orientation: { heading: 0, pitch: Cesium.Math.toRadians(-45), roll: 0 },
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

  const camModeTitle = $derived(
    cameraMode === 'free'   ? 'Camera: Free'        :
    cameraMode === 'follow' ? 'Camera: Follow UAV'  :
                              'Camera: Orbit UAV'
  );
</script>

<div class="map3d-wrapper">
  <div class="cesium-container" bind:this={cesiumContainer}></div>

  <div class="map-controls-corner">
    <button
      class="map-control-btn map-mode-btn"
      onclick={() => onToggleMapView?.()}
      title="2D View"
      aria-label="Switch to 2D view"
    >
      2D
    </button>

    <button
      class="map-control-btn map-cam-btn"
      class:mode-free={cameraMode === 'free'}
      class:mode-follow={cameraMode === 'follow'}
      class:mode-orbit={cameraMode === 'orbit'}
      onclick={cycleCameraMode}
      title={camModeTitle}
      aria-label={camModeTitle}
    >
      {#if cameraMode === 'follow'}
        <svg class="cam-icon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <polygon points="12,6 7.5,17.5 12,15.2 16.5,17.5" fill="currentColor"/>
        </svg>
      {:else if cameraMode === 'orbit'}
        <svg class="cam-icon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <circle cx="12" cy="12" r="3" fill="currentColor"/>
          <path d="M12 4 A8 8 0 0 1 20 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
          <polyline points="18,8 20,12 16,11" fill="currentColor"/>
          <path d="M12 20 A8 8 0 0 1 4 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
          <polyline points="6,16 4,12 8,13" fill="currentColor"/>
        </svg>
      {:else}
        <svg class="cam-icon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <polygon class="north-tri" points="12,4.6 9.9,8.6 14.1,8.6"/>
          <g transform="translate(0 -1.5) rotate(-70 12 15)">
            <polygon points="12,8.6 7.7,19.6 12,17.4 16.3,19.6" fill="currentColor"/>
          </g>
        </svg>
      {/if}
    </button>

    <button class="map-control-btn map-zoom-btn" onclick={() => zoom3D(1)}  title="Zoom in"  aria-label="Zoom in">+</button>
    <button class="map-control-btn map-zoom-btn" onclick={() => zoom3D(-1)} title="Zoom out" aria-label="Zoom out">-</button>
  </div>
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

  :global(.cesium-viewer) {
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  /* ── Controls corner — identical layout to Map.svelte ── */
  .map-controls-corner {
    position: absolute;
    bottom: 8px;
    right: 8px;
    z-index: 10000;
    display: flex;
    flex-direction: column;
    gap: 8px;
    pointer-events: all;
  }

  .map-control-btn {
    box-sizing: border-box;
    width: 38px;
    height: 38px;
    background: rgba(46, 46, 46, 0.9);
    border: 2px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(8px);
    transition: background 0.2s, border-color 0.2s, color 0.2s;
    padding: 0;
  }

  .map-control-btn:hover {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
  }

  .map-zoom-btn {
    font-size: 23px;
    line-height: 1;
    font-weight: 700;
  }

  .map-mode-btn {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.03em;
  }

  /* Free = dimmed, no active lock */
  .map-cam-btn.mode-free {
    background: rgba(46, 46, 46, 0.45);
    border-color: rgba(55, 168, 219, 0.45);
    color: rgba(199, 223, 232, 0.95);
  }

  /* Follow = full blue (smooth chase) */
  .map-cam-btn.mode-follow {
    background: rgba(46, 46, 46, 0.92);
    border-color: rgba(55, 168, 219, 0.7);
    color: #37a8db;
  }

  /* Orbit = cyan/teal tint */
  .map-cam-btn.mode-orbit {
    background: rgba(0, 188, 212, 0.2);
    border-color: #00bcd4;
    color: #00bcd4;
  }

  .map-cam-btn:hover {
    background: rgba(55, 168, 219, 0.25) !important;
    border-color: #37a8db !important;
    color: #37a8db !important;
  }

  .cam-icon { overflow: visible; }
  .north-tri { fill: currentColor; opacity: 0.9; }
</style>
