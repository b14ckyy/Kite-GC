<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

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
  import { homePosition, homeMarkerShown } from "$lib/stores/home";
  import { editMode, geoWaypoints, launchPoint, replayActive, toDeg } from "$lib/stores/mission";
  import { autopilotSystem } from "$lib/stores/autopilotContext";
  import { arduMission } from "$lib/stores/missionArdupilot";
  import { cmdHasLocation } from "$lib/helpers/arduCommandCatalog";
  import { connection } from "$lib/stores/connection";
  import { frameMissionSignal } from "$lib/stores/mapCamera";
  import { destinationPoint } from "$lib/utils/geo";
  import MissionLayer from "./mission/MissionLayer.svelte";
  import SurveyPatternLayer from "./mission/SurveyPatternLayer.svelte";
  import TerrainCursorLayer from "./terrain/TerrainCursorLayer.svelte";
  import RfRayLayer from "./terrain/RfRayLayer.svelte";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import {
    segmentTrackByFlightMode,
    segmentTrackByAltitude,
    segmentTrackBySpeed,
    segmentTrackBySignal,
    getNavStateColor,
    type TrackColorMode,
    type TrackSegment,
  } from "$lib/helpers/trackColors";
  import { modeColor } from "$lib/helpers/flightModeRegistry";
  import { PLATFORM_MULTIROTOR, type PlatformType, type UavModelOverride } from "$lib/helpers/uavIcons";
  import { resolveModelKind, modelUri } from "$lib/helpers/uavModels";
  import { loadUavMesh, type UavMesh } from "$lib/helpers/uavMesh";
  import { renderUavTopDown } from "$lib/helpers/uavTopDown";
  import { toTelemetryData } from "$lib/adapters/telemetryAdapter";
  import { sunAltitudeDeg, cesiumLikeBrightness } from "$lib/utils/sun";
  import { ensureUserLocation, resolveUserLocation, userGeoLocation } from "$lib/helpers/userLocation";
  import { radarVehicles, radarSelection, enrichList, type RadarSnapshot, type EnrichedVehicle } from "$lib/stores/radarTracking";
  import { radarAlertLevels, type AlertLevel } from "$lib/controllers/radarAlerts";
  import { gcsLocation, gcsAccuracyM, setGcsManual } from "$lib/stores/gcsLocation";
  import { aeroData, aeroFocus, type AeroPoint, type Airspace } from "$lib/stores/airspace";
  import { airspaceStyle, aeroPointIconHtml, aeroPointInfo, airspaceMinZoom, airportMinZoom, obstacleMinZoom, RC_MIN_ZOOM, airspaceContainsPoint, airspaceIsRelevant, isAirspaceHidden } from "$lib/helpers/airspaceStyle";
  import { type RadarMapSettings, type GcsMode } from "$lib/stores/settings";
  import { openContextMenu } from "$lib/stores/contextMenu";
  import { t } from "svelte-i18n";
  import { contactColor, ffContactColor, contactVisibleOnMap, relevanceFactor } from "$lib/helpers/radarMap";
  import { pickShape, buildContactIconHtml } from "$lib/helpers/radarIcons";
  import { convertAltitude, convertSpeed, convertDistance, convertVerticalSpeed, formatConverted } from "$lib/utils/units";

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
    viewMode = $bindable<'free' | 'follow' | 'heading-follow'>('free'),
    radarActive = false,
    radarMapSettings = null,
    radarReference = null,
    radarRefAltM = null,
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
    /** 2D follow state, lifted so it persists across 2D↔3D remounts (bound by the parent). */
    viewMode?: 'free' | 'follow' | 'heading-follow';
    /** Radar master enable (renders nothing when off). */
    radarActive?: boolean;
    /** Map rendering controls for radar contacts, or null to render none. */
    radarMapSettings?: RadarMapSettings | null;
    /** Distance/bearing reference (UAV valid-fix else GCS), for enrichment. */
    radarReference?: { lat: number; lon: number } | null;
    /** Reference altitude (m) for the relative-altitude colour scale, or null (→ absolute fallback). */
    radarRefAltM?: number | null;
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
  let unsubFrameMission: (() => void) | undefined;
  let unsubConnForJump: (() => void) | undefined;

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

  // ── Direction marker lines (heading + course-over-ground) ────────────
  // Two short lines from the UAV nose: HDG (solid blue) along the FC heading, COG (dashed amber) along
  // the ground track — the angle between them is the crab. Colours match the compass widget. Drawn from
  // the SMOOTHED follow state (same filtering as the marker), geographic so they pan/zoom natively.
  let dirLayer: L.LayerGroup | undefined;
  let hdgLine: L.Polyline | undefined;
  let cogLine: L.Polyline | undefined;
  let hdgCasing: L.Polyline | undefined; // dark outline under each line for contrast on any terrain
  let cogCasing: L.Polyline | undefined;
  let turnLine: L.Polyline | undefined;  // predicted turn arc (thin white, drawn under the H/B lines)
  let turnCasing: L.Polyline | undefined;
  const DIR_LEAD_S = 15;     // line length = how far the UAV travels in this many seconds (velocity vector)
  const DIR_COG_MIN_SPEED = 1.5; // m/s — COG is noise below this → hide the amber line
  const TURN_SHOW_RATE = 5;  // deg/s — only START showing on a real turn (above fluctuation noise)
  const TURN_KEEP_RATE = 3;  // deg/s — once shown, keep while above this …
  const TURN_HOLD_MS = 2000; // … and for this long after it drops below (hysteresis vs. fluctuations)
  const TURN_MAX_SWEEP = 180; // cap the arc at a half-circle so it never spirals
  const TURN_SEG_DEG = 5;    // arc polyline resolution (degrees of turn per segment)
  // Turn-rate estimate (deg/s, signed): low-passed dCOG/dt on the SOURCE time base (wall clock live,
  // log timestamp in replay → playback speed doesn't distort it). The DISPLAYED rate is eased per-frame
  // in followLoop (followCurrent.turnRate) so the arc transitions smoothly instead of whipping.
  let turnRateDegS = 0;
  let prevCog: number | null = null;
  let prevCogTs = 0;
  let turnArcShown = false;    // hysteresis: is the arc currently displayed?
  let turnArcLastActiveTs = 0; // last source time the rate was ≥ TURN_KEEP_RATE

  // ── Foreign-vehicle (radar) contacts — isolated layer, diffed by id ──
  let radarLayer: L.LayerGroup | undefined;
  const radarMarkers = new Map<string, L.Marker>();
  let radarSnap: RadarSnapshot = { adsb: [], formationFlight: [], radio: [], lastUpdate: 0 };
  let radarSelectedId: string | null = null;
  let radarAlertMap: Map<string, AlertLevel> = new Map();
  let unsubRadar: (() => void) | undefined;
  let unsubRadarSel: (() => void) | undefined;
  let unsubRadarAlerts: (() => void) | undefined;
  const RADAR_BASE_PX = 42;
  const RADAR_MIN_PX = 24;

  /** Full hover/select readout for a contact (units honour the interface settings, like the panel). */
  function radarTooltip(v: EnrichedVehicle): string {
    const ui = get(settings).interface;
    const name = v.callsign?.trim() || v.id;
    const alt = v.altM == null ? '—' : formatConverted(convertAltitude(v.altM, ui.altitudeUnit), 0);
    const spd = v.groundSpeedMs == null ? '—' : formatConverted(convertSpeed(v.groundSpeedMs, ui.speedUnit), 0);
    const dist = v.distanceM == null
      ? '—'
      : formatConverted(convertDistance(v.distanceM, ui.distanceUnit), v.distanceM < 10000 ? 1 : 0);
    const brg = v.bearingDeg == null ? '—' : `${Math.round(v.bearingDeg)}°`;
    let vs = '';
    if (v.verticalSpeedMs != null && Math.abs(v.verticalSpeedMs) >= 0.5) {
      const a = formatConverted(convertVerticalSpeed(Math.abs(v.verticalSpeedMs), ui.verticalSpeedUnit), 1);
      vs = ` ${v.verticalSpeedMs > 0 ? '▲' : '▼'}${a}`;
    }
    return `${name} · ${alt}${vs} · ${spd} · ${dist} · ${brg}`;
  }

  /** Rebuild/diff the radar contact markers from the latest snapshot + map controls. */
  function updateRadar() {
    if (!map) return;
    if (!radarLayer) radarLayer = L.layerGroup().addTo(map);
    const ms = radarMapSettings;
    if (!radarActive || !ms) {
      if (radarMarkers.size) { radarLayer.clearLayers(); radarMarkers.clear(); }
      return;
    }
    const all = [...radarSnap.adsb, ...radarSnap.formationFlight, ...radarSnap.radio];
    const enriched = enrichList(all, radarReference ?? null);
    const seen = new Set<string>();
    for (const v of enriched) {
      if (!contactVisibleOnMap(v, radarRefAltM, ms)) continue;
      seen.add(v.id);
      const rel = relevanceFactor(v.distanceM, ms.radiusKm, ms.showAll);
      // FormationFlight icons render 20% larger than ADS-B.
      const sizeMul = v.system === 'formationFlight' ? 1.2 : 1;
      const size = Math.max(RADAR_MIN_PX, Math.round(RADAR_BASE_PX * (uiScale || 1) * (0.6 + 0.4 * rel) * sizeMul));
      const html = buildContactIconHtml({
        shape: pickShape(v.system, v.category, v.headingDeg != null),
        heading: v.headingDeg,
        // FormationFlight uses a state colour (armed/disarmed/lost); ADS-B uses the altitude scale.
        color: v.system === 'formationFlight'
          ? ffContactColor(v.extra?.ffState)
          : contactColor(v.altM, radarRefAltM),
        sizePx: size,
        opacity: rel,
        selected: v.id === radarSelectedId,
        label: v.callsign?.trim() || undefined,
        badgeLabel: v.system === 'formationFlight', // big single-letter id badge
        alertLevel: radarAlertMap.get(v.id) ?? null,
      });
      const icon = L.divIcon({ className: 'radar-divicon', html, iconSize: [size, size], iconAnchor: [size / 2, size / 2] });
      const existing = radarMarkers.get(v.id);
      if (existing) {
        existing.setLatLng([v.lat, v.lon]);
        existing.setIcon(icon);
        existing.setTooltipContent(radarTooltip(v));
      } else {
        const id = v.id;
        const m = L.marker([v.lat, v.lon], { icon, zIndexOffset: 400 });
        m.bindTooltip(radarTooltip(v), { direction: 'top', offset: [0, -size / 2], opacity: 0.95 });
        m.on('click', () => radarSelection.update((cur) => (cur === id ? null : id)));
        m.addTo(radarLayer);
        radarMarkers.set(id, m);
      }
    }
    for (const [id, m] of radarMarkers) {
      if (!seen.has(id)) { radarLayer.removeLayer(m); radarMarkers.delete(id); }
    }
  }

  // Rebuild when any radar control prop changes (snapshot/selection are handled by their subscriptions).
  $effect(() => {
    radarActive; radarMapSettings; radarReference; radarRefAltM; uiScale;
    if (map) updateRadar();
  });

  // Home position
  let homeMarker: L.Marker | undefined;
  let wasArmed = false;

  // ── GCS (ground-station) marker ──
  let gcsMarker: L.Marker | undefined;
  let gcsAccuracyCircle: L.Circle | undefined;
  let gcsSelected = false; // show the accuracy circle only while selected
  let unsubGcs: (() => void) | undefined;
  let unsubGcsAcc: (() => void) | undefined;
  let unsubHome2d: (() => void) | undefined;
  const gcsMode = $derived<GcsMode>($settings.gcsMode);

  /** Satellite-dish marker on a dark translucent disc; mode tweaks the affordance (drag ring / live dot). */
  function createGcsIcon(mode: GcsMode): L.DivIcon {
    return L.divIcon({
      className: "gcs-icon",
      html: `<div class="gcs-dot gcs-${mode}">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="#37a8db" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 10a7.31 7.31 0 0 0 10 10Z"/><path d="m9 15 3-3"/>
          <path d="M17 13a6 6 0 0 0-6-6"/><path d="M21 13A10 10 0 0 0 11 3"/>
        </svg>
        ${mode === "continuous" ? '<span class="gcs-live"></span>' : ""}
      </div>`,
      iconSize: [30, 30],
      iconAnchor: [15, 15],
    });
  }

  function updateGcsAccuracyCircle() {
    if (!map) return;
    const loc = get(gcsLocation);
    const acc = get(gcsAccuracyM);
    if (gcsSelected && loc && acc != null && acc > 0) {
      const ll: L.LatLngExpression = [loc.lat, loc.lon];
      if (gcsAccuracyCircle) {
        gcsAccuracyCircle.setLatLng(ll).setRadius(acc);
      } else {
        gcsAccuracyCircle = L.circle(ll, {
          radius: acc, color: "#37a8db", weight: 1, fillColor: "#37a8db", fillOpacity: 0.08, interactive: false,
        }).addTo(map);
      }
    } else if (gcsAccuracyCircle) {
      map.removeLayer(gcsAccuracyCircle);
      gcsAccuracyCircle = undefined;
    }
  }

  function updateGcsMarker() {
    if (!map) return;
    const loc = get(gcsLocation);
    if (gcsMode === "off" || !loc) {
      if (gcsMarker) { map.removeLayer(gcsMarker); gcsMarker = undefined; }
      gcsSelected = false;
      updateGcsAccuracyCircle();
      return;
    }
    const ll: L.LatLngExpression = [loc.lat, loc.lon];
    const draggable = gcsMode === "manual";
    if (!gcsMarker) {
      gcsMarker = L.marker(ll, { icon: createGcsIcon(gcsMode), draggable, zIndexOffset: 450 }).addTo(map);
      gcsMarker.bindTooltip(get(t)("settings.gcsMarker"), { direction: "top", offset: [0, -16], opacity: 0.95 });
      gcsMarker.on("dragend", () => {
        const p = gcsMarker!.getLatLng();
        setGcsManual(p.lat, p.lng); // pins it
      });
      gcsMarker.on("click", (e) => {
        L.DomEvent.stopPropagation(e);
        gcsSelected = !gcsSelected;
        updateGcsAccuracyCircle();
      });
    } else {
      gcsMarker.setLatLng(ll);
      gcsMarker.setIcon(createGcsIcon(gcsMode));
      if (gcsMarker.dragging) draggable ? gcsMarker.dragging.enable() : gcsMarker.dragging.disable();
    }
    updateGcsAccuracyCircle();
  }

  // Rebuild the GCS marker when the mode changes (location/accuracy handled by their subscriptions).
  $effect(() => { gcsMode; if (map) updateGcsMarker(); });

  /** Right-click on the empty map → "Set GCS here" (manual mode only). */
  function onMapContextMenu(e: L.LeafletMouseEvent) {
    if (gcsMode !== "manual") return;
    const { lat, lng } = e.latlng;
    openContextMenu(e.originalEvent.clientX, e.originalEvent.clientY, [
      { label: get(t)("settings.gcsSetHere"), icon: "📡", action: () => setGcsManual(lat, lng) },
    ]);
  }

  // ── Airspace Manager (aeronautical data) 2D layer ──
  let airspaceLayer: L.LayerGroup | undefined;
  let unsubAero: (() => void) | undefined;
  let unsubAeroFocus: (() => void) | undefined;
  const MAX_AERO_MARKERS = 1500; // cap rendered point markers per redraw (dense regions)
  // Airspaces currently drawn (passed the zoom filter) — the click handler tests these for
  // point-in-polygon so a single click lists *every* airspace stacked at that spot, not just the top one.
  let drawnAirspaces: Airspace[] = [];
  let aeroPopup: L.Popup | undefined; // the airspace-list popup; a second map click toggles it off

  /** Rebuild the airspace overlay: all airspaces (few), point features clipped to the current view. */
  function updateAirspace() {
    if (!map) return;
    if (!airspaceLayer) airspaceLayer = L.layerGroup().addTo(map);
    const group = airspaceLayer;
    group.clearLayers();
    if (!$settings.airspace.enabled) return; // OFF → nothing drawn
    const data = get(aeroData);
    const vis = get(settings).airspace.layers;
    const z = map.getZoom(); // zoom-density: each feature has a min-zoom by importance/size

    drawnAirspaces = [];
    if (vis.airspaces.d2) {
      for (const a of data.airspaces) {
        if (isAirspaceHidden(a)) continue; // FIR/UIR blanket the country → never drawn / clickable
        if (z < airspaceMinZoom(a)) continue;
        drawnAirspaces.push(a);
        const st = airspaceStyle(a);
        for (const ring of a.outlines) {
          const latlngs = ring.map(([lon, lat]) => [lat, lon] as [number, number]);
          // Non-interactive: clicks fall through to the map handler, which lists *all* airspaces
          // stacked at the click point (overlapping airspaces would otherwise hide one another).
          const poly = L.polygon(latlngs, {
            color: st.color, weight: st.weight, fillColor: st.fillColor, fillOpacity: st.fillOpacity,
            dashArray: st.dashArray, interactive: false,
          });
          group.addLayer(poly);
        }
      }
    }

    // Point layers — clipped to the visible bounds (a 500 km cache can hold thousands of obstacles).
    const bounds = map.getBounds();
    let count = 0;
    const addPoints = (pts: AeroPoint[], on: boolean, minZoom: (p: AeroPoint) => number) => {
      if (!on) return;
      for (const p of pts) {
        if (count >= MAX_AERO_MARKERS) break;
        if (z < minZoom(p)) continue;                 // zoom-density filter
        if (!bounds.contains([p.lat, p.lon])) continue; // clip to view
        const m = L.marker([p.lat, p.lon], {
          icon: L.divIcon({ className: "aero-divicon", html: aeroPointIconHtml(p), iconSize: [20, 20], iconAnchor: [10, 10] }),
          bubblingMouseEvents: false, // keep marker clicks out of the airspace-list map handler
        });
        const info = aeroPointInfo(p);
        m.bindPopup(`<b>${p.name || p.subtype || p.kind}</b>${info ? `<br>${info}` : ""}`);
        group.addLayer(m);
        count++;
      }
    };
    addPoints(data.obstacles, vis.obstacles.d2, obstacleMinZoom);
    addPoints(data.airports, vis.airports.d2, airportMinZoom);
    addPoints(data.rcAirfields, vis.rc.d2, () => RC_MIN_ZOOM);
  }

  /**
   * Map click → list *all* airspaces stacked at that point (overlapping airspaces hide one another
   * with per-polygon popups). Unclassified airspaces (no ICAO class) are skipped to cut clutter.
   * Sorted by lower limit so the lowest/most-relevant airspace is on top.
   */
  function onAirspaceClick(e: L.LeafletMouseEvent) {
    if (!map || !$settings.airspace.enabled) return;
    if (get(editMode)) return; // while editing waypoints, map clicks deselect WPs — don't pop the airspace list
    // Toggle: if the list popup is already open, a second map click just dismisses it.
    if (aeroPopup && aeroPopup.isOpen()) {
      map.closePopup(aeroPopup);
      aeroPopup = undefined;
      return;
    }
    const { lat, lng } = e.latlng;
    const all = drawnAirspaces.filter((a) => airspaceContainsPoint(a, lat, lng));
    // Drop unclassified "free" airspace when a classified/critical one covers the same point; if there
    // are only unclassified ones here, still show them (better than an empty popup).
    const relevant = all.filter(airspaceIsRelevant);
    const hits = (relevant.length ? relevant : all).sort((a, b) => a.lower.valueM - b.lower.valueM);
    if (hits.length === 0) return;
    const rows = hits
      .map(
        (a) =>
          `<div style="margin:3px 0;padding-left:7px;border-left:3px solid ${airspaceStyle(a).color};">` +
          `<b>${a.name}</b><br><span style="opacity:.8">${a.typeName}` +
          `${a.icaoClassName !== "?" ? ` · Class ${a.icaoClassName}` : ""} · ` +
          `${a.lower.label} – ${a.upper.label}</span></div>`,
      )
      .join("");
    // closeOnClick:false → the map click is handled here (toggle), not by Leaflet's auto-close.
    aeroPopup = L.popup({ maxWidth: 280, closeOnClick: false })
      .setLatLng(e.latlng)
      .setContent(`<div style="max-height:240px;overflow:auto;">${rows}</div>`)
      .openOn(map);
  }

  // Redraw on enable-toggle, new data, or visibility change (data/visibility via subscriptions below).
  $effect(() => { void $settings.airspace.enabled; if (map) updateAirspace(); });
  // Toggle the heading/course/turn nose lines on/off (clears or redraws immediately, not just next frame).
  $effect(() => { void $settings.directionLines; if (map) redrawDirLines(); });

  // Follow mode (bindable prop, see $props above):
  // - free: manual map movement
  // - follow: center on UAV, no rotation
  // - heading-follow: center on UAV and rotate with heading
  let mapHeading = 0;

  // ── Position smoothing ──────────────────────────────────────────────
  // Telemetry/playback arrives at ~2 Hz; applying it directly snaps the map +
  // marker. A rAF loop eases the displayed position toward the latest target
  // (exponential, ~250 ms catch-up), decoupled from the update rate. Large
  // jumps (replay scrub, new flight, first fix) snap instead of gliding.
  interface FollowPt { lat: number; lon: number; heading: number; pitch: number; roll: number; course: number; speed: number; turnRate: number; }
  let followTarget: FollowPt | null = null;
  let followCurrent: FollowPt | null = null;
  let activeFollowMarker: L.Marker | undefined;
  let activeColor = '#37a8db';
  let followRaf: number | null = null;
  let followLastT = 0;
  // True only while applyFollowFrame() drives a programmatic recenter. Leaflet fires `moveend`
  // SYNCHRONOUSLY inside setView, so saveMapState would otherwise persist the UAV-locked centre
  // 60×/s — writing the settings store from inside the render flush (→ effect_update_depth_exceeded)
  // and pointlessly thrashing localStorage. The follow centre is not user map state; skip it.
  let followDrivingView = false;
  const FOLLOW_TAU_MS = 200; // exp. time constant (~250 ms to mostly catch up)
  const FOLLOW_SNAP_M = 300; // jump farther than this → snap, don't glide

  /** Set the latest position + attitude target; the rAF loop eases toward it and redraws the model
   *  marker every frame (so the orientation is smooth, not stepped at the data rate). */
  function setFollowTarget(lat: number, lon: number, heading: number, pitch: number, roll: number, color: string, marker: L.Marker | undefined, course = heading, speed = 0, timeMs = 0) {
    if (!isValidGpsCoordinate(lat, lon)) return;
    activeFollowMarker = marker;
    activeColor = color;

    // Turn-rate estimate: low-passed dCOG/dt on the source time base. Only trusted while moving; a
    // standstill, a time gap or a scrub resets it so the arc never lags or spins from stale COG.
    if (speed > DIR_COG_MIN_SPEED && prevCog != null && timeMs > 0) {
      const dtS = (timeMs - prevCogTs) / 1000;
      if (dtS > 0.05 && dtS < 4) {
        const dCog = ((course - prevCog + 540) % 360) - 180; // shortest-path COG change
        turnRateDegS += ((dCog / dtS) - turnRateDegS) * 0.25; // low-pass
      } else {
        turnRateDegS = 0; // gap / scrub
      }
    } else {
      turnRateDegS = 0;
    }
    prevCog = speed > DIR_COG_MIN_SPEED ? course : null;
    prevCogTs = timeMs;

    // Arc show/hide hysteresis on the source time base: start only above TURN_SHOW_RATE (a real turn),
    // then stay visible while ≥ TURN_KEEP_RATE and for TURN_HOLD_MS after it drops below.
    const absRate = Math.abs(turnRateDegS);
    if (absRate >= TURN_KEEP_RATE) turnArcLastActiveTs = timeMs;
    if (!turnArcShown && absRate > TURN_SHOW_RATE) turnArcShown = true;
    else if (turnArcShown && timeMs - turnArcLastActiveTs > TURN_HOLD_MS) turnArcShown = false;

    followTarget = { lat, lon, heading, pitch, roll, course, speed, turnRate: turnRateDegS };
    if (!followCurrent) {
      followCurrent = { ...followTarget };
      applyFollowFrame();
    } else if (L.latLng(followCurrent.lat, followCurrent.lon).distanceTo([lat, lon]) > FOLLOW_SNAP_M) {
      followCurrent = { ...followTarget }; // big jump → snap
      turnRateDegS = 0; prevCog = null; turnArcShown = false; // jump → drop the stale turn rate + arc
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
    const dc = ((followTarget.course - followCurrent.course + 540) % 360) - 180; // course, shortest path
    followCurrent.course = (followCurrent.course + dc * k + 360) % 360;
    followCurrent.speed += (followTarget.speed - followCurrent.speed) * k;
    followCurrent.turnRate += (followTarget.turnRate - followCurrent.turnRate) * k; // eased → smooth arc
    applyFollowFrame();
    const dist = L.latLng(followCurrent.lat, followCurrent.lon).distanceTo([followTarget.lat, followTarget.lon]);
    if (dist < 0.5 && Math.abs(dh) < 0.3 && Math.abs(followTarget.pitch - followCurrent.pitch) < 0.3 && Math.abs(dr) < 0.3 && Math.abs(dc) < 0.3 && Math.abs(followTarget.turnRate - followCurrent.turnRate) < 0.3) {
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
    redrawDirLines();
    // Don't fight an in-progress zoom animation (would snap mid-zoom).
    if (viewMode !== 'free' && !(map as unknown as { _animatingZoom?: boolean })._animatingZoom) {
      followDrivingView = true;
      map.setView(ll, map.getZoom(), { animate: false }); // fires moveend synchronously → saveMapState (guarded)
      followDrivingView = false;
      if (viewMode === 'heading-follow') {
        mapHeading = followCurrent.heading;
        mapContainer?.style.setProperty('--map-rotation', `${-mapHeading}deg`);
      }
    }
  }

  /** Redraw the HDG/COG nose lines from the SMOOTHED follow state (so they track the UAV stably, the
   *  same filtering as the marker). Geographic lines → they pan/zoom natively, no per-frame reprojection.
   *  Hidden when there's no active marker; the COG line hides below walking pace (COG is noise there). */
  function redrawDirLines() {
    if (!map) return;
    if (!$settings.directionLines || !activeFollowMarker || !followCurrent) {
      for (const p of [turnCasing, turnLine, hdgCasing, hdgLine, cogCasing, cogLine]) p?.setLatLngs([]);
      return;
    }
    if (!dirLayer) {
      // Dedicated pane above the overlay pane (flight track, z=400) but below the marker pane (UAV
      // model, z=600), so the lines sit over the track yet under the aircraft.
      if (!map.getPane('dirLines')) {
        const pane = map.createPane('dirLines');
        pane.style.zIndex = '450';
        pane.style.pointerEvents = 'none';
      }
      dirLayer = L.layerGroup().addTo(map);
      // Add order = draw order: the turn arc first (bottom, under the H/B lines), then dark casings,
      // then the coloured H/B lines on top. All in the dirLines pane → above the track, below the UAV.
      turnCasing = L.polyline([], { pane: 'dirLines', color: '#000', weight: 3.5, opacity: 0.3, lineCap: 'round', interactive: false }).addTo(dirLayer);
      turnLine = L.polyline([], { pane: 'dirLines', color: '#fff', weight: 1.5, opacity: 0.95, lineCap: 'round', interactive: false }).addTo(dirLayer);
      hdgCasing = L.polyline([], { pane: 'dirLines', color: '#000', weight: 6, opacity: 0.3, lineCap: 'round', interactive: false }).addTo(dirLayer);
      cogCasing = L.polyline([], { pane: 'dirLines', color: '#000', weight: 6, opacity: 0.3, lineCap: 'round', interactive: false }).addTo(dirLayer);
      hdgLine = L.polyline([], { pane: 'dirLines', color: '#37a8db', weight: 3, opacity: 1, lineCap: 'round', interactive: false }).addTo(dirLayer);
      cogLine = L.polyline([], { pane: 'dirLines', color: '#f5a623', weight: 3, opacity: 1, dashArray: '6 5', lineCap: 'round', interactive: false }).addTo(dirLayer);
    }
    const { lat, lon, heading, course, speed } = followCurrent;
    const len = speed * DIR_LEAD_S; // 15 s of travel → the lines represent ground speed
    const h = destinationPoint(lat, lon, heading, len);
    const hll: L.LatLngExpression[] = [[lat, lon], [h.lat, h.lon]];
    hdgCasing!.setLatLngs(hll);
    hdgLine!.setLatLngs(hll);
    if (speed > DIR_COG_MIN_SPEED) {
      const c = destinationPoint(lat, lon, course, len);
      const cll: L.LatLngExpression[] = [[lat, lon], [c.lat, c.lon]];
      cogCasing!.setLatLngs(cll);
      cogLine!.setLatLngs(cll);
    } else {
      cogCasing!.setLatLngs([]);
      cogLine!.setLatLngs([]);
    }

    // Turn-radius arc: predicted ground path starting at the nose along COG, curving at the filtered
    // turn rate. Arc length = 15 s of travel but capped at a 180° sweep (no spiral). Hidden when slow
    // or nearly straight (the COG line already shows that case).
    const rate = followCurrent.turnRate; // eased per-frame → smooth
    const sweep = Math.max(-TURN_MAX_SWEEP, Math.min(TURN_MAX_SWEEP, rate * DIR_LEAD_S)); // signed deg
    if (turnArcShown && speed > DIR_COG_MIN_SPEED && Math.abs(rate) > 0.5 && Math.abs(sweep) > TURN_SEG_DEG) {
      const rAbs = speed / (Math.abs(rate) * Math.PI / 180); // turn radius (m)
      const n = Math.max(2, Math.ceil(Math.abs(sweep) / TURN_SEG_DEG));
      const stepDeg = sweep / n;
      const segDist = rAbs * Math.abs(stepDeg) * Math.PI / 180; // chord ≈ arc per small step
      const pts: L.LatLngExpression[] = [[lat, lon]];
      let cLat = lat, cLon = lon;
      for (let i = 1; i <= n; i++) {
        const b = course + stepDeg * (i - 0.5); // midpoint bearing per segment
        const p = destinationPoint(cLat, cLon, b, segDist);
        cLat = p.lat; cLon = p.lon;
        pts.push([cLat, cLon]);
      }
      turnCasing!.setLatLngs(pts);
      turnLine!.setLatLngs(pts);
    } else {
      turnCasing!.setLatLngs([]);
      turnLine!.setLatLngs([]);
    }
  }

  let followTitle = $derived.by(() => {
    if (viewMode === 'free') return 'Follow mode: Free';
    if (viewMode === 'follow') return 'Follow mode: Follow';
    return 'Follow mode: Heading Follow';
  });

  function updateUavPosition(lat: number, lon: number, heading: number, navState = 0, roll = 0, pitch = 0, course = heading, speed = 0, timeMs = 0) {
    if (!map) return;
    if (!isValidGpsCoordinate(lat, lon)) return;

    const color = getNavStateColor(navState); // marker = nav state (the track shows flight mode) — see COLORED_TRACK_PLAN
    if (!uavMarker) {
      uavMarker = L.marker([lat, lon], { icon: makeModelIcon(modelSizePx(lat)), zIndexOffset: 1000 }).addTo(map);
    }
    setFollowTarget(lat, lon, heading, pitch, roll, color, uavMarker, course, speed, timeMs); // eases + redraws the model
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

  /** The dedicated green "H" marker is shown only for an authoritative FC home (or a replay's start) —
   *  a manual reference is represented by the draggable orange "L" launch marker instead. Driven by the
   *  homePosition store (see homeMarkerShown), not by the arm transition. */
  function renderHomeMarker() {
    if (!map) return;
    const h = get(homePosition);
    if (!get(homeMarkerShown)) {
      if (homeMarker) { try { map.removeLayer(homeMarker); } catch {} homeMarker = undefined; }
      return;
    }
    const pos: L.LatLngExpression = [h.lat, h.lon];
    if (homeMarker) {
      homeMarker.setLatLng(pos);
    } else {
      homeMarker = L.marker(pos, { icon: createHomeIcon(), zIndexOffset: 500 }).addTo(map);
    }
  }

  function updateTrail(lat: number, lon: number, modePrimary: string) {
    if (!map) return;
    const pos = L.latLng(lat, lon);

    // Only add if moved enough from last point
    if (trailCurrentPositions.length > 0 &&
        pos.distanceTo(trailCurrentPositions[trailCurrentPositions.length - 1]) < MIN_TRAIL_DIST) {
      return;
    }

    const color = modeColor(modePrimary);

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
      // Frame the whole track only in free mode — don't yank the view away from
      // an active follow (which centers on the UAV/playback marker).
      if (viewMode === 'free') {
        map.fitBounds(L.latLngBounds(positions), { padding: [36, 36] });
      }
      lastPlaybackTrackKey = nextKey;
    }
  }

  // ── Auto-framing: mission fit + go-to-UAV on connect (free pan only) ─────────
  let pendingUavJump = false; // set on a fresh connect; cleared after the first jump (once per connect)

  /** Lat/lngs of the active mission's location waypoints + home/launch (when set), for fit-bounds. */
  function collectMissionLatLngs(): L.LatLng[] {
    const pts: L.LatLng[] = [];
    if (get(autopilotSystem) === 'inav') {
      for (const wp of get(geoWaypoints)) {
        if (wp.lat !== 0 || wp.lon !== 0) pts.push(L.latLng(toDeg(wp.lat), toDeg(wp.lon)));
      }
      // launchPoint is INAV-only planning state — only fold it in for INAV, never a stale INAV launch
      // into an ArduPilot/PX4 fit (that inflated the bounds to a continent-wide zoom-out).
      const lp = get(launchPoint);
      if (lp) pts.push(L.latLng(lp.lat, lp.lng));
    } else {
      for (const wp of get(arduMission)) {
        if (cmdHasLocation(wp.command) && (wp.lat !== 0 || wp.lon !== 0)) pts.push(L.latLng(wp.lat / 1e7, wp.lon / 1e7));
      }
    }
    // Only the authoritative FC home (source 'fc'). The 'manual' home mirrors the INAV launchPoint
    // (already folded in above for INAV) — including it for ArduPilot dragged a stale INAV-planning
    // launch into the bounds → continent-wide zoom-out.
    const hp = get(homePosition);
    if (hp?.set && hp.source === 'fc') pts.push(L.latLng(hp.lat, hp.lon));
    return pts;
  }

  /** Frame the whole loaded mission once — free pan only, and never over a replay (the replay frames
   *  its own track; a replay-linked mission must not steal the view). */
  function frameMission() {
    if (!map || viewMode !== 'free' || get(replayActive)) return;
    const pts = collectMissionLatLngs();
    if (pts.length === 0) return;
    map.fitBounds(L.latLngBounds(pts), { padding: [40, 40], maxZoom: 16 });
  }

  function updatePlaybackMarker(point: TelemetryRecord | null) {
    if (!map) return;

    if (!point || point.lat == null || point.lon == null || !isValidGpsCoordinate(point.lat, point.lon)) {
      if (activeFollowMarker === playbackMarker) {
        activeFollowMarker = undefined;
        redrawDirLines(); // clears the nose lines (no active marker)
      }
      if (playbackMarker) {
        map.removeLayer(playbackMarker);
        playbackMarker = undefined;
      }
      return;
    }

    const color = getNavStateColor(point.nav_state ?? 0); // marker = nav state
    // Attitude from the same unified adapter the AHI / 3D model use. The icon points along the FC fused
    // HEADING (`td.yaw`), NOT the GPS course (`point.heading` = COG) — so it shows the real crab.
    const td = toTelemetryData(point, fcVariant);
    const heading = td.yaw;
    if (!playbackMarker) {
      playbackMarker = L.marker([point.lat, point.lon], { icon: makeModelIcon(modelSizePx(point.lat)), zIndexOffset: 900 }).addTo(map);
    }
    setFollowTarget(point.lat, point.lon, heading, td.pitch, td.roll, color, playbackMarker, td.course, td.groundSpeed, point.timestamp_ms ?? 0); // eases + redraws
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
    if (!map || followDrivingView) return; // ignore programmatic follow recenters (see followDrivingView)
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
    map.on("contextmenu", onMapContextMenu); // right-click → "Set GCS here" (manual)
    map.on("click", () => { if (gcsSelected) { gcsSelected = false; updateGcsAccuracyCircle(); } }); // deselect GCS

    // GCS marker follows its store; accuracy circle follows the accuracy + selection.
    unsubGcs = gcsLocation.subscribe(() => updateGcsMarker());
    unsubGcsAcc = gcsAccuracyM.subscribe(() => updateGcsAccuracyCircle());
    // Home "H" marker follows the homePosition store (FC home / replay only — see renderHomeMarker).
    unsubHome2d = homePosition.subscribe(() => renderHomeMarker());

    // Airspace overlay: redraw on new data; re-clip points to the view on pan. Layer-visibility changes
    // ride the `$settings.airspace` effect below (visibility now lives in the persisted settings store).
    unsubAero = aeroData.subscribe(() => updateAirspace());
    unsubAeroFocus = aeroFocus.subscribe((f) => { if (f && map) map.setView([f.lat, f.lon], Math.max(map.getZoom(), 11)); });
    map.on("moveend", updateAirspace);
    map.on("click", onAirspaceClick); // click empty map / airspace fill → list all airspaces there

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

    // Restore a persisted follow mode (the parent keeps `viewMode` across 2D↔3D remounts).
    if (viewMode !== 'free') {
      map.dragging.disable();
      setZoomAnchor('center');
      if (viewMode === 'heading-follow') applyHeadingUpSize(true);
      applyFollowFrame();
    }

    // Subscribe to telemetry for UAV position, flight trail, and home detection
    unsubTelemetry = telemetry.subscribe((t) => {
      if (t.lastUpdate > 0) {
        updateUavPosition(t.lat, t.lon, t.yaw, t.navState, t.roll, t.pitch, t.course, t.groundSpeed, t.lastUpdate); // drives the smoother (marker + follow)

        // Go-to-UAV on connect: jump once to the craft at a sensible zoom, deferred to the first 3D fix
        // (no fix ⇒ no UAV rendered). Free pan only; following already centres on the UAV.
        if (pendingUavJump && viewMode === 'free' && !get(replayActive) && t.fixType >= 3 && isValidGpsCoordinate(t.lat, t.lon)) {
          map?.setView([t.lat, t.lon], 16);
          pendingUavJump = false;
        }

        // Flight trail: colored by flight mode while armed; a thin black
        // monitoring line while disarmed (pre-arm).
        const armed = (t.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
        if (isValidGpsCoordinate(t.lat, t.lon)) {
          if (armed) updateTrail(t.lat, t.lon, t.flightMode.primary);
          else updatePreArmTrail(t.lat, t.lon);
        }

        // Trail reset on arm (home is set centrally in +page → the home marker follows homePosition).
        if (armed && !wasArmed && t.fixType >= 2 && t.lat !== 0) {
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

    // Frame the loaded mission on a real load event (signal increments; ignore the initial emission).
    let frameSigInit = true;
    unsubFrameMission = frameMissionSignal.subscribe(() => {
      if (frameSigInit) { frameSigInit = false; return; }
      frameMission();
    });

    // Go-to-UAV: arm a one-shot jump on a fresh connect (not on a 2D↔3D remount while connected).
    let prevConnStatus = get(connection).status;
    unsubConnForJump = connection.subscribe((c) => {
      if (c.status === 'connected' && prevConnStatus !== 'connected') pendingUavJump = true;
      else if (c.status === 'disconnected') pendingUavJump = false;
      prevConnStatus = c.status;
    });

    // React to provider changes from settings
    let currentProviderId = s.mapProvider;
    unsubSettings = settings.subscribe((next) => {
      if (next.mapProvider !== currentProviderId) {
        currentProviderId = next.mapProvider;
        applyProvider(getProviderById(currentProviderId));
      }
    });

    // Foreign-vehicle contacts: rebuild the radar layer on every snapshot / selection change.
    unsubRadar = radarVehicles.subscribe((s) => { radarSnap = s; updateRadar(); });
    unsubRadarSel = radarSelection.subscribe((id) => { radarSelectedId = id; updateRadar(); });
    unsubRadarAlerts = radarAlertLevels.subscribe((m) => { radarAlertMap = m; updateRadar(); });

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
    unsubRadar?.();
    unsubRadarSel?.();
    unsubRadarAlerts?.();
    unsubGcs?.();
    unsubGcsAcc?.();
    unsubHome2d?.();
    unsubAero?.();
    unsubAeroFocus?.();
    if (unsubTelemetry) unsubTelemetry();
    if (unsubSettings) unsubSettings();
    unsubFrameMission?.();
    unsubConnForJump?.();
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
      if (dirLayer) map.removeLayer(dirLayer);
      if (homeMarker) map.removeLayer(homeMarker);
      if (radarLayer) map.removeLayer(radarLayer);
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
    <RfRayLayer {map} />
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

  /* Foreign-vehicle (radar) contact icons */
  :global(.radar-divicon) {
    background: none;
    border: none;
  }
  :global(.radar-divicon .radar-icon) {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  :global(.radar-divicon .radar-icon-label) {
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translate(-50%, 1px);
    font: 600 9px 'Segoe UI', Tahoma, sans-serif;
    color: #fff;
    text-shadow: 0 0 2px #000, 0 0 2px #000, 0 1px 1px #000;
    white-space: nowrap;
    pointer-events: none;
  }
  /* FormationFlight single-letter id: a bigger badge with a translucent grey background. */
  :global(.radar-divicon .radar-icon-label.badge) {
    font-size: 15px;
    font-weight: 700;
    background: rgba(40, 40, 40, 0.55);
    padding: 0 5px;
    border-radius: 4px;
    transform: translate(-50%, 3px);
  }
  /* Conflict-alert ring around a contact icon — pulsing glow (yellow caution / red warning). */
  :global(.radar-divicon .radar-alert-ring) {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 150%;
    height: 150%;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    box-sizing: border-box;
    pointer-events: none;
  }
  :global(.radar-divicon .radar-alert-ring.caution) {
    border: 2px solid #f4c020;
    box-shadow: 0 0 7px 1px #f4c020, inset 0 0 4px #f4c020;
    animation: radar-alert-caution 1s ease-in-out infinite;
  }
  :global(.radar-divicon .radar-alert-ring.warning) {
    border: 3px solid #ff2a2a;
    box-shadow: 0 0 12px 3px #ff2a2a, inset 0 0 7px #ff2a2a;
    animation: radar-alert-warning 1s ease-in-out infinite;
  }
  @keyframes radar-alert-caution {
    0%, 100% { opacity: 0.45; transform: translate(-50%, -50%) scale(0.9); }
    50% { opacity: 1; transform: translate(-50%, -50%) scale(1.1); }
  }
  @keyframes radar-alert-warning {
    0%, 100% { opacity: 0.55; transform: translate(-50%, -50%) scale(0.85); }
    50% { opacity: 1; transform: translate(-50%, -50%) scale(1.15); }
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.radar-divicon .radar-alert-ring) { animation: none !important; }
  }

  /* ── GCS (ground-station) marker ── */
  :global(.gcs-icon) { background: none; border: none; }
  :global(.gcs-icon .gcs-dot) {
    position: relative;
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: rgba(40, 42, 44, 0.62);
    border: 2px solid rgba(55, 168, 219, 0.85);
    box-shadow: 0 0 6px rgba(0, 0, 0, 0.45);
  }
  /* Manual: signal that it's draggable. */
  :global(.gcs-icon .gcs-dot.gcs-manual) {
    cursor: move;
    border-style: dashed;
  }
  /* Continuous: a small green "live" dot, top-right. */
  :global(.gcs-icon .gcs-dot .gcs-live) {
    position: absolute;
    top: -1px;
    right: -1px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #59aa29;
    border: 1px solid #fff;
    box-shadow: 0 0 4px #59aa29;
    animation: gcs-live-pulse 1.4s ease-in-out infinite;
  }
  @keyframes gcs-live-pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.gcs-icon .gcs-dot .gcs-live) { animation: none; }
  }
</style>
