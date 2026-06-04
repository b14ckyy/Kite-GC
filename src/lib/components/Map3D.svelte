<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import * as Cesium from "cesium";
  import "cesium/Build/Cesium/Widgets/widgets.css";

  // Set Cesium base URL for Workers/Assets (defined in vite.config.js)
  if (typeof window !== 'undefined') {
    (window as any).CESIUM_BASE_URL = '/cesium';
  }
    import { telemetry } from "$lib/stores/telemetry";
  import { homePosition } from "$lib/stores/home";
  import { connection, type ConnectionStatus } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import { getProviderById } from "$lib/config/mapProviders";
  import type { MapProvider } from "$lib/config/mapProviders";
  import { getCachedTile, putCachedTile, initTileCache } from "$lib/cache/tileCache";
  import { isKnownUnavailable, isPlaceholderTile } from "$lib/cache/tileAvailability";
  import { isValidGpsCoordinate } from "$lib/helpers/telemetry";
  import {
    segmentTrackByFlightMode,
    segmentTrackByAltitude,
    segmentTrackBySpeed,
    segmentTrackBySignal,
    trackPointColorizer,
    getNavStateColor,
    classifyMode,
    classifyFlightMode,
    type TrackColorMode,
    type TrackSegment,
  } from "$lib/helpers/trackColors";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import type { PlatformType } from "$lib/helpers/uavIcons";
  import { PLATFORM_MULTIROTOR } from "$lib/helpers/uavIcons";
  import {
    mission, showMission, replayActive, launchPoint,
    hasLocation, toDeg, WpAction, type Waypoint, type Mission,
  } from "$lib/stores/mission";
  import { wpIconSpec } from "$lib/helpers/missionIcons";
  import {
    buildDisplayNumbers, isFlightPathWp, findMissionEndIndex, findPreviousGeoWp,
  } from "$lib/helpers/missionGeometry";
  import { resolveMissionAltitudes, type WpMsl } from "$lib/helpers/terrainProfile";

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
  let playbackTrackEntity: Cesium.Entity | undefined;
  // Static full-track line segments — built once per track/color change.
  let playbackTrackParts: Cesium.Entity[] = [];
  // Progressive ground shadow + altitude curtain up to the current replay
  // position — grows behind the UAV so you can read flown progress. Chunked into
  // fixed-size colour runs: finalized chunks are created once and never touched
  // (no flicker, bounded entity count); only the small in-progress chunk is
  // recreated as it grows.
  let decoFinalized: Cesium.Entity[] = [];          // completed chunks (shadow + curtain)
  let decoActiveShadow: Cesium.Entity | undefined;
  let decoActiveCurtain: Cesium.Entity | undefined;
  let decoActivePos: Cesium.Cartesian3[] = [];      // in-progress chunk positions
  let decoActiveColor = '';
  let decoValidTrack: TelemetryRecord[] = [];        // valid points of the loaded track
  let decoColorMode: TrackColorMode = 'flightmode';
  let decoPointColor: (r: TelemetryRecord) => string = () => '#f5a623';
  let decoRenderedCount = 0;                         // flown points drawn (append cursor)
  let decoLastFlown = 0;                             // last observed flown count (direction)
  let decoThrottleUntil = 0;
  let decoTrailingTimer: ReturnType<typeof setTimeout> | null = null;
  let decoRebuildTimer: ReturnType<typeof setTimeout> | null = null; // reverse-scrub debounce
  let decoLoading = false;                           // suppress deco growth during an (async) track load
  let curtainEnabled = true;                         // settings.altitudeCurtain3D
  const DECO_CHUNK_MAX = 150;                        // finalize a chunk at this many points

  // ── Mission overlay (mirrors the 2D map: same markers/lines + drop-lines) ──
  let missionEntities: Cesium.Entity[] = [];
  let missionRenderToken = 0;                        // guards async terrain races
  let curMission: Mission = get(mission);
  let curShowMission = get(showMission);
  let curReplayActive = get(replayActive);
  let curLaunch = get(launchPoint);
  let playbackMarkerEntity: Cesium.Entity | undefined;
  let unsubTelemetry: (() => void) | undefined;
  let unsubHome: (() => void) | undefined;
  let unsubSettingsWatch: (() => void) | undefined;
  let unsubMissionStore: (() => void) | undefined;
  let unsubShowMissionStore: (() => void) | undefined;
  let unsubReplayStore: (() => void) | undefined;
  let unsubLaunchStore: (() => void) | undefined;

    // Trail (live tracking) — initialized in updateTrail3D
  let trailPositions: Cesium.Cartesian3[] = [];
  let lastTrailLat = 0;
  let lastTrailLon = 0;
  let trailSegments3D: { entity: Cesium.Entity; color: string }[] = [];
  let activeTrailEntity: Cesium.Entity | undefined;
  let activeTrailPositions: Cesium.Cartesian3[] = [];
  let trailCurrentColor3D = '';
  const MIN_TRAIL_DIST_3D = 1; // meters
  // Pre-arm trail: a thin plain black, ground-clamped line of GPS movement while
  // DISARMED (monitoring only). Cleared on arm; the colored flight trail takes over.
  let preArmTrailEntity: Cesium.Entity | undefined;
  let preArmPositions3D: Cesium.Cartesian3[] = [];
  let lastPreArmLat = 0;
  let lastPreArmLon = 0;

  // Camera mode: free (no lock) | follow (smooth chase) | orbit (locked target, free orbit)
  type Camera3DMode = 'free' | 'follow' | 'orbit';
  let cameraMode = $state<Camera3DMode>('free');

  // Range (meters to target) for follow and orbit modes. Updated by zoom buttons and
  // mouse-wheel zoom. Separate from free mode which uses Cesium's native zoom.
  let lockRange = 200;

  // Follow cam pitch: user-adjustable, clamped to 0 (horizon) … -π/2 (top-down).
  // Driven by a custom vertical-drag handler (setFollowCameraControls) — Cesium's
  // own rotate is disabled in follow so a sideways drag can't fight the heading lock.
  let followPitch = -20 * (Math.PI / 180);
  // Custom pitch-drag state for heading-locked follow.
  let camDragHandler: Cesium.ScreenSpaceEventHandler | undefined;
  let pitchDragActive = false;
  let pitchDragLastY = 0;
  const FOLLOW_PITCH_SENS = 0.005; // radians per pixel of vertical drag

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

  // Geoid undulation N = ellipsoid − MSL, derived from terrain data
  // (cesiumGround_ellipsoid − copernicusGround_MSL) at the first track point —
  // GPS-independent, so a tower/rooftop start isn't snapped to ground.
  let geoidOffset = 0;
  // GPS MSL at the first fix — the absolute anchor for the (relative, fused)
  // track altitude. Track ellipsoid = startMslGps + geoidOffset + nav_alt_m.
  let startMslGps = 0;

  // Live session: geoidOffset is derived once per live connection from the
  // terrain at the first live GPS fix (so the live UAV sits at the right height
  // without needing a log loaded first).
  let liveGeoidComputed = false;
  let liveGeoidPending = false;
  // Connection-edge detection for source-switch clearing.
  let prevConnStatus: ConnectionStatus = get(connection).status;
  // Set on a fresh connect; the next telemetry frame decides whether to clear
  // (only if the UAV is DISARMED — armed = connection recovery, keep the track).
  let pendingConnectArmCheck = false;
  let unsubConnection: (() => void) | undefined;

  // One-shot camera recenter after a (re)mount. The 2D↔3D toggle remounts this
  // component, so this fires once on every switch to 3D.
  let needsInitialRecenter = true;
  // Debounced imagery refresh after over-zoom placeholders are NEWLY detected:
  // re-requests the 1–3 tiles that slipped through before the placeholder hash
  // was confirmed, so they get replaced by the parent tile without a manual zoom.
  let imageryRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  let activeProviderId = 'osm';

    // Home arm tracking for trail reset on re-arm
  let wasArmed = false;
  const ARMING_FLAG_ARMED = 2;

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

  /** Tile coordinates + provider for over-zoom placeholder detection. */
  type TileMeta = { providerId: string; z: number; x: number; y: number };

  /**
   * Load a tile image — checks IndexedDB cache first, then fetches from network.
   */
  async function loadCachedImage(url: string, meta?: TileMeta): Promise<HTMLImageElement> {
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
          fetchAndCacheImage(url, meta).then(resolve, reject);
        };
        img.src = cached;
      });
    }
    // Cache miss — fetch from network
    return fetchAndCacheImage(url, meta);
  }

  /**
   * Fetch a tile from network, store in IndexedDB cache, return as Image.
   * Throws on error (404, CORS, network) — Cesium will keep the parent tile visible.
   */
  async function fetchAndCacheImage(url: string, meta?: TileMeta): Promise<HTMLImageElement> {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`Tile ${resp.status}`);
    const buf = await resp.arrayBuffer();
    // Over-zoom placeholder? Reject (Cesium keeps the parent z-1 tile) and don't
    // cache it; the region's max zoom is now learned so siblings short-circuit.
    if (meta) {
      const wasKnown = isKnownUnavailable(meta.providerId, meta.z, meta.x, meta.y);
      if (isPlaceholderTile(meta.providerId, meta.z, meta.x, meta.y, buf, url)) {
        // Newly learned region → re-request visible tiles so placeholders that
        // already slipped through (before hash confirmation) get replaced.
        if (!wasKnown) scheduleImageryRefresh();
        throw new Error('placeholder tile (over-zoom)');
      }
    }
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
    const detectId = provider.detectPlaceholders ? provider.id : undefined;
    (imgProvider as any).requestImage = function (
      x: number, y: number, level: number, _request?: unknown
    ): Promise<HTMLImageElement> {
      // Known over-zoom placeholder for this region → fail fast so Cesium keeps
      // the parent (z-1) tile, no network round-trip.
      if (detectId && isKnownUnavailable(detectId, level, x, y)) {
        return Promise.reject(new Error('tile unavailable (over-zoom)'));
      }
      const tileUrl = buildTileUrl(cesiumUrl, x, y, level, subdomains);
      const meta = detectId ? { providerId: detectId, z: level, x, y } : undefined;
      return loadCachedImage(tileUrl, meta);
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

  /**
   * Recenter the camera on the current content once, deferred until the canvas
   * has a real size — the first 2D→3D switch can run this before layout, which
   * made the old inline flyTo a no-op. Targets the UAV (replay marker / live
   * UAV), falling back to the track-start anchor.
   */
  function recenter3D() {
    if (!viewer) return;
    const tryFly = (attempt: number) => {
      if (!viewer) return;
      const c = viewer.canvas;
      if ((c.clientWidth < 2 || c.clientHeight < 2) && attempt < 30) {
        requestAnimationFrame(() => tryFly(attempt + 1));
        return;
      }
      const target = playbackMarkerEntity ?? uavEntity ?? playbackTrackEntity;
      if (!target) return;
      viewer.flyTo(target, {
        duration: 1.2,
        offset: new Cesium.HeadingPitchRange(0, Cesium.Math.toRadians(-45), 0),
      });
    };
    requestAnimationFrame(() => tryFly(0));
  }

  /**
   * Re-apply the active imagery provider (re-requests all visible tiles).
   * Debounced — called when an over-zoom placeholder is NEWLY detected, so the
   * tiles already rendered as placeholder are re-fetched; now-known over-zoom
   * regions short-circuit to the parent tile instead of the blank.
   */
  function scheduleImageryRefresh() {
    if (imageryRefreshTimer != null) return; // batch a burst of detections
    imageryRefreshTimer = setTimeout(() => {
      imageryRefreshTimer = null;
      applyMapProvider(activeProviderId);
    }, 500);
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
      curtainEnabled = s.altitudeCurtain3D ?? true;
    });
    unsubSettings(); // read once, unsubscribe
    activeProviderId = mapProviderId;

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

      const armed = (telem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;

      // Decide clear-on-connect from the first telemetry frame after a connect:
      // only wipe the map if the UAV is DISARMED. If it's armed we assume a
      // connection recovery and keep the existing track.
      if (pendingConnectArmCheck) {
        pendingConnectArmCheck = false;
        if (!armed) clearAllMapData();
      }

      if (!isValidGpsCoordinate(telem.lat, telem.lon)) return;

      // While a replay log is shown, ignore live telemetry for the map — the
      // replay track/marker owns it (prevents the live UAV lingering over replay).
      if (curReplayActive) { wasArmed = armed; return; }

      // Derive the geoid undulation for the live location once per session.
      ensureLiveGeoid(telem.lat, telem.lon);

      // Use MSL altitude + geoid offset for correct ellipsoid height.
      // Fall back to relative baro altitude + geoid offset.
      const altMsl = telem.altMsl ?? telem.altitude;
      const alt = Math.max(altMsl + geoidOffset, 0);
      updateUavPosition3D(telem.lat, telem.lon, alt, telem.yaw, telem.activeFlightModeFlags, armed);
      if (!armed) updatePreArmTrail3D(telem.lat, telem.lon);
      // Live: recenter once after the UAV exists (every 2D→3D switch remounts us).
      if (needsInitialRecenter && uavEntity) { needsInitialRecenter = false; recenter3D(); }
      trackFollowPosition(telem.lat, telem.lon, alt, telem.yaw);

      if (cameraMode === 'follow') {
        updateChaseCamera(telem.lat, telem.lon, alt, telem.yaw);
      } else if (cameraMode === 'orbit') {
        updateOrbitCamera(telem.lat, telem.lon, alt);
      }

      // Trail reset on arm transition (same as Map.svelte): drop the pre-arm
      // black line and start the colored flight trail fresh.
      if (armed && !wasArmed && telem.fixType >= 2 && telem.lat !== 0) {
        resetTrail3D();
        resetPreArmTrail3D();
      }
      wasArmed = armed;

      viewer.scene.requestRender();
    });

    // Subscribe to home position
    unsubHome = homePosition.subscribe((home) => {
      if (!viewer || !home.set) return;
      updateHomePosition3D(home.lat, home.lon, home.alt);
      viewer.scene.requestRender();
    });

    // Watch for live setting changes (map provider, altitude curtain toggle)
    let currentProviderId = mapProviderId;
    unsubSettingsWatch = settings.subscribe((next) => {
      if (next.mapProvider !== currentProviderId) {
        currentProviderId = next.mapProvider;
        activeProviderId = currentProviderId;
        applyMapProvider(currentProviderId);
      }
      const curtain = next.altitudeCurtain3D ?? true;
      if (curtain !== curtainEnabled) {
        curtainEnabled = curtain;
        forceDecoRebuild(); // add/remove the curtain walls
      }
    });

    // Mission overlay — re-render on mission / visibility / launch changes.
    unsubMissionStore = mission.subscribe((m) => { curMission = m; scheduleMissionRender(); });
    unsubShowMissionStore = showMission.subscribe((v) => { curShowMission = v; scheduleMissionRender(); });
    unsubReplayStore = replayActive.subscribe((v) => {
      // Leaving replay (replay → live/planning) is a source switch → always wipe.
      const leavingReplay = curReplayActive && !v;
      curReplayActive = v;
      if (leavingReplay) clearAllMapData();
      scheduleMissionRender();
    });
    unsubLaunchStore = launchPoint.subscribe((v) => { curLaunch = v; scheduleMissionRender(); });

    // Connection edge: on a fresh (re)connect, flag the next telemetry frame to
    // decide clearing (only if DISARMED) and force a live-geoid recompute.
    unsubConnection = connection.subscribe((c) => {
      const was = prevConnStatus;
      prevConnStatus = c.status;
      if (c.status === 'connected' && was !== 'connected') {
        pendingConnectArmCheck = true;
        liveGeoidComputed = false;
        liveGeoidPending = false;
      }
    });
  });

    onDestroy(() => {
    chaseLerpActive = false;
    orbitLerpActive = false;
    if (decoTrailingTimer != null) clearTimeout(decoTrailingTimer);
    if (decoRebuildTimer != null) clearTimeout(decoRebuildTimer);
    if (imageryRefreshTimer != null) clearTimeout(imageryRefreshTimer);
    if (camDragHandler) { camDragHandler.destroy(); camDragHandler = undefined; }
    unsubTelemetry?.();
    unsubHome?.();
    unsubSettingsWatch?.();
    unsubMissionStore?.();
    unsubShowMissionStore?.();
    unsubReplayStore?.();
    unsubLaunchStore?.();
    unsubConnection?.();
    if (viewer && !viewer.isDestroyed()) {
      // Clean up trail segments (they will be destroyed with viewer, but be explicit)
      viewer.entities.removeAll();
      viewer.destroy();
    }
  });

  // ── UAV Entity ─────────────────────────────────────────────────────

  function updateUavPosition3D(lat: number, lon: number, alt: number, heading: number, flightModeFlags = 0, armed = false) {
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
        // Direction symbol omitted in 3D for now — a proper 3D model will replace it later.
      });
    } else {
      (uavEntity.position as Cesium.ConstantPositionProperty).setValue(position);
      (uavEntity.orientation as Cesium.ConstantProperty).setValue(orientation);

      if (uavEntity.point) {
        uavEntity.point.color = new Cesium.ConstantProperty(cesiumColor);
      }
    }

    // Live trail — only while armed (no trail in the disarmed state)
    if (armed) updateTrail3D(lat, lon, alt);
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

    // ── Live Trail (Flightmode-colored segments) ───────────────────────

  function updateTrail3D(lat: number, lon: number, alt: number) {
    if (!viewer) return;

    const pos = Cesium.Cartesian3.fromDegrees(lon, lat, alt);

    // Skip if too close to last point
    if (lastTrailLat !== 0 || lastTrailLon !== 0) {
      const prev = Cesium.Cartesian3.fromDegrees(lastTrailLon, lastTrailLat, alt);
      const dist = Cesium.Cartesian3.distance(pos, prev);
      if (dist < MIN_TRAIL_DIST_3D) return;
    }

    lastTrailLat = lat;
    lastTrailLon = lon;
    trailPositions.push(pos);

    // We need flightModeFlags from telemetry — fall back to unknown
    // (telemetry store is already providing data; we re-read inside updateUavPosition3D)
    // Since this is called from updateUavPosition3D, we need flightModeFlags
    // but we don't have them here. Instead, we get them from the telemetry store.

    // Color by flight mode (same logic as Map.svelte updateTrail)
    // Read active flight mode from the telemetry store directly
    const telem = get(telemetry);
    const color = classifyFlightMode(telem.activeFlightModeFlags ?? 0).color;

    // Color changed → finalize the active segment and start a new one
    if (color !== trailCurrentColor3D && activeTrailPositions.length >= 2) {
      if (activeTrailEntity) {
        trailSegments3D.push({ entity: activeTrailEntity, color: trailCurrentColor3D });
        activeTrailEntity = undefined;
      }
      // Start new segment from last point for continuity
      activeTrailPositions = [activeTrailPositions[activeTrailPositions.length - 1]];
    }

    trailCurrentColor3D = color;
    activeTrailPositions.push(pos);

    // Update or create the active (in-progress) polyline
    if (activeTrailPositions.length >= 2) {
      const cesiumColor = Cesium.Color.fromCssColorString(color).withAlpha(0.7);
      // Use a shallow copy for the positions array so Cesium sees the updated array
      const positionsCopy = [...activeTrailPositions];

      if (activeTrailEntity) {
        // For an already-created entity, update its polyline positions via CallbackProperty
        // Since Cesium Entity polyline positions are not easily mutated, we remove and re-create
        viewer.entities.remove(activeTrailEntity);
      }
      activeTrailEntity = viewer.entities.add({
        polyline: {
          positions: positionsCopy,
          width: 2,
          material: new Cesium.ColorMaterialProperty(cesiumColor),
          clampToGround: false,
        },
      });
    }
  }

  /**
   * Wipe all *source-specific* map data (playback track + progressive deco, live
   * trail, live + replay UAV markers, home) and reset altitude/geoid + session
   * state. The mission overlay is intentionally KEPT (it is source-independent)
   * and re-placed at the reset geoid. Called on source switches:
   *  - leaving replay (replay → live/planning),
   *  - a fresh live connect while DISARMED.
   * (log → log and live → replay are handled at the top of updatePlaybackTrack3D.)
   */
  function clearAllMapData() {
    if (!viewer) return;
    // Playback track + progressive shadow/curtain
    for (const e of playbackTrackParts) viewer.entities.remove(e);
    playbackTrackParts = [];
    if (playbackTrackEntity) {
      viewer.entities.remove(playbackTrackEntity);
      playbackTrackEntity = undefined;
    }
    clearDeco();
    decoValidTrack = [];
    // Live + pre-arm trails
    resetTrail3D();
    resetPreArmTrail3D();
    // Markers (live UAV, replay marker, home)
    if (uavEntity) { viewer.entities.remove(uavEntity); uavEntity = undefined; }
    if (playbackMarkerEntity) { viewer.entities.remove(playbackMarkerEntity); playbackMarkerEntity = undefined; }
    if (homeEntity) { viewer.entities.remove(homeEntity); homeEntity = undefined; }
    // Altitude / geoid / arm-session state
    geoidOffset = 0;
    startMslGps = 0;
    wasArmed = false;
    liveGeoidComputed = false;
    liveGeoidPending = false;
    // Camera follow state (so it re-anchors on the new source)
    chaseInited = false;
    orbitInited = false;
    // Mission stays — re-place it at the reset geoid.
    scheduleMissionRender();
    viewer.scene.requestRender();
  }

  /**
   * Derive the geoid undulation N = cesiumGround_ellipsoid − copernicusGround_MSL
   * at the live location, once per live session. Live UAV ellipsoid height is
   * `altMsl + geoidOffset`, so without this the craft sinks by the full local
   * undulation (~tens of m). Guarded by `liveGeoidPending` (one sample in flight).
   */
  async function ensureLiveGeoid(lat: number, lon: number) {
    if (!viewer || liveGeoidComputed || liveGeoidPending) return;
    liveGeoidPending = true;
    try {
      const terrainProvider = await waitForTerrain(viewer);
      if (terrainProvider) {
        const refPos = Cesium.Cartographic.fromDegrees(lon, lat);
        const sampled = await Cesium.sampleTerrainMostDetailed(terrainProvider, [refPos]);
        const copernicusGround = await invoke<number | null>('terrain_elevation', { lat, lon });
        if (sampled[0] && sampled[0].height != null && copernicusGround != null) {
          geoidOffset = sampled[0].height - copernicusGround;
          liveGeoidComputed = true;
          console.log(`[Map3D] Live geoid N: ${geoidOffset.toFixed(1)}m (cesium=${sampled[0].height.toFixed(1)}, copernicusMSL=${copernicusGround.toFixed(1)})`);
          scheduleMissionRender();
          viewer.scene.requestRender();
        }
      }
    } catch (e) {
      console.warn('[Map3D] Live geoid sample failed', e);
    } finally {
      liveGeoidPending = false;
    }
  }

  /** Thin plain black, ground-clamped trail of GPS movement while disarmed. */
  function updatePreArmTrail3D(lat: number, lon: number) {
    if (!viewer) return;
    if (lastPreArmLat !== 0 || lastPreArmLon !== 0) {
      const a = Cesium.Cartesian3.fromDegrees(lon, lat, 0);
      const b = Cesium.Cartesian3.fromDegrees(lastPreArmLon, lastPreArmLat, 0);
      if (Cesium.Cartesian3.distance(a, b) < MIN_TRAIL_DIST_3D) return;
    }
    lastPreArmLat = lat;
    lastPreArmLon = lon;
    preArmPositions3D.push(Cesium.Cartesian3.fromDegrees(lon, lat, 0));
    if (preArmPositions3D.length >= 2) {
      if (preArmTrailEntity) viewer.entities.remove(preArmTrailEntity);
      preArmTrailEntity = viewer.entities.add({
        polyline: {
          positions: [...preArmPositions3D],
          width: 1,
          material: new Cesium.ColorMaterialProperty(Cesium.Color.BLACK.withAlpha(0.8)),
          clampToGround: true,
        },
      });
    }
  }

  function resetPreArmTrail3D() {
    if (preArmTrailEntity && viewer) { viewer.entities.remove(preArmTrailEntity); preArmTrailEntity = undefined; }
    preArmPositions3D = [];
    lastPreArmLat = 0;
    lastPreArmLon = 0;
  }

  /** Reset the live trail (called when re-arming or clearing). */
  function resetTrail3D() {
    if (!viewer) return;
    for (const seg of trailSegments3D) {
      viewer.entities.remove(seg.entity);
    }
    trailSegments3D = [];
    if (activeTrailEntity) {
      viewer.entities.remove(activeTrailEntity);
      activeTrailEntity = undefined;
    }
    activeTrailPositions = [];
    trailCurrentColor3D = '';
    trailPositions = [];
    lastTrailLat = 0;
    lastTrailLon = 0;
  }

  // ── Playback Track ─────────────────────────────────────────────────

  $effect(() => {
    if (!viewer) return;
    updatePlaybackTrack3D(playbackTrack, trackColorMode);
  });

    async function updatePlaybackTrack3D(track: TelemetryRecord[], colorMode: TrackColorMode) {
    if (!viewer) return;

    // Mark a load in progress and drop the previous track reference up front:
    // this function is async (awaits terrain), and the playbackPoint effect may
    // fire updateFlownDeco() during the await — the guard + empty track stop it
    // from appending old (or mixing old+new) deco points.
    decoLoading = true;
    decoValidTrack = [];

    // Remove old line segments, progressive deco, and the flyTo anchor
    for (const e of playbackTrackParts) viewer.entities.remove(e);
    playbackTrackParts = [];
    clearDeco();
    if (playbackTrackEntity) {
      viewer.entities.remove(playbackTrackEntity);
      playbackTrackEntity = undefined;
    }

    // Loading a (new) replay track is a source switch: wipe any lingering live
    // data — the persistent live UAV, its trail, and the home marker — so we
    // don't stack markers / draw a line across continents. Reset the live-geoid
    // flag so a later live reconnect re-derives it. (Mission is kept.)
    if (track.length >= 2) {
      resetTrail3D();
      resetPreArmTrail3D();
      if (uavEntity) { viewer.entities.remove(uavEntity); uavEntity = undefined; }
      if (homeEntity) { viewer.entities.remove(homeEntity); homeEntity = undefined; }
      liveGeoidComputed = false;
      liveGeoidPending = false;
    }

    if (track.length < 2) { decoValidTrack = []; decoLoading = false; return; }

    // Find first valid GPS point to compute geoid undulation
    const firstPt = track.find(
      (p) => p.lat != null && p.lon != null && isValidGpsCoordinate(p.lat!, p.lon!) && p.alt_m != null
    );

    // Anchor: GPS MSL at the first fix (absolute reference for the relative,
    // fused track altitude). Includes any real height-above-ground at the start
    // (e.g. tower/rooftop) — we do NOT snap it to the ground.
    startMslGps = firstPt?.alt_m ?? 0;

    // Geoid undulation N = cesiumGround_ellipsoid − copernicusGround_MSL at the
    // first point. Derived purely from terrain (NOT the UAV's GPS altitude), so
    // the offset is the true MSL→ellipsoid conversion regardless of how high the
    // craft is when armed. Must wait for Cesium World Terrain to finish loading.
    const terrainProvider = await waitForTerrain(viewer);
    if (firstPt && terrainProvider) {
      try {
        const refPos = Cesium.Cartographic.fromDegrees(firstPt.lon!, firstPt.lat!);
        const sampled = await Cesium.sampleTerrainMostDetailed(terrainProvider, [refPos]);
        const copernicusGround = await invoke<number | null>('terrain_elevation', { lat: firstPt.lat, lon: firstPt.lon });
        if (sampled[0] && sampled[0].height != null) {
          // Prefer the Copernicus MSL ground (same source as terrain analysis /
          // AGL waypoints); fall back to GPS MSL only if it's unavailable.
          const groundMsl = copernicusGround ?? (firstPt.alt_m ?? 0);
          geoidOffset = sampled[0].height - groundMsl;
          console.log(`[Map3D] Geoid N: ${geoidOffset.toFixed(1)}m (cesium=${sampled[0].height.toFixed(1)}, copernicusMSL=${groundMsl.toFixed(1)}, startGPS=${startMslGps.toFixed(1)})`);
        }
      } catch (e) {
        console.warn('[Map3D] Terrain sample failed, geoidOffset=0', e);
      }
    } else {
      console.warn('[Map3D] No terrain provider available, geoidOffset=0');
    }

    // Filter to valid GPS points and convert to Cartesian3 with geoid correction
    const validTrack = track.filter(
      (p) => p.lat != null && p.lon != null && isValidGpsCoordinate(p.lat!, p.lon!)
    );
    if (validTrack.length < 2) return;

    // Build a lookup map: lat,lng key → RELATIVE (fused, arming-relative) altitude
    // for each valid track point. We use nav_alt_m (EKF, smooth, 0 at arm), with
    // baro as a fallback — NOT raw GPS altitude (too erratic for the track shape).
    const relLookup = new Map<string, number>();
    for (const pt of validTrack) {
      const key = `${pt.lat!.toFixed(6)},${pt.lon!.toFixed(6)}`;
      relLookup.set(key, pt.nav_alt_m ?? pt.baro_alt_m ?? 0);
    }

    // Helper: [lat, lon] → Cesium Cartesian3. Ellipsoid height = the GPS-MSL start
    // anchor + geoid undulation + the point's relative fused altitude. This keeps
    // the start at its true height (tower preserved) and the track smooth.
    function segmentToPositions3D(points: [number, number][]): Cesium.Cartesian3[] {
      return points.map(([lat, lon]) => {
        const key = `${lat.toFixed(6)},${lon.toFixed(6)}`;
        const rel = relLookup.get(key) ?? 0;
        return Cesium.Cartesian3.fromDegrees(lon, lat, startMslGps + geoidOffset + rel);
      });
    }

    // The static flight line for a segment: a coloured polyline with a black
    // outline. The ground shadow + altitude curtain are drawn separately and
    // progressively (see updateFlownDeco), so they can grow behind the UAV.
    function addTrackLine(positions: Cesium.Cartesian3[], cssColor: string) {
      if (!viewer || positions.length < 2) return;
      const color = Cesium.Color.fromCssColorString(cssColor);
      playbackTrackParts.push(viewer.entities.add({
        polyline: {
          positions,
          width: 5,
          material: new Cesium.PolylineOutlineMaterialProperty({
            color: color.withAlpha(0.95),
            outlineColor: Cesium.Color.BLACK.withAlpha(0.9),
            outlineWidth: 2,
          }),
          clampToGround: false,
        },
      }));
    }

    // Build color-segmented polylines
    let segments: TrackSegment[] = [];

    if (colorMode === 'flightmode') {
      segments = segmentTrackByFlightMode(validTrack as TelemetryRecord[], fcVariant);
    } else if (colorMode === 'altitude' || colorMode === 'speed' || colorMode === 'signal') {
      const warnAlt = get(settings).warnAltitudeM ?? 120;
      const result =
        colorMode === 'altitude' ? segmentTrackByAltitude(validTrack as TelemetryRecord[], warnAlt) :
        colorMode === 'speed'    ? segmentTrackBySpeed(validTrack as TelemetryRecord[]) :
                                   segmentTrackBySignal(validTrack as TelemetryRecord[]);
      segments = result.segments;
    }

    // Use a parent entity as a grouping container so we can flyTo() the whole track
    // We add individual polyline entities as children for proper colored segments.
    let firstPosition: Cesium.Cartesian3 | undefined;
    let bounds: Cesium.Cartesian3[] = [];

    if (segments.length > 0) {
      for (const seg of segments) {
        if (seg.points.length < 2) continue;
        const positions = segmentToPositions3D(seg.points);
        if (positions.length < 2) continue;
        if (!firstPosition) firstPosition = positions[0];
        bounds.push(...positions);
        addTrackLine(positions, seg.color);
      }
    } else {
      // Fallback: single-color line (e.g. 'none' mode)
      const positions = segmentToPositions3D(
        validTrack.map((p) => [p.lat!, p.lon!] as [number, number])
      );
      if (positions.length < 2) { decoLoading = false; return; }
      firstPosition = positions[0];
      bounds = positions;
      addTrackLine(positions, '#f5a623');
    }

    // Hand the track to the progressive shadow/curtain renderer and draw the
    // portion flown so far (full track when not replaying).
    decoValidTrack = validTrack as TelemetryRecord[];
    decoColorMode = colorMode;
    decoPointColor = trackPointColorizer(
      decoValidTrack, colorMode, fcVariant, get(settings).warnAltitudeM ?? 120,
    );
    decoThrottleUntil = 0; // clearDeco above reset the cursor
    decoLastFlown = 0;
    decoLoading = false; // load complete — allow deco growth again
    updateFlownDeco();
    scheduleMissionRender(); // geoidOffset may have changed → re-place the mission

    // Create a dummy entity at the first position as a recenter fallback anchor.
    if (firstPosition && bounds.length >= 2) {
      playbackTrackEntity = viewer.entities.add({
        position: firstPosition,
        point: { pixelSize: 0 }, // invisible
      });
      // Recenter on load (covers a 2D→3D switch with a log + log→log switches),
      // deferred until the canvas is laid out so the first switch isn't a no-op.
      needsInitialRecenter = false;
      recenter3D();
    }

    viewer.scene.requestRender();
  }

  // ── Progressive ground shadow + altitude curtain ───────────────────
  // The flight LINE is static/full; the shadow + curtain are drawn only up to
  // the current replay position so they build up behind the UAV (showing flown
  // progress). Chunked into fixed-size colour runs so the entity count stays
  // bounded and only the small in-progress chunk is redrawn (no flicker, scales
  // to hour-long logs). When not replaying (playbackPoint null) the full track
  // is shown.

  function posFromRecord(p: TelemetryRecord): Cesium.Cartesian3 {
    const rel = p.nav_alt_m ?? p.baro_alt_m ?? 0; // relative fused altitude (matches the track line)
    return Cesium.Cartesian3.fromDegrees(p.lon!, p.lat!, startMslGps + geoidOffset + rel);
  }

  /** Create the shadow (+ optional curtain) entities for one chunk. */
  function addShadowCurtain(positions: Cesium.Cartesian3[], cssColor: string): { shadow: Cesium.Entity; curtain?: Cesium.Entity } {
    const color = Cesium.Color.fromCssColorString(cssColor);
    const shadow = viewer!.entities.add({
      polyline: {
        positions,
        width: 3,
        material: new Cesium.ColorMaterialProperty(Cesium.Color.BLACK.withAlpha(0.3)),
        clampToGround: true,
      },
    });
    let curtain: Cesium.Entity | undefined;
    if (curtainEnabled) {
      curtain = viewer!.entities.add({
        wall: {
          positions,
          minimumHeights: positions.map(() => 0),
          material: new Cesium.ColorMaterialProperty(color.withAlpha(0.22)),
          outline: false,
        },
      });
    }
    return { shadow, curtain };
  }

  /** Drop the in-progress chunk's entities (it gets recreated as it grows). */
  function reopenActiveChunk() {
    if (!viewer) return;
    if (decoActiveShadow) { viewer.entities.remove(decoActiveShadow); decoActiveShadow = undefined; }
    if (decoActiveCurtain) { viewer.entities.remove(decoActiveCurtain); decoActiveCurtain = undefined; }
  }

  /** Turn the current in-progress chunk positions into a permanent chunk. */
  function finalizeActiveChunk() {
    if (!viewer || decoActivePos.length < 2) return;
    const { shadow, curtain } = addShadowCurtain([...decoActivePos], decoActiveColor);
    decoFinalized.push(shadow);
    if (curtain) decoFinalized.push(curtain);
  }

  /** Remove all deco (finalized + active) and reset the cursor. Also cancels any
   *  pending grow/rebuild timers so a stale timer can't repaint after a clear
   *  (e.g. a log switch drawing a chunk across the old + new track). */
  function clearDeco() {
    if (!viewer) return;
    if (decoRebuildTimer != null) { clearTimeout(decoRebuildTimer); decoRebuildTimer = null; }
    if (decoTrailingTimer != null) { clearTimeout(decoTrailingTimer); decoTrailingTimer = null; }
    for (const e of decoFinalized) viewer.entities.remove(e);
    decoFinalized = [];
    reopenActiveChunk();
    decoActivePos = [];
    decoActiveColor = '';
    decoRenderedCount = 0;
  }

  /** Append valid-track points [fromIdx, toIdx) to the deco, finalizing chunks
   *  on colour change or when they reach DECO_CHUNK_MAX, then redraw the small
   *  in-progress chunk. Existing finalized chunks are never touched. */
  function appendDeco(fromIdx: number, toIdx: number) {
    if (!viewer) return;
    reopenActiveChunk(); // we'll recreate the in-progress chunk at the end
    for (let i = fromIdx; i < toIdx; i++) {
      const p = decoValidTrack[i];
      if (!p || p.lat == null || p.lon == null) continue;
      const pos = posFromRecord(p);
      const color = decoPointColor(p);
      if (decoActivePos.length === 0) {
        decoActiveColor = color;
        decoActivePos = [pos];
        continue;
      }
      if (color !== decoActiveColor || decoActivePos.length >= DECO_CHUNK_MAX) {
        finalizeActiveChunk();
        decoActivePos = [decoActivePos[decoActivePos.length - 1]]; // overlap for continuity
        decoActiveColor = color;
      }
      decoActivePos.push(pos);
    }
    if (decoActivePos.length >= 2) {
      const { shadow, curtain } = addShadowCurtain([...decoActivePos], decoActiveColor);
      decoActiveShadow = shadow;
      decoActiveCurtain = curtain;
    }
    viewer.scene.requestRender();
  }

  function computeFlownCount(): number {
    const pt = playbackPoint;
    if (!pt || pt.timestamp_ms == null) return decoValidTrack.length;
    let n = 0;
    for (const p of decoValidTrack) {
      if (p.timestamp_ms != null && p.timestamp_ms <= pt.timestamp_ms) n++;
      else break;
    }
    return n;
  }

  /** Debounced rebuild after reverse scrubbing — rebuild once, 1 s after the
   *  last backward movement, to the settled position (no per-tick flicker). */
  function armReverseRebuild() {
    if (decoRebuildTimer != null) clearTimeout(decoRebuildTimer);
    decoRebuildTimer = setTimeout(() => {
      decoRebuildTimer = null;
      clearDeco();
      const target = computeFlownCount();
      appendDeco(0, target);
      decoRenderedCount = target;
    }, 1000);
  }

  /** Grow (forward) the deco; on reverse scrub, hide it and rebuild after a
   *  short settle so rapid back-scrubbing doesn't flicker. */
  function updateFlownDeco() {
    if (!viewer || decoLoading) return; // a track load is mid-flight (async) — don't grow yet
    const flownCount = computeFlownCount();
    const goingBack = flownCount < decoLastFlown;
    decoLastFlown = flownCount;

    if (goingBack) {
      // Reverse → clear now, rebuild 1 s after the last backward movement.
      clearDeco();
      armReverseRebuild();
      return;
    }

    // Forward (or no change). A forward move cancels a pending reverse rebuild
    // and rebuilds immediately from the cleared state.
    if (decoRebuildTimer != null) { clearTimeout(decoRebuildTimer); decoRebuildTimer = null; }
    if (flownCount === decoRenderedCount) return;

    // Throttle bursts; trailing call lands the exact extent on pause.
    const now = performance.now();
    if (now < decoThrottleUntil) {
      if (decoTrailingTimer == null) {
        decoTrailingTimer = setTimeout(() => { decoTrailingTimer = null; updateFlownDeco(); }, 90);
      }
      return;
    }
    decoThrottleUntil = now + 90;
    appendDeco(decoRenderedCount, flownCount); // forward → continue the chunks
    decoRenderedCount = flownCount;
  }

  /** Full deco rebuild at the current extent (curtain toggled on/off). */
  function forceDecoRebuild() {
    if (decoRebuildTimer != null) { clearTimeout(decoRebuildTimer); decoRebuildTimer = null; }
    clearDeco();
    decoThrottleUntil = 0;
    decoLastFlown = 0;
    updateFlownDeco();
  }

  // ── Mission overlay ────────────────────────────────────────────────
  // Mirrors the 2D map: identical marker SVGs (as viewport-facing billboards)
  // and identical line colours/styles, drawn as an always-visible overlay
  // (depthFailMaterial / disableDepthTestDistance). The only 3D addition is a
  // thin dashed drop-line from each waypoint down to the ground.

  const LAUNCH_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 44" width="32" height="44">
    <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z" fill="#f39c12" stroke="#fff" stroke-width="2"/>
    <text x="16" y="20" text-anchor="middle" fill="#fff" font-size="13" font-weight="bold" font-family="sans-serif">L</text></svg>`;

  function missionBillboard(lon: number, lat: number, height: number, svg: string, w: number, h: number, ax: number, ay: number, alpha = 1) {
    const ent = viewer!.entities.add({
      position: Cesium.Cartesian3.fromDegrees(lon, lat, height),
      billboard: {
        image: 'data:image/svg+xml,' + encodeURIComponent(svg),
        width: w, height: h,
        pixelOffset: new Cesium.Cartesian2(w / 2 - ax, h / 2 - ay),
        disableDepthTestDistance: Number.POSITIVE_INFINITY, // overlay, never occluded
        color: alpha < 1 ? Cesium.Color.WHITE.withAlpha(alpha) : Cesium.Color.WHITE,
      },
    });
    missionEntities.push(ent);
  }

  function missionLine(positions: Cesium.Cartesian3[], cssColor: string, alpha: number, width: number, dash: boolean) {
    const color = Cesium.Color.fromCssColorString(cssColor).withAlpha(alpha);
    const mat = () => dash
      ? new Cesium.PolylineDashMaterialProperty({ color, dashLength: 10 })
      : new Cesium.ColorMaterialProperty(color);
    const ent = viewer!.entities.add({
      polyline: { positions, width, material: mat(), depthFailMaterial: mat(), clampToGround: false },
    });
    missionEntities.push(ent);
  }

  function scheduleMissionRender() { void renderMission3D(); }

  async function renderMission3D() {
    if (!viewer) return;
    const token = ++missionRenderToken;
    for (const e of missionEntities) viewer.entities.remove(e);
    missionEntities = [];

    // Replay → follow the MISSION toggle; planning/live → always shown.
    const visible = !curReplayActive || curShowMission;
    const wps = curMission.waypoints;
    const hasGeo = wps.some((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
    if (!visible || !hasGeo) { viewer.scene.requestRender(); return; }

    const { alts, launchGround } = await resolveMissionAltitudes(wps, curLaunch);
    if (token !== missionRenderToken || !viewer) return; // superseded by a newer render

    const wpPos = (i: number): Cesium.Cartesian3 | null => {
      const a = alts.get(i);
      if (!a) return null;
      return Cesium.Cartesian3.fromDegrees(toDeg(wps[i].lon), toDeg(wps[i].lat), a.altMsl + geoidOffset);
    };

    const displayNums = buildDisplayNumbers(wps);
    const endIdx = findMissionEndIndex(wps);

    // Launch → first waypoint connector (orange dashed), + launch marker.
    if (curLaunch) {
      const firstGeo = wps.findIndex((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
      const fp = firstGeo >= 0 ? wpPos(firstGeo) : null;
      if (fp) {
        // Anchor the launch marker on the terrain at the launch point.
        const launchHeight = (launchGround ?? 0) + geoidOffset;
        const lpos = Cesium.Cartesian3.fromDegrees(curLaunch.lng, curLaunch.lat, launchHeight);
        missionLine([lpos, fp], '#f39c12', 0.7, 2, true);
        missionBillboard(curLaunch.lng, curLaunch.lat, launchHeight, LAUNCH_SVG, 32, 44, 16, 44);
      }
    }

    // Flight path (active blue + greyed-beyond-end grey dashed), markers, modifier lines, drop-lines.
    const fpActive: Cesium.Cartesian3[] = [];
    const fpGreyed: Cesium.Cartesian3[] = [];
    let enteredGreyed = false;

    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      if (!hasLocation(wp.action) || (wp.lat === 0 && wp.lon === 0)) {
        // Jump / RTH connector lines (use the previous geo-WP as origin)
        if (wp.action === WpAction.Jump && wp.p1 > 0) {
          const src = findPreviousGeoWp(wps, i);
          const tgtIdx = wp.p1 - 1;
          const srcIdx = src ? wps.indexOf(src) : -1;
          const a = srcIdx >= 0 ? wpPos(srcIdx) : null;
          const b = hasLocation(wps[tgtIdx]?.action) ? wpPos(tgtIdx) : null;
          if (a && b) missionLine([a, b], '#8e44ad', 0.8, 2, true);
        }
        if (wp.action === WpAction.Rth) {
          const src = findPreviousGeoWp(wps, i);
          const srcIdx = src ? wps.indexOf(src) : -1;
          const firstGeo = wps.findIndex((w) => isFlightPathWp(w.action) && !(w.lat === 0 && w.lon === 0));
          const a = srcIdx >= 0 ? wpPos(srcIdx) : null;
          const b = firstGeo >= 0 ? wpPos(firstGeo) : null;
          if (a && b) missionLine([a, b], '#e67e22', 0.7, 2, true);
        }
        continue;
      }

      const p = wpPos(i);
      if (!p) continue;
      const greyed = endIdx >= 0 && i > endIdx;
      if (greyed) enteredGreyed = true;

      if (isFlightPathWp(wp.action)) {
        if (!greyed) fpActive.push(p);
        else {
          if (fpGreyed.length === 0 && fpActive.length > 0) fpGreyed.push(fpActive[fpActive.length - 1]);
          fpGreyed.push(p);
        }
      }

      // Drop-line to the ground (white dashed + black dashed outline behind it).
      const a = alts.get(i);
      if (a) {
        const top = p;
        const bottom = Cesium.Cartesian3.fromDegrees(toDeg(wp.lon), toDeg(wp.lat), (a.ground ?? 0) + geoidOffset);
        missionLine([top, bottom], '#000000', 0.85, 3.5, true);  // outline
        missionLine([top, bottom], '#ffffff', 0.95, 1.5, true);  // white dashed
      }

      // Waypoint marker — same SVG as 2D, as a viewport-facing billboard.
      const spec = wpIconSpec(wp, displayNums.get(i) ?? 0, false);
      missionBillboard(toDeg(wp.lon), toDeg(wp.lat), a ? a.altMsl + geoidOffset : 0, spec.svg, spec.width, spec.height, spec.anchorX, spec.anchorY, greyed ? 0.35 : 1);
    }

    if (fpActive.length > 1) missionLine(fpActive, '#37a8db', 0.8, 3, false);
    if (fpGreyed.length > 1) missionLine(fpGreyed, '#666666', 0.4, 2, true);
    void enteredGreyed;

    viewer.scene.requestRender();
  }

  // ── Playback Marker ────────────────────────────────────────────────

  $effect(() => {
    if (!viewer) return;
    updatePlaybackMarker3D(playbackPoint);
    updateFlownDeco(); // grow shadow/curtain to the current replay position
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
    const alt = startMslGps + geoidOffset + (point.nav_alt_m ?? point.baro_alt_m ?? 0);
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
        // Direction symbol omitted in 3D for now — a proper 3D model will replace it later.
      });
    } else {
      (playbackMarkerEntity.position as Cesium.ConstantPositionProperty).setValue(position);
      if (playbackMarkerEntity.point) {
        playbackMarkerEntity.point.color = new Cesium.ConstantProperty(
          Cesium.Color.fromCssColorString(color)
        );
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

  /**
   * Toggle the heading-locked follow input model. When enabled, Cesium's own
   * rotate/tilt/look/pan are disabled (a sideways drag would otherwise rotate
   * the heading that the chase loop forces back every frame → jitter); pitch is
   * driven by a custom vertical-drag handler instead. Zoom (→ lockRange) stays.
   */
  function setFollowCameraControls(enabled: boolean) {
    if (!viewer) return;
    const ssc = viewer.scene.screenSpaceCameraController;
    if (enabled) {
      ssc.enableRotate = false;
      ssc.enableTilt = false;
      ssc.enableLook = false;
      ssc.enableTranslate = false;
      if (!camDragHandler) {
        camDragHandler = new Cesium.ScreenSpaceEventHandler(viewer.scene.canvas);
        camDragHandler.setInputAction((e: Cesium.ScreenSpaceEventHandler.PositionedEvent) => {
          pitchDragActive = true;
          pitchDragLastY = e.position.y;
        }, Cesium.ScreenSpaceEventType.LEFT_DOWN);
        camDragHandler.setInputAction((e: Cesium.ScreenSpaceEventHandler.MotionEvent) => {
          if (!pitchDragActive) return;
          const dy = e.endPosition.y - pitchDragLastY;
          pitchDragLastY = e.endPosition.y;
          // Drag down → look further down (more negative); up → toward horizon. 0 … −90°.
          followPitch = Math.max(-Math.PI / 2, Math.min(0, followPitch - dy * FOLLOW_PITCH_SENS));
          viewer?.scene.requestRender();
        }, Cesium.ScreenSpaceEventType.MOUSE_MOVE);
        camDragHandler.setInputAction(() => { pitchDragActive = false; }, Cesium.ScreenSpaceEventType.LEFT_UP);
      }
    } else {
      ssc.enableRotate = true;
      ssc.enableTilt = true;
      ssc.enableLook = true;
      ssc.enableTranslate = true;
      if (camDragHandler) { camDragHandler.destroy(); camDragHandler = undefined; }
      pitchDragActive = false;
    }
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

    // followPitch is driven by the custom vertical-drag handler (not read back
    // from the camera), and heading is always locked to the UAV — so a sideways
    // drag can't induce the heading fight that caused the jitter.

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
      followPitch = -20 * (Math.PI / 180);
      chaseInited = false;
      setFollowCameraControls(true);
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
      setFollowCameraControls(false); // restore Cesium's free rotate for orbit
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
      setFollowCameraControls(false); // ensure Cesium's controls are restored
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
    resetTrail3D();
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
