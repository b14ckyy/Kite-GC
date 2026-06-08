<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
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
  import { gcsLocation } from "$lib/stores/gcsLocation";
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
    classifyFlightMode,
    type TrackColorMode,
    type TrackSegment,
  } from "$lib/helpers/trackColors";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import { toTelemetryData } from "$lib/adapters/telemetryAdapter";
  import type { PlatformType, UavModelOverride } from "$lib/helpers/uavIcons";
  import { PLATFORM_MULTIROTOR } from "$lib/helpers/uavIcons";
  import { modelUriForPlatform } from "$lib/helpers/uavModels";
  import {
    mission, showMission, replayActive, launchPoint,
    hasLocation, toDeg, WpAction, type Waypoint, type Mission,
  } from "$lib/stores/mission";
  import { wpIconSpec } from "$lib/helpers/missionIcons";
  import {
    buildDisplayNumbers, isFlightPathWp, findMissionEndIndex, findPreviousGeoWp,
  } from "$lib/helpers/missionGeometry";
  import { resolveMissionAltitudes, type WpMsl } from "$lib/helpers/terrainProfile";
  import { sunAltitudeDeg, cesiumLikeBrightness } from "$lib/utils/sun";
  import { ensureUserLocation, resolveUserLocation, userGeoLocation } from "$lib/helpers/userLocation";
  import FpvHud from "$lib/components/FpvHud.svelte";
  import { convertSpeed, convertAltitude, convertDistance, convertVerticalSpeed, formatConverted } from "$lib/utils/units";
  import { haversineDistance, bearing } from "$lib/utils/geo";
  import type { SpeedUnit, AltitudeUnit, RadarMapSettings, GcsMode } from "$lib/stores/settings";
  import { radarVehicles, radarSelection, type RadarSnapshot } from "$lib/stores/radarTracking";
  import { contactColor, ffContactColor, contactVisibleOnMap, REL_OVERRIDE_M } from "$lib/helpers/radarMap";
  import { ARROW_POLY, contactModelClass, radarModelUri, type RadarModelClass } from "$lib/helpers/radar3d";
  import { radarAlertLevels, ALERT_CONFIG, type AlertLevel } from "$lib/controllers/radarAlerts";

  let {
    active = true,
    playbackTrack = [],
    playbackPoint = null,
    replayStartEpochMs = null,
    trackColorMode = 'flightmode' as TrackColorMode,
    platformType = PLATFORM_MULTIROTOR as PlatformType,
    modelOverride = 'auto' as UavModelOverride,
    fcVariant = 'INAV',
    mapViewMode = '3d' as '2d' | '3d',
    onToggleMapView,
    onCamFocus,
    radarActive = false,
    radarMapSettings = null,
    radarRefAltM = null,
    radarReference = null,
  }: {
    active?: boolean;
    playbackTrack?: TelemetryRecord[];
    playbackPoint?: TelemetryRecord | null;
    /** Absolute flight-start epoch (ms); playbackPoint.timestamp_ms is relative to this. */
    replayStartEpochMs?: number | null;
    trackColorMode?: TrackColorMode;
    platformType?: PlatformType;
    modelOverride?: UavModelOverride;
    fcVariant?: string;
    mapViewMode?: '2d' | '3d';
    onToggleMapView?: () => void;
    /** Fired on camera move-end with the focus point over the globe — drives the radar query centre. */
    onCamFocus?: (lat: number, lon: number) => void;
    /** Radar master enable (renders no contacts when off). */
    radarActive?: boolean;
    /** Map rendering controls for radar contacts, or null. */
    radarMapSettings?: RadarMapSettings | null;
    /** Reference altitude (m MSL) for the relative-altitude colour scale / ground gating, or null. */
    radarRefAltM?: number | null;
    /** Distance/bearing reference (UAV valid-fix else GCS) for the selected-contact label, or null. */
    radarReference?: { lat: number; lon: number } | null;
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

  // ── Sun / lighting ────────────────────────────────────────────────
  let lightingEnabled = false;                       // settings.realLighting3D → globe sun-shading
  let replayTimeEnabled = false;                     // settings.logReplayTime → clock from log timestamp
  let nightModeSetting: 'off' | 'auto' | 'on' = 'off'; // settings.nightMode2D (also applies to 3D)
  // Dev tool: override the sky clock with a manual time-of-day to preview lighting.
  let devTimeActive = $state(false);                 // slider overrides clock when on
  let devTimeMin = $state(12 * 60);                  // minutes since midnight (local solar at view lon)
  // Night dimming: Cesium's own night side is ×0.3; we darken ONLY the imagery layers to
  // match (entities/sky stay bright, like the 2D map). Applied as the *darker of the two*
  // sources — never stacked on top of the real-lighting night shading.
  const NIGHT_BRIGHTNESS_3D = 0.3;
  let appliedImageryBrightness = 1.0;                // last value pushed to imagery layers
  let nightTimer3D: ReturnType<typeof setInterval> | undefined; // auto re-check (system-time drift)
  let unsubUserGeo: (() => void) | undefined;        // recompute when OS geolocation resolves

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
  //            | fpv (first-person: camera replaces the model, follows all axes)
  type Camera3DMode = 'free' | 'follow' | 'orbit' | 'fpv';
  let cameraMode = $state<Camera3DMode>('free');

  // ── FPV (first-person view) ─────────────────────────────────────────
  const FPV_FOV_MIN = 30;            // narrowest "lens" (deg, horizontal)
  const FPV_FOV_MAX = 120;           // widest "lens"
  const FPV_EYE_HEIGHT_M = 0.5;      // raise the eye slightly above the track to avoid trail clipping
  const FPV_TRACK_ALPHA = 0.4;       // flight track is dimmed so it doesn't fill the view
  let fpvFov = $state(60);           // horizontal field of view (deg), the FPV "zoom"
  let fpvWheelHandler: Cesium.ScreenSpaceEventHandler | undefined;
  // Live HUD data (raw SI) for the FPV overlay — updated from the active source (replay/live).
  let hud = $state({ heading: 0, pitch: 0, roll: 0, altM: 0, speedMs: 0 });
  let hudSpeedUnit = $state<SpeedUnit>('kmh');
  let hudAltUnit = $state<AltitudeUnit>('m');
  const fpvScratchM3 = new Cesium.Matrix3();
  const fpvScratchDir = new Cesium.Cartesian3();
  const fpvScratchUp = new Cesium.Cartesian3();

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

  // geoidOffset is derived ONCE per scene from the terrain at the first thing that
  // gets drawn — live GPS fix, replay track, OR a mission/launch waypoint. Deriving
  // it from a waypoint (not just a UAV) means the 3D mission preview is height-correct
  // without a live link or a loaded log. Generalises to future ADS-B / followers
  // (compute from their first position).
  //
  // A SINGLE-FLIGHT awaitable promise (computeGeoidOnce) backs it: when a flight log
  // with a linked mission loads, the track and the mission both kick a computation
  // almost simultaneously — they share the one in-flight promise, so the mission waits
  // for the SAME offset instead of drawing at 0 and racing a re-render.
  let geoidComputed = false;
  let geoidPromise: Promise<boolean> | null = null; // the in-flight single-flight computation
  let geoidGen = 0; // bumped on a source switch so an in-flight sample can't apply a stale offset
  // Connection-edge detection for source-switch clearing.
  let prevConnStatus: ConnectionStatus = get(connection).status;
  // Set on a fresh connect; the next telemetry frame decides whether to clear
  // (only if the UAV is DISARMED — armed = connection recovery, keep the track).
  let pendingConnectArmCheck = false;
  let unsubConnection: (() => void) | undefined;

  // ── Foreign-vehicle (radar) 3D contacts ──────────────────────────────
  // One record per contact id, holding the live data + Cesium entities; CallbackProperties read from
  // the record so we diff (update fields) rather than recreate entities each snapshot. Flat extruded
  // silhouette sized in px by camera distance, drop-line + ground circle gated to the colour-scale zone.
  type Radar3dRec = {
    id: string;
    lat: number; lon: number;
    headingDeg: number | null;
    modelClass: RadarModelClass; // which radar glb to render (mapped from system + ADS-B category)
    callsign: string;          // label text (callsign or id)
    altM: number;              // altitude (m) for the label
    groundSpeedMs: number | null;
    verticalSpeedMs: number | null;
    contactEll: number;        // contact ellipsoid height (MSL + geoid)
    color: Cesium.Color;       // altitude-coded tint
    showGround: boolean;       // drop-line + circle visible (Δ ≤ +2000 m, or debug+show-all)
    selected: boolean;
    hideRadiusM: number;       // radius beyond which the contact is hidden (showAll → 1000 km)
    // Drop-line colour held in a single ConstantProperty so we update it IN PLACE (setValue) instead of
    // replacing the material each poll — replacing rebuilds the material (the colour-coded "blink").
    dropColorCP?: Cesium.ConstantProperty;
    dropColor?: Cesium.Color;
    alertLevel: AlertLevel | null; // conflict-alert highlight (pulsing red/yellow), or null
    groundSig?: string;        // last-synced ground signature — skip the whole ground update if unchanged
    entities: Cesium.Entity[];
  };
  const radar3dRecs = new Map<string, Radar3dRec>();
  // New contacts are created a few per frame (a dense area can add ~100 at once — building all their
  // models + ground geometry in one frame stutters). The rec is in `radar3dRecs` immediately (with no
  // entities yet) and queued here; `drainRadarCreateQueue` builds them incrementally.
  const radar3dCreateQueue: Radar3dRec[] = [];
  let radar3dCreateRaf = 0;
  let radar3dSnap: RadarSnapshot = { adsb: [], formationFlight: [], radio: [], lastUpdate: 0 };
  let radar3dSelectedId: string | null = null;
  let radar3dAlertLevels: Map<string, AlertLevel> = new Map();
  let unsubRadar3d: (() => void) | undefined;
  let unsubRadarSel3d: (() => void) | undefined;
  let unsubRadarAlerts3d: (() => void) | undefined;
  // Click/hover picking: map each contact entity back to its id; handler set up in onMount.
  const radar3dEntityIds = new WeakMap<Cesium.Entity, string>();
  let radar3dPickHandler: Cesium.ScreenSpaceEventHandler | undefined;

  // One-shot camera recenter after a (re)mount. The 2D↔3D toggle remounts this
  // component, so this fires once on every switch to 3D.
  let needsInitialRecenter = true;

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
    // NOTE: we deliberately do NOT trigger a full imagery refresh here. Re-applying
    // the provider does layers.removeAll() — a full-globe teardown that blanks every
    // tile to dark blue and, when it fires per newly-crossed region during a 3D replay
    // over a sparse area, storms into a stutter/permanent-blue collapse. The 1–2 blank
    // tiles that slipped through before the hash was confirmed are self-correcting:
    // any camera move re-requests them (now known → parent shown).
    if (meta && isPlaceholderTile(meta.providerId, meta.z, meta.x, meta.y, buf, url)) {
      throw new Error('placeholder tile (over-zoom)');
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

    // Fresh layers default to brightness 1.0 → reset our cache and re-apply night dim.
    appliedImageryBrightness = 1.0;
    updateNightDim3D();

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
    // Suppressed right after a 2D→3D switch: the camera was just synced to the 2D
    // viewport (setCameraFromMapView) and must not be yanked away by a content
    // fly-to triggered by the mount's track effect. A genuine later log-load (well
    // after the switch) is past the window and still frames the new track.
    if (performance.now() < recenterSuppressUntil) return;
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

  // Suppress content fly-to until this timestamp (set by a 2D→3D camera sync).
  let recenterSuppressUntil = 0;
  // Pitch used when framing the 2D viewport in free mode (steep-ish, near top-down 2D).
  const SYNC_PITCH = Cesium.Math.toRadians(-55);

  /**
   * Point the 3D camera at the spot the 2D (Leaflet) map currently shows (its
   * persisted `settings.map.center`). Only the GROUND TARGET is taken from 2D — the
   * camera keeps its OWN zoom/heading/pitch, so a switch never resets the 3D zoom
   * (2D↔3D zooms are independent; transferring zoom across was unreliable over
   * mountainous terrain anyway).
   *
   * If the 2D map wasn't panned since we left 3D, the EXACT captured camera matrix is
   * replayed (setView) — re-deriving it via a ground pick would drift the zoom every
   * round-trip, because the pick hits TERRAIN (height > 0) while a lookAt targets the
   * ellipsoid (height 0). If the 2D map WAS panned, the camera re-targets the new
   * centre keeping its zoom/angle. First-ever open (no snapshot) derives a starting
   * range from the 2D zoom. Applied synchronously (no fly-to).
   */
  function setCameraFromMapView(attempt = 0) {
    if (!viewer) return;
    const m = get(settings).map;
    if (!m?.center) { recenter3D(); return; }
    const [lat, lon] = m.center;
    const snap = cam3dSnapshot;
    if (snap) {
      const panned = Cesium.Cartesian3.distance(
        Cesium.Cartesian3.fromDegrees(lon, lat),
        Cesium.Cartesian3.fromDegrees(snap.targetLon, snap.targetLat),
      ) > 8; // metres → user moved the 2D map
      if (!panned) {
        // Exact restore — replay the captured matrix so the zoom can't drift.
        viewer.camera.setView({ destination: snap.position, orientation: { heading: snap.heading, pitch: snap.pitch, roll: snap.roll } });
      } else {
        // Re-target the new 2D centre, keeping 3D's own zoom/angle.
        viewer.camera.lookAt(Cesium.Cartesian3.fromDegrees(lon, lat, 0), new Cesium.HeadingPitchRange(snap.heading, snap.pitch, snap.range));
        viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
      }
      recenterSuppressUntil = performance.now() + 1500;
      viewer.scene.requestRender();
      return;
    }
    // First-ever open: derive a starting range from the 2D zoom (needs canvas + FOV).
    const c = viewer.canvas;
    if ((c.clientWidth < 2 || c.clientHeight < 2) && attempt < 30) {
      requestAnimationFrame(() => setCameraFromMapView(attempt + 1));
      return;
    }
    const hPx = c.clientHeight || 600;
    const mpp = 156543.03392 * Math.cos(lat * Math.PI / 180) / Math.pow(2, m.zoom ?? 14);
    let fovy = Cesium.Math.toRadians(45);
    try { const f = (viewer.camera.frustum as Cesium.PerspectiveFrustum).fovy; if (f && isFinite(f)) fovy = f; } catch { /* aspectRatio not ready yet */ }
    const range = Math.max(50, (mpp * hPx) / (2 * Math.tan(fovy / 2)));
    viewer.camera.lookAt(Cesium.Cartesian3.fromDegrees(lon, lat, 0), new Cesium.HeadingPitchRange(0, SYNC_PITCH, range));
    viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY); // release the frame so manual controls work
    recenterSuppressUntil = performance.now() + 1500;
    viewer.scene.requestRender();
  }

  /** Re-anchor a locked (follow/orbit) camera onto the UAV after a 2D→3D switch. */
  function reanchorLockCamera() {
    if (!pHas) { setCameraFromMapView(); return; }
    chaseInited = false;
    orbitInited = false;
    if (cameraMode === 'fpv') {
      const q = smEntity && (smEntity.orientation as Cesium.ConstantProperty).getValue(viewer!.clock.currentTime) as Cesium.Quaternion | undefined;
      if (q) updateFpvCamera(q, pToLat, pToLon, pToAlt);
    } else if (cameraMode === 'follow') updateChaseCamera(pToLat, pToLon, pToAlt, aToHead);
    else if (cameraMode === 'orbit') updateOrbitCamera(pToLat, pToLon, pToAlt);
    viewer?.scene.requestRender();
  }

  // The free-mode 3D camera captured when switching away to 2D: the full matrix (for an
  // exact, drift-free restore when the 2D map wasn't panned) + the ground target & range
  // (to re-target if it was). Re-applied on every return to 3D so the zoom/heading/pitch
  // the user set survives a 2D round-trip.
  type Cam3DSnapshot = {
    position: Cesium.Cartesian3; heading: number; pitch: number; roll: number;
    targetLat: number; targetLon: number; range: number;
  };
  let cam3dSnapshot: Cam3DSnapshot | null = null;

  /**
   * Ground point + spherical offset the 3D camera currently looks at (screen centre).
   * Exposed (instance method) so +page can read it on a 3D→2D switch and re-centre the
   * 2D map on the same spot.
   */
  export function getCamFocus(): { lat: number; lon: number; range: number; heading: number; pitch: number } | null {
    if (!viewer) return null;
    const scene = viewer.scene, canvas = viewer.canvas;
    const screenCentre = new Cesium.Cartesian2(canvas.clientWidth / 2, canvas.clientHeight / 2);
    let ground: Cesium.Cartesian3 | undefined;
    const ray = viewer.camera.getPickRay(screenCentre);
    if (ray) ground = scene.globe.pick(ray, scene);
    if (!ground) ground = viewer.camera.pickEllipsoid(screenCentre) ?? undefined;
    if (!ground) return null;
    const carto = Cesium.Cartographic.fromCartesian(ground);
    return {
      lat: Cesium.Math.toDegrees(carto.latitude),
      lon: Cesium.Math.toDegrees(carto.longitude),
      range: Cesium.Cartesian3.distance(viewer.camera.positionWC, ground),
      heading: viewer.camera.heading,
      pitch: viewer.camera.pitch,
    };
  }

  /** Geographic point directly under the camera (nadir) — used as the radar query centre when the view
   *  hits no ground (looking at the horizon/sky). Always defined while the viewer is alive. */
  export function getCamSubpoint(): { lat: number; lon: number } | null {
    if (!viewer) return null;
    const c = viewer.camera.positionCartographic;
    return { lat: Cesium.Math.toDegrees(c.latitude), lon: Cesium.Math.toDegrees(c.longitude) };
  }

  // Activate/deactivate when the 2D↔3D toggle flips `active`. Inactive → snapshot the
  // free-mode camera's own zoom/angle and pause the render loop (viewer stays in RAM,
  // entities keep updating from the stores). Active → resume, resize, and frame the view:
  //  • locked (follow/orbit) → re-anchor onto the UAV;
  //  • free → target the 2D spot, keeping 3D's own zoom/angle (no zoom reset).
  $effect(() => {
    const on = active; // the only tracked dependency — this effect reacts to the 2D/3D toggle
    const v = viewer;
    if (!v) return;
    // Everything below is imperative viewer state; keep it untracked so cycling the camera
    // mode (incl. exitFpv writing `cameraMode`) doesn't re-run or self-trigger this effect.
    untrack(() => {
      if (on) {
        v.useDefaultRenderLoop = true;
        v.resize();
        // Restore the remembered camera mode for this 3D session.
        if (cameraMode === 'fpv') enterFpv();
        else if (cameraMode !== 'free' && pHas) reanchorLockCamera();
        else setCameraFromMapView();
        v.scene.requestRender();
      } else {
        // Leaving 3D while in FPV: undo FPV's viewer changes (camera inputs, model/track,
        // wheel handler) so nothing carries over and blocks the map — but keep cameraMode
        // 'fpv' so the next activate re-enters FPV.
        if (cameraMode === 'fpv') restoreFromFpv();
        // Remember 3D's own camera (only in free mode; a locked excursion keeps the last
        // free snapshot so returning to free still has it).
        if (cameraMode === 'free') {
          const f = getCamFocus();
          if (f) cam3dSnapshot = {
            position: v.camera.positionWC.clone(),
            heading: v.camera.heading, pitch: v.camera.pitch, roll: v.camera.roll,
            targetLat: f.lat, targetLon: f.lon, range: f.range,
          };
        }
        v.useDefaultRenderLoop = false;
      }
    });
  });

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
      lightingEnabled = s.realLighting3D ?? false;
      replayTimeEnabled = s.logReplayTime ?? false;
      nightModeSetting = s.nightMode2D ?? 'off';
      hudSpeedUnit = s.interface?.speedUnit ?? 'kmh';
      hudAltUnit = s.interface?.altitudeUnit ?? 'm';
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

    // Lighting — real sun shading on the globe (opt-in). The sky Sun/Moon billboards
    // always render; this only toggles the day/night terminator on the terrain.
    // (Night Mode ON forces this off for a flat ground — handled in updateNightDim3D.)
    viewer.scene.globe.enableLighting = lightingEnabled && nightModeSetting !== 'on';

    // Initial camera: frame the SAME spot the 2D map currently shows (center + zoom),
    // positioned immediately — no fly-to sweep. Mirrors every later 2D→3D switch.
    setCameraFromMapView();

    // Seed the sky clock (wall-clock now, or per the time-source priority).
    applyClockTime();
    // Seed night dimming + keep it fresh as the real system time drifts (auto mode).
    ensureUserLocation(); // OS geolocation for Night-Mode auto (resolves async)
    unsubUserGeo = userGeoLocation.subscribe(() => updateNightDim3D()); // recompute once it resolves
    updateNightDim3D();
    nightTimer3D = setInterval(updateNightDim3D, 60_000);
    viewer.camera.moveEnd.addEventListener(updateNightDim3D); // location may cross the terminator
    // Report the camera focus over the globe so the radar online-query centre can follow the 3D view.
    viewer.camera.moveEnd.addEventListener(() => {
      if (!active) return;
      const f = getCamFocus();
      if (f) onCamFocus?.(f.lat, f.lon);
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
      ensureGeoid(telem.lat, telem.lon);

      // Use MSL altitude + geoid offset for correct ellipsoid height.
      // Fall back to relative baro altitude + geoid offset.
      const altMsl = telem.altMsl ?? telem.altitude;
      const alt = Math.max(altMsl + geoidOffset, 0);
      updateUavPosition3D(telem.lat, telem.lon, alt, telem.yaw, telem.navState, armed, telem.roll, telem.pitch);

      // FPV HUD data (live source).
      hud.heading = telem.yaw; hud.pitch = telem.pitch; hud.roll = telem.roll;
      hud.altM = telem.altitude; hud.speedMs = telem.groundSpeed;
      if (!armed) updatePreArmTrail3D(telem.lat, telem.lon);
      // Live: recenter once after the UAV exists (every 2D→3D switch remounts us).
      if (needsInitialRecenter && uavEntity) { needsInitialRecenter = false; recenter3D(); }
      // Follow/orbit camera is driven from the smoothed state inside the motion smoother.

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
        applyMapProvider(currentProviderId);
      }
      const curtain = next.altitudeCurtain3D ?? true;
      if (curtain !== curtainEnabled) {
        curtainEnabled = curtain;
        forceDecoRebuild(); // add/remove the curtain walls
      }
      const lighting = next.realLighting3D ?? false;
      if (lighting !== lightingEnabled) {
        lightingEnabled = lighting;
        updateNightDim3D(); // owns enableLighting + re-evaluates the night dim
      }
      const replayTime = next.logReplayTime ?? false;
      if (replayTime !== replayTimeEnabled) {
        replayTimeEnabled = replayTime;
        applyClockTime();
      }
      const nightMode = next.nightMode2D ?? 'off';
      if (nightMode !== nightModeSetting) {
        nightModeSetting = nightMode;
        updateNightDim3D();
      }
      hudSpeedUnit = next.interface?.speedUnit ?? 'kmh';
      hudAltUnit = next.interface?.altitudeUnit ?? 'm';
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

    // Foreign-vehicle contacts: click to select (sync with list/2D), hover → pointer cursor.
    radar3dPickHandler = new Cesium.ScreenSpaceEventHandler(viewer.canvas);
    radar3dPickHandler.setInputAction((e: Cesium.ScreenSpaceEventHandler.PositionedEvent) => {
      const id = radarPickId(e.position);
      if (id != null) radarSelection.update((cur) => (cur === id ? null : id));
    }, Cesium.ScreenSpaceEventType.LEFT_CLICK);
    radar3dPickHandler.setInputAction((e: Cesium.ScreenSpaceEventHandler.MotionEvent) => {
      if (viewer) viewer.canvas.style.cursor = radarPickId(e.endPosition) != null ? 'pointer' : '';
    }, Cesium.ScreenSpaceEventType.MOUSE_MOVE);

    // Foreign-vehicle contacts: rebuild the 3D radar entities on snapshot / selection change.
    unsubRadar3d = radarVehicles.subscribe((s) => { radar3dSnap = s; updateRadar3D(); });
    unsubRadarSel3d = radarSelection.subscribe((id) => {
      radar3dSelectedId = id;
      for (const rec of radar3dRecs.values()) { rec.selected = rec.id === id; syncRec(rec); }
      viewer?.scene.requestRender();
    });
    // Conflict-alert highlight: re-evaluate whenever the alert set changes (drives the pulse-render mode).
    unsubRadarAlerts3d = radarAlertLevels.subscribe((m) => { radar3dAlertLevels = m; if (viewer) updateRadar3D(); });
    unsubGcs3d = gcsLocation.subscribe(() => updateGcs3d());

    // Connection edge: on a fresh (re)connect, flag the next telemetry frame to
    // decide clearing (only if DISARMED) and force a live-geoid recompute.
    unsubConnection = connection.subscribe((c) => {
      const was = prevConnStatus;
      prevConnStatus = c.status;
      if (c.status === 'connected' && was !== 'connected') {
        pendingConnectArmCheck = true;
        geoidGen++; geoidPromise = null;
        geoidComputed = false;
      }
    });
  });

    onDestroy(() => {
    chaseLerpActive = false;
    orbitLerpActive = false;
    if (smRaf) cancelAnimationFrame(smRaf);
    if (radar3dCreateRaf) cancelAnimationFrame(radar3dCreateRaf);
    if (nightTimer3D) clearInterval(nightTimer3D);
    unsubUserGeo?.();
    if (viewer && !viewer.isDestroyed()) viewer.camera.moveEnd.removeEventListener(updateNightDim3D);
    if (decoTrailingTimer != null) clearTimeout(decoTrailingTimer);
    if (decoRebuildTimer != null) clearTimeout(decoRebuildTimer);
    if (camDragHandler) { camDragHandler.destroy(); camDragHandler = undefined; }
    uninstallFpvWheel();
    unsubTelemetry?.();
    unsubRadar3d?.();
    unsubRadarSel3d?.();
    unsubRadarAlerts3d?.();
    unsubGcs3d?.();
    radar3dPickHandler?.destroy();
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

  // ── Radar (foreign-vehicle) 3D rendering ─────────────────────────────
  const RADAR_CYAN = Cesium.Color.CYAN;
  const RADAR_ALERT_RED = Cesium.Color.fromCssColorString('#ff2a2a');
  const RADAR_ALERT_YELLOW = Cesium.Color.fromCssColorString('#f4c020');
  // Ground circle = exactly the Stage-2 collision miss radius (R_cpa) — the "never enter" blob — so the
  // visual and the alert threshold stay deckungsgleich if R_cpa later becomes user-tunable.
  const CIRCLE_RADIUS_M = ALERT_CONFIG.rCpa;
  /** 0→1→0 once per second, for the alert pulse (evaluated per frame while continuous-rendering). */
  function alertPulse01(): number {
    return 0.5 + 0.5 * Math.sin((Date.now() / 1000) * Math.PI * 2);
  }

  /** Build ECEF positions for a local (east/north, unit) polygon scaled by `sizeM` + heading-rotated. */
  function radarLocalPositions(
    lon: number, lat: number, pts: [number, number][], sizeM: number, headingDeg: number | null,
  ): Cesium.Cartesian3[] {
    const enu = Cesium.Transforms.eastNorthUpToFixedFrame(Cesium.Cartesian3.fromDegrees(lon, lat, 0));
    const h = (headingDeg ?? 0) * Math.PI / 180;
    const ch = Math.cos(h), sh = Math.sin(h);
    return pts.map(([x, y]) => {
      const e = (x * ch + y * sh) * sizeM;
      const n = (-x * sh + y * ch) * sizeM;
      return Cesium.Matrix4.multiplyByPoint(enu, new Cesium.Cartesian3(e, n, 0), new Cesium.Cartesian3());
    });
  }

  function clearRadar3D() {
    radar3dCreateQueue.length = 0;
    if (radar3dCreateRaf) { cancelAnimationFrame(radar3dCreateRaf); radar3dCreateRaf = 0; }
    if (!viewer || !radar3dRecs.size) { if (viewer) viewer.scene.requestRenderMode = true; return; }
    for (const rec of radar3dRecs.values()) for (const e of rec.entities) viewer.entities.remove(e);
    radar3dRecs.clear();
    viewer.scene.requestRenderMode = true; // no contacts → back to on-demand rendering
    viewer.scene.requestRender();
  }

  /** Build queued contacts a few per frame so a dense area doesn't stutter the main thread. */
  function drainRadarCreateQueue() {
    radar3dCreateRaf = 0;
    if (!viewer) { radar3dCreateQueue.length = 0; return; }
    const BATCH = 8;
    let n = 0;
    while (radar3dCreateQueue.length && n < BATCH) {
      const rec = radar3dCreateQueue.shift()!;
      if (radar3dRecs.get(rec.id) === rec) { createRadarEntities(rec); n++; } // still wanted (not removed)
    }
    viewer.scene.requestRender();
    if (radar3dCreateQueue.length) radar3dCreateRaf = requestAnimationFrame(drainRadarCreateQueue);
  }

  // Contacts render like the UAV: a real glb MODEL (oriented to heading, altitude-tinted, minimumPixelSize
  // for a screen-size floor) — no flicker and the heading reads from the 3D shape. The ground projection
  // is a filled CLAMP_TO_GROUND ellipse + a filled heading arrow (drop-line is a polyline).
  const RADAR_MODEL_MIN_PX = 48;
  const DROP_DEPTH_M = 12000; // drop-line length below the contact (terrain depth test clips it at ground)

  /** Create the per-contact entities once (filled in by syncRec). */
  function createRadarEntities(rec: Radar3dRec) {
    if (!viewer) return;
    const model = viewer.entities.add({
      model: {
        uri: radarModelUri(rec.modelClass),
        minimumPixelSize: RADAR_MODEL_MIN_PX,
        maximumScale: 4000,
        scale: 5.2,
        // REPLACE (not MIX): the contact takes the EXACT altitude colour regardless of the glb's own
        // colours — so any model (even white) shows the true height-scale colour without washing it out.
        colorBlendMode: Cesium.ColorBlendMode.REPLACE,
        heightReference: Cesium.HeightReference.NONE,
      },
      // Floating info label under the model: callsign + altitude, slightly transparent.
      label: {
        font: '600 14px "Segoe UI", Tahoma, sans-serif',
        fillColor: Cesium.Color.WHITE.withAlpha(0.9),
        outlineColor: Cesium.Color.BLACK.withAlpha(0.85),
        outlineWidth: 2,
        style: Cesium.LabelStyle.FILL_AND_OUTLINE,
        verticalOrigin: Cesium.VerticalOrigin.TOP,
        pixelOffset: new Cesium.Cartesian2(0, 26),
        showBackground: true,
        backgroundColor: Cesium.Color.BLACK.withAlpha(0.35),
        backgroundPadding: new Cesium.Cartesian2(5, 3),
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
      },
    });
    // Drop-line: a thin dashed colour-coded line over a black dashed backing (contrast). The colour lives
    // in a ConstantProperty we update in place (setValue) — building a NEW material object each poll makes
    // Cesium rebuild the line (a black flash); updating the uniform in place doesn't. The ground-sync
    // guard also means an unchanged contact is never touched at all.
    rec.dropColor = (rec.selected ? RADAR_CYAN : rec.color).withAlpha(0.95);
    rec.dropColorCP = new Cesium.ConstantProperty(rec.dropColor);
    const dropBg = viewer.entities.add({
      polyline: { width: 4, material: new Cesium.PolylineDashMaterialProperty({ color: Cesium.Color.BLACK.withAlpha(0.7), dashLength: 16 }) },
    });
    const drop = viewer.entities.add({
      polyline: { width: 2, material: new Cesium.PolylineDashMaterialProperty({ color: rec.dropColorCP, dashLength: 16 }) },
    });
    const circle = viewer.entities.add({
      ellipse: {
        semiMajorAxis: CIRCLE_RADIUS_M, semiMinorAxis: CIRCLE_RADIUS_M,
        heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
        classificationType: Cesium.ClassificationType.TERRAIN,
        outlineColor: RADAR_CYAN, outlineWidth: 2,
      },
    });
    // Arrow as a clampToGround POLYLINE (not a polygon): polylines update smoothly.
    const arrow = viewer.entities.add({ polyline: { width: 4, clampToGround: true } });
    rec.entities = [model, dropBg, drop, circle, arrow];
    for (const e of rec.entities) radar3dEntityIds.set(e, rec.id); // for click/hover picking
    syncRec(rec);
  }

  /** Contact id under a window position (click/hover), or null. */
  function radarPickId(windowPos: Cesium.Cartesian2): string | null {
    if (!viewer) return null;
    const picked = viewer.scene.pick(windowPos);
    const ent = picked?.id;
    return ent instanceof Cesium.Entity ? (radar3dEntityIds.get(ent) ?? null) : null;
  }

  /** Update the contact model (position/orientation/colour/size) — cheap, no geometry rebuild. */
  function syncRadarModel(rec: Radar3dRec) {
    const e = rec.entities[0];
    if (!e.model) return;
    const pos = Cesium.Cartesian3.fromDegrees(rec.lon, rec.lat, rec.contactEll);
    e.position = new Cesium.ConstantPositionProperty(pos);
    e.orientation = new Cesium.ConstantProperty(uavOrientation(pos, rec.headingDeg ?? 0, 0, 0));
    e.model.color = new Cesium.ConstantProperty(rec.color);
    e.model.minimumPixelSize = new Cesium.ConstantProperty(rec.selected ? RADAR_MODEL_MIN_PX * 1.3 : RADAR_MODEL_MIN_PX);
    e.model.silhouetteColor = new Cesium.ConstantProperty(RADAR_CYAN);
    e.model.silhouetteSize = new Cesium.ConstantProperty(rec.selected ? 2 : 0);
    const ddc = new Cesium.ConstantProperty(new Cesium.DistanceDisplayCondition(0, rec.hideRadiusM));
    e.model.distanceDisplayCondition = ddc;
    if (e.label) {
      e.label.text = new Cesium.ConstantProperty(radarLabelText(rec));
      e.label.distanceDisplayCondition = ddc;
    }
  }

  /** Label text: callsign + altitude normally; the full ADS-B readout (like the 2D hover) when selected. */
  function radarLabelText(rec: Radar3dRec): string {
    if (!rec.selected) {
      return `${rec.callsign}\n${formatConverted(convertAltitude(rec.altM, hudAltUnit), 0)}`;
    }
    const ui = get(settings).interface;
    const alt = formatConverted(convertAltitude(rec.altM, ui.altitudeUnit), 0);
    const spd = rec.groundSpeedMs == null ? '—' : formatConverted(convertSpeed(rec.groundSpeedMs, ui.speedUnit), 0);
    let vs = '';
    if (rec.verticalSpeedMs != null && Math.abs(rec.verticalSpeedMs) >= 0.5) {
      const a = formatConverted(convertVerticalSpeed(Math.abs(rec.verticalSpeedMs), ui.verticalSpeedUnit), 1);
      vs = ` ${rec.verticalSpeedMs > 0 ? '▲' : '▼'}${a}`;
    }
    let dist = '—';
    let brg = '—';
    if (radarReference) {
      const d = haversineDistance(radarReference.lat, radarReference.lon, rec.lat, rec.lon);
      dist = formatConverted(convertDistance(d, ui.distanceUnit), d < 10000 ? 1 : 0);
      brg = `${Math.round(bearing(radarReference.lat, radarReference.lon, rec.lat, rec.lon))}°`;
    }
    return `${rec.callsign}\n${alt}${vs}\n${spd} · ${dist} · ${brg}`;
  }

  /** Update the ground projection (drop-line + filled circle + heading arrow). Solid materials reassigned
   *  per poll do NOT blink (only the dash material did). */
  function syncRadarGround(rec: Radar3dRec) {
    // Skip entirely when nothing relevant changed: the snapshot fires at the (1 Hz) receiver poll rate,
    // and re-touching a contact's ground geometry every snapshot — even unchanged — flashes the line.
    const sig = `${rec.lat.toFixed(6)},${rec.lon.toFixed(6)},${Math.round(rec.contactEll)},${rec.headingDeg ?? 'x'},${rec.showGround},${rec.selected},${rec.color.toCssColorString()},${rec.hideRadiusM},${rec.alertLevel ?? 'n'}`;
    if (sig === rec.groundSig) return;
    rec.groundSig = sig;
    const [, dropBg, drop, circle, arrow] = rec.entities;
    // FormationFlight: just the model + a thin SOLID drop-line in the state colour (no dashed backing,
    // no ground circle, no heading arrow) — visually distinct from ADS-B.
    const isFf = rec.modelClass === 'ff';
    const ddc = new Cesium.ConstantProperty(new Cesium.DistanceDisplayCondition(0, rec.hideRadiusM));
    const top = Cesium.Cartesian3.fromDegrees(rec.lon, rec.lat, rec.contactEll);
    // Drop straight down well below the surface; the terrain depth test clips it at the ground — so we
    // never need a (synchronous, slow) terrain-height sample per contact.
    const bot = Cesium.Cartesian3.fromDegrees(rec.lon, rec.lat, rec.contactEll - DROP_DEPTH_M);
    if (dropBg.polyline) {
      dropBg.polyline.positions = new Cesium.ConstantProperty([top, bot]);
      dropBg.polyline.distanceDisplayCondition = ddc;
    }
    dropBg.show = rec.showGround && !isFf;
    if (drop.polyline) {
      drop.polyline.positions = new Cesium.ConstantProperty([top, bot]);
      if (isFf) {
        // Thin solid line in the state colour (no dashes); reassigning a SOLID material doesn't blink.
        drop.polyline.material = new Cesium.ColorMaterialProperty((rec.selected ? RADAR_CYAN : rec.color).withAlpha(0.95));
        drop.polyline.width = new Cesium.ConstantProperty(1.6);
      } else {
        // ADS-B: dashed, colour updated IN PLACE (no material replace → no blink), only when it changed.
        const desired = (rec.selected ? RADAR_CYAN : rec.color).withAlpha(0.95);
        if (rec.dropColorCP && (!rec.dropColor || !Cesium.Color.equals(rec.dropColor, desired))) {
          rec.dropColor = desired;
          rec.dropColorCP.setValue(desired);
        }
        drop.polyline.width = new Cesium.ConstantProperty(2);
      }
      drop.polyline.distanceDisplayCondition = ddc;
    }
    drop.show = rec.showGround;
    circle.position = new Cesium.ConstantPositionProperty(Cesium.Cartesian3.fromDegrees(rec.lon, rec.lat));
    if (circle.ellipse && !isFf) {
      if (rec.alertLevel) {
        // Alerting: the whole 1 km collision blob pulses — red (warning) / yellow (caution). The blob is
        // exactly R_cpa, so it reads as the "never enter" zone, unmissable from afar.
        const base = rec.alertLevel === 'warning' ? RADAR_ALERT_RED : RADAR_ALERT_YELLOW;
        circle.ellipse.material = new Cesium.ColorMaterialProperty(
          new Cesium.CallbackProperty(() => base.withAlpha(0.3 + 0.45 * alertPulse01()), false),
        );
        circle.ellipse.outline = new Cesium.ConstantProperty(false);
      } else {
        circle.ellipse.material = new Cesium.ColorMaterialProperty(rec.color.brighten(0.45, new Cesium.Color()).withAlpha(0.5));
        circle.ellipse.outline = new Cesium.ConstantProperty(rec.selected);
      }
      circle.ellipse.distanceDisplayCondition = ddc;
    }
    circle.show = !isFf && (rec.showGround || rec.alertLevel != null);
    if (arrow.polyline && !isFf) {
      const a = radarLocalPositions(rec.lon, rec.lat, ARROW_POLY, CIRCLE_RADIUS_M * 0.9, rec.headingDeg);
      a.push(a[0]); // close the outline
      arrow.polyline.positions = new Cesium.ConstantProperty(a);
      arrow.polyline.material = new Cesium.ColorMaterialProperty((rec.selected ? RADAR_CYAN : Cesium.Color.BLACK).withAlpha(0.9));
      arrow.polyline.distanceDisplayCondition = ddc;
    }
    arrow.show = !isFf && rec.showGround && rec.headingDeg != null;
  }

  function syncRec(rec: Radar3dRec) {
    if (rec.entities.length === 0) return; // still queued for creation — will sync on create
    syncRadarModel(rec);
    syncRadarGround(rec);
  }

  /** Diff the 3D radar entities from the latest snapshot + map controls. */
  function updateRadar3D() {
    if (!viewer) return;
    const ms = radarMapSettings;
    if (!radarActive || !ms) { clearRadar3D(); return; }
    // Local contacts are world-anchored, so under showAll the hide radius is large (1000 km) — don't cull
    // a stationary receiver's traffic just because the camera panned far away.
    const hideR = ms.showAll ? 1_000_000 : ms.radiusKm * 1000;
    const all = [...radar3dSnap.adsb, ...radar3dSnap.formationFlight, ...radar3dSnap.radio];
    const seen = new Set<string>();
    for (const v of all) {
      if (v.altM == null) continue;                          // no altitude → can't place in 3D
      if (!contactVisibleOnMap(v, radarRefAltM, ms)) continue;
      seen.add(v.id);
      const delta = radarRefAltM != null ? v.altM - radarRefAltM : null;
      const withinZone = delta != null && delta <= REL_OVERRIDE_M;
      const showGround = withinZone || (import.meta.env.DEV && ms.showAll);
      // FormationFlight uses a state colour (armed/disarmed/lost); ADS-B uses the altitude scale.
      const col = v.system === 'formationFlight'
        ? ffContactColor(v.extra?.ffState)
        : contactColor(v.altM, radarRefAltM);
      const cesColor = Cesium.Color.fromCssColorString(col.fill).withAlpha(col.fillOpacity);
      const contactEll = v.altM + geoidOffset;
      const modelClass = contactModelClass(v.system, v.category, v.headingDeg != null);
      const callsign = v.callsign?.trim() || v.id;
      let rec = radar3dRecs.get(v.id);
      if (!rec) {
        rec = {
          id: v.id, lat: v.lat, lon: v.lon, headingDeg: v.headingDeg, modelClass, callsign, altM: v.altM,
          groundSpeedMs: v.groundSpeedMs, verticalSpeedMs: v.verticalSpeedMs,
          contactEll, color: cesColor, showGround, selected: v.id === radar3dSelectedId,
          alertLevel: radar3dAlertLevels.get(v.id) ?? null,
          hideRadiusM: hideR, entities: [],
        };
        radar3dRecs.set(v.id, rec);
        radar3dCreateQueue.push(rec);          // build incrementally (see drainRadarCreateQueue)
      } else {
        rec.lat = v.lat; rec.lon = v.lon; rec.headingDeg = v.headingDeg; rec.modelClass = modelClass;
        rec.callsign = callsign; rec.altM = v.altM;
        rec.groundSpeedMs = v.groundSpeedMs; rec.verticalSpeedMs = v.verticalSpeedMs;
        rec.contactEll = contactEll; rec.color = cesColor;
        rec.showGround = showGround; rec.selected = v.id === radar3dSelectedId; rec.hideRadiusM = hideR;
        rec.alertLevel = radar3dAlertLevels.get(v.id) ?? null;
        syncRec(rec);
      }
    }
    for (const [id, rec] of radar3dRecs) {
      if (!seen.has(id)) { for (const e of rec.entities) viewer.entities.remove(e); radar3dRecs.delete(id); }
    }
    if (radar3dCreateQueue.length && !radar3dCreateRaf) radar3dCreateRaf = requestAnimationFrame(drainRadarCreateQueue);
    // Any active alert → render continuously so the pulse animates (CallbackProperty materials); otherwise
    // back to on-demand (requestRenderMode) to save the GPU. The alert geometry itself stays static.
    let anyAlert = false;
    for (const rec of radar3dRecs.values()) if (rec.alertLevel) { anyAlert = true; break; }
    viewer.scene.requestRenderMode = !anyAlert;
    viewer.scene.requestRender();
  }

  // Rebuild when any radar control prop changes (snapshot/selection handled by subscriptions).
  $effect(() => {
    radarActive; radarMapSettings; radarRefAltM;
    if (viewer) updateRadar3D();
  });

  // In a radar-only scene (no connected UAV/track) the geoid offset is never computed, so contacts
  // (placed at MSL + geoidOffset) sink under the terrain by the local undulation (~tens of m). Compute it
  // once at the GCS reference, then re-place the contacts at the corrected height.
  $effect(() => {
    if (!viewer || !radarActive) return;
    const ref = radarReference;
    if (!ref) return;
    void computeGeoidOnce(ref.lat, ref.lon).then((ok) => { if (ok) updateRadar3D(); });
  });

  // ── GCS (ground-station) billboard ──────────────────────────────────
  let gcsEntity: Cesium.Entity | undefined;
  let unsubGcs3d: (() => void) | undefined;
  const gcsMode3d = $derived<GcsMode>($settings.gcsMode);

  /** Satellite-dish-on-disc as an SVG data URI for the billboard. */
  const GCS_BILLBOARD_IMG = (() => {
    const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 40 40">' +
      '<circle cx="20" cy="20" r="15" fill="rgba(40,42,44,0.72)" stroke="#37a8db" stroke-width="2.5"/>' +
      '<g transform="translate(8,8)" fill="none" stroke="#37a8db" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M4 10a7.31 7.31 0 0 0 10 10Z"/><path d="m9 15 3-3"/>' +
      '<path d="M17 13a6 6 0 0 0-6-6"/><path d="M21 13A10 10 0 0 0 11 3"/></g></svg>';
    return "data:image/svg+xml;base64," + btoa(svg);
  })();

  function updateGcs3d() {
    if (!viewer) return;
    const loc = get(gcsLocation);
    if (gcsMode3d === "off" || !loc) {
      if (gcsEntity) { viewer.entities.remove(gcsEntity); gcsEntity = undefined; viewer.scene.requestRender(); }
      return;
    }
    const pos = Cesium.Cartesian3.fromDegrees(loc.lon, loc.lat);
    if (!gcsEntity) {
      gcsEntity = viewer.entities.add({
        position: new Cesium.ConstantPositionProperty(pos),
        billboard: {
          image: GCS_BILLBOARD_IMG,
          scale: 0.9,
          verticalOrigin: Cesium.VerticalOrigin.CENTER,
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      });
    } else {
      gcsEntity.position = new Cesium.ConstantPositionProperty(pos);
    }
    viewer.scene.requestRender();
  }

  $effect(() => { gcsMode3d; if (viewer) updateGcs3d(); });

  // ── Sky clock (sun/moon position) ──────────────────────────────────
  // Cesium positions the Sun/Moon from real ephemeris at viewer.clock.currentTime.
  // We drive that clock from one of three sources (priority): the dev time slider,
  // the replay log's timestamp (if enabled), else real wall-clock now.

  /** Build a JulianDate for a local-solar time-of-day at the currently viewed longitude. */
  function julianFromLocalTimeOfDay(minutes: number): Cesium.JulianDate {
    // Longitude of what the camera looks at → local solar noon ≈ 12:00 on the slider.
    let lonDeg = 0;
    if (viewer) {
      try { lonDeg = Cesium.Math.toDegrees(viewer.camera.positionCartographic.longitude); } catch { lonDeg = 0; }
    }
    const utcHours = minutes / 60 - lonDeg / 15; // UTC = localSolar − lon/15
    const now = new Date();
    const baseUtcMidnight = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
    return Cesium.JulianDate.fromDate(new Date(baseUtcMidnight + utcHours * 3_600_000));
  }

  /** Apply the active time source to the Cesium clock and re-render. */
  function applyClockTime() {
    if (!viewer) return;
    let jd: Cesium.JulianDate;
    if (devTimeActive) {
      jd = julianFromLocalTimeOfDay(devTimeMin);
    } else if (replayTimeEnabled && curReplayActive && replayStartEpochMs != null && playbackPoint?.timestamp_ms != null) {
      // timestamp_ms is flight-relative → add the absolute flight-start epoch.
      jd = Cesium.JulianDate.fromDate(new Date(replayStartEpochMs + playbackPoint.timestamp_ms));
    } else {
      jd = Cesium.JulianDate.now();
    }
    viewer.clock.currentTime = jd;
    viewer.scene.requestRender();
    // Clock moved → real-lighting day/night may have flipped; re-evaluate the dim.
    updateNightDim3D();
  }

  // ── Night dimming (imagery only) ───────────────────────────────────

  /** Camera target longitude/latitude in degrees (for the local sun calc). */
  function cameraLonLat(): { lat: number; lon: number } {
    if (!viewer) return { lat: 0, lon: 0 };
    try {
      const c = viewer.camera.positionCartographic;
      return { lat: Cesium.Math.toDegrees(c.latitude), lon: Cesium.Math.toDegrees(c.longitude) };
    } catch {
      return { lat: 0, lon: 0 };
    }
  }

  /** Set brightness on all imagery layers (1 = normal, 0.3 = night), only if it changed. */
  function applyImageryBrightness(factor: number) {
    if (!viewer || Math.abs(factor - appliedImageryBrightness) < 0.005) return;
    appliedImageryBrightness = factor;
    const layers = viewer.imageryLayers;
    for (let i = 0; i < layers.length; i++) layers.get(i).brightness = factor;
    viewer.scene.requestRender();
  }

  /**
   * Night dimming as the *darker of two continuous brightness curves*, never stacked:
   *  - cesiumFactor: the real-lighting day/night shading at the VIEWED location & clock time
   *    (smooth 1.0→0.3 across the terminator; 1.0 if real lighting is off).
   *  - nightFactor:  the Night-Mode auto curve at the USER's physical location & system time.
   * We push the imagery to min(cesium, night) WITHOUT double-darkening: since Cesium's lighting
   * already multiplies the globe by cesiumFactor, the extra imagery dim is the ratio
   * min(c,n)/c — i.e. 1.0 when Cesium is already as dark (terminator preserved), <1 only where
   * Night Mode wants it darker than Cesium. Special cases:
   *  - Night Mode ON  → flat 0.3: force lighting off (uniform ground) + imagery 0.3; sky/sun stay real.
   *  - Night Mode OFF → imagery 1.0; lighting follows the real-lighting setting.
   */
  function updateNightDim3D() {
    if (!viewer) return;

    // Night Mode ON overrides the ground lighting so the whole globe is a flat 0.3 (sky/sun still real).
    const lightingActive = lightingEnabled && nightModeSetting !== 'on';
    if (viewer.scene.globe.enableLighting !== lightingActive) {
      viewer.scene.globe.enableLighting = lightingActive;
      viewer.scene.requestRender(); // requestRenderMode: redraw now, not on the next camera move
    }

    let factor = 1.0;
    if (nightModeSetting === 'on') {
      factor = NIGHT_BRIGHTNESS_3D;
    } else if (nightModeSetting === 'auto') {
      const view = cameraLonLat();
      const clockDate = Cesium.JulianDate.toDate(viewer.clock.currentTime);
      const cesiumFactor = lightingActive
        ? cesiumLikeBrightness(sunAltitudeDeg(clockDate, view.lat, view.lon))
        : 1.0;
      const u = resolveUserLocation(); // OS geo → UAV GPS → home → persisted map centre (NOT camera)
      const nightFactor = cesiumLikeBrightness(sunAltitudeDeg(new Date(), u.lat, u.lon));
      factor = Math.min(cesiumFactor, nightFactor) / cesiumFactor;
    }
    applyImageryBrightness(factor);
  }

  // ── UAV Entity ─────────────────────────────────────────────────────

  // Low-poly UAV models (static/models/): +X = nose, Y-up. Quad = aviation nav-light rotor rings
  // (left/port = red, right/starboard = green → an inverted attitude is readable) + cyan nose arrow.
  // Arrow = generic flat marker for non-multirotor / unknown craft (until plane/heli models exist).
  // Tinted lightly by flight-mode colour (MIX) so the mode still reads; minimumPixelSize keeps it
  // visible far out.
  // Model selection (override > platform) lives in the shared uavModels helper (also used by 2D map).
  function currentModelUri(): string {
    return modelUriForPlatform(platformType, modelOverride);
  }
  // Live-swap the marker model when the override (or platform type) changes mid-session.
  $effect(() => {
    const uri = currentModelUri(); // tracks modelOverride + platformType
    for (const e of [uavEntity, playbackMarkerEntity]) {
      if (e?.model) e.model.uri = new Cesium.ConstantProperty(uri);
    }
    viewer?.scene.requestRender();
  });
  // Heading offset stays 0 — the model's own frame is yaw-corrected in the .glb generators
  // (ROOT_YAW_Y) so the explicit body-axis construction below needs no runtime fudge.
  const MODEL_HEADING_OFFSET_DEG = 0;
  // Attitude → orientation, built from EXPLICIT aircraft body axes in the local ENU frame (not by
  // permuting Cesium-HPR's pitch/roll slots — that only worked near level and broke at high bank /
  // inverted). Sequence: yaw about Up, pitch about the right axis (nose up/down), roll about the
  // nose axis (bank) — correct at ALL attitudes. Signs match the AHI widget: INAV pitch is negative
  // = nose up (→ −1), roll is positive = right-wing-down (→ +1). The model's LOCAL frame after the
  // glTF Y-up→Z-up load is nose=+X, up=+Z, left=+Y, so we map (nose, left, up) → world.
  const MODEL_PITCH_SIGN = -1;
  const MODEL_ROLL_SIGN = 1;
  function uavOrientation(position: Cesium.Cartesian3, headingDeg: number, pitchDeg = 0, rollDeg = 0) {
    const h = Cesium.Math.toRadians(headingDeg + MODEL_HEADING_OFFSET_DEG);
    const th = Cesium.Math.toRadians(MODEL_PITCH_SIGN * pitchDeg);
    const ph = Cesium.Math.toRadians(MODEL_ROLL_SIGN * rollDeg);
    const ch = Math.cos(h), sh = Math.sin(h), ct = Math.cos(th), st = Math.sin(th), cp = Math.cos(ph), sp = Math.sin(ph);
    const enu = Cesium.Transforms.eastNorthUpToFixedFrame(position);
    const c = new Cesium.Cartesian4();
    const E = Cesium.Matrix4.getColumn(enu, 0, c); const ex = E.x, ey = E.y, ez = E.z;
    const N = Cesium.Matrix4.getColumn(enu, 1, c); const nx = N.x, ny = N.y, nz = N.z;
    const U = Cesium.Matrix4.getColumn(enu, 2, c); const ux = U.x, uy = U.y, uz = U.z;
    // a·E + b·N + d·U → ECEF
    const comb = (a: number, b: number, d: number) => new Cesium.Cartesian3(a * ex + b * nx + d * ux, a * ey + b * ny + d * uy, a * ez + b * nz + d * uz);
    // body axes (ENU coefficients): yaw → pitch(about right) → roll(about nose)
    const nose = comb(ct * sh, ct * ch, st);
    const right = comb(cp * ch + sp * st * sh, -cp * sh + sp * st * ch, -sp * ct);
    const up = comb(sp * ch - cp * st * sh, -sp * sh - cp * st * ch, cp * ct);
    const left = Cesium.Cartesian3.negate(right, new Cesium.Cartesian3());
    const m = new Cesium.Matrix3(
      nose.x, left.x, up.x,
      nose.y, left.y, up.y,
      nose.z, left.z, up.z,
    );
    return Cesium.Quaternion.fromRotationMatrix(m, new Cesium.Quaternion());
  }
  function uavModelGraphics(tint: Cesium.Color, uri: string) {
    return {
      uri,
      minimumPixelSize: 73,
      maximumScale: 4000,
      scale: 5.2,
      color: tint,
      colorBlendMode: Cesium.ColorBlendMode.MIX,
      colorBlendAmount: 0.2,
      heightReference: Cesium.HeightReference.NONE,
    };
  }

  // ── UAV motion smoothing (adaptive interpolation, separate for position + attitude) ──
  // The replay player ticks at a fixed rate, but the underlying GPS/attitude samples change at
  // their own (often lower) rate. We re-base an interpolation ONLY when a value actually CHANGES,
  // and the transition time is the MEDIAN of recent real-change intervals — a median (not an
  // average) means a single aliased/missed update can't corrupt the timing and cause a stutter.
  // Each re-base starts from the CURRENTLY DISPLAYED state (not the last target), so a slightly-off
  // interval only changes velocity — never a jump or a mid-glide pause. Position and attitude are
  // tracked independently (e.g. 5 Hz GPS + 10 Hz attitude). A far jump (scrub / source switch /
  // first sample) snaps. The smoothed state also drives the follow/orbit camera.
  let smEntity: Cesium.Entity | undefined;
  let smRaf = 0;
  // position channel: interpolate from→to over pInt (started at pT0); lat/lon/alt held as scalars
  let pFromLat = 0, pFromLon = 0, pFromAlt = 0, pToLat = 0, pToLon = 0, pToAlt = 0;
  let pT0 = 0, pInt = 0.2, pHas = false;
  // attitude channel
  let aFrom: Cesium.Quaternion | null = null, aTo: Cesium.Quaternion | null = null;
  let aFromHead = 0, aToHead = 0, aT0 = 0, aInt = 0.1;
  const pBuf: number[] = [], aBuf: number[] = [];
  const SM_MIN = 0.05, SM_MAX = 1.5, SM_SNAP_M = 25, SM_POS_EPS = 0.05, SM_LEAD = 1.12, SM_BUF = 8;
  const lerpN = (a: number, b: number, t: number) => a + (b - a) * t;
  const median = (a: number[]) => { const s = [...a].sort((x, y) => x - y); const m = s.length >> 1; return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2; };
  const pushInterval = (buf: number[], dt: number) => {
    buf.push(dt); if (buf.length > SM_BUF) buf.shift();
    return Math.min(SM_MAX, Math.max(SM_MIN, median(buf) * SM_LEAD));
  };
  const cart = (lat: number, lon: number, alt: number) => Cesium.Cartesian3.fromDegrees(lon, lat, alt);

  function resetUavSmoothing() {
    if (smRaf) cancelAnimationFrame(smRaf);
    smRaf = 0; smEntity = undefined; pHas = false; aFrom = aTo = null;
    pInt = 0.2; aInt = 0.1; pBuf.length = 0; aBuf.length = 0;
  }

  function pushUavSample(entity: Cesium.Entity, lat: number, lon: number, alt: number, heading: number, quat: Cesium.Quaternion) {
    const now = performance.now();
    const farJump = pHas && Cesium.Cartesian3.distance(cart(pToLat, pToLon, pToAlt), cart(lat, lon, alt)) > SM_SNAP_M;
    if (smEntity !== entity || !pHas || !aTo || farJump) {
      // Snap: first sample, source/entity switch, or a teleport (scrub).
      if (smRaf) { cancelAnimationFrame(smRaf); smRaf = 0; }
      smEntity = entity;
      pFromLat = pToLat = lat; pFromLon = pToLon = lon; pFromAlt = pToAlt = alt; pT0 = now; pHas = true; pBuf.length = 0;
      aFrom = quat; aTo = quat; aFromHead = aToHead = heading; aT0 = now; aBuf.length = 0;
      applySmoothed(cart(lat, lon, alt), quat, lat, lon, alt, heading);
      return;
    }
    // Position: re-base only on a real move, continuing from the current displayed point.
    if (Cesium.Cartesian3.distance(cart(pToLat, pToLon, pToAlt), cart(lat, lon, alt)) > SM_POS_EPS) {
      const pf = Math.min(1, ((now - pT0) / 1000) / pInt);
      pFromLat = lerpN(pFromLat, pToLat, pf); pFromLon = lerpN(pFromLon, pToLon, pf); pFromAlt = lerpN(pFromAlt, pToAlt, pf);
      pToLat = lat; pToLon = lon; pToAlt = alt;
      pInt = pushInterval(pBuf, (now - pT0) / 1000); pT0 = now;
    }
    // Attitude: re-base only on a real change, from the current displayed orientation.
    if (!Cesium.Quaternion.equalsEpsilon(aTo!, quat, 1e-5)) {
      const af = Math.min(1, ((now - aT0) / 1000) / aInt);
      aFrom = Cesium.Quaternion.slerp(aFrom!, aTo!, af, new Cesium.Quaternion());
      aFromHead = lerpAngle(aFromHead, aToHead, af);
      aTo = quat; aToHead = heading;
      aInt = pushInterval(aBuf, (now - aT0) / 1000); aT0 = now;
    }
    if (!smRaf) smRaf = requestAnimationFrame(smTick);
  }

  function smTick() {
    smRaf = 0;
    if (!viewer || !smEntity || !pHas || !aFrom || !aTo) return;
    const now = performance.now();
    const pf = Math.min(1, ((now - pT0) / 1000) / pInt);
    const af = Math.min(1, ((now - aT0) / 1000) / aInt);
    const lat = lerpN(pFromLat, pToLat, pf), lon = lerpN(pFromLon, pToLon, pf), alt = lerpN(pFromAlt, pToAlt, pf);
    const quat = Cesium.Quaternion.slerp(aFrom!, aTo!, af, new Cesium.Quaternion());
    const heading = lerpAngle(aFromHead, aToHead, af);
    applySmoothed(cart(lat, lon, alt), quat, lat, lon, alt, heading);
    viewer.scene.requestRender();
    if (pf < 1 || af < 1) smRaf = requestAnimationFrame(smTick);
  }

  function applySmoothed(pos: Cesium.Cartesian3, quat: Cesium.Quaternion, lat: number, lon: number, alt: number, heading: number) {
    if (!smEntity) return;
    (smEntity.position as Cesium.ConstantPositionProperty).setValue(pos);
    (smEntity.orientation as Cesium.ConstantProperty).setValue(quat);
    // Drive the camera from the smoothed state (the camera fns are cheap target-setters).
    trackFollowPosition(lat, lon, alt, heading);
    if (cameraMode === 'fpv') updateFpvCamera(quat, lat, lon, alt);
    else if (cameraMode === 'follow') updateChaseCamera(lat, lon, alt, heading);
    else if (cameraMode === 'orbit') updateOrbitCamera(lat, lon, alt);
  }

  function updateUavPosition3D(lat: number, lon: number, alt: number, heading: number, navState = 0, armed = false, roll = 0, pitch = 0) {
    if (!viewer) return;

    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);
    const color = getNavStateColor(navState); // marker = nav state (the track shows flight mode)
    const cesiumColor = Cesium.Color.fromCssColorString(color);

    // Full attitude: heading (INAV 0=N CW = Cesium) + pitch + roll (signs via the constants above).
    const orientation = uavOrientation(position, heading, pitch, roll);

    if (!uavEntity) {
      uavEntity = viewer.entities.add({
        position,
        orientation: orientation as any,
        model: uavModelGraphics(cesiumColor, currentModelUri()),
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
      });
    } else if (uavEntity.model) {
      uavEntity.model.color = new Cesium.ConstantProperty(cesiumColor);
    }
    // Position + attitude go through the adaptive smoother (also drives the camera).
    pushUavSample(uavEntity, lat, lon, alt, heading, orientation);

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
      const cesiumColor = Cesium.Color.fromCssColorString(color).withAlpha(fpvAlpha(0.7));
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
    resetUavSmoothing();
    // Markers (live UAV, replay marker, home)
    if (uavEntity) { viewer.entities.remove(uavEntity); uavEntity = undefined; }
    if (playbackMarkerEntity) { viewer.entities.remove(playbackMarkerEntity); playbackMarkerEntity = undefined; }
    if (homeEntity) { viewer.entities.remove(homeEntity); homeEntity = undefined; }
    // Altitude / geoid / arm-session state
    geoidOffset = 0;
    startMslGps = 0;
    wasArmed = false;
    geoidGen++; geoidPromise = null;
    geoidComputed = false;
    // Camera follow state (so it re-anchors on the new source)
    chaseInited = false;
    orbitInited = false;
    // Mission stays — re-place it at the reset geoid.
    scheduleMissionRender();
    viewer.scene.requestRender();
  }

  /**
   * Derive the geoid undulation N = cesiumGround_ellipsoid − copernicusGround_MSL at
   * `lat`/`lon`, ONCE per scene. Heights placed as `MSL + geoidOffset` (live UAV, track,
   * mission waypoints, …) would otherwise sink by the full local undulation (~tens of m).
   * Single-flight + awaitable: concurrent callers (a loading track + its linked mission)
   * share the one promise and all see the same offset. Resolves to whether it succeeded;
   * on failure (no terrain / no Copernicus ground) callers draw at offset 0 (best effort).
   * `fallbackGroundMsl` (the replay's first-fix GPS MSL) substitutes for a missing
   * Copernicus ground so the replay still gets an offset.
   */
  function computeGeoidOnce(lat: number, lon: number, fallbackGroundMsl?: number): Promise<boolean> {
    if (geoidComputed) return Promise.resolve(true);
    if (geoidPromise) return geoidPromise; // join the in-flight computation
    if (!viewer) return Promise.resolve(false);
    const v = viewer, gen = geoidGen;
    geoidPromise = (async () => {
      try {
        const terrainProvider = await waitForTerrain(v);
        if (!terrainProvider) { console.warn('[Map3D] No terrain provider available, geoidOffset=0'); return false; }
        const refPos = Cesium.Cartographic.fromDegrees(lon, lat);
        const sampled = await Cesium.sampleTerrainMostDetailed(terrainProvider, [refPos]);
        if (!sampled[0] || sampled[0].height == null) return false;
        const copernicusGround = await invoke<number | null>('terrain_elevation', { lat, lon });
        const groundMsl = copernicusGround ?? fallbackGroundMsl;
        if (groundMsl == null) { console.warn('[Map3D] No ground MSL for geoid, geoidOffset=0'); return false; }
        if (gen !== geoidGen) return false; // a source switch happened mid-sample → discard
        geoidOffset = sampled[0].height - groundMsl;
        geoidComputed = true;
        console.log(`[Map3D] Geoid N: ${geoidOffset.toFixed(1)}m (cesium=${sampled[0].height.toFixed(1)}, groundMSL=${groundMsl.toFixed(1)})`);
        return true;
      } catch (e) {
        console.warn('[Map3D] Geoid sample failed', e);
        return false;
      } finally {
        geoidPromise = null;
      }
    })();
    return geoidPromise;
  }

  /** Compute the geoid offset (if not yet done) and re-place the mission at the new height. */
  async function ensureGeoid(lat: number, lon: number) {
    const ok = await computeGeoidOnce(lat, lon);
    if (ok) { scheduleMissionRender(); viewer?.scene.requestRender(); }
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
      resetUavSmoothing();
      if (uavEntity) { viewer.entities.remove(uavEntity); uavEntity = undefined; }
      if (homeEntity) { viewer.entities.remove(homeEntity); homeEntity = undefined; }
      geoidGen++; geoidPromise = null;
      geoidComputed = false;
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
    // Geoid offset for the track ellipsoid heights. Uses the SAME single-flight path as
    // the mission, so a linked mission loading moments later (see +page) shares this exact
    // computation and draws at the same height instead of racing it. Copernicus MSL is
    // preferred; the first-fix GPS MSL is the fallback ground.
    if (firstPt) await computeGeoidOnce(firstPt.lat!, firstPt.lon!, firstPt.alt_m ?? undefined);

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
            color: color.withAlpha(fpvAlpha(0.95)),
            outlineColor: Cesium.Color.BLACK.withAlpha(fpvAlpha(0.9)),
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

    // Re-place the replay model at the corrected height. The playbackPoint effect places
    // it as soon as the track loads — but that can run BEFORE this function's (async) geoid
    // computation finishes, leaving the model a few metres off the ground until the first
    // position update. Now that the geoid offset is ready, snap it onto the first point.
    if (playbackPoint) {
      resetUavSmoothing();
      updatePlaybackMarker3D(playbackPoint);
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

    // The mission sits at `altMsl + geoidOffset`, so it must WAIT for the geoid offset.
    // computeGeoidOnce is single-flight: if a track/live fix is already computing it, we
    // join that promise (same offset); otherwise we derive it from the first waypoint
    // (mission preview with no live/replay). On failure (no terrain) we draw at offset 0.
    if (!geoidComputed) {
      const g = wps.find((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
      if (g) {
        await computeGeoidOnce(toDeg(g.lat), toDeg(g.lon));
        if (token !== missionRenderToken || !viewer) return; // superseded while awaiting
      }
    }

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
    // Move the sky clock along the flight time when "Log Replay Time" is on
    // (dev slider, if active, wins).
    if (replayTimeEnabled && !devTimeActive) applyClockTime();
  });

  function updatePlaybackMarker3D(point: TelemetryRecord | null) {
    if (!viewer) return;

    if (!point || point.lat == null || point.lon == null || !isValidGpsCoordinate(point.lat, point.lon)) {
      if (playbackMarkerEntity) {
        resetUavSmoothing();
        viewer.entities.remove(playbackMarkerEntity);
        playbackMarkerEntity = undefined;
      }
      return;
    }

    const lat = point.lat;
    const lon = point.lon;
    const alt = startMslGps + geoidOffset + (point.nav_alt_m ?? point.baro_alt_m ?? 0);
    const heading = point.heading ?? 0;
    const color = getNavStateColor(point.nav_state ?? 0); // marker = nav state
    const cesiumColor = Cesium.Color.fromCssColorString(color);
    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);
    // Attitude from the SAME unified adapter the AHI widget uses (consistent across
    // INAV / ArduPilot / live / replay) rather than the raw record.
    const td = toTelemetryData(point, fcVariant);
    const orientation = uavOrientation(position, heading, td.pitch, td.roll);

    // FPV HUD data (replay source).
    hud.heading = heading; hud.pitch = td.pitch; hud.roll = td.roll;
    hud.altM = point.nav_alt_m ?? point.baro_alt_m ?? 0;
    hud.speedMs = point.speed_ms ?? 0;

    if (!playbackMarkerEntity) {
      playbackMarkerEntity = viewer.entities.add({
        position,
        orientation: orientation as any,
        model: uavModelGraphics(cesiumColor, currentModelUri()),
      });
    } else if (playbackMarkerEntity.model) {
      playbackMarkerEntity.model.color = new Cesium.ConstantProperty(cesiumColor);
    }
    // Position + attitude (and the follow/orbit camera) go through the adaptive smoother.
    pushUavSample(playbackMarkerEntity, lat, lon, alt, heading, orientation);

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

  // Previous frame's look target — lockRange (mouse-wheel zoom) is measured against THIS, not the
  // newly-moved target, so the UAV's own radial motion isn't baked into the zoom (zoom-drift bug).
  let chaseLastTarget: Cesium.Cartesian3 | undefined;
  let orbitLastCenter: Cesium.Cartesian3 | undefined;

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

    // Sync lockRange from mouse-wheel zoom only — measure the camera distance against the PREVIOUS
    // frame's target (where the camera was framed), not the new moved one, so the UAV's own radial
    // motion can't drift the zoom in/out.
    if (chaseLastTarget) {
      const userRange = Cesium.Cartesian3.distance(viewer.camera.positionWC, chaseLastTarget);
      if (userRange > 0.01) lockRange = Math.max(LOCK_ZOOM_MIN, Math.min(LOCK_ZOOM_MAX, userRange));
    }

    // HPR.heading = the camera's LOOK direction. Setting it to UAV heading means
    // the camera looks the same way as the UAV and is therefore positioned BEHIND it.
    const behindHeading = chaseCurrent.heading * (Math.PI / 180);

    viewer.camera.lookAt(target, new Cesium.HeadingPitchRange(behindHeading, followPitch, lockRange));
    chaseLastTarget = Cesium.Cartesian3.clone(target, chaseLastTarget);
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

    // Mouse-wheel zoom only — measure against the previous center, not the new (moved) one.
    if (orbitLastCenter) {
      const userRange = Cesium.Cartesian3.distance(viewer.camera.positionWC, orbitLastCenter);
      if (userRange > 0.01) lockRange = Math.max(LOCK_ZOOM_MIN, Math.min(LOCK_ZOOM_MAX, userRange));
    }

    viewer.camera.lookAt(newCenter, new Cesium.HeadingPitchRange(h, p, lockRange));
    orbitLastCenter = Cesium.Cartesian3.clone(newCenter, orbitLastCenter);
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

  // ── FPV (first-person view) ────────────────────────────────────────

  /** Track-line alpha for the current mode (FPV dims the flight track so it doesn't fill the view). */
  function fpvAlpha(base: number): number {
    return cameraMode === 'fpv' ? FPV_TRACK_ALPHA : base;
  }

  /** Re-alpha the already-built track entities when entering/leaving FPV. */
  function setTrackOpacity(fpv: boolean) {
    if (!viewer) return;
    const time = viewer.clock.currentTime;
    const setA = (prop: Cesium.Property | undefined, a: number) => {
      if (!prop) return;
      const col = (prop as Cesium.ConstantProperty).getValue(time) as Cesium.Color | undefined;
      if (col) (prop as Cesium.ConstantProperty).setValue(col.withAlpha(a));
    };
    for (const e of playbackTrackParts) {
      const m = e.polyline?.material as Cesium.PolylineOutlineMaterialProperty | undefined;
      if (m) { setA(m.color, fpv ? FPV_TRACK_ALPHA : 0.95); setA(m.outlineColor, fpv ? FPV_TRACK_ALPHA : 0.9); }
    }
    const setTrail = (ent?: Cesium.Entity) => {
      const m = ent?.polyline?.material as Cesium.ColorMaterialProperty | undefined;
      if (m) setA(m.color, fpv ? FPV_TRACK_ALPHA : 0.7);
    };
    for (const s of trailSegments3D) setTrail(s.entity);
    setTrail(activeTrailEntity);
    viewer.scene.requestRender();
  }

  /** Hide/show the UAV model(s) — in FPV the camera sits where the model would be. */
  function setModelHiddenForFpv(hide: boolean) {
    if (uavEntity) uavEntity.show = !hide;
    if (playbackMarkerEntity) playbackMarkerEntity.show = !hide;
  }

  /** Place the camera at the model (raised slightly) and orient it exactly like the model. */
  function updateFpvCamera(quat: Cesium.Quaternion, lat: number, lon: number, alt: number) {
    if (!viewer) return;
    if (smEntity) smEntity.show = false; // model is replaced by the camera in FPV
    const rot = Cesium.Matrix3.fromQuaternion(quat, fpvScratchM3);
    const dir = Cesium.Matrix3.getColumn(rot, 0, fpvScratchDir); // nose / forward axis
    const up = Cesium.Matrix3.getColumn(rot, 2, fpvScratchUp);   // body up (so bank tilts the view)
    const dest = Cesium.Cartesian3.fromDegrees(lon, lat, alt + FPV_EYE_HEIGHT_M);
    viewer.camera.setView({ destination: dest, orientation: { direction: dir, up } });
  }

  /** Apply the FPV "lens" — horizontal field of view, 30°…120°. */
  function applyFpvFov() {
    if (!viewer) return;
    const frustum = viewer.camera.frustum as Cesium.PerspectiveFrustum;
    if (frustum && frustum.fov !== undefined) {
      frustum.fov = Cesium.Math.toRadians(fpvFov);
      viewer.scene.requestRender();
    }
  }
  /** Restore Cesium's default 60° frustum on leaving FPV. */
  function restoreFov() {
    if (!viewer) return;
    const frustum = viewer.camera.frustum as Cesium.PerspectiveFrustum;
    if (frustum && frustum.fov !== undefined) {
      frustum.fov = Cesium.Math.toRadians(60);
      viewer.scene.requestRender();
    }
  }

  function installFpvWheel() {
    if (fpvWheelHandler || !viewer) return;
    fpvWheelHandler = new Cesium.ScreenSpaceEventHandler(viewer.scene.canvas);
    // Wheel up = zoom in = narrower lens; wheel down = wider.
    fpvWheelHandler.setInputAction((delta: number) => zoom3D(delta > 0 ? 1 : -1), Cesium.ScreenSpaceEventType.WHEEL);
  }
  function uninstallFpvWheel() {
    if (fpvWheelHandler) { fpvWheelHandler.destroy(); fpvWheelHandler = undefined; }
  }

  function enterFpv() {
    if (!viewer) return;
    cameraMode = 'fpv';
    setFollowCameraControls(false);
    chaseLerpActive = false; orbitLerpActive = false; chaseInited = false; orbitInited = false;
    viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
    viewer.scene.screenSpaceCameraController.enableInputs = false; // FPV fully drives the camera
    applyFpvFov();
    setModelHiddenForFpv(true);
    setTrackOpacity(true);
    installFpvWheel();
    // Initial snap from the current smoothed attitude (works even when paused at a point).
    if (smEntity && pHas) {
      const q = (smEntity.orientation as Cesium.ConstantProperty).getValue(viewer.clock.currentTime) as Cesium.Quaternion | undefined;
      if (q) updateFpvCamera(q, pToLat, pToLon, pToAlt);
    }
    viewer.scene.requestRender();
  }

  /** Undo FPV's viewer changes (inputs, lens, model/track, wheel) WITHOUT touching the mode —
   *  used both to leave FPV (exitFpv) and to suspend it while the 3D view is hidden. */
  function restoreFromFpv() {
    if (!viewer) return;
    viewer.scene.screenSpaceCameraController.enableInputs = true;
    restoreFov();
    setModelHiddenForFpv(false);
    setTrackOpacity(false);
    uninstallFpvWheel();
    viewer.camera.lookAtTransform(Cesium.Matrix4.IDENTITY);
    viewer.scene.requestRender();
  }

  function exitFpv() {
    restoreFromFpv();
    cameraMode = 'free';
  }

  function cycleCameraMode() {
    if (cameraMode === 'orbit') { enterFpv(); return; }
    if (cameraMode === 'fpv') { exitFpv(); return; }
    if (cameraMode === 'free') {
      cameraMode = 'follow';
      lockRange = 200;
      followPitch = -20 * (Math.PI / 180);
      chaseInited = false;
      chaseLastTarget = undefined;
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
      orbitLastCenter = undefined;
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
    }
    // orbit → fpv and fpv → free are handled by the early returns above.
  }

  // ── Zoom ───────────────────────────────────────────────────────────

  // Zoom limits for follow / orbit modes
  const LOCK_ZOOM_MIN = 20;
  const LOCK_ZOOM_MAX = 500;

  function zoom3D(dir: 1 | -1) {
    if (!viewer) return;
    if (cameraMode === 'fpv') {
      // FPV "zoom" = the lens FOV (narrower = zoom in), 30°…120°.
      fpvFov = Math.max(FPV_FOV_MIN, Math.min(FPV_FOV_MAX, fpvFov + (dir > 0 ? -10 : 10)));
      applyFpvFov();
      return;
    }
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
    cameraMode === 'orbit'  ? 'Camera: Orbit UAV'   :
                              'Camera: FPV (first-person)'
  );
</script>

<div class="map3d-wrapper">
  <div class="cesium-container" bind:this={cesiumContainer}></div>

  {#if cameraMode === 'fpv'}
    {@const sp = convertSpeed(hud.speedMs, hudSpeedUnit)}
    {@const al = convertAltitude(hud.altM, hudAltUnit)}
    <FpvHud
      heading={hud.heading}
      pitch={hud.pitch}
      roll={hud.roll}
      speed={sp.value}
      speedUnit={sp.unit}
      altitude={al.value}
      altitudeUnit={al.unit}
      fov={fpvFov}
    />
  {/if}

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
      class:mode-fpv={cameraMode === 'fpv'}
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
      {:else if cameraMode === 'fpv'}
        <svg class="cam-icon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <path d="M2 12 C5 6 19 6 22 12 C19 18 5 18 2 12 Z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/>
          <circle cx="12" cy="12" r="3.2" fill="currentColor"/>
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

  {#if import.meta.env.DEV}
    <!-- DEV-only sun/time previewer: drag to scrub the time-of-day and watch the lighting. -->
    <div class="dev-time-tool">
      <label class="dev-time-row">
        <input
          type="checkbox"
          bind:checked={devTimeActive}
          onchange={() => applyClockTime()}
        />
        <span class="dev-time-label">Time override</span>
        <span class="dev-time-clock">
          {Math.floor(devTimeMin / 60).toString().padStart(2, '0')}:{(devTimeMin % 60).toString().padStart(2, '0')}
        </span>
      </label>
      <input
        class="dev-time-slider"
        type="range"
        min="0"
        max="1439"
        step="1"
        bind:value={devTimeMin}
        disabled={!devTimeActive}
        oninput={() => applyClockTime()}
      />
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

  :global(.cesium-viewer) {
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  /* ── DEV-only time-of-day previewer (top-right) ── */
  .dev-time-tool {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 10000;
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 200px;
    padding: 8px 10px;
    background: rgba(46, 46, 46, 0.9);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    backdrop-filter: blur(8px);
    pointer-events: all;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .dev-time-row {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
  }
  .dev-time-label {
    color: #c7dfe8;
    font-size: 12px;
  }
  .dev-time-clock {
    margin-left: auto;
    color: #37a8db;
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    font-size: 13px;
  }
  .dev-time-slider {
    width: 100%;
    accent-color: #37a8db;
    cursor: pointer;
  }
  .dev-time-slider:disabled {
    cursor: default;
    opacity: 0.45;
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

  /* FPV = amber tint (first-person) */
  .map-cam-btn.mode-fpv {
    background: rgba(245, 166, 35, 0.2);
    border-color: #f5a623;
    color: #f5a623;
  }

  .map-cam-btn:hover {
    background: rgba(55, 168, 219, 0.25) !important;
    border-color: #37a8db !important;
    color: #37a8db !important;
  }

  .cam-icon { overflow: visible; }
  .north-tri { fill: currentColor; opacity: 0.9; }
</style>
