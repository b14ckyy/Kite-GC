<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

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
    import { telemetry, altReference, groundAnchor, resolveTrueMsl } from "$lib/stores/telemetry";
  import { liveTrack, type LiveTrackPoint } from "$lib/stores/liveTrack";
  import { homePosition, homeLocked, type HomePosition } from "$lib/stores/home";
  import { connection, type ConnectionStatus } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import { gcsLocation } from "$lib/stores/gcsLocation";
  import { getProviderById } from "$lib/config/mapProviders";
  import type { MapProvider } from "$lib/config/mapProviders";
  import { getCachedTile, putCachedTile, initTileCache } from "$lib/cache/tileCache";
  import { isKnownUnavailable, isPlaceholderTile } from "$lib/cache/tileAvailability";
  import { isValidGpsCoordinate, MIN_FIX_SATELLITES } from "$lib/helpers/telemetry";
  import {
    segmentTrackByFlightMode,
    segmentTrackByAltitude,
    segmentTrackBySpeed,
    segmentTrackBySignal,
    trackPointColorizer,
    getNavStateColor,
    type TrackColorMode,
    type TrackSegment,
  } from "$lib/helpers/trackColors";
  import { modeColor } from "$lib/helpers/flightModeRegistry";
  import type { TelemetryRecord } from "$lib/stores/flightlog";
  import { toTelemetryData } from "$lib/adapters/telemetryAdapter";
  import type { PlatformType, UavModelOverride } from "$lib/helpers/uavIcons";
  import { PLATFORM_MULTIROTOR } from "$lib/helpers/uavIcons";
  import { flightPathVector } from "$lib/utils/flightPath";
  import { modelUriForPlatform } from "$lib/helpers/uavModels";
  import {
    mission, showMission, replayActive, launchPoint,
    hasLocation, toDeg, WpAction, type Waypoint, type Mission,
  } from "$lib/stores/mission";
  import { wpIconSpec, type WpIconSpec } from "$lib/helpers/missionIcons";
  import {
    arduMission, type ArduWaypoint,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_TERRAIN_ALT, MAV_CMD_DO_JUMP,
  } from "$lib/stores/missionArdupilot";
  import { cmdHasLocation, cmdStandaloneCoordinate, cmdIsTakeoff } from "$lib/helpers/arduCommandCatalog";
  import { arduWpIconSpec } from "$lib/helpers/missionIconsArdupilot";
  import { autopilotSystem, type AutopilotSystem } from "$lib/stores/autopilotContext";
  import { frameMissionSignal } from "$lib/stores/mapCamera";
  import { activeWpNumber } from "$lib/stores/navStatus";
  import {
    buildDisplayNumbers, isFlightPathWp, findMissionEndIndex, findPreviousGeoWp,
  } from "$lib/helpers/missionGeometry";
  import { resolveMissionAltitudes, type WpMsl } from "$lib/helpers/terrainProfile";
  import { sunAltitudeDeg, cesiumLikeBrightness } from "$lib/utils/sun";
  import { ensureUserLocation, resolveUserLocation, userGeoLocation } from "$lib/helpers/userLocation";
  import FpvHud from "$lib/components/FpvHud.svelte";
  import { convertSpeed, convertAltitude, convertDistance, convertVerticalSpeed, formatConverted } from "$lib/utils/units";
  import { haversineDistance, bearing, destinationPoint } from "$lib/utils/geo";
  import type { SpeedUnit, AltitudeUnit, RadarMapSettings, GcsMode } from "$lib/stores/settings";
  import { radarVehicles, radarSelection, type RadarSnapshot } from "$lib/stores/radarTracking";
  import { aeroData, type Airspace, type AltLimit } from "$lib/stores/airspace";
  import { airspaceStyle, airspaceContainsPoint, airspaceIsRelevant, isAirspaceHidden, airportBillboard } from "$lib/helpers/airspaceStyle";
  import { contactColor, ffContactColor, contactVisibleOnMap, REL_OVERRIDE_M } from "$lib/helpers/radarMap";
  import { ARROW_POLY, contactModelClass, radarModelUri, type RadarModelClass } from "$lib/helpers/radar3d";
  import { radarAlertLevels, ALERT_CONFIG, type AlertLevel } from "$lib/controllers/radarAlerts";
  import { safehomeWorking, isSafehomeEmpty } from "$lib/stores/safehome";
  import { geozoneWorking, geozoneMissionResult, GEOZONE_SHAPE_CIRCULAR } from "$lib/stores/geozone";
  import { geozonePathStyle, geozoneRadiusM } from "$lib/helpers/geozoneStyle";
  import { fenceWorking, FENCE_SHAPE_CIRCLE } from "$lib/stores/fence";
  import { fencePathStyle, fenceRadiusM } from "$lib/helpers/fenceStyle";
  import { rallyWorking } from "$lib/stores/rally";
  import { t } from "svelte-i18n";
  import { buildApproachGeometry } from "$lib/helpers/autolandGeometry";

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
  let missionEntities: Cesium.Entity[] = [];         // billboards (markers, launch, active glow)
  let missionPrimitives: Cesium.Primitive[] = [];    // overlay lines — depth-test-free primitives (no z-fight)
  let missionRenderToken = 0;                        // guards async terrain races
  let lastMissionSig = '';                           // signature of the drawn model → skip identical redraws
  // Cache for the expensive terrain-altitude resolution, keyed by a signature of the inputs that affect
  // it (waypoint positions/alt-frames + launch/home). A 3D re-open or any redundant render trigger then
  // reuses the resolved altitudes instead of re-sampling terrain (the 15–20 s cost), while cheap visual
  // changes (active-WP highlight, greying, geoid) still rebuild + redraw the model each render.
  let inavAltCache: { sig: string; alts: Map<number, WpMsl>; launchGround: number | null } | null = null;
  let arduAltCache: { sig: string; alts: Map<number, ArduWpAlt>; homeRefMsl: number | null } | null = null;
  let curMission: Mission = get(mission);
  let curShowMission = get(showMission);
  let curReplayActive = get(replayActive);
  let curLaunch = get(launchPoint);
  let curActiveWp3d = get(activeWpNumber); // FC's active target WP (0 = none) → pulsing green glow
  let unsubActiveWp3d: (() => void) | undefined;
  let wpPulseActive = false; // an active-WP glow is on screen → keep rendering continuously
  let playbackMarkerEntity: Cesium.Entity | undefined;
  let unsubTelemetry: (() => void) | undefined;
  let unsubHome: (() => void) | undefined;
  let unsubSettingsWatch: (() => void) | undefined;
  let unsubMissionStore: (() => void) | undefined;
  let unsubShowMissionStore: (() => void) | undefined;
  let unsubReplayStore: (() => void) | undefined;
  let unsubLaunchStore: (() => void) | undefined;
  let unsubLiveTrack: (() => void) | undefined;

  // ArduPilot mission overlay — separate model/store from INAV, but reuses the shared icon specs
  // (arduWpIconSpec) and the same billboard/line/glow/drop-line primitives. The active autopilot
  // system picks which mission renders (3D mirror of the 2D MissionLayer switcher).
  let curArduMission: ArduWaypoint[] = get(arduMission);
  let curAutopilotSystem: AutopilotSystem = get(autopilotSystem);
  let curHome3d: HomePosition = get(homePosition);
  // Last home we actually acted on. ArduPilot re-sends HOME_POSITION ~0.2 Hz and its reported home
  // jitters sub-metre (EKF origin), so we re-render only when home *meaningfully* moved — otherwise the
  // 5 s rebuild drops a frame on the mission polylines (flicker).
  let lastRenderedHome3d: { set: boolean; lat: number; lon: number; alt: number; source: string } | null = null;
  let unsubArduMission: (() => void) | undefined;
  let unsubAutopilot: (() => void) | undefined;

  // Live trail — driven entirely by the `liveTrack` store (the full flown history since arm), so the
  // whole track renders in 3D regardless of when 3D was first opened, and at the correct height once
  // the geoid offset is known. Flightmode-coloured segments: finalized runs are static polylines; the
  // growing run uses a CallbackProperty (Cesium reads the array each frame → no entity churn / flicker).
  let lastTrailLat = 0;
  let lastTrailLon = 0;
  let trailSegments3D: { entity: Cesium.Entity; color: string }[] = [];
  let activeTrailEntity: Cesium.Entity | undefined;
  let activeTrailPositions: Cesium.Cartesian3[] = [];
  let trailCurrentColor3D = '';
  let trailConsumed = 0; // how many liveTrack points are already in the trail (incremental append cursor)
  // Newest point held back one segment: the UAV marker is smoothed (lags the raw fix), so drawing the
  // trail to the latest point shoots the coloured line ahead of the craft — very visible in FPV. We
  // commit a point only once the NEXT one arrives, so the drawn tip always trails the live position.
  let pendingTrailPos: Cesium.Cartesian3 | undefined;
  let pendingTrailColor = '';
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
  let hud = $state({ heading: 0, pitch: 0, roll: 0, altM: 0, speedMs: 0, fpmGamma: 0, fpmCrab: 0, fpmShown: false });
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
  // Go-to-UAV on connect: armed on a fresh connect, fired once on the first 3D fix (free look only).
  let pendingUavJump3d = false;
  let unsubConnection: (() => void) | undefined;
  let unsubFrameMission3d: (() => void) | undefined;

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
    modelSig?: string;         // last-synced model signature — skip the model update if unchanged
    entities: Cesium.Entity[];
    bundleClass?: RadarModelClass; // model class of the currently-assigned bundle (for pool return)
  };
  // A reusable 5-entity bundle (model + ground geometry). Creating a Cesium `Model` per contact is the
  // expensive part (per-instance node graph + shader pipeline → main-thread "Scripting" stall), so we
  // POOL bundles instead of destroying/recreating them as contacts enter/leave the view: a contact that
  // leaves hides its bundle and returns it; an arriving contact reuses a free one (just re-positioned/
  // -coloured). The model glb is class-specific, so free bundles are keyed by model class (reusing across
  // classes would need a uri swap → re-pays the setup we're avoiding).
  type Radar3dBundle = { entities: Cesium.Entity[]; dropColorCP: Cesium.ConstantProperty; dropColor: Cesium.Color; modelClass: RadarModelClass };
  const radar3dRecs = new Map<string, Radar3dRec>();
  const radar3dFree = new Map<RadarModelClass, Radar3dBundle[]>();
  // When no free bundle of the needed class exists, the rec is queued here and `drainRadarCreateQueue`
  // builds new bundles a few per frame (a dense first load can need ~150 at once — building all their
  // models in one frame stutters). The rec is in `radar3dRecs` immediately (with no entities yet).
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

  // ── Airspace Manager: obstacle columns (3D) ──────────────────────────
  // Static hazards (masts, turbines, towers) as slim vertical columns. OpenAIP gives height (AGL) but
  // no diameter → a fixed slim footprint, real-world-sized so it scales perspectively with distance
  // (not a fixed-size sprite). Terrain-relative extrusion keeps the column on correct AGL height.
  const obstacle3dEntities: Cesium.Entity[] = [];
  let unsubAero3d: (() => void) | undefined;
  let uavLatLon: { lat: number; lon: number } | undefined; // last good UAV fix → fallback reference
  let airspaceEnabled = false;     // tracks the master toggle so the settings-watch can add/remove columns
  let obstacleD3 = false;          // tracks the obstacle 3D toggle (settings-watch rebuild trigger)
  let obstacleDistKm = 5;          // tracks the obstacle range (settings-watch rebuild trigger)
  let obstacle3dGen = 0;           // bumped per rebuild so an in-flight terrain sample can't apply a stale set
  let aeroRefGround: { lat: number; lon: number } | undefined; // camera ground of the last build
  let obstacleMoveTimer: ReturnType<typeof setTimeout> | undefined; // debounce camera-move rebuilds

  // ── Airspace Manager: airspace volumes (3D) ──────────────────────────
  // Extruded polygons (floor → ceiling) for the airspaces relevant to the reference (inside / ≤5 km
  // laterally) — the relevance filter keeps clutter + GPU cost down. Altitudes: MSL/FL → value + the
  // app's geoid offset (locally correct since we only draw nearby airspaces); GND → terrain.
  const airspace3dPrimitives: Cesium.Primitive[] = [];
  let airspaceD3 = false;          // tracks the airspace 3D toggle (settings-watch rebuild trigger)
  let airspace3dGen = 0;           // race guard for the async terrain sample
  const AIRSPACE_3D_LATERAL_M = 5000;   // render airspaces with a boundary within this lateral distance
  const AIRSPACE_3D_MAX_EXTENT_KM = 80; // "inside" only renders airspaces up to this size (skip CTAs/upper air)
  const AIRSPACE_3D_MAX = 60;           // cap rendered volumes

  // ── Airspace Manager: airports (3D) ──────────────────────────────────
  // Real runways (OpenAIP carries heading + length/width) drawn as terrain-draped rectangles, oriented
  // by trueHeading and centred on the airport reference point + a type-coloured marker/label. Airports
  // without runways (heliports / small fields) get the marker only.
  const airport3dEntities: Cesium.Entity[] = [];
  let airportD3 = false;   // tracks the airport 3D toggle (settings-watch rebuild trigger)
  let airportDistKm = 15;  // tracks the airfield range (settings-watch rebuild trigger)
  const OBSTACLE_3D_RADIUS_M = 8;   // slim footprint (no diameter from the API) — tunable
  // OpenAIP often omits the AGL height (and offers no derivable top). When missing we render a typed
  // *estimated* column — tall for identified wind turbines, modest otherwise — drawn visibly distinct
  // (translucent + yellow outline) so an estimated height never masquerades as surveyed data.
  const OBSTACLE_3D_TURBINE_H = 120; // estimated height for a height-less wind turbine
  const OBSTACLE_3D_DEFAULT_H = 40;  // estimated height for a height-less generic obstacle
  const OBSTACLE_3D_MAX = 1200;      // cap rendered columns (dense regions → nearest-N to the reference)

  // ── Safehome + fixed-wing autoland overlay (INAV) ────────────────────
  // Mirrors the 2D overlay (Map.svelte::updateSafehome): teardrop "H" markers + green max-distance ring
  // (disarmed-only) + yellow loiter ring + the planned approach path. Source is `safehomeWorking` (so
  // panel/drag edits reflect live). The approach path is drawn at its real 3D descent altitude (the
  // loiter ring sits at the downwind/approach altitude), so a terrain sample per safehome is needed.
  let safehomeEntities: Cesium.Entity[] = [];        // markers + rings + approach legs (all Entities)
  let unsubSafehome3d: (() => void) | undefined;
  let lastSafehomeArmed3d = false;                    // green ring is disarmed-only → redraw on arm change
  let safehome3dGen = 0;                              // race guard for the async terrain sample
  // Geozone overlay (INAV ≥8.0 FC config) — extruded volumes (circle → cylinder, polygon → hull).
  let geozone3dEntities: Cesium.Entity[] = [];
  let unsubGeozone3d: (() => void) | undefined;
  let geozone3dViolationEntities: Cesium.Entity[] = [];
  let unsubGeozoneViol3d: (() => void) | undefined;
  let geozone3dGen = 0;                               // race guard for the async terrain sample
  let geozoneD3 = true;                               // last-seen geozones 3D toggle (default on)
  // Geofence overlay (ArduPilot/PX4 MAVLink FC config) — extruded volumes to the global ALT_MAX.
  let fence3dEntities: Cesium.Entity[] = [];
  let unsubFence3d: (() => void) | undefined;
  let fence3dGen = 0;                                 // race guard for the async terrain sample
  let fenceD3 = true;                                 // last-seen fence 3D toggle (default on)
  // Rally points overlay (ArduPilot/PX4 RTL divert points) — ground-clamped labelled markers.
  let rally3dEntities: Cesium.Entity[] = [];
  let unsubRally3d: (() => void) | undefined;
  let rallyD3 = true;                                 // last-seen rally 3D toggle (default on)

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

  /** Frame the whole loaded mission once (free look only, not over a replay). BoundingSphere from the
   *  active mission's location WPs + home/launch; range 0 lets Cesium fit the sphere to the viewport. */
  function frameMission3d() {
    if (!viewer || !isFreeLook() || get(replayActive)) return;
    // Read the stores FRESH (not the cached cur* mirrors) so a clear-and-switch can't frame against a
    // stale system/mission/home — this must behave identically to the 2D Map.collectMissionLatLngs.
    const carts: Cesium.Cartesian3[] = [];
    if (get(autopilotSystem) === 'inav') {
      for (const wp of get(mission).waypoints) {
        if (hasLocation(wp.action) && (wp.lat !== 0 || wp.lon !== 0)) carts.push(Cesium.Cartesian3.fromDegrees(toDeg(wp.lon), toDeg(wp.lat)));
      }
      // launchPoint is INAV-only planning state — never mix a stale INAV launch into an ArduPilot fit.
      const lp = get(launchPoint);
      if (lp) carts.push(Cesium.Cartesian3.fromDegrees(lp.lng, lp.lat));
    } else {
      for (const wp of get(arduMission)) {
        if (cmdHasLocation(wp.command) && (wp.lat !== 0 || wp.lon !== 0)) carts.push(Cesium.Cartesian3.fromDegrees(wp.lon / 1e7, wp.lat / 1e7));
      }
    }
    // Only the authoritative FC home (source 'fc'); the 'manual' home mirrors the INAV launchPoint
    // (handled above for INAV) and would drag a stale INAV-planning launch into an ArduPilot fit.
    const hp = get(homePosition);
    if (hp.set && hp.source === 'fc') carts.push(Cesium.Cartesian3.fromDegrees(hp.lon, hp.lat));
    if (carts.length === 0) return;
    pendingUavJump3d = false; // a mission load is the latest positioning intent
    const sphere = Cesium.BoundingSphere.fromPoints(carts);
    const range = sphere.radius > 1 ? 0 : 600; // 0 = fit the whole sphere to the viewport; 600 m for a single WP
    viewer.camera.flyToBoundingSphere(sphere, {
      duration: 1.2,
      offset: new Cesium.HeadingPitchRange(0, Cesium.Math.toRadians(-45), range),
    });
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
  /** Apply a Cesium Ion token entered after the viewer was created (no token at init = no world
   *  terrain). Sets the global token and swaps world terrain in live, so the 3D view gains real
   *  terrain without an app restart. */
  export function applyIonToken(token: string) {
    const t = token.trim();
    if (!t || !viewer) return;
    Cesium.Ion.defaultAccessToken = t;
    try {
      viewer.scene.setTerrain(Cesium.Terrain.fromWorldTerrain({ requestVertexNormals: true }));
      viewer.scene.requestRender();
    } catch (e) {
      console.warn('[Map3D] applyIonToken: failed to enable world terrain', e);
    }
  }

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

  /** Ground-projected camera geometry for the free-look ADS-B query (see +page `radarQueryView`):
   *  the nadir subpoint, the screen-centre ground hit (`null` when looking above the horizon), and
   *  the camera heading (deg). All over-ground — the query is a circle on the surface. */
  export function getCamGeo(): { sub: { lat: number; lon: number }; focus: { lat: number; lon: number } | null; headingDeg: number } | null {
    if (!viewer) return null;
    const c = viewer.camera.positionCartographic;
    const f = getCamFocus();
    return {
      sub: { lat: Cesium.Math.toDegrees(c.latitude), lon: Cesium.Math.toDegrees(c.longitude) },
      focus: f ? { lat: f.lat, lon: f.lon } : null,
      headingDeg: Cesium.Math.toDegrees(viewer.camera.heading),
    };
  }

  /** True when the 3D camera is in free-look (not locked to the UAV in follow/orbit/fpv). */
  export function isFreeLook(): boolean {
    return cameraMode === 'free';
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
        updateObstacles3D();  // (re)build obstacle columns for the now-visible 3D view
        updateAirspaces3D();  // (re)build airspace volumes for the now-visible 3D view
        updateAirports3D();   // (re)build airports/runways for the now-visible 3D view
        updateGeozones3D();   // (re)build geozone volumes for the now-visible 3D view
        updateGeozoneViolations3D(); // (re)build the red mission-violation overlay
        updateFence3D();      // (re)build geofence volumes for the now-visible 3D view
        updateRally3D();      // (re)build rally markers for the now-visible 3D view
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

    // ── Camera input model ──────────────────────────────────────────────
    // Default Cesium binds TILT to the middle button (+ Ctrl+Left) and ZOOM to the right button —
    // awkward on touchpads / touchscreens that have no middle button. Remap to a middle-button-free
    // scheme: LEFT = rotate, RIGHT-drag = tilt, WHEEL/PINCH = zoom (middle + Ctrl+Left kept as
    // extras for mouse users; PINCH stays on zoom+tilt for native touch gestures). Set once here;
    // it persists across camera modes (follow/fpv just toggle the enable* flags on top).
    const ssc = viewer.scene.screenSpaceCameraController;
    ssc.rotateEventTypes = [Cesium.CameraEventType.LEFT_DRAG];
    ssc.tiltEventTypes = [
      Cesium.CameraEventType.RIGHT_DRAG,
      Cesium.CameraEventType.MIDDLE_DRAG,
      Cesium.CameraEventType.PINCH,
      { eventType: Cesium.CameraEventType.LEFT_DRAG, modifier: Cesium.KeyboardEventModifier.CTRL },
    ];
    ssc.zoomEventTypes = [Cesium.CameraEventType.WHEEL, Cesium.CameraEventType.PINCH];
    // Cap zoom-out so the camera can't drift into the full-globe / "space" regime, where Cesium's
    // control behaviour changes and widgets can cover the globe. ~8000 km stays near-surface and
    // consistent while still allowing a generous wide view.
    ssc.maximumZoomDistance = 8_000_000;

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
    // Obstacle/airspace range windows follow the camera: re-cull (debounced) once the ground focus moves.
    viewer.camera.moveEnd.addEventListener(() => {
      if (!active || !airspaceEnabled || (!obstacleD3 && !airspaceD3 && !airportD3)) return;
      if (obstacleMoveTimer) clearTimeout(obstacleMoveTimer);
      obstacleMoveTimer = setTimeout(() => {
        const f = getCamFocus();
        if (!f) return;
        const moved = aeroRefGround
          ? haversineDistance(aeroRefGround.lat, aeroRefGround.lon, f.lat, f.lon)
          : Infinity;
        if (moved > 500) { // only when the focus shifted > 500 m
          if (obstacleD3) updateObstacles3D();
          if (airspaceD3) updateAirspaces3D();
          if (airportD3) updateAirports3D();
        }
      }, 400);
    });

        // Subscribe to live telemetry
    unsubTelemetry = telemetry.subscribe((telem) => {
      if (!viewer) return;

      const armed = (telem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;

      // Safehome green (max-distance) ring is disarmed-only → redraw on arm-state change (mirror 2D).
      if (armed !== lastSafehomeArmed3d) { lastSafehomeArmed3d = armed; updateSafehome3D(); }

      // Decide clear-on-connect from the first telemetry frame after a connect:
      // only wipe the map if the UAV is DISARMED. If it's armed we assume a
      // connection recovery and keep the existing track.
      if (pendingConnectArmCheck) {
        pendingConnectArmCheck = false;
        if (!armed) clearAllMapData();
      }

      if (!isValidGpsCoordinate(telem.lat, telem.lon)) return;
      uavLatLon = { lat: telem.lat, lon: telem.lon }; // reference for nearest-N obstacle culling

      // While a replay log is shown, ignore live telemetry for the map — the
      // replay track/marker owns it (prevents the live UAV lingering over replay).
      if (curReplayActive) { wasArmed = armed; return; }

      // Derive the geoid undulation for the live location once per session.
      ensureGeoid(telem.lat, telem.lon);

      // Use MSL altitude + geoid offset for correct ellipsoid height. Relative-only protocols (LTM/CRSF)
      // are anchored to the ground MSL captured at arm; with no anchor we fall back to the raw value
      // (unchanged legacy behaviour) rather than dropping the UAV.
      const altMsl = resolveTrueMsl(telem.altMsl, get(altReference), get(groundAnchor)) ?? telem.altMsl ?? telem.altitude;
      const alt = Math.max(altMsl + geoidOffset, 0);
      updateUavPosition3D(telem.lat, telem.lon, alt, telem.yaw, telem.navState, armed, telem.roll, telem.pitch);

      // Go-to-UAV on connect: jump once to the craft (range ~600 m ≈ 2D zoom 16), deferred to the first
      // 3D fix. Free look only; follow/orbit/fpv already track the UAV.
      if (pendingUavJump3d && isFreeLook() && telem.fixType >= 3 && telem.numSat >= MIN_FIX_SATELLITES) {
        pendingUavJump3d = false;
        viewer.camera.flyToBoundingSphere(
          new Cesium.BoundingSphere(Cesium.Cartesian3.fromDegrees(telem.lon, telem.lat, alt), 1),
          { duration: 1.2, offset: new Cesium.HeadingPitchRange(0, Cesium.Math.toRadians(-45), 600) },
        );
      }

      // FPV HUD data (live source).
      hud.heading = telem.yaw; hud.pitch = telem.pitch; hud.roll = telem.roll;
      hud.altM = telem.altitude; hud.speedMs = telem.groundSpeed;
      {
        const fv = flightPathVector(telem.groundSpeed, telem.vario, telem.course, telem.yaw);
        hud.fpmGamma = fv.gamma; hud.fpmCrab = fv.crab; hud.fpmShown = fv.shown;
      }
      if (!armed) updatePreArmTrail3D(telem.lat, telem.lon);
      // Live: recenter once after the UAV exists (every 2D→3D switch remounts us).
      if (needsInitialRecenter && uavEntity) { needsInitialRecenter = false; recenter3D(); }
      // Follow/orbit camera is driven from the smoothed state inside the motion smoother.

      // On arm: drop the pre-arm black line. The coloured flight trail resets itself via
      // clearLiveTrack() (+page) → the liveTrack subscription rebuilds it from the (now empty) store.
      if (armed && !wasArmed && telem.fixType >= 2 && telem.lat !== 0) {
        resetPreArmTrail3D();
      }
      wasArmed = armed;

      viewer.scene.requestRender();
    });

    // Live 3D trail — driven by the liveTrack store (full flown history since arm), so the whole
    // track shows in 3D no matter when 3D was first opened. We wait for the geoid offset (ensureGeoid
    // triggers the first rebuild) so points aren't placed sunk into the ground; afterwards we append
    // only the new tail (cheap, no churn). A shrink (clearLiveTrack on re-arm) triggers a full rebuild.
    unsubLiveTrack = liveTrack.subscribe((pts) => {
      if (!viewer || !geoidComputed) return; // geoid not ready yet → ensureGeoid will rebuild
      if (pts.length < trailConsumed) { rebuildLiveTrail3D(); return; }
      if (pts.length === trailConsumed) return;
      for (let i = trailConsumed; i < pts.length; i++) appendTrailPoint3D(pts[i]);
      trailConsumed = pts.length;
      viewer.scene.requestRender();
    });

    // Subscribe to home position
    // Green "H" home entity: shown only for an authoritative FC home (or a replay's start) — a manual
    // reference is the orange "L" launch billboard instead. scheduleMissionRender() refreshes that
    // billboard's visibility when the lock state flips.
    unsubHome = homePosition.subscribe((home) => {
      curHome3d = home; // ArduPilot REL/TERRAIN altitude reference + takeoff anchor
      if (!viewer) return;
      // ArduPilot re-sends HOME_POSITION ~0.2 Hz and its reported home jitters sub-metre. Acting on each
      // tick (move the marker, rebuild the mission overlay, request a frame) makes the depth-tested
      // mission polylines drop a frame on re-add → a 5 s flicker. Act only on a meaningful change.
      const lh = lastRenderedHome3d;
      const changed = !lh
        || lh.set !== home.set || lh.source !== home.source
        || Math.abs(home.lat - lh.lat) > 1e-5   // ≈ 1.1 m
        || Math.abs(home.lon - lh.lon) > 1e-5
        || Math.abs(home.alt - lh.alt) > 1.0;    // 1 m
      if (!changed) return;
      lastRenderedHome3d = { set: home.set, lat: home.lat, lon: home.lon, alt: home.alt, source: home.source };
      if (home.set && (home.source === 'fc' || home.source === 'replay')) {
        updateHomePosition3D(home.lat, home.lon, home.alt);
      } else if (homeEntity) {
        viewer.entities.remove(homeEntity); homeEntity = undefined;
      }
      scheduleMissionRender();
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
      const aspEnabledChg = next.airspace.enabled !== airspaceEnabled;
      if (aspEnabledChg || next.airspace.layers.obstacles.d3 !== obstacleD3 || next.airspace.obstacleDistanceKm !== obstacleDistKm) {
        airspaceEnabled = next.airspace.enabled;
        obstacleD3 = next.airspace.layers.obstacles.d3;
        obstacleDistKm = next.airspace.obstacleDistanceKm;
        updateObstacles3D(); // master toggle / 3D-visibility / range change → rebuild the columns
      }
      if (aspEnabledChg || next.airspace.layers.airspaces.d3 !== airspaceD3) {
        airspaceEnabled = next.airspace.enabled;
        airspaceD3 = next.airspace.layers.airspaces.d3;
        updateAirspaces3D(); // master toggle / airspace 3D-visibility change → rebuild the volumes
      }
      if (aspEnabledChg || next.airspace.layers.airports.d3 !== airportD3 || next.airspace.airfieldDistanceKm !== airportDistKm) {
        airspaceEnabled = next.airspace.enabled;
        airportD3 = next.airspace.layers.airports.d3;
        airportDistKm = next.airspace.airfieldDistanceKm;
        updateAirports3D(); // master toggle / airport 3D-visibility / range change → rebuild airports
      }
      // Geozones are FC config, independent of the OpenAIP subsystem master toggle → watch only their own toggle.
      if (next.airspace.layers.geozones.d3 !== geozoneD3) {
        geozoneD3 = next.airspace.layers.geozones.d3;
        updateGeozones3D();
      }
      // Geofence — same: FC config, watch only its own toggle.
      if (next.airspace.layers.fence.d3 !== fenceD3) {
        fenceD3 = next.airspace.layers.fence.d3;
        updateFence3D();
      }
      // Rally points — same: FC config, watch only its own toggle.
      if (next.airspace.layers.rally.d3 !== rallyD3) {
        rallyD3 = next.airspace.layers.rally.d3;
        updateRally3D();
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
    // Active target WP (live MSP_NAV_STATUS / replay) → re-render so its pulsing glow follows the FC.
    unsubActiveWp3d = activeWpNumber.subscribe((v) => { curActiveWp3d = v; scheduleMissionRender(); });
    // ArduPilot mission overlay + autopilot-system switch (3D mirror of the 2D MissionLayer switcher).
    unsubArduMission = arduMission.subscribe((m) => { curArduMission = m; scheduleMissionRender(); });
    unsubAutopilot = autopilotSystem.subscribe((s) => { curAutopilotSystem = s; scheduleMissionRender(); });

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

    // Safehome + autoland overlay follows the working copy (load / panel edit / drag / "+"). Arm-state
    // changes (the green max-distance ring is disarmed-only) ride the telemetry handler below.
    unsubSafehome3d = safehomeWorking.subscribe(() => updateSafehome3D());
    // On first 3D open the world-terrain provider isn't swapped in yet (still Ellipsoid), so the initial
    // draw samples ground = 0 and the approach path renders flat. Redraw once terrain is ready so it gets
    // its real 3D height — without needing a parameter change to re-trigger the render.
    void waitForTerrain(viewer).then((tp) => { if (tp) updateSafehome3D(); });

    // Geozone volumes follow the working copy (downloaded at handshake; reflects live edits).
    unsubGeozone3d = geozoneWorking.subscribe(() => updateGeozones3D());
    void waitForTerrain(viewer).then((tp) => { if (tp) updateGeozones3D(); });
    // Red mission-violation overlay follows the safety-check result.
    unsubGeozoneViol3d = geozoneMissionResult.subscribe(() => updateGeozoneViolations3D());

    // Geofence volumes follow the working copy (downloaded at handshake; reflects live edits).
    unsubFence3d = fenceWorking.subscribe(() => updateFence3D());
    void waitForTerrain(viewer).then((tp) => { if (tp) updateFence3D(); });
    // Rally markers follow the working copy.
    unsubRally3d = rallyWorking.subscribe(() => updateRally3D());

    // Obstacle columns + airspace volumes: rebuild on new aero data (toggle / range changes ride the
    // settings-watch below; camera-pan re-culls via the debounced moveEnd handler — the window follows
    // the camera).
    unsubAero3d = aeroData.subscribe(() => { updateObstacles3D(); updateAirspaces3D(); updateAirports3D(); });

    // Connection edge: on a fresh (re)connect, flag the next telemetry frame to
    // decide clearing (only if DISARMED) and force a live-geoid recompute.
    unsubConnection = connection.subscribe((c) => {
      const was = prevConnStatus;
      prevConnStatus = c.status;
      if (c.status === 'connected' && was !== 'connected') {
        pendingConnectArmCheck = true;
        pendingUavJump3d = true; // jump to the UAV on the first 3D fix after this connect
        geoidGen++; geoidPromise = null;
        geoidComputed = false;
      }
      if (c.status === 'disconnected') pendingUavJump3d = false;
      if (c.status !== was) scheduleMissionRender(); // launch "L" visibility depends on the connection
    });

    // Frame the mission on a real load event (signal increments; ignore the initial emission).
    let frameSigInit3d = true;
    unsubFrameMission3d = frameMissionSignal.subscribe(() => {
      if (frameSigInit3d) { frameSigInit3d = false; return; }
      frameMission3d();
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
    unsubSafehome3d?.();
    unsubGeozone3d?.();
    unsubGeozoneViol3d?.();
    unsubFence3d?.();
    unsubRally3d?.();
    unsubAero3d?.();
    if (obstacleMoveTimer) clearTimeout(obstacleMoveTimer);
    radar3dPickHandler?.destroy();
    unsubHome?.();
    unsubSettingsWatch?.();
    unsubMissionStore?.();
    unsubShowMissionStore?.();
    unsubReplayStore?.();
    unsubLaunchStore?.();
    unsubActiveWp3d?.();
    unsubArduMission?.();
    unsubAutopilot?.();
    unsubLiveTrack?.();
    unsubConnection?.();
    unsubFrameMission3d?.();
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
    if (!viewer) return;
    for (const rec of radar3dRecs.values()) for (const e of rec.entities) viewer.entities.remove(e);
    radar3dRecs.clear();
    // Destroy the pooled (hidden) bundles too — a clear means the scene is going away.
    for (const list of radar3dFree.values()) for (const b of list) for (const e of b.entities) viewer.entities.remove(e);
    radar3dFree.clear();
    viewer.scene.requestRenderMode = true; // no contacts → back to on-demand rendering
    viewer.scene.requestRender();
  }

  /** Create new bundles a few per frame (reuse is free) so a dense first load doesn't stutter. */
  function drainRadarCreateQueue() {
    radar3dCreateRaf = 0;
    if (!viewer) { radar3dCreateQueue.length = 0; return; }
    const BATCH = 8; // new-bundle CREATIONS per frame (the expensive part); pool reuse doesn't count
    let created = 0;
    while (radar3dCreateQueue.length && created < BATCH) {
      const rec = radar3dCreateQueue.shift()!;
      if (radar3dRecs.get(rec.id) !== rec || rec.entities.length) continue; // gone, or already got a bundle
      if (!acquireBundleFor(rec)) { assignBundle(rec, createBundle(rec.modelClass)); created++; }
    }
    viewer.scene.requestRender();
    if (radar3dCreateQueue.length) radar3dCreateRaf = requestAnimationFrame(drainRadarCreateQueue);
  }

  // Contacts render like the UAV: a real glb MODEL (oriented to heading, altitude-tinted, minimumPixelSize
  // for a screen-size floor) — no flicker and the heading reads from the 3D shape. The ground projection
  // is a filled CLAMP_TO_GROUND ellipse + a filled heading arrow (drop-line is a polyline).
  const RADAR_MODEL_MIN_PX = 48;
  const DROP_DEPTH_M = 12000; // drop-line length below the contact (terrain depth test clips it at ground)

  /** Build a fresh, reusable 5-entity bundle for the given model class (the expensive part — one Cesium
   *  `Model` instance). The contact-specific values are filled in later by syncRec via `assignBundle`. */
  function createBundle(modelClass: RadarModelClass): Radar3dBundle {
    const model = viewer!.entities.add({
      model: {
        uri: radarModelUri(modelClass),
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
    const dropColor = Cesium.Color.WHITE.withAlpha(0.95);
    const dropColorCP = new Cesium.ConstantProperty(dropColor);
    const dropBg = viewer!.entities.add({
      polyline: { width: 4, material: new Cesium.PolylineDashMaterialProperty({ color: Cesium.Color.BLACK.withAlpha(0.7), dashLength: 16 }) },
    });
    const drop = viewer!.entities.add({
      polyline: { width: 2, material: new Cesium.PolylineDashMaterialProperty({ color: dropColorCP, dashLength: 16 }) },
    });
    const circle = viewer!.entities.add({
      ellipse: {
        semiMajorAxis: CIRCLE_RADIUS_M, semiMinorAxis: CIRCLE_RADIUS_M,
        heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
        classificationType: Cesium.ClassificationType.TERRAIN,
        outlineColor: RADAR_CYAN, outlineWidth: 2,
      },
    });
    // Arrow as a clampToGround POLYLINE (not a polygon): polylines update smoothly.
    const arrow = viewer!.entities.add({ polyline: { width: 4, clampToGround: true } });
    return { entities: [model, dropBg, drop, circle, arrow], dropColorCP, dropColor, modelClass };
  }

  /** Attach a bundle to a contact: re-point picking, force a full re-sync, show, and sync. */
  function assignBundle(rec: Radar3dRec, bundle: Radar3dBundle) {
    rec.entities = bundle.entities;
    rec.dropColorCP = bundle.dropColorCP;
    rec.dropColor = bundle.dropColor;
    rec.bundleClass = bundle.modelClass;
    rec.groundSig = undefined; // new contact → force a full ground + model re-sync
    rec.modelSig = undefined;
    for (const e of rec.entities) radar3dEntityIds.set(e, rec.id); // re-point click/hover picking
    rec.entities[0].show = true; // model always visible while active (ground entities gated in syncRadarGround)
    syncRec(rec);
  }

  /** Reuse a free bundle of the contact's model class, if one is available. */
  function acquireBundleFor(rec: Radar3dRec): boolean {
    const b = radar3dFree.get(rec.modelClass)?.pop();
    if (!b) return false;
    assignBundle(rec, b);
    return true;
  }

  /** Contact left the view: hide its bundle and return it to the pool (keep in RAM for reuse). */
  function releaseBundle(rec: Radar3dRec) {
    if (!rec.entities.length) return;
    for (const e of rec.entities) e.show = false;
    const cls = rec.bundleClass ?? rec.modelClass;
    const list = radar3dFree.get(cls) ?? [];
    list.push({ entities: rec.entities, dropColorCP: rec.dropColorCP!, dropColor: rec.dropColor!, modelClass: cls });
    radar3dFree.set(cls, list);
    rec.entities = [];
  }

  /** Contact id under a window position (click/hover), or null. */
  function radarPickId(windowPos: Cesium.Cartesian2): string | null {
    if (!viewer) return null;
    const picked = viewer.scene.pick(windowPos);
    const ent = picked?.id;
    return ent instanceof Cesium.Entity ? (radar3dEntityIds.get(ent) ?? null) : null;
  }

  /** Update the contact model (position/orientation/colour/size) — cheap, no geometry rebuild. Skipped
   *  entirely when nothing relevant changed (the snapshot fires up to 5 Hz; most contacts are unchanged). */
  function syncRadarModel(rec: Radar3dRec) {
    const e = rec.entities[0];
    if (!e.model) return;
    const sig = `${rec.lat.toFixed(6)},${rec.lon.toFixed(6)},${rec.contactEll.toFixed(1)},${rec.headingDeg ?? 'x'},${rec.color.toCssColorString()},${rec.selected},${rec.hideRadiusM},${rec.altM},${rec.callsign}`;
    if (sig === rec.modelSig && !rec.selected) return; // selected re-syncs every poll for the live distance/bearing label
    rec.modelSig = sig;
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

  /**
   * Rebuild the obstacle columns from the current aero snapshot. Static features → rebuilt only on
   * data / visibility change (not per camera frame).
   *
   * Heights: we sample Cesium's own terrain (ellipsoidal) at each obstacle and place the column
   * absolutely on it (base → base + heightM). This is **geoid-independent** and always sits exactly on
   * the ground, robust to camera/UAV position and to on-demand rendering — unlike RELATIVE_TO_GROUND
   * clamping, which drifts when the terrain under the obstacle isn't loaded (off-screen). A slim
   * real-world footprint makes the column shrink perspectively with distance (not a fixed-size sprite).
   */
  async function updateObstacles3D() {
    if (!viewer) return;
    const gen = ++obstacle3dGen;
    for (const e of obstacle3dEntities) viewer.entities.remove(e);
    obstacle3dEntities.length = 0;

    const air = get(settings).airspace;
    // Skip the (terrain-sampling) build while the 3D view is hidden — the activation effect rebuilds on
    // re-entry. `active` is the only inexpensive gate that matters here.
    if (!active || !air.enabled || !air.layers.obstacles.d3) { viewer.scene.requestRender(); return; }

    let obstacles = get(aeroData).obstacles;
    if (obstacles.length === 0) { viewer.scene.requestRender(); return; }

    // Horizontal sight-line limit: render only obstacles within the configured range of the camera's
    // ground point (falls back to the UAV / radar reference when the camera looks at the sky). Keeps
    // the scene to nearby hazards and bounds the terrain-sampling cost.
    const camGround = getCamFocus();
    const ref = (camGround ? { lat: camGround.lat, lon: camGround.lon } : null) ?? radarReference ?? uavLatLon;
    aeroRefGround = ref ?? undefined;
    if (ref) {
      const maxM = air.obstacleDistanceKm * 1000;
      obstacles = obstacles
        .map((p) => ({ p, d: haversineDistance(ref.lat, ref.lon, p.lat, p.lon) }))
        .filter((x) => x.d <= maxM)
        .sort((a, b) => a.d - b.d)
        .slice(0, OBSTACLE_3D_MAX)
        .map((x) => x.p);
    } else if (obstacles.length > OBSTACLE_3D_MAX) {
      obstacles = obstacles.slice(0, OBSTACLE_3D_MAX); // no reference yet → just cap
    }
    if (obstacles.length === 0) { viewer.scene.requestRender(); return; }

    // Sample the real terrain height (above the ellipsoid) at every obstacle in one batched call.
    let groundEll: (number | undefined)[] = [];
    const tp = viewer.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      try {
        const carto = obstacles.map((p) => Cesium.Cartographic.fromDegrees(p.lon, p.lat));
        const sampled = await Cesium.sampleTerrainMostDetailed(tp, carto);
        if (gen !== obstacle3dGen || !viewer) return; // a newer rebuild superseded this one
        groundEll = sampled.map((c) => c.height);
      } catch (e) {
        console.warn("[Map3D] obstacle terrain sample failed", e);
      }
    }

    // Surveyed height → solid orange; estimated (height-less) → translucent + yellow outline.
    const fillKnown = Cesium.Color.fromCssColorString("#e8740c").withAlpha(0.45);
    const outlineKnown = Cesium.Color.fromCssColorString("#ff9a2e").withAlpha(0.9);
    const fillEst = Cesium.Color.fromCssColorString("#ffd24a").withAlpha(0.16);
    const outlineEst = Cesium.Color.fromCssColorString("#ffd24a").withAlpha(0.7);
    for (let i = 0; i < obstacles.length; i++) {
      const p = obstacles[i];
      const estimated = p.heightM == null; // no surveyed AGL height → typed default, drawn distinctly
      const h = p.heightM ?? (p.subtype === "Wind Turbine" ? OBSTACLE_3D_TURBINE_H : OBSTACLE_3D_DEFAULT_H);
      const ellipse: Cesium.EllipseGraphics.ConstructorOptions = {
        semiMinorAxis: OBSTACLE_3D_RADIUS_M,
        semiMajorAxis: OBSTACLE_3D_RADIUS_M,
        material: estimated ? fillEst : fillKnown,
        outline: true,
        outlineColor: estimated ? outlineEst : outlineKnown,
        numberOfVerticalLines: 4,
      };
      const base = groundEll[i];
      if (base != null && isFinite(base)) {
        ellipse.height = base; // absolute ellipsoidal ground from the terrain sample
        ellipse.extrudedHeight = base + h;
      } else {
        // Terrain not ready / sample failed → clamp to ground as a fallback so it still shows.
        ellipse.height = 0;
        ellipse.heightReference = Cesium.HeightReference.RELATIVE_TO_GROUND;
        ellipse.extrudedHeight = h;
        ellipse.extrudedHeightReference = Cesium.HeightReference.RELATIVE_TO_GROUND;
      }
      const ent = viewer.entities.add({ position: Cesium.Cartesian3.fromDegrees(p.lon, p.lat), ellipse });
      obstacle3dEntities.push(ent);
    }
    viewer.scene.requestRender();
  }

  /** Lateral nearest-vertex distance (m) from a point to an airspace outline. */
  function airspaceLateralM(a: Airspace, lat: number, lon: number): number {
    let best = Number.POSITIVE_INFINITY;
    for (const ring of a.outlines) {
      for (const [vlon, vlat] of ring) {
        const d = haversineDistance(lat, lon, vlat, vlon);
        if (d < best) best = d;
      }
    }
    return best;
  }

  /** Bounding-box diagonal (km) of an airspace — used to skip rendering country-sized airspaces. */
  function airspaceMaxExtentKm(a: Airspace): number {
    let minLat = 90, maxLat = -90, minLon = 180, maxLon = -180;
    for (const ring of a.outlines) {
      for (const [lon, lat] of ring) {
        if (lat < minLat) minLat = lat; if (lat > maxLat) maxLat = lat;
        if (lon < minLon) minLon = lon; if (lon > maxLon) maxLon = lon;
      }
    }
    return haversineDistance(minLat, minLon, maxLat, maxLon) / 1000;
  }

  /** Ellipsoidal height (m) for an airspace altitude limit. GND → terrain; MSL/STD(FL) → value + geoid. */
  function limitEll(lim: AltLimit, groundEll: number): number {
    return lim.datum === "gnd" ? groundEll + lim.valueM : lim.valueM + geoidOffset;
  }

  /**
   * Rebuild the airspace volumes relevant to the reference (camera ground, else UAV/GCS). An airspace is
   * relevant when the reference is inside it or within AIRSPACE_3D_LATERAL_M of its boundary. FIR/UIR and
   * unclassified "free" airspace are skipped (huge / clutter — same spirit as the 2D click list). Extruded
   * floor→ceiling polygons, class-coloured + translucent.
   */
  async function updateAirspaces3D() {
    if (!viewer) return;
    const gen = ++airspace3dGen;
    for (const p of airspace3dPrimitives) viewer.scene.primitives.remove(p); // remove() also destroys
    airspace3dPrimitives.length = 0;

    const air = get(settings).airspace;
    if (!active || !air.enabled || !air.layers.airspaces.d3) { viewer.scene.requestRender(); return; }

    const camGround = getCamFocus();
    // Culling / which airspaces to show follows the camera ground focus...
    const ref = (camGround ? { lat: camGround.lat, lon: camGround.lon } : null) ?? radarReference ?? uavLatLon;
    if (!ref) { viewer.scene.requestRender(); return; }
    // ...but the patterned "nearest wall" is relative to the UAV (or GCS) — that's the proximity warning
    // reference, independent of where the camera looks. No real reference (e.g. fake GPS without an FC) →
    // no wall is drawn (never fall back to the camera, which would mark misleading walls). A fake position
    // fed through a connected FC arrives as telemetry → it IS a valid uavRef, so walls still draw.
    const uavRef = radarReference ?? uavLatLon;
    aeroRefGround = ref;
    // Ensure the geoid offset is known for this region (kept once; rebuild when it resolves).
    if (!geoidComputed) void computeGeoidOnce(ref.lat, ref.lon).then((ok) => { if (ok) updateAirspaces3D(); });

    const relevant = get(aeroData).airspaces.filter((a) => {
      if (isAirspaceHidden(a) || !airspaceIsRelevant(a)) return false; // FIR/UIR + unclassified clutter
      if (airspaceLateralM(a, ref.lat, ref.lon) <= AIRSPACE_3D_LATERAL_M) return true; // a wall is near
      // Inside, but only for reasonably-sized airspaces — never render country-sized upper air / CTAs.
      return airspaceContainsPoint(a, ref.lat, ref.lon) && airspaceMaxExtentKm(a) <= AIRSPACE_3D_MAX_EXTENT_KM;
    }).slice(0, AIRSPACE_3D_MAX);
    if (relevant.length === 0) { viewer.scene.requestRender(); return; }

    // Sample terrain once per airspace (first-ring centroid) for GND-referenced floors/ceilings.
    const centroids = relevant.map((a) => {
      const ring = a.outlines[0] ?? [];
      let sx = 0, sy = 0;
      for (const [lon, lat] of ring) { sx += lon; sy += lat; }
      const n = Math.max(1, ring.length);
      return Cesium.Cartographic.fromDegrees(sx / n, sy / n);
    });
    let groundEll: number[] = centroids.map(() => 0);
    const tp = viewer.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      try {
        const sampled = await Cesium.sampleTerrainMostDetailed(tp, centroids);
        if (gen !== airspace3dGen || !viewer) return; // superseded
        groundEll = sampled.map((c) => (isFinite(c.height) ? c.height : 0));
      } catch (e) {
        console.warn("[Map3D] airspace terrain sample failed", e);
      }
    }

    // All primitives are raw (not Entities) so they can be allowPicking:false → out of scene.pick and
    // not a click target. The volume is a plain, very faint translucent hull (presence only); only the
    // boundary section nearest the reference is given a pattern (proximity reference / approach warning).
    const polyVF = Cesium.PerInstanceColorAppearance.VERTEX_FORMAT;
    const wallVF = Cesium.MaterialAppearance.MaterialSupport.TEXTURED.vertexFormat;
    const addPrim = (prim: Cesium.Primitive) => { viewer!.scene.primitives.add(prim); airspace3dPrimitives.push(prim); };

    for (let i = 0; i < relevant.length; i++) {
      const a = relevant[i];
      const g = groundEll[i];
      const floor = limitEll(a.lower, g);
      const ceil = limitEll(a.upper, g);
      if (!(ceil > floor)) continue; // bad/degenerate altitude band
      const col = Cesium.Color.fromCssColorString(airspaceStyle(a).color);

      // Faint hull (no pattern) so the airspace extent is just visible.
      for (const ring of a.outlines) {
        if (ring.length < 3) continue;
        const geometry = new Cesium.PolygonGeometry({
          polygonHierarchy: new Cesium.PolygonHierarchy(ring.map(([lon, lat]) => Cesium.Cartesian3.fromDegrees(lon, lat))),
          height: floor, extrudedHeight: ceil, perPositionHeight: false, vertexFormat: polyVF,
        });
        addPrim(new Cesium.Primitive({
          geometryInstances: new Cesium.GeometryInstance({
            geometry,
            attributes: { color: Cesium.ColorGeometryInstanceAttribute.fromColor(col.withAlpha(0.07)) },
          }),
          appearance: new Cesium.PerInstanceColorAppearance({ translucent: true, closed: false }),
          allowPicking: false, asynchronous: false,
        }));
      }

      // Patterned "facing" walls = approach reference. An edge faces the UAV when BOTH:
      //  (1) the UAV's foot of perpendicular falls within the edge (t∈[0,1] = UAV in its ground zone), and
      //  (2) the UAV is on the edge's OUTWARD side (away from the ring centroid) — without (2) the test
      //      also matches the opposite/back wall, which spans the same perpendicular band.
      // Robust for long edges + curved boundaries (several consecutive front edges face at once). Plus one
      // neighbour each side; if the UAV is past every edge (a corner), use the single nearest edge.
      // No UAV/GCS reference → skip (only the hull renders), never mark camera-relative walls.
      if (uavRef) {
      const mLat = 111320, mLon = 111320 * Math.cos((uavRef.lat * Math.PI) / 180);
      const walls: Cesium.GeometryInstance[] = [];
      for (const ring of a.outlines) {
        const ec = ring.length - 1; // edge count (closed ring: ring[ec] == ring[0])
        if (ec < 1) continue;
        // Ring centroid in UAV-local metres (UAV at origin), to orient each edge's outward normal.
        let cx = 0, cy = 0;
        for (let k = 0; k < ec; k++) { cx += (ring[k][0] - uavRef.lon) * mLon; cy += (ring[k][1] - uavRef.lat) * mLat; }
        cx /= ec; cy /= ec;
        const facing: boolean[] = new Array(ec).fill(false);
        let anyFacing = false, nearest = 0, nearestD = Number.POSITIVE_INFINITY;
        for (let k = 0; k < ec; k++) {
          const p1 = ring[k], p2 = ring[k + 1];
          const ax = (p1[0] - uavRef.lon) * mLon, ay = (p1[1] - uavRef.lat) * mLat; // UAV at origin
          const bx = (p2[0] - uavRef.lon) * mLon, by = (p2[1] - uavRef.lat) * mLat;
          const dx = bx - ax, dy = by - ay, len2 = dx * dx + dy * dy;
          const t = len2 > 0 ? -(ax * dx + ay * dy) / len2 : 0; // projection param (unclamped)
          // Outward normal (perpendicular to the edge, pointing away from the centroid).
          const mx = (ax + bx) / 2, my = (ay + by) / 2;
          let nx = -dy, ny = dx;
          if (nx * (mx - cx) + ny * (my - cy) < 0) { nx = -nx; ny = -ny; }
          const outward = mx * nx + my * ny < 0; // UAV(origin) on the outward side: (−mid)·n > 0
          if (t >= 0 && t <= 1 && outward) { facing[k] = true; anyFacing = true; }
          const tc = Math.max(0, Math.min(1, t));
          const d = Math.hypot(ax + tc * dx, ay + tc * dy);
          if (d < nearestD) { nearestD = d; nearest = k; }
        }
        if (!anyFacing) facing[nearest] = true; // corner / inside: UAV not outward of any edge → nearest
        const draw = facing.slice();
        for (let k = 0; k < ec; k++) if (facing[k]) { draw[(k - 1 + ec) % ec] = true; draw[(k + 1) % ec] = true; }
        for (let k = 0; k < ec; k++) {
          if (!draw[k]) continue;
          const p1 = ring[k], p2 = ring[k + 1];
          walls.push(new Cesium.GeometryInstance({
            geometry: Cesium.WallGeometry.fromConstantHeights({
              positions: [Cesium.Cartesian3.fromDegrees(p1[0], p1[1]), Cesium.Cartesian3.fromDegrees(p2[0], p2[1])],
              minimumHeight: floor, maximumHeight: ceil, vertexFormat: wallVF,
            }),
          }));
        }
      }
      if (walls.length) {
        const mat = Cesium.Material.fromType("Grid", {
          color: col.withAlpha(0.55),
          cellAlpha: 0.1,
          lineCount: new Cesium.Cartesian2(6, 4),
          lineThickness: new Cesium.Cartesian2(1.4, 1.4),
        });
        addPrim(new Cesium.Primitive({
          geometryInstances: walls,
          appearance: new Cesium.MaterialAppearance({ material: mat, translucent: true }),
          allowPicking: false, asynchronous: false,
        }));
      }
      } // if (uavRef)
    }
    viewer.scene.requestRender();
  }

  /**
   * Render airports within the configured airfield range of the reference (camera ground, else UAV/GCS)
   * as a type-coloured, ground-clamped marker + name label. (OpenAIP has no runway threshold coordinates,
   * so a projected runway would just cut through the airport point — not usable; markers only.)
   */
  function updateAirports3D() {
    if (!viewer) return;
    for (const e of airport3dEntities) viewer.entities.remove(e);
    airport3dEntities.length = 0;

    const air = get(settings).airspace;
    if (!active || !air.enabled || !air.layers.airports.d3) { viewer.scene.requestRender(); return; }

    const camGround = getCamFocus();
    const ref = (camGround ? { lat: camGround.lat, lon: camGround.lon } : null) ?? radarReference ?? uavLatLon;
    if (ref) aeroRefGround = ref;
    let airports = get(aeroData).airports;
    if (ref) {
      const maxM = air.airfieldDistanceKm * 1000;
      airports = airports.filter((p) => haversineDistance(ref.lat, ref.lon, p.lat, p.lon) <= maxM);
    }
    if (airports.length === 0) { viewer.scene.requestRender(); return; }

    for (const p of airports) {
      // Same badge as the 2D map (disc + star / "H"), constant ~24 px screen size like a map marker.
      airport3dEntities.push(viewer.entities.add({
        position: Cesium.Cartesian3.fromDegrees(p.lon, p.lat),
        billboard: {
          image: airportBillboard(p.typeId),
          width: 24, height: 24,
          verticalOrigin: Cesium.VerticalOrigin.CENTER,
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY, // stay visible like a 2D marker
        },
        label: {
          text: p.name || p.subtype,
          font: '600 12px "Segoe UI", sans-serif',
          fillColor: Cesium.Color.WHITE, outlineColor: Cesium.Color.BLACK.withAlpha(0.85), outlineWidth: 2,
          style: Cesium.LabelStyle.FILL_AND_OUTLINE,
          pixelOffset: new Cesium.Cartesian2(0, -17),
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
          scaleByDistance: new Cesium.NearFarScalar(2000, 1.0, 30000, 0.6),
          translucencyByDistance: new Cesium.NearFarScalar(25000, 1.0, 32000, 0.0),
        },
      }));
    }
    viewer.scene.requestRender();
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
        // Reuse a free bundle of this class instantly; only queue a NEW build when the pool is empty.
        if (!acquireBundleFor(rec)) radar3dCreateQueue.push(rec);
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
      if (!seen.has(id)) { releaseBundle(rec); radar3dRecs.delete(id); } // pool the bundle, don't destroy
    }
    if (radar3dCreateQueue.length && !radar3dCreateRaf) radar3dCreateRaf = requestAnimationFrame(drainRadarCreateQueue);
    // A pulse (radar alert or active-WP glow) needs continuous rendering; otherwise on-demand. Shared
    // helper so the alert and WP pulses don't fight over requestRenderMode.
    syncContinuousRender();
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

  // ── Safehome + autoland overlay (3D mirror of Map.svelte::updateSafehome) ────────────────────────
  /** Teardrop "H" pin as an SVG data URI (the 2D marker is HTML, not reusable here). Tip at the bottom
   *  centre → place with verticalOrigin BOTTOM so the point sits on the coordinate. */
  function safehomeBillboardSvg(enabled: boolean): string {
    const bg = enabled ? '#59aa29' : '#888';
    const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" width="26" height="32" viewBox="0 0 26 32">' +
      `<path d="M13 31C13 31 24 18.5 24 11A11 11 0 1 0 2 11C2 18.5 13 31 13 31Z" fill="${bg}" stroke="#000" stroke-width="2"/>` +
      '<text x="13" y="15" text-anchor="middle" font-family="Segoe UI, Tahoma, sans-serif" font-size="12" font-weight="bold" fill="#fff">H</text>' +
      '</svg>';
    return 'data:image/svg+xml,' + encodeURIComponent(svg);
  }

  /** A closed circle (lat/lon ring) of `segments` points around a centre — used for the radius rings. */
  function safehomeCircle(lat: number, lon: number, radiusM: number, segments = 64): [number, number][] {
    const out: [number, number][] = [];
    for (let i = 0; i <= segments; i++) {
      const p = destinationPoint(lat, lon, (360 / segments) * i, radiusM);
      out.push([p.lat, p.lon]);
    }
    return out;
  }

  /** Redraw the safehome markers + radius rings (+ 3D approach paths). Mirrors the 2D overlay; the
   *  approach path and the elevated loiter ring need the safehome's ground ellipsoid height, so those
   *  are drawn after an async terrain sample (guarded by `safehome3dGen`, like the airspace overlay). */
  async function updateSafehome3D() {
    if (!viewer) return;
    const gen = ++safehome3dGen;
    for (const e of safehomeEntities) viewer.entities.remove(e);
    safehomeEntities.length = 0;
    if (!get(settings).showSafehomes) { viewer.scene.requestRender(); return; }
    const cfg = get(safehomeWorking);
    if (!cfg) { viewer.scene.requestRender(); return; }

    const armed = (get(telemetry).armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
    const maxDistM = cfg.safehome_max_distance_cm != null ? cfg.safehome_max_distance_cm / 100 : null;
    const loiterM = cfg.loiter_radius_cm != null ? cfg.loiter_radius_cm / 100 : null;
    const approachLenM = cfg.autoland.approach_length_cm != null ? cfg.autoland.approach_length_cm / 100 : 0;

    const flat = (pts: [number, number][]) => pts.flatMap(([la, lo]) => [lo, la]);
    const addGroundRing = (lat: number, lon: number, radiusM: number, css: string) => {
      safehomeEntities.push(viewer!.entities.add({
        polyline: {
          positions: Cesium.Cartesian3.fromDegreesArray(flat(safehomeCircle(lat, lon, radiusM))),
          clampToGround: true, width: 2.5, material: Cesium.Color.fromCssColorString(css),
        },
      }));
    };

    // Per-safehome jobs that need a terrain sample (elevated loiter ring + the 3D approach path).
    const jobs: { lat: number; lon: number; ap: typeof cfg.approaches[number] }[] = [];

    for (const sh of cfg.safehomes) {
      if (isSafehomeEmpty(sh)) continue;
      const lat = sh.lat / 1e7, lon = sh.lon / 1e7;

      // Rings + approach only for ENABLED safehomes (a disabled slot keeps just the grey marker).
      if (sh.enabled) {
        if (maxDistM && !armed) addGroundRing(lat, lon, maxDistM, '#59aa29');  // green, disarmed-only
        const ap = cfg.approaches.find((a) => a.index === sh.index);
        const hasApproach = !!ap && cfg.has_autoland && approachLenM > 0 && !!loiterM;
        if (hasApproach) {
          jobs.push({ lat, lon, ap: ap! }); // elevated loiter ring + path drawn after the terrain sample
        } else if (loiterM) {
          addGroundRing(lat, lon, loiterM, '#f5a623'); // no approach data → loiter ring on the ground
        }
      }

      // Marker — always (grey when disabled). Display-only in 3D; editing stays on 2D + the panel.
      safehomeEntities.push(viewer.entities.add({
        position: Cesium.Cartesian3.fromDegrees(lon, lat),
        billboard: {
          image: safehomeBillboardSvg(sh.enabled),
          width: 26, height: 32,
          verticalOrigin: Cesium.VerticalOrigin.BOTTOM,
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      }));
    }
    viewer.scene.requestRender(); // markers + ground rings show immediately

    if (jobs.length === 0) return;

    // Ground ellipsoid height per safehome — for the elevated loiter ring + the descending approach path.
    let groundEll: number[] = jobs.map(() => 0);
    const tp = viewer.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      try {
        const sampled = await Cesium.sampleTerrainMostDetailed(tp, jobs.map((j) => Cesium.Cartographic.fromDegrees(j.lon, j.lat)));
        if (gen !== safehome3dGen || !viewer) return; // superseded by a newer redraw
        groundEll = sampled.map((c) => (isFinite(c.height) ? c.height : 0));
      } catch (e) {
        console.warn('[Map3D] safehome terrain sample failed', e);
      }
    }

    for (let i = 0; i < jobs.length; i++) {
      const { lat, lon, ap } = jobs[i];
      const gEll = groundEll[i];
      // Set safehomes show the loaded approach_alt as-is (0 = unconfigured → a flat ground path); empty
      // (0,0) slots aren't drawn at all, and the editor pre-fills the default only for those.
      const approachAltM = ap.approach_alt_cm / 100;
      // Top of the descent = downwind/approach altitude, drawn RELATIVE to the safehome's own ground (a
      // visual 3D depiction, not a metric reference). We deliberately ignore sea_level_ref / geoid: anchor
      // the whole pattern to the home height, otherwise an MSL/geoid-0 top detaches the path from the
      // ground and the descent collapses onto one level.
      const topEll = gEll + approachAltM;

      // Yellow loiter ring at the downwind altitude.
      if (loiterM) {
        safehomeEntities.push(viewer.entities.add({
          polyline: {
            positions: Cesium.Cartesian3.fromDegreesArrayHeights(
              safehomeCircle(lat, lon, loiterM).flatMap(([la, lo]) => [lo, la, topEll]),
            ),
            arcType: Cesium.ArcType.NONE, width: 2.5, material: Cesium.Color.fromCssColorString('#f5a623'),
          },
        }));
      }

      // Approach path drawn at its real 3D descent altitude (downwind level → base −33 % → final to ground).
      for (const leg of buildApproachGeometry(lat, lon, {
        heading1: ap.heading1, heading2: ap.heading2,
        approachLengthM: approachLenM, loiterRadiusM: loiterM!, approachDirection: ap.approach_direction,
      })) {
        const positions = leg.points.map(([la, lo], k) =>
          Cesium.Cartesian3.fromDegrees(lo, la, gEll + (leg.altFrac[k] ?? 0) * (topEll - gEll)));
        safehomeEntities.push(viewer.entities.add({
          polyline: {
            positions, arcType: Cesium.ArcType.NONE, width: 4,
            material: Cesium.Color.fromCssColorString(leg.final ? '#f78a05' : '#2e6fff'), // orange final, blue downwind/base
          },
        }));
      }
    }
    viewer.scene.requestRender();
  }

  // Redraw on the display toggle (mirrors the 2D `$effect` on `$settings.showSafehomes`).
  $effect(() => { void $settings.showSafehomes; if (viewer) updateSafehome3D(); });

  // ── Geozones overlay (INAV ≥8.0; see docs/active/GEOZONES.md) ─────────────────────────────────
  /** Rebuild the geozone volumes (read-only in P1). Circle → extruded cylinder, polygon → extruded
   *  hull; blue inclusive / amber exclusive. Floor/ceiling from the zone's min/max altitude (AGL →
   *  terrain-sampled ground, AMSL → + geoid); max 0 = no upper limit → a tall visual cap. Gated by the
   *  3D layer toggle (independent of the OpenAIP airspace subsystem). */
  async function updateGeozones3D() {
    if (!viewer) return;
    const gen = ++geozone3dGen;
    for (const e of geozone3dEntities) viewer.entities.remove(e);
    geozone3dEntities.length = 0;
    if (!active || !get(settings).airspace.layers.geozones.d3) { viewer.scene.requestRender(); return; }
    const cfg = get(geozoneWorking);
    if (!cfg || !cfg.has_geozones || cfg.zones.length === 0) { viewer.scene.requestRender(); return; }

    const NO_LIMIT_H = 1000; // visual extrusion height (m) for a "no upper limit" (max 0) zone

    // One representative point per zone (circle centre / polygon centroid) for the AGL ground sample.
    const reps = cfg.zones.map((z) => {
      if (z.shape === GEOZONE_SHAPE_CIRCULAR) {
        const c = z.vertices[0];
        return Cesium.Cartographic.fromDegrees((c?.lon ?? 0) / 1e7, (c?.lat ?? 0) / 1e7);
      }
      let sx = 0, sy = 0;
      for (const v of z.vertices) { sx += v.lon; sy += v.lat; }
      const n = Math.max(1, z.vertices.length);
      return Cesium.Cartographic.fromDegrees(sx / n / 1e7, sy / n / 1e7);
    });
    let groundEll: number[] = reps.map(() => 0);
    const tp = viewer.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      try {
        const sampled = await Cesium.sampleTerrainMostDetailed(tp, reps);
        if (gen !== geozone3dGen || !viewer) return; // superseded by a newer redraw
        groundEll = sampled.map((c) => (isFinite(c.height) ? c.height : 0));
      } catch (e) {
        console.warn("[Map3D] geozone terrain sample failed", e);
      }
    }

    for (let i = 0; i < cfg.zones.length; i++) {
      const z = cfg.zones[i];
      if (z.vertices.length === 0) continue;
      const baseRef = z.is_sealevel_ref ? geoidOffset : groundEll[i]; // AMSL → + geoid; AGL → terrain ground
      const floorEll = baseRef + z.min_alt_cm / 100;                  // min 0 = ground / sea level
      const ceilEll = z.max_alt_cm > 0 ? baseRef + z.max_alt_cm / 100 : floorEll + NO_LIMIT_H; // max 0 = ∞
      if (!(ceilEll > floorEll)) continue;

      const st = geozonePathStyle(z); // same type+action scheme as 2D (colour, weight, dash, fill)
      const lineColor = Cesium.Color.fromCssColorString(st.color).withAlpha(st.opacity);
      // The line variants in 3D go on real polylines: Cesium outline width/dash on extruded volumes is
      // unreliable (WebGL clamps to 1 px, no dash). None → dashed; Pos-Hold/RTH → thick (st.weight).
      const lineMat = st.dashArray
        ? new Cesium.PolylineDashMaterialProperty({ color: lineColor, dashLength: 16.0 })
        : lineColor;

      // Footprint ring ([lat,lon], closed) + the circle centre/radius for the fill volume.
      let ring: [number, number][];
      let circleCentre: { lat: number; lon: number; r: number } | null = null;
      if (z.shape === GEOZONE_SHAPE_CIRCULAR) {
        const c = z.vertices[0];
        const r = geozoneRadiusM(z);
        if (r == null || r <= 0) continue;
        const lat = c.lat / 1e7, lon = c.lon / 1e7;
        circleCentre = { lat, lon, r };
        ring = safehomeCircle(lat, lon, r);
      } else {
        ring = z.vertices.map((v) => [v.lat / 1e7, v.lon / 1e7] as [number, number]);
        if (ring.length < 3) continue;
        ring.push(ring[0]); // close the loop
      }

      // Translucent volume — only for (type, action) combos that get a fill (mwp scheme); no entity
      // outline (the boundary polylines below carry the line style).
      if (st.fill) {
        const fillMaterial = Cesium.Color.fromCssColorString(st.color).withAlpha(0.13);
        if (circleCentre) {
          geozone3dEntities.push(viewer.entities.add({
            position: Cesium.Cartesian3.fromDegrees(circleCentre.lon, circleCentre.lat),
            ellipse: { semiMinorAxis: circleCentre.r, semiMajorAxis: circleCentre.r, height: floorEll, extrudedHeight: ceilEll, material: fillMaterial, outline: false },
          }));
        } else {
          const positions = Cesium.Cartesian3.fromDegreesArray(z.vertices.flatMap((v) => [v.lon / 1e7, v.lat / 1e7]));
          geozone3dEntities.push(viewer.entities.add({
            polygon: { hierarchy: new Cesium.PolygonHierarchy(positions), height: floorEll, extrudedHeight: ceilEll, perPositionHeight: false, material: fillMaterial, outline: false },
          }));
        }
      }

      // Boundary rings: floor (always) + ceiling (only a real ceiling, max>0) — width/dash per action.
      const ringAt = (h: number) => Cesium.Cartesian3.fromDegreesArrayHeights(ring.flatMap(([la, lo]) => [lo, la, h]));
      geozone3dEntities.push(viewer.entities.add({
        polyline: { positions: ringAt(floorEll), arcType: Cesium.ArcType.NONE, width: st.weight, material: lineMat },
      }));
      if (z.max_alt_cm > 0) {
        geozone3dEntities.push(viewer.entities.add({
          polyline: { positions: ringAt(ceilEll), arcType: Cesium.ArcType.NONE, width: st.weight, material: lineMat },
        }));
      }
    }
    viewer.scene.requestRender();
  }

  /** Red overlay for the mission legs flagged by the geozone safety check, drawn at the mission's 3D
   *  height (launch-relative, matching the mission line). Hint only. */
  function updateGeozoneViolations3D() {
    if (!viewer) return;
    for (const e of geozone3dViolationEntities) viewer.entities.remove(e);
    geozone3dViolationEntities.length = 0;
    const res = get(geozoneMissionResult);
    if (!active || !res.active || res.segments.length === 0) { viewer.scene.requestRender(); return; }
    for (const seg of res.segments) {
      // Match the mission line exactly: it places each WP at altMsl + geoidOffset (resolveMissionAltitudes).
      const positions = [
        Cesium.Cartesian3.fromDegrees(seg.a.lon, seg.a.lat, seg.a.altMsl + geoidOffset),
        Cesium.Cartesian3.fromDegrees(seg.b.lon, seg.b.lat, seg.b.altMsl + geoidOffset),
      ];
      geozone3dViolationEntities.push(viewer.entities.add({
        polyline: { positions, arcType: Cesium.ArcType.NONE, width: 5, material: Cesium.Color.fromCssColorString("#ff2d2d") },
      }));
    }
    viewer.scene.requestRender();
  }

  // ── Geofence overlay (ArduPilot/PX4; see docs/active/GEOFENCE.md) ──────────────────────────────
  /** Rebuild the geofence volumes. Circle → extruded cylinder, polygon → extruded hull; blue inclusion
   *  / amber exclusion. Fences have no per-zone altitude, so all zones extrude from the terrain ground
   *  up to the global vertical limit (ArduPilot FENCE_ALT_MAX / PX4 GF_MAX_VER_DIST), with a visual
   *  fallback cap when no such param exists. Gated by the fence 3D layer toggle. */
  async function updateFence3D() {
    if (!viewer) return;
    const gen = ++fence3dGen;
    for (const e of fence3dEntities) viewer.entities.remove(e);
    fence3dEntities.length = 0;
    if (!active || !get(settings).airspace.layers.fence.d3) { viewer.scene.requestRender(); return; }
    const cfg = get(fenceWorking);
    if (!cfg || !cfg.has_fence || cfg.zones.length === 0) { viewer.scene.requestRender(); return; }

    const NO_LIMIT_H = 120; // visual extrusion height (m) when no vertical-limit param is set
    const altParam = cfg.params.find((p) => p.name === "FENCE_ALT_MAX" || p.name === "GF_MAX_VER_DIST");
    const altMaxM = altParam && altParam.value > 0 ? altParam.value : NO_LIMIT_H;

    // One representative point per zone (circle centre / polygon centroid) for the AGL ground sample.
    const reps = cfg.zones.map((z) => {
      if (z.shape === FENCE_SHAPE_CIRCLE) {
        const c = z.vertices[0];
        return Cesium.Cartographic.fromDegrees((c?.lon ?? 0) / 1e7, (c?.lat ?? 0) / 1e7);
      }
      let sx = 0, sy = 0;
      for (const v of z.vertices) { sx += v.lon; sy += v.lat; }
      const n = Math.max(1, z.vertices.length);
      return Cesium.Cartographic.fromDegrees(sx / n / 1e7, sy / n / 1e7);
    });
    let groundEll: number[] = reps.map(() => 0);
    const tp = viewer.scene.terrainProvider;
    if (tp && !(tp instanceof Cesium.EllipsoidTerrainProvider)) {
      try {
        const sampled = await Cesium.sampleTerrainMostDetailed(tp, reps);
        if (gen !== fence3dGen || !viewer) return; // superseded by a newer redraw
        groundEll = sampled.map((c) => (isFinite(c.height) ? c.height : 0));
      } catch (e) {
        console.warn("[Map3D] fence terrain sample failed", e);
      }
    }

    for (let i = 0; i < cfg.zones.length; i++) {
      const z = cfg.zones[i];
      if (z.vertices.length === 0) continue;
      const floorEll = groundEll[i];
      const ceilEll = floorEll + altMaxM;

      const st = fencePathStyle(z); // blue inclusion / amber exclusion
      const lineColor = Cesium.Color.fromCssColorString(st.color).withAlpha(st.opacity);

      // Footprint ring ([lat,lon], closed) + the circle centre/radius for the fill volume.
      let ring: [number, number][];
      let circleCentre: { lat: number; lon: number; r: number } | null = null;
      if (z.shape === FENCE_SHAPE_CIRCLE) {
        const c = z.vertices[0];
        const r = fenceRadiusM(z);
        if (r == null || r <= 0) continue;
        const lat = c.lat / 1e7, lon = c.lon / 1e7;
        circleCentre = { lat, lon, r };
        ring = safehomeCircle(lat, lon, r);
      } else {
        ring = z.vertices.map((v) => [v.lat / 1e7, v.lon / 1e7] as [number, number]);
        if (ring.length < 3) continue;
        ring.push(ring[0]); // close the loop
      }

      // Translucent fill volume (no entity outline — the boundary polylines below carry the line style).
      const fillMaterial = Cesium.Color.fromCssColorString(st.color).withAlpha(0.13);
      if (circleCentre) {
        fence3dEntities.push(viewer.entities.add({
          position: Cesium.Cartesian3.fromDegrees(circleCentre.lon, circleCentre.lat),
          ellipse: { semiMinorAxis: circleCentre.r, semiMajorAxis: circleCentre.r, height: floorEll, extrudedHeight: ceilEll, material: fillMaterial, outline: false },
        }));
      } else {
        const positions = Cesium.Cartesian3.fromDegreesArray(z.vertices.flatMap((v) => [v.lon / 1e7, v.lat / 1e7]));
        fence3dEntities.push(viewer.entities.add({
          polygon: { hierarchy: new Cesium.PolygonHierarchy(positions), height: floorEll, extrudedHeight: ceilEll, perPositionHeight: false, material: fillMaterial, outline: false },
        }));
      }

      // Boundary rings: floor + ceiling.
      const ringAt = (h: number) => Cesium.Cartesian3.fromDegreesArrayHeights(ring.flatMap(([la, lo]) => [lo, la, h]));
      fence3dEntities.push(viewer.entities.add({
        polyline: { positions: ringAt(floorEll), arcType: Cesium.ArcType.NONE, width: st.weight, material: lineColor },
      }));
      fence3dEntities.push(viewer.entities.add({
        polyline: { positions: ringAt(ceilEll), arcType: Cesium.ArcType.NONE, width: st.weight, material: lineColor },
      }));
    }
    viewer.scene.requestRender();
  }

  // ── Rally points overlay (ArduPilot/PX4; see docs/active/GEOFENCE.md) ──────────────────────────
  /** Rebuild the rally-point markers: a green ground-clamped point + label per point. Rally altitude is
   *  relative to home (not drawn here — the markers mark the ground location). Gated by the rally 3D toggle. */
  function updateRally3D() {
    if (!viewer) return;
    for (const e of rally3dEntities) viewer.entities.remove(e);
    rally3dEntities.length = 0;
    if (!active || !get(settings).airspace.layers.rally.d3) { viewer.scene.requestRender(); return; }
    const cfg = get(rallyWorking);
    if (!cfg || !cfg.has_rally || cfg.points.length === 0) { viewer.scene.requestRender(); return; }

    const tr = get(t);
    cfg.points.forEach((p, i) => {
      const pos = Cesium.Cartesian3.fromDegrees(p.lon / 1e7, p.lat / 1e7);
      rally3dEntities.push(viewer!.entities.add({
        position: pos,
        point: {
          pixelSize: 12,
          color: Cesium.Color.fromCssColorString("#59aa29"),
          outlineColor: Cesium.Color.WHITE,
          outlineWidth: 2,
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
        label: {
          text: `${tr("rally.abbrev")}${i + 1}`,
          font: "bold 12px 'Segoe UI', sans-serif",
          fillColor: Cesium.Color.WHITE,
          showBackground: true,
          backgroundColor: Cesium.Color.fromCssColorString("#59aa29").withAlpha(0.85),
          verticalOrigin: Cesium.VerticalOrigin.BOTTOM,
          pixelOffset: new Cesium.Cartesian2(0, -14),
          heightReference: Cesium.HeightReference.CLAMP_TO_GROUND,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        },
      }));
    });
    viewer.scene.requestRender();
  }

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
    // Live trail is built from the liveTrack store (see the liveTrack subscription), not here.
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

  /** Material for a trail segment of the given CSS colour (FPV-dimmed alpha). */
  function trailMaterial(color: string): Cesium.ColorMaterialProperty {
    return new Cesium.ColorMaterialProperty(Cesium.Color.fromCssColorString(color).withAlpha(fpvAlpha(0.7)));
  }

  /** Bake the current growing run into a static polyline (called on a colour change). */
  function finalizeActiveSegment() {
    if (!viewer || activeTrailPositions.length < 2) return;
    const seg = viewer.entities.add({
      polyline: {
        positions: [...activeTrailPositions],
        width: 2,
        material: trailMaterial(trailCurrentColor3D),
        clampToGround: false,
      },
    });
    trailSegments3D.push({ entity: seg, color: trailCurrentColor3D });
  }

  /** Append one liveTrack point to the colour-segmented 3D trail. The growing run is a single
   *  CallbackProperty polyline (no remove/re-add per point → no flicker); a flight-mode change bakes
   *  the completed run into a static segment and recolours the growing one. */
  function appendTrailPoint3D(pt: LiveTrackPoint) {
    if (!viewer) return;
    const alt = pt.alt_m + geoidOffset; // raw MSL + geoid → ellipsoid height (correct once geoid is known)
    const pos = Cesium.Cartesian3.fromDegrees(pt.lon, pt.lat, alt);

    if (lastTrailLat !== 0 || lastTrailLon !== 0) {
      const prev = Cesium.Cartesian3.fromDegrees(lastTrailLon, lastTrailLat, alt);
      if (Cesium.Cartesian3.distance(pos, prev) < MIN_TRAIL_DIST_3D) return;
    }
    lastTrailLat = pt.lat;
    lastTrailLon = pt.lon;

    // Hold the newest point back by one: commit the previous (now-confirmed) point, keep this one
    // pending. The drawn tip stays one segment behind the live (smoothed) UAV → no FPV overshoot.
    if (pendingTrailPos) commitTrailPoint(pendingTrailPos, pendingTrailColor);
    pendingTrailPos = pos;
    pendingTrailColor = modeColor(pt.mode_primary);
  }

  /** Add one confirmed point to the colour-segmented active run (a colour change bakes the completed
   *  run into a static segment and recolours the growing one). */
  function commitTrailPoint(pos: Cesium.Cartesian3, color: string) {
    if (!viewer) return;
    if (color !== trailCurrentColor3D && activeTrailPositions.length >= 2) {
      finalizeActiveSegment();
      activeTrailPositions = [activeTrailPositions[activeTrailPositions.length - 1]];
      if (activeTrailEntity?.polyline) activeTrailEntity.polyline.material = trailMaterial(color);
    }
    trailCurrentColor3D = color;
    activeTrailPositions.push(pos);

    if (!activeTrailEntity) {
      activeTrailEntity = viewer.entities.add({
        polyline: {
          positions: new Cesium.CallbackProperty(() => activeTrailPositions, false),
          width: 2,
          material: trailMaterial(color),
          clampToGround: false,
        },
      });
    }
  }

  /** Rebuild the whole 3D trail from the liveTrack store at the current geoid offset. Used when the
   *  geoid offset becomes known (corrects the first points) and on a clear/re-arm (store shrinks). */
  function rebuildLiveTrail3D() {
    if (!viewer) return;
    resetTrail3D();
    const pts = get(liveTrack);
    for (const pt of pts) appendTrailPoint3D(pt);
    trailConsumed = pts.length;
    viewer.scene.requestRender();
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

  /** Compute the geoid offset (if not yet done) and re-place the mission at the new height. Once the
   *  offset is known this is a no-op — without the guard it re-rendered the mission on *every* telemetry
   *  frame (computeGeoidOnce resolves true immediately when already computed), which flickered the
   *  waypoints (renderMission3D removes them, awaits terrain, re-adds). */
  async function ensureGeoid(lat: number, lon: number) {
    if (geoidComputed) return; // already derived → nothing to re-place (mission store changes re-render)
    const ok = await computeGeoidOnce(lat, lon);
    // Re-place everything that was drawn at offset 0 before the geoid was known: the mission and the
    // full live trail (its first points were placed at the wrong height / sunk into the ground).
    if (ok) { scheduleMissionRender(); rebuildLiveTrail3D(); viewer?.scene.requestRender(); }
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
    trailConsumed = 0;
    pendingTrailPos = undefined;
    pendingTrailColor = '';
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
      segments = segmentTrackByFlightMode(validTrack as TelemetryRecord[]);
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
      decoValidTrack, colorMode, get(settings).warnAltitudeM ?? 120,
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

  // ── Active waypoint pulse (mirrors the 2D `mission-wp-active`: the marker itself pulses, 2 s period) ──
  /** 0→1→0 over a 2 s period (matches the 2D keyframe), evaluated per frame while continuous-rendering. */
  function wpPulse01(): number {
    return 0.5 + 0.5 * Math.sin((Date.now() / 1000) * Math.PI); // sin(π·t) → 2 s period
  }
  /** Soft radial green glow, drawn BEHIND the active WP marker and sized to cover its body, with a
   *  pulsing alpha + scale (mimics the 2D marker's green drop-shadow "glow" pulse — no size change to
   *  the marker icon itself). Anchored like the marker (same verticalOrigin) so it sits over the icon,
   *  not at the on-ground tip. */
  const WP_GLOW_SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">' +
    '<defs><radialGradient id="g" cx="50%" cy="50%" r="50%">' +
    '<stop offset="0%" stop-color="#7CFF3A" stop-opacity="1"/>' +
    '<stop offset="45%" stop-color="#59aa29" stop-opacity="0.7"/>' +
    '<stop offset="100%" stop-color="#59aa29" stop-opacity="0"/></radialGradient></defs>' +
    '<circle cx="32" cy="32" r="32" fill="url(#g)"/></svg>';
  function addActiveWpGlow(lon: number, lat: number, height: number, spec: ReturnType<typeof wpIconSpec>) {
    // Centre the glow on the marker's HEAD (the round blob), not the on-ground anchor:
    //  - teardrops (bottom-anchored, anchorY ≈ height): head sits ~0.64·h above the tip;
    //  - centred icons (anchorY ≈ h/2): the head IS the on-coordinate centre.
    // CENTER origin + a fixed pixelOffset keeps the glow on the head (billboards are screen-space, so
    // the offset tracks the marker at any zoom) and lets the scale pulse grow symmetrically from it.
    const bottomAnchored = spec.anchorY >= spec.height - 1;
    const headUpPx = bottomAnchored ? spec.height * 0.64 : spec.height / 2 - spec.anchorY;
    const d = spec.width * 1.3; // ≈ the head diameter + halo bleed
    const ent = viewer!.entities.add({
      position: Cesium.Cartesian3.fromDegrees(lon, lat, height),
      billboard: {
        image: 'data:image/svg+xml,' + encodeURIComponent(WP_GLOW_SVG),
        width: d, height: d,
        horizontalOrigin: Cesium.HorizontalOrigin.CENTER,
        verticalOrigin: Cesium.VerticalOrigin.CENTER,
        pixelOffset: new Cesium.Cartesian2(0, -headUpPx), // negative Y = up (onto the head)
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
        scale: new Cesium.CallbackProperty(() => 0.9 + 0.35 * wpPulse01(), false),
        color: new Cesium.CallbackProperty(() => Cesium.Color.WHITE.withAlpha(0.2 + 0.65 * wpPulse01()), false),
      },
    });
    missionEntities.push(ent);
  }

  /** Continuous rendering is needed while a pulse animates (active-WP glow OR a radar alert); otherwise
   *  fall back to on-demand (requestRenderMode) to spare the GPU. Both call sites use this so they agree. */
  function syncContinuousRender() {
    if (!viewer) return;
    let anyAlert = false;
    for (const rec of radar3dRecs.values()) if (rec.alertLevel) { anyAlert = true; break; }
    viewer.scene.requestRenderMode = !(wpPulseActive || anyAlert);
  }

  // Overlay mission line — a depth-test-free Primitive so it stays visible through terrain (like the
  // billboards) WITHOUT the z-fighting the old Entity dual-pass (material + depthFailMaterial) caused
  // against the globe. Drawn straight (ArcType.NONE — legs are short) and never writes depth.
  function missionLine(positions: Cesium.Cartesian3[], cssColor: string, alpha: number, width: number, dash: boolean) {
    if (!viewer || positions.length < 2) return;
    const color = Cesium.Color.fromCssColorString(cssColor).withAlpha(alpha);
    const material = dash
      ? Cesium.Material.fromType('PolylineDash', { color, dashLength: 16 })
      : Cesium.Material.fromType('Color', { color });
    const prim = new Cesium.Primitive({
      geometryInstances: new Cesium.GeometryInstance({
        geometry: new Cesium.PolylineGeometry({
          positions,
          width,
          arcType: Cesium.ArcType.NONE,
          vertexFormat: Cesium.PolylineMaterialAppearance.VERTEX_FORMAT,
        }),
      }),
      appearance: new Cesium.PolylineMaterialAppearance({
        material,
        translucent: true,
        // No depth test (always on top) and no depth write (doesn't occlude markers) → no terrain z-fight.
        // getDefaultRenderState builds a correct translucent render state; it exists at runtime but is
        // missing from the Cesium TS typings.
        // @ts-expect-error — getDefaultRenderState is untyped in the Cesium typings
        renderState: Cesium.Appearance.getDefaultRenderState(true, false, {
          depthTest: { enabled: false },
          depthMask: false,
        }),
      }),
      asynchronous: false, // build now → no one-frame flash on rebuild
    });
    viewer.scene.primitives.add(prim);
    missionPrimitives.push(prim);
  }

  function scheduleMissionRender() { void renderMission3D(); }

  // ── Unified mission overlay (INAV + ArduPilot) ──────────────────────
  // One renderer, one geoid path, one set of draw primitives. The two autopilots differ only in how
  // their mission model maps to geometry (INAV WpAction + alt_mode vs. ArduPilot MavCmd + frame), so
  // each has a thin *adapter* (buildInavModel / buildArduModel) that resolves its waypoints to a
  // protocol-neutral `Mission3DModel` in pure MSL. `renderMission3D` then draws any model identically —
  // exactly the 2D pattern (shared primitives + a per-platform mapper), now for 3D too.

  interface P3 { lat: number; lon: number; altMsl: number; }
  interface Mission3DWp {
    lat: number; lon: number; altMsl: number;
    ground: number | null;        // terrain MSL beneath the WP (drop-line target); null = no terrain
    spec: WpIconSpec;             // shared icon SVG spec (same source 2D uses)
    active: boolean;              // FC's current target WP → pulsing glow
    greyed: boolean;              // dimmed (beyond the mission end, INAV)
  }
  interface Mission3DLine { positions: P3[]; color: string; alpha: number; width: number; dashed: boolean; }
  interface Mission3DModel {
    wps: Mission3DWp[];
    lines: Mission3DLine[];       // flight path + jump/RTH/launch connectors, pre-styled
    launch: { lat: number; lon: number; groundMsl: number | null } | null; // orange "L" billboard (INAV)
    geoidRef: { lat: number; lon: number } | null; // reference point for the geoid offset
  }

  /** Draw a resolved mission model. Pure + synchronous: the geoid offset is applied here (the model is
   *  MSL), so the model build stays protocol-neutral and terrain/geoid handling lives in one place. */
  function drawMission3DModel(model: Mission3DModel) {
    if (!viewer) return;
    const toCart = (p: P3) => Cesium.Cartesian3.fromDegrees(p.lon, p.lat, p.altMsl + geoidOffset);

    // Lines (flight path, jump/RTH/launch connectors).
    for (const ln of model.lines) {
      if (ln.positions.length < 2) continue;
      missionLine(ln.positions.map(toCart), ln.color, ln.alpha, ln.width, ln.dashed);
    }

    // Launch "L" marker (the adapter already applied its visibility gating).
    if (model.launch) {
      const h = (model.launch.groundMsl ?? 0) + geoidOffset;
      missionBillboard(model.launch.lon, model.launch.lat, h, LAUNCH_SVG, 32, 44, 16, 44);
    }

    // Waypoints — drop-line to the ground, active-WP glow (behind), then the marker billboard.
    let anyActiveWp = false;
    for (const wp of model.wps) {
      const top = toCart(wp);
      if (wp.ground != null) {
        const bottom = toCart({ lat: wp.lat, lon: wp.lon, altMsl: wp.ground });
        missionLine([top, bottom], '#000000', 0.85, 3.5, true);  // outline
        missionLine([top, bottom], '#ffffff', 0.95, 1.5, true);  // white dashed
      }
      const h = wp.altMsl + geoidOffset;
      if (wp.active) { addActiveWpGlow(wp.lon, wp.lat, h, wp.spec); anyActiveWp = true; }
      missionBillboard(wp.lon, wp.lat, h, wp.spec.svg, wp.spec.width, wp.spec.height, wp.spec.anchorX, wp.spec.anchorY, wp.greyed ? 0.35 : 1);
    }

    wpPulseActive = anyActiveWp;
    // Recolour the violating legs in sync with the mission line (same geoidOffset + redraw timing) so
    // the red sits exactly on the path — not a stale overlay drawn before the geoid resolved.
    updateGeozoneViolations3D();
    syncContinuousRender(); // (de)activate continuous rendering for the pulse
    viewer.scene.requestRender();
  }

  async function renderMission3D() {
    if (!viewer) return;
    const token = ++missionRenderToken;

    // Build the protocol-neutral model (the only async part: terrain sampling). Replay → follow the
    // MISSION toggle; planning/live → always shown.
    const visible = !curReplayActive || curShowMission;
    const model = !visible
      ? null
      : curAutopilotSystem === 'ardupilot'
        ? await buildArduModel(token)
        : await buildInavModel(token);
    if (token !== missionRenderToken || !viewer) return; // superseded while building

    // The mission sits at `altMsl + geoidOffset`, so the offset must be ready before we draw.
    // computeGeoidOnce is single-flight + cached: a track/live fix may already be computing it.
    if (!geoidComputed && model?.geoidRef) {
      await computeGeoidOnce(model.geoidRef.lat, model.geoidRef.lon);
      if (token !== missionRenderToken || !viewer) return; // superseded while awaiting
    }

    // Skip the redraw when the result is visually identical to what's already on screen. Many triggers
    // (a jittering HOME re-broadcasting launchPoint, redundant store sets) ask for a re-render without
    // any real change; rebuilding identical entities makes the depth-tested polylines drop a frame
    // (the 5 s flicker). Quantised so sub-metre FC jitter doesn't count as a change.
    const sig = model ? missionModelSignature(model) : '';
    if (sig === lastMissionSig && missionEntities.length > 0) return;
    lastMissionSig = sig;

    // From here on it's synchronous: clear the old entities/primitives and draw the new ones in the SAME
    // frame, so the overlay never blanks between renders (no async gap — fixes the flicker on home ticks).
    for (const e of missionEntities) viewer.entities.remove(e);
    missionEntities = [];
    for (const p of missionPrimitives) viewer.scene.primitives.remove(p);
    missionPrimitives = [];
    if (!model) { wpPulseActive = false; syncContinuousRender(); viewer.scene.requestRender(); return; }
    drawMission3DModel(model);
  }

  /** A quantised fingerprint of the drawn model: positions to ~0.5 m, plus colours/flags/icon hashes.
   *  Identical fingerprint ⇒ nothing visible changed ⇒ skip the redraw (avoids polyline-rebuild flicker). */
  function missionModelSignature(m: Mission3DModel): string {
    const q = (n: number) => Math.round(n * 2) / 2;            // 0.5 m for heights
    const qc = (n: number) => Math.round(n * 2e5) / 2e5;       // ≈ 0.55 m for lat/lon
    const hash = (s: string) => { let h = 5381; for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0; return h; };
    const parts: string[] = [`g${q(geoidOffset)}`];
    for (const w of m.wps) parts.push(`w${qc(w.lat)},${qc(w.lon)},${q(w.altMsl)},${w.active ? 1 : 0},${w.greyed ? 1 : 0},${hash(w.spec.svg)}`);
    for (const ln of m.lines) {
      let lp = `l${ln.color}${ln.dashed ? 'd' : ''}${ln.width}`;
      for (const p of ln.positions) lp += `;${qc(p.lat)},${qc(p.lon)},${q(p.altMsl)}`;
      parts.push(lp);
    }
    if (m.launch) parts.push(`L${qc(m.launch.lat)},${qc(m.launch.lon)},${q(m.launch.groundMsl ?? 0)}`);
    return parts.join('|');
  }

  // ── INAV mission adapter ────────────────────────────────────────────
  /** Signature of the inputs that drive resolveMissionAltitudes (positions + alt-frame + launch). */
  function inavAltInputSig(wps: Waypoint[], launch: { lat: number; lng: number } | null): string {
    const parts: string[] = [launch ? `L${launch.lat.toFixed(7)},${launch.lng.toFixed(7)}` : 'L-'];
    for (let i = 0; i < wps.length; i++) {
      const w = wps[i];
      if (!hasLocation(w.action) || (w.lat === 0 && w.lon === 0)) continue;
      parts.push(`${i}:${w.lat},${w.lon},${w.altitude},${w.alt_mode ?? -1},${w.p3}`);
    }
    return parts.join('|');
  }

  async function buildInavModel(token: number): Promise<Mission3DModel | null> {
    const wps = curMission.waypoints;
    const firstGeoIdx = wps.findIndex((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
    if (firstGeoIdx < 0) return null;

    // Reuse the cached terrain resolution when the altitude-relevant inputs are unchanged (e.g. a 3D
    // re-open) — only the cheap model build below re-runs. Otherwise resolve + cache.
    const altSig = inavAltInputSig(wps, curLaunch);
    let alts: Map<number, WpMsl>;
    let launchGround: number | null;
    if (inavAltCache && inavAltCache.sig === altSig) {
      ({ alts, launchGround } = inavAltCache);
    } else {
      ({ alts, launchGround } = await resolveMissionAltitudes(wps, curLaunch));
      if (token !== missionRenderToken) return null;
      inavAltCache = { sig: altSig, alts, launchGround };
    }

    const wpMsl = (i: number): P3 | null => {
      const a = alts.get(i);
      return a ? { lat: toDeg(wps[i].lat), lon: toDeg(wps[i].lon), altMsl: a.altMsl } : null;
    };

    const displayNums = buildDisplayNumbers(wps);
    const endIdx = findMissionEndIndex(wps);
    const model: Mission3DModel = {
      wps: [], lines: [], launch: null,
      geoidRef: { lat: toDeg(wps[firstGeoIdx].lat), lon: toDeg(wps[firstGeoIdx].lon) },
    };

    // Launch → first-waypoint connector (orange dashed) + the "L" marker (live manual reference only;
    // hidden when FC-locked — the green "H" represents the same point — and in offline planning).
    if (curLaunch) {
      const fp = wpMsl(firstGeoIdx);
      if (fp) {
        const launchP: P3 = { lat: curLaunch.lat, lon: curLaunch.lng, altMsl: launchGround ?? 0 };
        model.lines.push({ positions: [launchP, fp], color: '#f39c12', alpha: 0.7, width: 2, dashed: true });
        if (!get(homeLocked) && get(connection).status === 'connected') {
          model.launch = { lat: curLaunch.lat, lon: curLaunch.lng, groundMsl: launchGround ?? 0 };
        }
      }
    }

    const fpActive: P3[] = [];
    const fpGreyed: P3[] = [];
    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      if (!hasLocation(wp.action) || (wp.lat === 0 && wp.lon === 0)) {
        // Jump / RTH connector lines (origin = the previous geo waypoint).
        if (wp.action === WpAction.Jump && wp.p1 > 0) {
          const src = findPreviousGeoWp(wps, i);
          const tgtIdx = wp.p1 - 1;
          const srcIdx = src ? wps.indexOf(src) : -1;
          const a = srcIdx >= 0 ? wpMsl(srcIdx) : null;
          const b = hasLocation(wps[tgtIdx]?.action) ? wpMsl(tgtIdx) : null;
          if (a && b) model.lines.push({ positions: [a, b], color: '#8e44ad', alpha: 0.8, width: 2, dashed: true });
        }
        if (wp.action === WpAction.Rth) {
          const src = findPreviousGeoWp(wps, i);
          const srcIdx = src ? wps.indexOf(src) : -1;
          const firstFp = wps.findIndex((w) => isFlightPathWp(w.action) && !(w.lat === 0 && w.lon === 0));
          const a = srcIdx >= 0 ? wpMsl(srcIdx) : null;
          const b = firstFp >= 0 ? wpMsl(firstFp) : null;
          if (a && b) model.lines.push({ positions: [a, b], color: '#e67e22', alpha: 0.7, width: 2, dashed: true });
        }
        continue;
      }
      const p = wpMsl(i);
      if (!p) continue;
      const a = alts.get(i);
      const greyed = endIdx >= 0 && i > endIdx;
      if (isFlightPathWp(wp.action)) {
        if (!greyed) fpActive.push(p);
        else {
          if (fpGreyed.length === 0 && fpActive.length > 0) fpGreyed.push(fpActive[fpActive.length - 1]);
          fpGreyed.push(p);
        }
      }
      model.wps.push({
        lat: p.lat, lon: p.lon, altMsl: p.altMsl, ground: a?.ground ?? null,
        spec: wpIconSpec(wp, displayNums.get(i) ?? 0, false),
        active: !greyed && curActiveWp3d > 0 && wp.number === curActiveWp3d,
        greyed,
      });
    }

    if (fpActive.length > 1) model.lines.push({ positions: fpActive, color: '#37a8db', alpha: 0.8, width: 3, dashed: false });
    if (fpGreyed.length > 1) model.lines.push({ positions: fpGreyed, color: '#666666', alpha: 0.4, width: 2, dashed: true });
    return model;
  }

  // ── ArduPilot mission adapter ───────────────────────────────────────
  // The location/altitude resolution is ArduPilot-specific (MavCmd + per-WP frame, takeoff anchoring);
  // everything visual then flows through the shared model + renderer.

  /** Takeoff carries no real coords — anchor it on FC home, else the centroid of the located waypoints. */
  function arduTakeoffLatLon3d(wps: ArduWaypoint[]): { lat: number; lon: number } | null {
    // Only the authoritative FC home — a 'manual' home is the stale INAV-launch mirror (often another
    // region / sea-level alt) and would put the REL base in the wrong place → WPs sinking into terrain.
    if (curHome3d.set && curHome3d.source === 'fc') return { lat: curHome3d.lat, lon: curHome3d.lon };
    // Offline the takeoff WP is ArduPilot's launch reference: prefer a positioned one, else the centroid.
    const tk = wps.find((w) => cmdIsTakeoff(w.command) && !(w.lat === 0 && w.lon === 0));
    if (tk) return { lat: tk.lat / 1e7, lon: tk.lon / 1e7 };
    let sLat = 0, sLon = 0, n = 0;
    for (const w of wps) {
      if (cmdIsTakeoff(w.command) || !cmdHasLocation(w.command)) continue;
      if (w.lat === 0 && w.lon === 0) continue;
      sLat += w.lat / 1e7; sLon += w.lon / 1e7; n++;
    }
    return n > 0 ? { lat: sLat / n, lon: sLon / n } : null;
  }

  function arduWpLatLon3d(wp: ArduWaypoint, wps: ArduWaypoint[]): { lat: number; lon: number } | null {
    if (cmdIsTakeoff(wp.command)) {
      // Offline the operator can position the takeoff freely (stored coords win) — mirror the 2D layer.
      // Connected, it anchors on the FC home (the real takeoff point). See ArduMissionLayer.wpDisplayLatLng.
      const conn = get(connection).status === 'connected';
      if (!conn && !(wp.lat === 0 && wp.lon === 0)) return { lat: wp.lat / 1e7, lon: wp.lon / 1e7 };
      return arduTakeoffLatLon3d(wps);
    }
    if (wp.lat === 0 && wp.lon === 0) return null;
    return { lat: wp.lat / 1e7, lon: wp.lon / 1e7 };
  }

  interface ArduWpAlt { altMsl: number; ground: number | null; lat: number; lon: number; }

  /** Resolve each ArduPilot waypoint to MSL using its altitude frame (AMSL = value, REL = home-MSL +
   *  value, TERRAIN = terrain + value) and sample the terrain ground beneath it for the drop-line. */
  async function resolveArduAltitudes3d(wps: ArduWaypoint[], homeRefMsl: number | null): Promise<Map<number, ArduWpAlt>> {
    const out = new Map<number, ArduWpAlt>();

    // Collect every locatable waypoint, then sample all terrain grounds in one batched
    // IPC call instead of one round-trip per waypoint (the dominant 3D-overlay cost).
    const located: { i: number; lat: number; lon: number }[] = [];
    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      if (!cmdHasLocation(wp.command) && !cmdStandaloneCoordinate(wp.command)) continue;
      const ll = arduWpLatLon3d(wp, wps);
      if (!ll) continue;
      located.push({ i, lat: ll.lat, lon: ll.lon });
    }

    const grounds = located.length > 0
      ? await invoke<(number | null)[]>('terrain_elevations', { points: located.map((p) => [p.lat, p.lon]) })
      : [];

    for (let k = 0; k < located.length; k++) {
      const { i, lat, lon } = located[k];
      const ground = grounds[k] ?? null;
      const wp = wps[i];
      let altMsl: number;
      if (wp.frame === MAV_FRAME_GLOBAL) altMsl = wp.alt;                                          // AMSL
      else if (wp.frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) altMsl = (ground ?? homeRefMsl ?? 0) + wp.alt; // TERRAIN
      else altMsl = (homeRefMsl ?? 0) + wp.alt;                                                    // REL
      out.set(i, { altMsl, ground, lat, lon });
    }
    return out;
  }

  /** Signature of the inputs that drive the ArduPilot altitude resolution (positions + frames + home + link). */
  function arduAltInputSig(wps: ArduWaypoint[]): string {
    const h = curHome3d;
    const conn = get(connection).status === 'connected' ? 1 : 0;
    const parts: string[] = [`H${h.set ? 1 : 0},${h.source},${h.lat.toFixed(7)},${h.lon.toFixed(7)},${h.alt}`, `C${conn}`];
    for (let i = 0; i < wps.length; i++) {
      const w = wps[i];
      parts.push(`${i}:${w.command},${w.frame},${w.lat},${w.lon},${w.alt}`);
    }
    return parts.join('|');
  }

  async function buildArduModel(token: number): Promise<Mission3DModel | null> {
    const wps = curArduMission;
    const hasGeo = wps.some((w) => cmdHasLocation(w.command) && (cmdIsTakeoff(w.command) || !(w.lat === 0 && w.lon === 0)));
    if (!hasGeo) return null;

    const g = wps.find((w) => cmdHasLocation(w.command) && !cmdIsTakeoff(w.command) && !(w.lat === 0 && w.lon === 0));
    const geoidRef = g ? { lat: g.lat / 1e7, lon: g.lon / 1e7 } : (curHome3d.set ? { lat: curHome3d.lat, lon: curHome3d.lon } : null);

    // Reuse the cached terrain resolution (home-ref + per-WP grounds) when the altitude-relevant inputs
    // are unchanged (e.g. a 3D re-open); only the cheap model build below re-runs. Otherwise resolve.
    const altSig = arduAltInputSig(wps);
    let alts: Map<number, ArduWpAlt>;
    let homeRefMsl: number | null;
    if (arduAltCache && arduAltCache.sig === altSig) {
      ({ alts, homeRefMsl } = arduAltCache);
    } else {
      // REL reference: the FC home MSL only when it's the authoritative FC home — a 'manual' home (the
      // stale INAV-launch mirror, alt ≈ 0) would anchor REL altitudes at sea level → WPs sink into the
      // ground. Offline we therefore sample the terrain under the takeoff/first waypoint instead.
      homeRefMsl = (curHome3d.set && curHome3d.source === 'fc') ? curHome3d.alt : null;
      if (homeRefMsl == null) {
        const anchor = arduTakeoffLatLon3d(wps);
        if (anchor) {
          homeRefMsl = await invoke<number | null>('terrain_elevation', { lat: anchor.lat, lon: anchor.lon });
          if (token !== missionRenderToken) return null;
        }
      }
      alts = await resolveArduAltitudes3d(wps, homeRefMsl);
      if (token !== missionRenderToken) return null;
      arduAltCache = { sig: altSig, alts, homeRefMsl };
    }

    const wpMsl = (i: number): P3 | null => {
      const a = alts.get(i);
      return a ? { lat: a.lat, lon: a.lon, altMsl: a.altMsl } : null;
    };

    const model: Mission3DModel = { wps: [], lines: [], launch: null, geoidRef };
    const fp: P3[] = [];
    const fpIdx: number[] = [];

    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      if (cmdHasLocation(wp.command) || cmdStandaloneCoordinate(wp.command)) {
        const a = alts.get(i);
        const p = a ? wpMsl(i) : null;
        if (a && p) {
          if (cmdHasLocation(wp.command)) { fp.push(p); fpIdx.push(i); }
          const displayNum = i + 1;
          model.wps.push({
            lat: p.lat, lon: p.lon, altMsl: p.altMsl, ground: a.ground,
            spec: arduWpIconSpec(wp, displayNum, false),
            active: curActiveWp3d > 0 && displayNum === curActiveWp3d,
            greyed: false,
          });
        }
      }

      // DO_JUMP connector (previous located WP → target), purple dashed — mirrors the 2D layer.
      if (wp.command === MAV_CMD_DO_JUMP && wp.param1 > 0) {
        const tIdx = Math.round(wp.param1) - 1;
        let prevLocIdx = -1;
        for (let k = i - 1; k >= 0; k--) { if (cmdHasLocation(wps[k].command)) { prevLocIdx = k; break; } }
        const a = prevLocIdx >= 0 ? wpMsl(prevLocIdx) : null;
        const b = wps[tIdx] && cmdHasLocation(wps[tIdx].command) ? wpMsl(tIdx) : null;
        if (a && b) model.lines.push({ positions: [a, b], color: '#8e44ad', alpha: 0.8, width: 2, dashed: true });
      }
    }

    // Flight-path legs (blue); a leg touching a takeoff is dashed (its position is an anchor estimate).
    for (let s = 0; s < fp.length - 1; s++) {
      const loose = cmdIsTakeoff(wps[fpIdx[s]]?.command) || cmdIsTakeoff(wps[fpIdx[s + 1]]?.command);
      model.lines.push({ positions: [fp[s], fp[s + 1]], color: '#37a8db', alpha: 0.8, width: 3, dashed: loose });
    }

    return model;
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
    const color = getNavStateColor(point.nav_state ?? 0); // marker = nav state
    const cesiumColor = Cesium.Color.fromCssColorString(color);
    const position = Cesium.Cartesian3.fromDegrees(lon, lat, alt);
    // Attitude from the SAME unified adapter the AHI widget uses (consistent across
    // INAV / ArduPilot / live / replay) rather than the raw record. NB: the model heading is the FC
    // fused HEADING (`td.yaw`), NOT the GPS course (`point.heading` = COG) — so the model/FPV/camera
    // show the real crab against the track instead of riding it like rails.
    const td = toTelemetryData(point, fcVariant);
    const heading = td.yaw;
    const orientation = uavOrientation(position, heading, td.pitch, td.roll);

    // FPV HUD data (replay source).
    hud.heading = heading; hud.pitch = td.pitch; hud.roll = td.roll;
    hud.altM = point.nav_alt_m ?? point.baro_alt_m ?? 0;
    hud.speedMs = point.speed_ms ?? 0;
    {
      const fv = flightPathVector(td.groundSpeed, td.vario, td.course, heading);
      hud.fpmGamma = fv.gamma; hud.fpmCrab = fv.crab; hud.fpmShown = fv.shown;
    }

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
  const LOCK_ZOOM_MAX = 1500; // max zoom-out distance to the UAV (m)

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
      fpmGamma={hud.fpmGamma}
      fpmCrab={hud.fpmCrab}
      fpmShown={hud.fpmShown}
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
