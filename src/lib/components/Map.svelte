<script module lang="ts">
  // Persisted ACROSS remounts (the 2D map remounts on every 2D↔3D toggle): which
  // playback track we last auto-framed (fitBounds). Without module scope this resets
  // on each remount, so switching back to 2D re-centres on the replay trail every time
  // — it must only frame once, on the first load from the DB.
  let lastPlaybackTrackKey = '';
</script>

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
  import { homePosition } from "$lib/stores/home";
  import MissionLayer from "./mission/MissionLayer.svelte";
  import SurveyPatternLayer from "./mission/SurveyPatternLayer.svelte";
  import TerrainCursorLayer from "./terrain/TerrainCursorLayer.svelte";
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
  import { PLATFORM_MULTIROTOR, type PlatformType, type UavModelOverride } from "$lib/helpers/uavIcons";
  import { resolveModelKind, modelUri } from "$lib/helpers/uavModels";
  import { loadUavMesh, type UavMesh } from "$lib/helpers/uavMesh";
  import { renderUavTopDown } from "$lib/helpers/uavTopDown";
  import { toTelemetryData } from "$lib/adapters/telemetryAdapter";
  import { sunAltitudeDeg, cesiumLikeBrightness } from "$lib/utils/sun";
  import { ensureUserLocation, resolveUserLocation, userGeoLocation } from "$lib/helpers/userLocation";

  let {
    playbackTrack = [],
    playbackPoint = null,
    nightMode2D = 'off',
    trackColorMode = 'flightmode' as TrackColorMode,
    platformType = PLATFORM_MULTIROTOR as PlatformType,
    modelOverride = 'auto' as UavModelOverride,
    uiScale = 1,
    fcVariant = 'INAV',
    mapViewMode = '2d' as '2d' | '3d',
    onToggleMapView,
  }: {
    playbackTrack?: TelemetryRecord[];
    playbackPoint?: TelemetryRecord | null;
    /** 2D imagery night dimming: off / auto (sun below horizon) / on. */
    nightMode2D?: 'off' | 'auto' | 'on';
    trackColorMode?: TrackColorMode;
    platformType?: PlatformType;
    modelOverride?: UavModelOverride;
    uiScale?: number;
    fcVariant?: string;
    mapViewMode?: '2d' | '3d';
    onToggleMapView?: () => void;
  } = $props();

  // ── UAV model marker (top-down canvas render of the same .glb used in 3D) ──
  // The marker icon is a persistent <canvas>; the follow rAF loop redraws it every frame with the
  // SMOOTHED position + attitude (same easing as the position smoother), so the orientation updates
  // at 60 fps instead of stepping at the 2–10 Hz data rate — no per-frame toDataURL/setIcon churn.
  let uavMesh: UavMesh | null = null;       // geometry for the current model kind
  let uavMeshToken = 0;                       // guards out-of-order async loads
  const MODEL_REAL_SPAN_M = 14;              // real-world span the model represents (zoom-scaled)
  const MODEL_MIN_PX = 100, MODEL_MAX_PX = 200;
  const MODEL_TINT = 0.28;                    // flight-mode colour mix (a bit stronger on the white base)

  // (Re)load the mesh whenever the resolved model kind changes (platform type or override).
  $effect(() => {
    const kind = resolveModelKind(platformType, modelOverride);
    const token = ++uavMeshToken;
    loadUavMesh(modelUri(kind)).then((m) => {
      if (token !== uavMeshToken) return; // a newer load superseded us
      uavMesh = m;
      applyFollowFrame(); // redraw with the new model
    }).catch(() => { /* keep the previous mesh / fallback dot */ });
  });

  function hexToRgb01(hex: string): [number, number, number] {
    const h = hex.replace('#', '');
    return [parseInt(h.slice(0, 2), 16) / 255, parseInt(h.slice(2, 4), 16) / 255, parseInt(h.slice(4, 6), 16) / 255];
  }
  /** Model pixel size at the current zoom (clamped), then scaled by the UI scaling setting. */
  function modelSizePx(lat: number): number {
    if (!map) return MODEL_MIN_PX;
    const mpp = 156543.03392 * Math.cos(lat * Math.PI / 180) / Math.pow(2, map.getZoom());
    return Math.round(Math.max(MODEL_MIN_PX, Math.min(MODEL_MAX_PX, MODEL_REAL_SPAN_M / mpp)) * (uiScale || 1));
  }
  // Resize the model when the UI scaling setting changes.
  $effect(() => { uiScale; if (map) onZoomEnd(); });
  /** A DivIcon whose content is a blank canvas of `size`px (drawn into by drawModel). */
  function makeModelIcon(size: number): L.DivIcon {
    return L.divIcon({
      className: 'uav-model-icon',
      html: `<canvas class="uav-model-canvas" width="${size}" height="${size}" style="display:block"></canvas>`,
      iconSize: [size, size], iconAnchor: [size / 2, size / 2],
    });
  }
  /** Draw the model into a marker's persistent canvas (cheap — no DOM churn). */
  function drawModel(marker: L.Marker | undefined, heading: number, pitch: number, roll: number, color: string) {
    const cv = marker?.getElement()?.querySelector('canvas') as HTMLCanvasElement | null;
    const ctx = cv?.getContext('2d');
    if (!cv || !ctx) return;
    if (!uavMesh) { // fallback dot until the mesh loads
      ctx.clearRect(0, 0, cv.width, cv.height);
      ctx.fillStyle = color; ctx.beginPath(); ctx.arc(cv.width / 2, cv.height / 2, 6, 0, 2 * Math.PI); ctx.fill();
      return;
    }
    renderUavTopDown(ctx, uavMesh, cv.width, heading, pitch, roll, hexToRgb01(color), MODEL_TINT);
  }
  /** Zoom changed → resize the marker canvases to the new pixel size, then redraw. */
  function onZoomEnd() {
    const lat = followCurrent?.lat ?? 0;
    for (const mk of [uavMarker, playbackMarker]) mk?.setIcon(makeModelIcon(modelSizePx(lat)));
    applyFollowFrame();
  }

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

  // ── Night dimming (2D imagery) ──────────────────────────────────────
  // Cesium darkens its globe's night side to ×0.3 brightness (GlobeFS: lambert*0.9 +
  // vertexShadowDarkness 0.3 → floor 0.3). We mirror that on the Leaflet imagery only.
  let nightDimFactor = 1.0;                    // current applied imagery brightness (1 = none)
  let nightTimer: ReturnType<typeof setInterval> | undefined; // auto re-check (live time drift)
  let unsubUserGeo: (() => void) | undefined;  // recompute when OS geolocation resolves

  // Flight trail (colored segments by flight mode)
  let trailSegments: L.Polyline[] = [];
  let trailCurrentColor = '';
  let trailCurrentPositions: L.LatLng[] = [];
  let activeTrailLine: L.Polyline | undefined;
  // Pre-arm trail: a thin plain black line of GPS movement while DISARMED
  // (monitoring only). Cleared on arm; the colored flight trail takes over.
  let preArmTrailLine: L.Polyline | undefined;
  let preArmPositions: L.LatLng[] = [];
  let playbackLayerGroup: L.LayerGroup | undefined;
  let playbackMarker: L.Marker | undefined;

  // Home position
  let homeMarker: L.Marker | undefined;
  let wasArmed = false;

  // Follow mode:
  // - free: manual map movement
  // - follow: center on UAV, no rotation
  // - heading-follow: center on UAV and rotate with heading
  let viewMode = $state<'free' | 'follow' | 'heading-follow'>('free');
  let mapHeading = 0;

  // ── Position smoothing ──────────────────────────────────────────────
  // Telemetry/playback arrives at ~2 Hz; applying it directly snaps the map +
  // marker. A rAF loop eases the displayed position toward the latest target
  // (exponential, ~250 ms catch-up), decoupled from the update rate. Large
  // jumps (replay scrub, new flight, first fix) snap instead of gliding.
  interface FollowPt { lat: number; lon: number; heading: number; pitch: number; roll: number; }
  let followTarget: FollowPt | null = null;
  let followCurrent: FollowPt | null = null;
  let activeFollowMarker: L.Marker | undefined;
  let activeColor = '#37a8db';
  let followRaf: number | null = null;
  let followLastT = 0;
  const FOLLOW_TAU_MS = 200; // exp. time constant (~250 ms to mostly catch up)
  const FOLLOW_SNAP_M = 300; // jump farther than this → snap, don't glide

  /** Set the latest position + attitude target; the rAF loop eases toward it and redraws the model
   *  marker every frame (so the orientation is smooth, not stepped at the data rate). */
  function setFollowTarget(lat: number, lon: number, heading: number, pitch: number, roll: number, color: string, marker: L.Marker | undefined) {
    if (!isValidGpsCoordinate(lat, lon)) return;
    activeFollowMarker = marker;
    activeColor = color;
    followTarget = { lat, lon, heading, pitch, roll };
    if (!followCurrent) {
      followCurrent = { ...followTarget };
      applyFollowFrame();
    } else if (L.latLng(followCurrent.lat, followCurrent.lon).distanceTo([lat, lon]) > FOLLOW_SNAP_M) {
      followCurrent = { ...followTarget }; // big jump → snap
      applyFollowFrame();
    }
    if (followRaf == null) {
      followLastT = 0;
      followRaf = requestAnimationFrame(followLoop);
    }
  }

  function followLoop(t: number) {
    if (!followTarget || !followCurrent) { followRaf = null; return; }
    const dt = followLastT ? Math.min(120, t - followLastT) : 16;
    followLastT = t;
    const k = 1 - Math.exp(-dt / FOLLOW_TAU_MS); // framerate-normalized ease
    followCurrent.lat += (followTarget.lat - followCurrent.lat) * k;
    followCurrent.lon += (followTarget.lon - followCurrent.lon) * k;
    const dh = ((followTarget.heading - followCurrent.heading + 540) % 360) - 180; // shortest path
    followCurrent.heading = (followCurrent.heading + dh * k + 360) % 360;
    followCurrent.pitch += (followTarget.pitch - followCurrent.pitch) * k;
    const dr = ((followTarget.roll - followCurrent.roll + 540) % 360) - 180; // shortest path (handles ±180 inversion)
    followCurrent.roll += dr * k;
    applyFollowFrame();
    const dist = L.latLng(followCurrent.lat, followCurrent.lon).distanceTo([followTarget.lat, followTarget.lon]);
    if (dist < 0.5 && Math.abs(dh) < 0.3 && Math.abs(followTarget.pitch - followCurrent.pitch) < 0.3 && Math.abs(dr) < 0.3) {
      followCurrent = { ...followTarget }; // settled — snap exactly + stop until next target
      applyFollowFrame();
      followRaf = null;
      return;
    }
    followRaf = requestAnimationFrame(followLoop);
  }

  /** Apply the eased frame: move + redraw the active marker always, recenter (+rotate) the map
   *  only while following. */
  function applyFollowFrame() {
    if (!map || !followCurrent) return;
    const ll: L.LatLngExpression = [followCurrent.lat, followCurrent.lon];
    if (activeFollowMarker) {
      activeFollowMarker.setLatLng(ll);
      drawModel(activeFollowMarker, followCurrent.heading, followCurrent.pitch, followCurrent.roll, activeColor);
    }
    // Don't fight an in-progress zoom animation (would snap mid-zoom).
    if (viewMode !== 'free' && !(map as unknown as { _animatingZoom?: boolean })._animatingZoom) {
      map.setView(ll, map.getZoom(), { animate: false });
      if (viewMode === 'heading-follow') {
        mapHeading = followCurrent.heading;
        mapContainer?.style.setProperty('--map-rotation', `${-mapHeading}deg`);
      }
    }
  }

  let followTitle = $derived.by(() => {
    if (viewMode === 'free') return 'Follow mode: Free';
    if (viewMode === 'follow') return 'Follow mode: Follow';
    return 'Follow mode: Heading Follow';
  });

  function updateUavPosition(lat: number, lon: number, heading: number, navState = 0, roll = 0, pitch = 0) {
    if (!map) return;
    if (!isValidGpsCoordinate(lat, lon)) return;

    const color = getNavStateColor(navState); // marker = nav state (the track shows flight mode) — see COLORED_TRACK_PLAN
    if (!uavMarker) {
      uavMarker = L.marker([lat, lon], { icon: makeModelIcon(modelSizePx(lat)), zIndexOffset: 1000 }).addTo(map);
    }
    setFollowTarget(lat, lon, heading, pitch, roll, color, uavMarker); // eases + redraws the model
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

  /** Thin plain black trail of GPS movement while disarmed (monitoring only). */
  function updatePreArmTrail(lat: number, lon: number) {
    if (!map) return;
    const pos = L.latLng(lat, lon);
    if (preArmPositions.length > 0 &&
        pos.distanceTo(preArmPositions[preArmPositions.length - 1]) < MIN_TRAIL_DIST) {
      return;
    }
    preArmPositions.push(pos);
    if (preArmPositions.length >= 2) {
      if (preArmTrailLine) {
        preArmTrailLine.setLatLngs(preArmPositions);
      } else {
        preArmTrailLine = L.polyline(preArmPositions, { color: '#000000', weight: 1, opacity: 0.8 }).addTo(map);
      }
    }
  }

  function resetPreArmTrail() {
    if (preArmTrailLine) { map?.removeLayer(preArmTrailLine); preArmTrailLine = undefined; }
    preArmPositions = [];
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
      const segments = segmentTrackByFlightMode(validTrack, fcVariant);
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
      // Frame the whole track only in free mode — don't yank the view away from
      // an active follow (which centers on the UAV/playback marker).
      if (viewMode === 'free') {
        map.fitBounds(L.latLngBounds(positions), { padding: [36, 36] });
      }
      lastPlaybackTrackKey = nextKey;
    }
  }

  function updatePlaybackMarker(point: TelemetryRecord | null) {
    if (!map) return;

    if (!point || point.lat == null || point.lon == null || !isValidGpsCoordinate(point.lat, point.lon)) {
      if (activeFollowMarker === playbackMarker) activeFollowMarker = undefined;
      if (playbackMarker) {
        map.removeLayer(playbackMarker);
        playbackMarker = undefined;
      }
      return;
    }

    const heading = point.heading ?? 0;
    const color = getNavStateColor(point.nav_state ?? 0); // marker = nav state
    // Attitude from the same unified adapter the AHI / 3D model use.
    const td = toTelemetryData(point, fcVariant);
    if (!playbackMarker) {
      playbackMarker = L.marker([point.lat, point.lon], { icon: makeModelIcon(modelSizePx(point.lat)), zIndexOffset: 900 }).addTo(map);
    }
    setFollowTarget(point.lat, point.lon, heading, td.pitch, td.roll, color, playbackMarker); // eases + redraws
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
      // Enable over-zoom placeholder detection on flagged base layers (ESRI sat).
      providerId: provider.detectPlaceholders ? provider.id : undefined,
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

    // Re-apply the current night dim to the freshly built layers.
    applyNightDim(nightDimFactor, true);
  }

  // ── Night dimming ───────────────────────────────────────────────────

  /**
   * Darken ONLY the imagery tile layers (telemetry/markers stay full bright) by a continuous
   * brightness factor (1 = none, 0.3 = full night). `force` re-applies to freshly built layers.
   */
  function applyNightDim(factor: number, force = false) {
    if (!force && Math.abs(factor - nightDimFactor) < 0.005) return;
    nightDimFactor = factor;
    const filter = factor < 0.999 ? `brightness(${factor.toFixed(3)})` : '';
    const setF = (layer?: L.TileLayer) => {
      const el = layer?.getContainer?.();
      if (el) {
        el.style.transition = 'filter 0.6s ease';
        el.style.filter = filter;
      }
    };
    setF(currentBase);
    for (const ol of currentOverlays) setF(ol);
  }

  /** Compute the imagery brightness for the current night setting and apply it. */
  function recomputeNight() {
    let factor = 1.0;
    if (nightMode2D === 'on') {
      factor = 0.3;
    } else if (nightMode2D === 'auto') {
      // Auto = user system-time + PHYSICAL location (sunset based), smooth — NOT log/camera.
      const u = resolveUserLocation(); // OS geo → UAV GPS → home → persisted map centre
      factor = cesiumLikeBrightness(sunAltitudeDeg(new Date(), u.lat, u.lon));
    }
    applyNightDim(factor);
  }

  // Re-check when the setting changes (auto also re-checks on map move + a timer).
  $effect(() => {
    void nightMode2D; // reactive dep
    recomputeNight();
  });

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

    // Initialize tile cache with persisted size limit
    initTileCache(s.mapCacheMaxMB);

    // Apply the persisted (or default) map provider
    applyProvider(getProviderById(s.mapProvider));

    map.on("moveend", saveMapState);
    map.on("zoomend", saveMapState);
    map.on("zoomend", onZoomEnd);   // resize the model canvas to the new zoom
    map.on("moveend", recomputeNight); // auto night: re-check after panning to a new region

    // Auto night mode: physical location + re-check every minute so wall-clock drift fades day↔night.
    ensureUserLocation(); // OS geolocation (resolves async)
    unsubUserGeo = userGeoLocation.subscribe(() => recomputeNight()); // recompute once it resolves
    nightTimer = setInterval(recomputeNight, 60_000);
    recomputeNight();

    // Invalidate size when container resizes (e.g. side panel toggle)
    const onResize = () => {
      if (viewMode === 'heading-follow') applyHeadingUpSize(true);
      map?.invalidateSize();
    };
    window.addEventListener("resize", onResize);

    // Fix tile rendering on initial load
    setTimeout(() => map?.invalidateSize(), 100);

    // Subscribe to telemetry for UAV position, flight trail, and home detection
    unsubTelemetry = telemetry.subscribe((t) => {
      if (t.lastUpdate > 0) {
        updateUavPosition(t.lat, t.lon, t.yaw, t.navState, t.roll, t.pitch); // drives the smoother (marker + follow)

        // Flight trail: colored by flight mode while armed; a thin black
        // monitoring line while disarmed (pre-arm).
        const armed = (t.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
        if (isValidGpsCoordinate(t.lat, t.lon)) {
          if (armed) updateTrail(t.lat, t.lon, t.activeFlightModeFlags);
          else updatePreArmTrail(t.lat, t.lon);
        }

        // Home position: set on arm transition when GPS has fix
        if (armed && !wasArmed && t.fixType >= 2 && t.lat !== 0) {
          homePosition.set({ lat: t.lat, lon: t.lon, alt: t.altitude, set: true });
          updateHomeMarker(t.lat, t.lon);
          // Clear trail for new flight
          for (const seg of trailSegments) { map?.removeLayer(seg); }
          trailSegments = [];
          if (activeTrailLine) { map?.removeLayer(activeTrailLine); activeTrailLine = undefined; }
          trailCurrentPositions = [];
          trailCurrentColor = '';
          // Drop the pre-arm line — the colored flight trail takes over.
          resetPreArmTrail();
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
    if (viewMode === 'free') {
      viewMode = 'follow';
      mapHeading = 0;
      mapContainer?.style.setProperty('--map-rotation', '0deg');
      applyHeadingUpSize(false);
      // Disable panning while following — the view is locked to the UAV, so a
      // drag would only fight the follow. Zoom stays enabled, but anchored to
      // the map centre (= UAV) instead of the cursor.
      map?.dragging.disable();
      setZoomAnchor('center');
      applyFollowFrame(); // center on the current position immediately
      return;
    }

    if (viewMode === 'follow') {
      viewMode = 'heading-follow';
      applyHeadingUpSize(true);
      applyFollowFrame(); // apply rotation immediately
      return; // dragging stays disabled
    }

    viewMode = 'free';
    mapHeading = 0;
    mapContainer?.style.setProperty('--map-rotation', '0deg');
    applyHeadingUpSize(false);
    map?.dragging.enable();
    setZoomAnchor('cursor');
  }

  /** Anchor wheel/dblclick/pinch zoom to the map centre (UAV in follow) or the
   *  cursor (free). Leaflet reads these options at zoom time, so mutating them
   *  live is enough — no handler re-init needed. */
  function setZoomAnchor(mode: 'center' | 'cursor') {
    if (!map) return;
    const v = mode === 'center' ? 'center' : true;
    map.options.scrollWheelZoom = v;
    map.options.doubleClickZoom = v;
    map.options.touchZoom = v;
  }

  function zoomIn() {
    map?.zoomIn();
  }

  function zoomOut() {
    map?.zoomOut();
  }

  $effect(() => {
    if (!map) return;
    updatePlaybackTrack(playbackTrack, trackColorMode);
  });

  // Replay marker + follow: updatePlaybackMarker feeds the smoother, which moves
  // the marker always and recenters the map when following. (Live: playbackPoint
  // is null → no-op; the telemetry path drives the smoother instead.)
  $effect(() => {
    if (!map) return;
    updatePlaybackMarker(playbackPoint);
  });

  onDestroy(() => {
    if (followRaf != null) cancelAnimationFrame(followRaf);
    if (nightTimer) clearInterval(nightTimer);
    unsubUserGeo?.();
    if (unsubTelemetry) unsubTelemetry();
    if (unsubSettings) unsubSettings();
    if (map) {
      map.off("moveend", saveMapState);
      map.off("zoomend", saveMapState);
      map.off("zoomend", onZoomEnd);
      map.off("moveend", recomputeNight);
      if (uavMarker) map.removeLayer(uavMarker);
      for (const seg of trailSegments) map.removeLayer(seg);
      if (activeTrailLine) map.removeLayer(activeTrailLine);
      if (preArmTrailLine) map.removeLayer(preArmTrailLine);
      if (playbackLayerGroup) map.removeLayer(playbackLayerGroup);
      if (playbackMarker) map.removeLayer(playbackMarker);
      if (homeMarker) map.removeLayer(homeMarker);
      map.remove();
    }
  });
</script>

<div class="map-wrapper">
  <div bind:this={mapContainer} class="map" style="--map-rotation: 0deg"></div>

  <div class="map-controls-corner">
    <button class="map-control-btn map-mode-btn"
            onclick={() => onToggleMapView?.()}
            title={mapViewMode === '2d' ? '3D View' : '2D View'}
            aria-label={mapViewMode === '2d' ? 'Switch to 3D view' : 'Switch to 2D view'}>
      {mapViewMode === '2d' ? '3D' : '2D'}
    </button>

    <button class="map-control-btn map-heading-btn"
            class:mode-free={viewMode === 'free'}
            class:mode-follow={viewMode === 'follow'}
            class:mode-heading={viewMode === 'heading-follow'}
            onclick={toggleViewMode}
            title={followTitle}
            aria-label={followTitle}>
      {#if viewMode === 'heading-follow'}
        <svg class="heading-icon heading-icon-up" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <polygon class="uav-arrow" points="12,6 7.5,17.5 12,15.2 16.5,17.5" />
        </svg>
      {:else}
        <svg class="heading-icon heading-icon-diag" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <polygon class="north-triangle" points="12,4.6 9.9,8.6 14.1,8.6" />
          <g transform="translate(0 -1.5) rotate(-70 12 15)">
            <polygon class="uav-arrow" points="12,8.6 7.7,19.6 12,17.4 16.3,19.6" />
          </g>
        </svg>
      {/if}
    </button>

    <button class="map-control-btn map-zoom-btn map-zoom-in" onclick={zoomIn} title="Zoom in" aria-label="Zoom in">+</button>
    <button class="map-control-btn map-zoom-btn map-zoom-out" onclick={zoomOut} title="Zoom out" aria-label="Zoom out">-</button>
  </div>

  {#if map}
    <MissionLayer {map} />
    <SurveyPatternLayer {map} />
    <TerrainCursorLayer {map} />
  {/if}
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

  .map-controls-corner {
    position: absolute;
    bottom: 8px;
    right: 8px;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    gap: 8px;
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

  .map-heading-btn.mode-free {
    background: rgba(46, 46, 46, 0.45);
    border-color: rgba(55, 168, 219, 0.45);
    color: rgba(199, 223, 232, 0.95);
    backdrop-filter: blur(4px);
  }

  .map-heading-btn.mode-follow,
  .map-heading-btn.mode-heading {
    background: rgba(46, 46, 46, 0.92);
    border-color: rgba(55, 168, 219, 0.7);
    color: #37a8db;
    backdrop-filter: blur(8px);
  }

  .map-heading-btn.mode-free:hover {
    background: rgba(55, 168, 219, 0.12);
    border-color: rgba(55, 168, 219, 0.75);
  }

  .heading-icon {
    width: 45px;
    height: 45px;
    overflow: visible;
  }

  .heading-icon .uav-arrow {
    fill: currentColor;
  }

  .heading-icon .north-triangle {
    fill: currentColor;
    opacity: 0.9;
  }

  .map-heading-btn.mode-free .heading-icon {
    opacity: 0.95;
  }

  .map-heading-btn.mode-follow .heading-icon,
  .map-heading-btn.mode-heading .heading-icon {
    opacity: 1;
  }

  .heading-icon-up .uav-arrow {
    transform-origin: 12px 12px;
  }

  /* Fix Leaflet icon paths broken by bundlers */
  :global(.leaflet-default-icon-path) {
    background-image: url("leaflet/dist/images/marker-icon.png");
  }
</style>
