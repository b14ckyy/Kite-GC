<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { connection, availablePorts, bleDevices } from "$lib/stores/connection";
  import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import { telemetry } from "$lib/stores/telemetry";
  import { startRadarListeners, configureRadar, setRadarCenter, setRadarNode } from "$lib/stores/radarTracking";
  import { startRadarAlerts } from "$lib/controllers/radarAlerts";
  import { startAlertAudio } from "$lib/controllers/alertAudio";
  import { get } from "svelte/store";
  import { t, locale } from 'svelte-i18n';
  import Map from "$lib/components/Map.svelte";
  import Map3D from "$lib/components/Map3D.svelte";
  import LogPlayer from "$lib/components/logbook/LogPlayer.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import BatchEditPopup from "$lib/components/mission/BatchEditPopup.svelte";
  import type { DialogButton, DialogOptions } from "$lib/components/ConfirmDialog.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import RelayPanel from "$lib/components/RelayPanel.svelte";
  import WindowResizeBorders from "$lib/components/WindowResizeBorders.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import NavRail from "$lib/components/NavRail.svelte";
  import PanelPlayground from "$lib/components/panel/PanelPlayground.svelte";
  import UavInfoPanel from "$lib/components/UavInfoPanel.svelte";
  import LogbookPanel from "$lib/components/logbook/LogbookPanel.svelte";
  import MissionPanel from "$lib/components/mission/MissionPanel.svelte";
  import MavCommandPanel from "$lib/components/control/MavCommandPanel.svelte";
  import VideoPanel from "$lib/components/video/VideoPanel.svelte";
  import RadarPanel from "$lib/components/RadarPanel.svelte";
  import AirspaceManagerPanel from "$lib/components/AirspaceManagerPanel.svelte";
  import RadarAlertBanner from "$lib/components/RadarAlertBanner.svelte";
  import StatusTextToasts from "$lib/components/StatusTextToasts.svelte";
  import { startStatusText } from "$lib/stores/statusText";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import { ensureUserLocation, requestUserLocation, userGeoLocation } from "$lib/helpers/userLocation";
  import { gcsLocation } from "$lib/stores/gcsLocation";
  import { fetchAero } from "$lib/stores/airspace";
  import { PlaybackController } from '$lib/controllers/playbackController';
  import { refreshSerialPorts, connectFC, disconnectFC, startBleScan, stopBleScan, startBleDeviceListener, stopBleDeviceListener, clearBleDevices } from '$lib/controllers/connectionController';
  import * as logbookCtrl from '$lib/controllers/logbookController';
  import * as widgetCtrl from '$lib/controllers/widgetController';
  import { isValidGpsCoordinate, isArmed } from '$lib/helpers/telemetry';
  import { liveTrack, appendLivePoint, clearLiveTrack } from '$lib/stores/liveTrack';
  import { toTelemetryData } from '$lib/adapters/telemetryAdapter';
  import { activeWpNumber, replayWpTotal } from '$lib/stores/navStatus';
  import { missionManagerOpen, missionManagerSelectedId, requestOpenFlightId, requestOpenMissionId } from '$lib/stores/missionManager';
  import { batteryManagerOpen } from '$lib/stores/batteryManager';
  import { missionDbForFlight, flightLoggedWpCount, missionDbSave, flightLinkMission, missionDbGeocode, flightSetBatterySerial, updateFlightNotes, getFlight, flightlogCommitPending, flightlogDiscardPending, flightlogContinuePending, scanOrphanSessions, recoverDiscard, recoverSaveIncomplete, recoverContinue, batteryDbFindBySerial, batteryDbAddUsage } from '$lib/stores/flightlog';
  import EndFlightDialog from "$lib/components/logbook/EndFlightDialog.svelte";
  import type { EndFlightStats } from "$lib/components/logbook/EndFlightDialog.svelte";
  import RecoveryPrompt from "$lib/components/logbook/RecoveryPrompt.svelte";
  import DisconnectArmedDialog from "$lib/components/logbook/DisconnectArmedDialog.svelte";
  import { haversineDistance, bearing, destinationPoint } from '$lib/utils/geo';
  import { buildMissionInput } from '$lib/helpers/missionLibrary';
  import { buildArduMissionInput } from '$lib/helpers/missionLibraryArdu';
  import { homePosition } from '$lib/stores/home';
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import { weatherTempDisplayFromC, weatherWindDisplayFromMs, weatherTempCFromDisplay, weatherWindMsFromDisplay, canonicalWeatherDescription } from "$lib/helpers/weather";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import { LARGE_BASE_VMIN } from "$lib/config/widgetRegistry";
  import FloatingVideoWindow from "$lib/components/video/FloatingVideoWindow.svelte";
  import { initVideo, videoState, videoStream, setVideoPrimary, registerPiPElement } from "$lib/stores/video";
  import TerrainAnalysisPanel from "$lib/components/terrain/TerrainAnalysisPanel.svelte";
  import { editMode, replayActive, mission, missionFlags, missionDownload, missionUpload, missionFcInfo, markMissionSynced, loadedMissionId, missionSetWaypoints, launchPoint, hasLocation, toDeg, type Waypoint } from "$lib/stores/mission";
  import { pendingSystemSwitch, autopilotSystem, setAutopilotSystem, confirmSystemSwitch } from "$lib/stores/autopilotContext";
  import { arduMission, arduSelectedWpIndex, arduLoadedMissionId, type ArduWaypoint } from "$lib/stores/missionArdupilot";
  import { terrainAnalysis, patchTerrainAnalysis } from "$lib/stores/terrainAnalysis";
  import { DEFAULT_RADAR, DEFAULT_AIRSPACE, BUILTIN_ADSB_PROVIDERS } from "$lib/stores/settings";
  import type { AppSettings, InterfaceSettings, PanelConfig, RadarSettings, GcsMode, AirspaceSettings, SystemMessagesLevel } from "$lib/stores/settings";
  import { layout, GRID_DEFAULTS } from '$lib/stores/layout';
  import {
    getDefaultFlightlogPath,
    getDefaultRawLogPath,
    type BlackboxImportProgress,
    type Flight,
    type FlightSummary,
    type TelemetryRecord,
  } from "$lib/stores/flightlog";
  import type { TrackColorMode } from "$lib/helpers/trackColors";
  import type { UavModelOverride } from "$lib/helpers/uavIcons";
  import { modeCategory } from "$lib/helpers/flightModeRegistry";

  // ── Layout zone CSS custom properties (driven by layout store) ──
  const gridBottomHeight = $derived(
    $layout.bottomDock.sizeOverride ?? GRID_DEFAULTS.bottomDockHeight
  );
  const gridSideWidth = $derived(
    $layout.sideDock.sizeOverride ?? GRID_DEFAULTS.sideDockWidth
  );

  // Map view mode: 2D (Leaflet) or 3D (CesiumJS)
  let mapViewMode = $state<'2d' | '3d'>('2d');
  // 3D is expensive to spin up (Cesium viewer + terrain). Mount it lazily on the
  // first switch to 3D, then KEEP it mounted (hidden behind the 2D map) so further
  // toggles are instant — no viewer re-init. The Map3D `active` prop pauses its
  // render loop while hidden.
  let map3dEverOpened = $state(false);
  $effect(() => { if (mapViewMode === '3d') map3dEverOpened = true; });
  // Waypoints can only be edited on the 2D map → entering edit mode forces 2D (untracked read/write so
  // toggling the view later doesn't re-trigger this; it reacts to the edit-mode transition only).
  $effect(() => { if ($editMode) untrack(() => { if (mapViewMode === '3d') mapViewMode = '2d'; }); });
  // Map3D instance handle — used to read the 3D camera focus on a 3D→2D switch so
  // the 2D map can re-centre on the same spot (keeping its own zoom).
  let map3dRef: {
    getCamFocus?: () => { lat: number; lon: number; range: number } | null;
    getCamSubpoint?: () => { lat: number; lon: number } | null;
    getCamGeo?: () => { sub: { lat: number; lon: number }; focus: { lat: number; lon: number } | null; headingDeg: number } | null;
    isFreeLook?: () => boolean;
  } | undefined = $state();
  // 2D follow state, lifted here so it survives the 2D map's remount on each 2D↔3D toggle
  // (the 3D camera mode persists on its own since Map3D stays mounted).
  let map2dViewMode = $state<'free' | 'follow' | 'heading-follow'>('free');

  function toggleMapView() {
    if (mapViewMode === '3d') {
      // 3D → 2D: hand the spot the 3D camera looks at to the 2D map (its zoom stays).
      const f = map3dRef?.getCamFocus?.();
      if (f) {
        const s = get(settings);
        settings.patch({ map: { center: [f.lat, f.lon], zoom: s.map.zoom } });
      }
      mapViewMode = '2d';
    } else {
      map3dEverOpened = true;
      mapViewMode = '3d';
    }
  }

  // Measured container dimensions (bind:clientWidth/Height on grid zones)
  let bottomDockW = $state(800);
  let bottomDockH = $state(200);
  let sideDockW = $state(200);
  let sideDockH = $state(400);

  // Viewport size (for the snapped floating-video reserve)
  let winW = $state(typeof window !== 'undefined' ? window.innerWidth : 1280);
  let winH = $state(typeof window !== 'undefined' ? window.innerHeight : 720);
  // Width the bottom dock must yield to the bottom-left snapped video window.
  const videoReserve = $derived(
    $videoState.floating && $videoState.floatSnapped
      ? Math.min($videoState.floatHeightFrac * winH * ($videoState.aspect || 16 / 9), winW * 0.7) + 16
      : 0,
  );

  // Map-swap: the full-size video sink shown in the map zone when videoPrimary.
  let mapVideoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    if (mapVideoEl) mapVideoEl.srcObject = $videoStream;
  });

  // Persistent (always-mounted) source element for native Picture-in-Picture, so
  // the PiP window survives closing the Video panel. Hidden but rendered/playing.
  let pipVideoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    if (pipVideoEl) {
      pipVideoEl.srcObject = $videoStream;
      registerPiPElement(pipVideoEl);
    }
  });

  // Global UI scale (1 = 100%, up to 2). Zooms the chrome via `.ui-scale`; the map
  // (`.layer-map`) stays unzoomed/native. See docs/archive/UI_SCALING.md.
  let uiScale = $state(1);

  // Floating-window rect (must match FloatingVideoWindow's own computation) — used
  // to place the map inside the window's frame when the view is swapped. The window
  // lives in the zoomed `.ui-scale` layer but the map is unzoomed, so the visual rect
  // is the window's logical rect * uiScale.
  const FLOAT_HDR = 20; // floating-window header height
  const floatH = $derived(Math.round($videoState.floatHeightFrac * winH));
  const floatW = $derived(Math.min(Math.round(floatH * ($videoState.aspect || 16 / 9)), Math.round(winW * 0.7)));
  const floatLeft = $derived($videoState.floatSnapped ? 8 : $videoState.floatX);
  const floatTop = $derived($videoState.floatSnapped ? winH - floatH - 30 : $videoState.floatY);
  // When video is primary, the map occupies the window's body (below the header).
  const mapInFrame = $derived($videoState.videoPrimary && $videoState.status === 'live');
  const mapFrameStyle = $derived(
    `left:${floatLeft * uiScale}px; top:${(floatTop + FLOAT_HDR) * uiScale}px; width:${floatW * uiScale}px; height:${(floatH - FLOAT_HDR) * uiScale}px;`,
  );

  // Per-container px-per-unit: 1 unit = cross-axis fraction so that
  // LARGE_BASE_VMIN units == cross-axis px (widget fills dock height/width).
  // This fully decouples bottom dock and side dock scaling.
  // Subtract zone padding (6px each side) from cross-axis measurement.
  const DOCK_PAD = 6;
  const bottomPxPerUnit = $derived((bottomDockH - 2 * DOCK_PAD) / LARGE_BASE_VMIN);
  const sidePxPerUnit   = $derived((sideDockW  - 2 * DOCK_PAD) / LARGE_BASE_VMIN);

  // Available space expressed in abstract units (container px / pxPerUnit)
  // Bottom: subtract edit button (28px) + wrapper gap (6px) + zone padding (12px)
  const bottomAvailUnits = $derived(Math.max(0, (bottomDockW - 34 - 2 * DOCK_PAD - videoReserve) / bottomPxPerUnit));
  const rightAvailUnits  = $derived(Math.max(0, (sideDockH - 2 * DOCK_PAD) / sidePxPerUnit));

  let appVersion = $state("...");
  let selectedTransport = $state<TransportType>('serial');
  let selectedProtocol = $state<ProtocolType>('msp');
  let selectedPort = $state("");
  let selectedBaud = $state(115200);
  let tcpHost = $state("192.168.1.1");
  let tcpPort = $state(5761);
  let selectedBleDevice = $state("");
  let bleDeviceList = $state<BleDeviceInfo[]>([]);
  let isBleScanning = $state(false);
  let isConnecting = $state(false);
  let errorMsg = $state("");
  let navPanelOpen = $state(false);
  let activeTab = $state("uav-info");
  // Telemetry Relay dropdown (under the connection bar).
  let relayPanelOpen = $state(false);

  // Terrain Analysis overlay (NavRail-triggered, full-width over the map)
  let terrainOpen = $state(false);
  terrainAnalysis.subscribe((s) => { terrainOpen = s.open; });

  // Dev-only debug panel (tree-shaken in production builds)
  const DEV_MODE = import.meta.env.DEV;
  let debugOpen = $state(false);
  let DebugPanelCmp: any = $state(null);
  if (DEV_MODE) {
    import('$lib/components/DebugPanel.svelte').then(m => { DebugPanelCmp = m.default; });
  }

  // Reactive telemetry subscription
  let liveTelem = $state(get(telemetry));
  let prevArmed = false;
  // Seed `prevArmed` from the FIRST valid telemetry frame of each connection so a reconnect mid-flight
  // (already armed) is NOT seen as a disarmed→armed edge — the home/launch marker must stay put on
  // reconnect and only move on a genuine arm transition observed live. Reset on each fresh connect.
  let armEdgeInit = false;
  // Live flight-stats accumulator (armed period) — drives the End-Flight summary when there is
  // no DB recording (the recorded case reads the finalized stats from the flight row instead).
  let armStartMs = 0;
  let accMaxAlt = 0, accMaxSpeed = 0, accMaxDist = 0, accMah = 0;
  let accStartLat: number | null = null, accStartLon: number | null = null;
  telemetry.subscribe((t) => {
    liveTelem = t;
    // Accumulate the live flown track (RAM) for the Terrain Analyzer
    const armed = isArmed(t.armingFlags, t.lastUpdate);
    // Baseline the armed state on the first real frame after (re)connect → no false edge on reconnect.
    if (!armEdgeInit && t.lastUpdate > 0) { armEdgeInit = true; prevArmed = armed; }
    if (armed && !prevArmed) {
      clearLiveTrack();
      // reset the flight-stats accumulator for the new flight
      armStartMs = t.lastUpdate || Date.now();
      accMaxAlt = 0; accMaxSpeed = 0; accMaxDist = 0; accMah = 0;
      accStartLat = null; accStartLon = null;
      endFlightDialog?.close(); // re-arming dismisses a lingering End-Flight dialog
      // warm the Copernicus tile for the current area so it's ready
      if (isValidGpsCoordinate(t.lat, t.lon)) {
        void invoke('terrain_elevation', { lat: t.lat, lon: t.lon }).catch(() => {});
      }
    }
    if (armed) {
      if (t.altitude > accMaxAlt) accMaxAlt = t.altitude;
      if (t.groundSpeed > accMaxSpeed) accMaxSpeed = t.groundSpeed;
      if (t.mAhDrawn > accMah) accMah = t.mAhDrawn;
      if (isValidGpsCoordinate(t.lat, t.lon)) {
        if (accStartLat == null) { accStartLat = t.lat; accStartLon = t.lon; }
        else {
          const d = haversineDistance(accStartLat, accStartLon as number, t.lat, t.lon);
          if (d > accMaxDist) accMaxDist = d;
        }
      }
    }
    // Require a known flight mode before recording a track point: a GPS frame can arrive before the
    // first post-handshake HEARTBEAT, and appending then bakes a grey "unknown-mode" leading segment
    // into the (immutable) live track — visible as a grey start of the 3D trail until the next mode.
    if (armed && isValidGpsCoordinate(t.lat, t.lon) && t.flightMode.primary) {
      appendLivePoint(t.lat, t.lon, t.altMsl, t.flightMode.primary, t.lastUpdate || Date.now());
    }
    // Home on arm: the FC sets home at the launch point. Authoritative (locked green "H") when
    // connected via MSP/MAVLink; otherwise (future telemetry-only tracking) seed the manual launch
    // reference once from the current fix (mirrored into a manual home below → the widget points to it).
    if (armed && !prevArmed && t.fixType >= 2 && isValidGpsCoordinate(t.lat, t.lon)) {
      if (get(connection).status === 'connected') {
        homePosition.set({ lat: t.lat, lon: t.lon, alt: t.altitude, set: true, source: 'fc' });
        launchPoint.set({ lat: t.lat, lng: t.lon });
      } else if (get(homePosition).source !== 'fc') {
        launchPoint.set({ lat: t.lat, lng: t.lon });
      }
    }
    if (!armed && prevArmed) {
      void handleDisarm(t.lastUpdate || Date.now());
    }
    prevArmed = armed;
  });

  // Mirror the manual launch reference into the Home store so the Home widget points at the
  // draggable "L" marker when there is no authoritative FC home. Skipped when home is FC-locked or
  // a replay (those own the Home store); never downgrades an FC home.
  launchPoint.subscribe((lp) => {
    if (!lp) return;
    // Never mirror an invalid / 0,0 launch into Home: before a GPS fix the launch auto-place can fall
    // back to the map centre (≈ 0,0), which would otherwise light up the Home widget with a bogus
    // ~5800 km distance to null island. Home stays unset until a real reference (FC home on arm, a
    // valid manual placement, or replay) exists.
    if (!isValidGpsCoordinate(lp.lat, lp.lng)) return;
    const h = get(homePosition);
    if (h.source === 'fc' || h.source === 'replay') return;
    if (h.set && h.lat === lp.lat && h.lon === lp.lng) return;
    homePosition.set({ lat: lp.lat, lon: lp.lng, alt: h.alt, set: true, source: 'manual' });
  });

  // On disarm: show the End-Flight summary. When DB recording is on, the
  // flight-recording-ended listener shows the full (editable) dialog instead.
  async function handleDisarm(disarmMs: number): Promise<void> {
    const durationSec = armStartMs ? Math.round((disarmMs - armStartMs) / 1000) : 0;
    if (durationSec < 5) return; // ignore trivial bench arm/disarm
    if (flightLoggingEnabled && flightRecordingEnabled) return; // recorded → handled on -ended
    try {
      await endFlightDialog.show({
        stats: {
          durationSec,
          maxAltM: accMaxAlt || null,
          maxSpeedMs: accMaxSpeed || null,
          maxDistM: accMaxDist || null,
          batteryUsedMah: accMah || null,
        },
        recorded: false,
      });
    } catch (e) {
      console.warn('[end-flight] summary dialog failed', e);
    }
  }

  // Switch default baud rate when protocol changes
  // Track previous protocol to detect actual user-initiated changes
  // svelte-ignore state_referenced_locally
  let prevProtocol = $state(selectedProtocol);
  $effect(() => {
    if (selectedProtocol !== prevProtocol) {
      prevProtocol = selectedProtocol;
      if (selectedProtocol === 'mavlink') {
        selectedBaud = 57600;
      } else {
        selectedBaud = 115200;
      }
    }
  });
  bleDevices.subscribe((d) => {
    bleDeviceList = d;
    // Auto-select the first discovered device while scanning (no manual pick yet).
    if (d.length > 0 && !selectedBleDevice) selectedBleDevice = d[0].id;
  });

  // Settings state for the settings panel
  let attitudeRateHz = $state(5);
  let positionRateHz = $state(2);
  let airspeedEnabled = $state(false);
  let mavlinkFullTelemetry = $state(false);
  let flightLoggingEnabled = $state(false);
  let flightRecordingEnabled = $state(false);
  let flightLogDbPath = $state("");
  let flightLogRawPath = $state("");
  let flightLogRawEnabled = $state(false);
  let flightLogRawAlways = $state(false);
  let defaultFlightLogPath = $state("");
  let defaultRawLogPath = $state("");
  let mapProvider = $state("osm");
  let mapCacheMaxMB = $state(200);
  let cesiumIonToken = $state("");
  let altitudeCurtain3D = $state(true);
  let realLighting3D = $state(false);
  let logReplayTime = $state(false);
  let nightMode2D = $state<'off' | 'auto' | 'on'>('off');
  let gcsMode = $state<GcsMode>('manual');
  let radarSettings = $state<RadarSettings>({ ...DEFAULT_RADAR });
  let airspaceSettings = $state<AirspaceSettings>({ ...DEFAULT_AIRSPACE });
  let defaultWpAltitudeM = $state(50);
  let defaultPhTimeSec = $state(30);
  let warnAltitudeM = $state(120);
  let systemMessages = $state<SystemMessagesLevel>('all');
  let interfaceSettings = $state<InterfaceSettings>({
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  });
  let trackColorMode = $state<TrackColorMode>('flightmode');
  let modelOverride = $state<UavModelOverride>('auto'); // 3D UAV-model override (Replay control)

  // Logbook state
  let logbookLoading = $state(false);
  let blackboxImporting = $state(false);
  let blackboxImportProgress = $state<BlackboxImportProgress | null>(null);
  let flightSummaries = $state<FlightSummary[]>([]);
  let selectedFlight: Flight | null = $state(null);
  let selectedFlightTrack = $state<TelemetryRecord[]>([]);
  let selectedFlightTrackCount = $state(0);
  let selectedFlightId: number | null = $state(null);
  let selectedFlightNotes = $state("");

  let weatherTempC = $state("");
  let weatherWindMs = $state("");
  let weatherWindDir = $state("");
  let weatherDesc = $state("");
  let weatherEditing = $state(false);
  let playbackActive = $state(false);
  let playbackPlaying = $state(false);
  let playbackIndex = $state(0);
  let playbackSpeed = $state(1);
  const playbackCtrl = new PlaybackController();
  let logbookMinimized = $state(false);

  // Replay source: 'live' or 'blackbox' — for linked flights, switches which track is shown
  let replaySource = $state<'live' | 'blackbox'>('live');
  // Track for the linked partner (loaded on demand)
  let linkedPartnerTrack = $state<TelemetryRecord[]>([]);

  // Shared in-app dialog (replaces all native confirm/alert calls)
  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  let endFlightDialog: ReturnType<typeof EndFlightDialog>;
  let recoveryPrompt: ReturnType<typeof RecoveryPrompt>;
  let disconnectArmedDialog: ReturnType<typeof DisconnectArmedDialog>;
  // True after "Continue on Reconnect" until the next connection resolves the recovered session.
  let awaitingResumeReconnect = $state(false);

  async function showDialog(opts: DialogOptions): Promise<string | null> {
    return confirmDialog.show(opts);
  }

  async function showInfo(title: string, message: string): Promise<void> {
    await confirmDialog.show({ title, message });
  }

  // Widget panel state
  const defaultPanels: PanelConfig = {
    bottom: ['home', 'battery', 'speed', 'ahi', 'altitude', 'gps', 'compass'],
    right: ['rawTelemetry'],
  };
  let panels = $state<PanelConfig>(defaultPanels);
  let widgetEditMode = $state(false);

  // Cache stats subscription
  let cacheStats = $state<TileCacheStats>({ usedBytes: 0, maxBytes: 0, tileCount: 0 });
  tileCacheStats.subscribe((s) => { cacheStats = s; });

  const baudRates = [115200, 57600, 38400, 19200, 9600, 230400, 460800, 921600];

  // NavRail icons — migrating from glyphs to flat, high-contrast inline SVG (monochrome,
  // `currentColor` so they follow the rail's inactive/hover/active colours). UAV Info uses a
  // flight-controller (microchip) icon: neutral across UAV types, matches the panel content
  // (FC variant/version/board/sensors). Remaining tabs stay glyphs until converted.
  const ICON_UAV_INFO = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/><rect x="9.7" y="9.7" width="4.6" height="4.6" rx="0.6"/><path d="M9 6.5V3.8M12 6.5V3.8M15 6.5V3.8M9 17.5v2.7M12 17.5v2.7M15 17.5v2.7M6.5 9H3.8M6.5 12H3.8M6.5 15H3.8M17.5 9h2.7M17.5 12h2.7M17.5 15h2.7"/></svg>';

  // 6-tooth gear (Settings) — original-style proportions (chunky body, modest teeth) but
  // with sharp (non-rounded) tooth corners; solid + punched centre hole (evenodd).
  const ICON_SETTINGS = '<svg viewBox="0 0 24 24" fill="currentColor"><path fill-rule="evenodd" clip-rule="evenodd" d="M19.52 9.26 22.31 10 22.31 14 19.52 14.74 18.13 17.14 18.89 19.92 15.42 21.93 13.39 19.88 10.61 19.88 8.58 21.93 5.11 19.92 5.87 17.14 4.48 14.74 1.69 14 1.69 10 4.48 9.26 5.87 6.86 5.11 4.08 8.58 2.07 10.61 4.12 13.39 4.12 15.42 2.07 18.89 4.08 18.13 6.86ZM12 8.5A3.5 3.5 0 1 0 12 15.5 3.5 3.5 0 0 0 12 8.5Z"/></svg>';
  // Solid spiral notebook (Logbook): filled cover with knocked-out (transparent) 2px text
  // lines + spiral binding holes on the left (mask = white keeps, black cuts out).
  const ICON_LOGBOOK = '<svg viewBox="0 0 24 24"><defs><mask id="kg-nb"><rect x="4" y="3" width="16" height="18" rx="2" fill="#fff"/><circle cx="7" cy="6.5" r="1"/><circle cx="7" cy="9.7" r="1"/><circle cx="7" cy="12.9" r="1"/><circle cx="7" cy="16.1" r="1"/><rect x="9.8" y="6.5" width="7" height="2" rx="1"/><rect x="9.8" y="11" width="7" height="2" rx="1"/><rect x="9.8" y="15.5" width="5" height="2" rx="1"/></mask></defs><rect x="4" y="3" width="16" height="18" rx="2" fill="currentColor" mask="url(#kg-nb)"/></svg>';
  // Classic filled map marker with a punched-out (transparent) centre dot (Mission).
  const ICON_MISSION = '<svg viewBox="0 0 24 24" fill="currentColor"><path fill-rule="evenodd" clip-rule="evenodd" d="M12 2.5C8.4 2.5 5.5 5.4 5.5 9c0 4.8 6.5 12.5 6.5 12.5S18.5 13.8 18.5 9c0-3.6-2.9-6.5-6.5-6.5Zm0 4.1A2.4 2.4 0 1 0 12 11.4 2.4 2.4 0 0 0 12 6.6Z"/></svg>';
  // Two solid peaks, slightly raised (Terrain).
  const ICON_TERRAIN = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M1.5 20 8.5 5 13 14 16.5 8.5 22.5 20Z"/></svg>';
  // Solid flat movie camera (Video): two reels + body + lens funnel.
  const ICON_VIDEO = '<svg viewBox="0 0 24 24" fill="currentColor"><circle cx="7" cy="7" r="2.9"/><circle cx="12.6" cy="7" r="2.9"/><rect x="2.5" y="9.5" width="13" height="9" rx="1.6"/><path d="M15.5 12 21.5 9.5V18.5L15.5 16Z"/></svg>';
  // Radar dish on a mast with two sweep arcs (Radar / foreign-vehicle tracking).
  // Stylised radar scope: outer ring + inner range ring + sweep line + contact blips.
  const ICON_RADAR = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="4.3"/><path d="M12 12 18.7 6.4"/><circle cx="17.6" cy="9.4" r="1.15" fill="currentColor" stroke="none"/><circle cx="7.6" cy="15.4" r="0.85" fill="currentColor" stroke="none"/><circle cx="9.4" cy="6.8" r="0.85" fill="currentColor" stroke="none"/></svg>';

  // Stacked layers (Airspace Manager / aeronautical data).
  const ICON_AIRSPACE = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"><path d="M12 3 21 7.5 12 12 3 7.5Z"/><path d="M3 12 12 16.5 21 12"/><path d="M3 16.5 12 21 21 16.5"/></svg>';

  // Joystick/gamepad (Vehicle Control) — two sticks in a rounded gamepad body.
  const ICON_CONTROL = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="7.5" width="19" height="11" rx="4.5"/><circle cx="8" cy="13" r="2.1"/><circle cx="16" cy="13" r="2.1"/><path d="M8 10.9v-1M16 15.1v1"/></svg>';

  // The vehicle-control panel is MAVLink-only (ArduPilot/PX4) and only meaningful while connected.
  const isMavlinkConnected = $derived(
    $connection.status === 'connected' && $connection.protocolType === 'mavlink'
  );

  const allTabs = [
    { id: "uav-info", label: () => $t('nav.uavInfo'), icon: ICON_UAV_INFO },
    { id: "mission", label: () => $t('nav.mission'), icon: ICON_MISSION },
    { id: "control", label: () => $t('nav.control'), icon: ICON_CONTROL },
    { id: "terrain", label: () => $t('nav.terrain'), icon: ICON_TERRAIN },
    { id: "logbook", label: () => $t('nav.logbook'), icon: ICON_LOGBOOK },
    { id: "radar", label: () => $t('nav.radar'), icon: ICON_RADAR },
    { id: "airspace", label: () => $t('nav.airspace'), icon: ICON_AIRSPACE },
    { id: "video", label: () => $t('nav.video'), icon: ICON_VIDEO },
    { id: "settings", label: () => $t('nav.settings'), icon: ICON_SETTINGS },
  ];
  const tabs = $derived(
    allTabs.filter(t =>
      (t.id !== 'logbook' || flightLoggingEnabled) &&
      (t.id !== 'control' || isMavlinkConnected) && // control tab only when connected via MAVLink
      (t.id !== 'radar' || radarSettings.enabled) && // radar tab only when the master switch is on
      (t.id !== 'airspace' || airspaceSettings.enabled) // airspace tab only when its master switch is on
    )
  );

  // Permanent DEV-only reference panel (empty framework playground) at the end of the rail —
  // a "DEV" text button instead of an icon; only present in dev builds.
  const devTab = { id: "dev-playground", label: () => "DEV Playground", icon: '<span style="font-size:11px;font-weight:700;letter-spacing:0.5px">DEV</span>' };
  const railTabs = $derived([
    ...tabs,
    ...(DEV_MODE ? [{ id: "__sep__", label: () => "", icon: "" }, devTab] : []),
  ]);

  // Highlight the terrain rail button while its overlay is open
  const railActiveTab = $derived(terrainOpen ? 'terrain' : activeTab);

  let ports: PortInfo[] = $state([]);
  let connStatus: string = $state("disconnected");
  let fcInfo = $state<FcInfo | null>(null);

  // Subscribe to stores
  connection.subscribe((c) => {
    const wasConnected = connStatus === 'connected';
    connStatus = c.status;
    fcInfo = c.fcInfo;
    // Fresh connection → re-baseline the arm-edge detector (see armEdgeInit).
    if (!wasConnected && c.status === 'connected') armEdgeInit = false;
    // Auto-refresh the logbook on disconnect (picks up a just-recorded live flight) — replaces
    // the manual Refresh button. Disarm is covered by the flight-recording-ended listener.
    if (wasConnected && c.status !== 'connected') {
      void loadLogbook();
      // Lost the FC → the locked home becomes a manual reference (keeps its position so planning
      // continues; the marker reverts to a draggable orange "L").
      const h = get(homePosition);
      if (h.source === 'fc') homePosition.set({ ...h, source: 'manual' });
    }
  });
  availablePorts.subscribe((p) => {
    ports = p;
  });

  // One geolocation check at app start (refreshes the persisted user location for Night-Mode auto).
  ensureUserLocation();

  // Restore persisted settings
  const saved = get(settings);
  selectedPort = saved.lastPort;
  selectedBaud = saved.lastBaud;
  selectedProtocol = (saved.lastProtocol === 'mavlink' ? 'mavlink' : 'msp') as ProtocolType;
  // Restore the full last-used connection path so nothing has to be re-entered.
  if (saved.lastTransport === 'serial' || saved.lastTransport === 'tcp' || saved.lastTransport === 'udp' || saved.lastTransport === 'ble') {
    selectedTransport = saved.lastTransport;
  }
  if (saved.lastHost) tcpHost = saved.lastHost;
  if (saved.lastTcpPort) tcpPort = saved.lastTcpPort;
  if (saved.lastBleDevice) selectedBleDevice = saved.lastBleDevice;
  navPanelOpen = saved.navPanelOpen;
  // Drop any legacy "-v2" suffix from a persisted tab (the migration scaffolding is gone now).
  activeTab = (saved.activeTab ?? 'uav-info').replace(/-v2$/, '');
  attitudeRateHz = saved.attitudeRateHz;
  positionRateHz = saved.positionRateHz;
  airspeedEnabled = saved.airspeedEnabled;
  mavlinkFullTelemetry = saved.mavlinkFullTelemetry;
  flightLoggingEnabled = saved.flightLoggingEnabled;
  flightRecordingEnabled = saved.flightRecordingEnabled ?? false;
  flightLogDbPath = saved.flightLogDbPath;
  flightLogRawPath = saved.flightLogRawPath ?? '';
  flightLogRawEnabled = saved.flightLogRawEnabled;
  flightLogRawAlways = saved.flightLogRawAlways ?? false;
  mapProvider = saved.mapProvider;
  mapCacheMaxMB = saved.mapCacheMaxMB;
  cesiumIonToken = saved.cesiumIonToken ?? '';
  altitudeCurtain3D = saved.altitudeCurtain3D ?? true;
  realLighting3D = saved.realLighting3D ?? false;
  logReplayTime = saved.logReplayTime ?? false;
  nightMode2D = saved.nightMode2D ?? 'off';
  gcsMode = saved.gcsMode ?? 'manual';
  if (saved.radar) radarSettings = saved.radar;
  if (saved.airspace) airspaceSettings = saved.airspace;
  defaultWpAltitudeM = saved.defaultWpAltitudeM;
  defaultPhTimeSec = saved.defaultPhTimeSec;
  warnAltitudeM = saved.warnAltitudeM;
  systemMessages = saved.systemMessages ?? 'all';
  uiScale = saved.uiScale ?? 1;
  interfaceSettings = saved.interface ?? {
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  };
  panels = saved.panels ?? defaultPanels;

  // ── Radar (foreign-vehicle tracking) — independent of the main connection ──
  /** Free-look: cap the query centre's offset from the camera nadir (and the radius) at 150 km. */
  const FREE_LOOK_MAX_OFFSET_KM = 150;
  /** ONLINE ADS-B query centre + radius — all measured over the ground (the query is a surface circle).
   *  - **Free-look 3D:** centre = the screen-centre ground point, but its offset from the camera nadir
   *    (subpoint) is capped at 150 km; if the view runs past that — or hits no ground (looking above the
   *    horizon) — the centre is projected 150 km along the look direction. The radius = that horizontal
   *    offset, floored at the configured download radius. So the camera sits at the circle's near edge
   *    and looks into it; straight-down collapses the offset → the configured radius.
   *  - **UAV-locked 3D (follow/orbit/fpv) and 2D:** the UAV/reference (or 2D map centre) + the configured
   *    radius — unchanged from before.
   *  (Distance/bearing labels use `radarReference` separately.) */
  function radarQueryView(): { lat: number; lon: number; radiusKm: number } {
    const cfgKm = radarSettings.adsb.radiusKm > 0 ? radarSettings.adsb.radiusKm : 25;

    if (mapViewMode === '3d' && map3dRef?.isFreeLook?.()) {
      const g = map3dRef.getCamGeo?.();
      if (g) {
        const maxM = FREE_LOOK_MAX_OFFSET_KM * 1000;
        let center: { lat: number; lon: number };
        let offsetM: number;
        if (g.focus) {
          const d = haversineDistance(g.sub.lat, g.sub.lon, g.focus.lat, g.focus.lon);
          if (d <= maxM) {
            center = g.focus;
            offsetM = d;
          } else {
            const brg = bearing(g.sub.lat, g.sub.lon, g.focus.lat, g.focus.lon);
            center = destinationPoint(g.sub.lat, g.sub.lon, brg, maxM);
            offsetM = maxM;
          }
        } else {
          // Above the horizon: project 150 km along the camera heading.
          center = destinationPoint(g.sub.lat, g.sub.lon, g.headingDeg, maxM);
          offsetM = maxM;
        }
        return { lat: center.lat, lon: center.lon, radiusKm: Math.max(cfgKm, offsetM / 1000) };
      }
    }

    // UAV-locked 3D → centre on the UAV/reference; 2D → the map centre. Both at the configured radius.
    if (mapViewMode === '3d' && radarReference) {
      return { lat: radarReference.lat, lon: radarReference.lon, radiusKm: cfgKm };
    }
    const c = get(settings).map.center;
    return { lat: c[0], lon: c[1], radiusKm: cfgKm };
  }
  /** Distance/bearing reference for ALL tracked vehicles: the connected UAV (valid fix), else the
   *  GCS marker location (null when the GCS marker is OFF). (MSP is implicitly the UAV; others inherit.) */
  const radarReference = $derived.by<{ lat: number; lon: number } | null>(() => {
    const t = $telemetry;
    if (connStatus === 'connected' && isValidGpsCoordinate(t.lat, t.lon) && t.fixType >= 2) {
      return { lat: t.lat, lon: t.lon };
    }
    return $gcsLocation;
  });
  /** GCS ground level (m MSL) from terrain data at the GCS location — used as the colour-scale
   *  reference altitude when no UAV is connected (the geolocation API carries no altitude). */
  let gcsGroundAltM = $state<number | null>(null);
  $effect(() => {
    const g = $gcsLocation;
    if (connStatus === 'connected' || !g) { gcsGroundAltM = null; return; }
    let cancelled = false;
    invoke<number | null>('terrain_elevation', { lat: g.lat, lon: g.lon })
      .then((e) => { if (!cancelled) gcsGroundAltM = e; })
      .catch(() => { if (!cancelled) gcsGroundAltM = null; });
    return () => { cancelled = true; };
  });
  /** Reference altitude (m MSL) for the relative-altitude colour scale: the UAV's GPS MSL altitude when
   *  connected with a fix, else the GCS terrain ground level (else null → absolute colour fallback). */
  const radarRefAltM = $derived.by<number | null>(() => {
    const t = $telemetry;
    if (connStatus === 'connected' && isValidGpsCoordinate(t.lat, t.lon) && t.fixType >= 2) {
      return t.altMsl;
    }
    return gcsGroundAltM;
  });
  /** ADS-B-via-MSP available: connected + the FC reports the feature (INAV 8.0+; MAVLink has no features). */
  const mspAdsbSupported = $derived(
    connStatus === 'connected' && fcInfo != null && fcInfo.features != null && fcInfo.features.adsb_msp,
  );

  let lastRadarCenterKey = '';
  /** Push the live query centre (+3D auto radius) when it moved meaningfully. Cheap; no pipeline restart. */
  function updateRadarCenter() {
    if (!radarSettings.enabled || !radarSettings.adsb.enabled) return;
    const v = radarQueryView();
    const key = `${v.lat.toFixed(3)},${v.lon.toFixed(3)},${v.radiusKm?.toFixed(0) ?? ''}`;
    if (key === lastRadarCenterKey) return;
    lastRadarCenterKey = key;
    void setRadarCenter(v.lat, v.lon, v.radiusKm);
  }
  /** Build + push the backend radar config (starts/stops the pipeline). */
  function pushRadarConfig() {
    const { lat, lon } = radarQueryView();
    lastRadarCenterKey = '';
    // Don't clear per-provider status here: the backend now reconfigures in place (keeps the aggregator),
    // so the live provider counts shouldn't blink on an unrelated source toggle. Disabled providers stop
    // emitting and aren't shown anyway.
    void configureRadar({
      enabled: radarSettings.enabled,
      sim: radarSettings.sim,
      simCenter: [lat, lon],
      adsb: {
        enabled: radarSettings.adsb.enabled,
        // Built-ins (url from code + persisted on/off) + the custom providers.
        online: [
          ...BUILTIN_ADSB_PROVIDERS.map((b) => ({ name: b.name, url: b.url, enabled: radarSettings.adsb.builtins[b.name] ?? true })),
          ...radarSettings.adsb.online,
        ],
        local: radarSettings.adsb.local,
        // Only request the FC's ADS-B list when the connected INAV (8.0+) actually supports it.
        mspFromFc: radarSettings.adsb.mspFromFc && mspAdsbSupported,
        radiusKm: radarSettings.adsb.radiusKm,
        pollSec: radarSettings.adsb.pollSec,
        center: [lat, lon],
      },
      formationFlight: {
        enabled: radarSettings.formationFlight.enabled,
        port: radarSettings.formationFlight.port,
        baud: radarSettings.formationFlight.baud,
        nodeName: radarSettings.formationFlight.nodeName,
      },
    });
  }
  if (typeof window !== 'undefined') {
    void startRadarListeners();
    startRadarAlerts();
    startAlertAudio();
    void startStatusText();
    pushRadarConfig();
    // Query centre follows the map view: 2D pans update settings.map.center (broad subscribe, gated by
    // the ~100 m key); 3D camera moves come via Map3D's onCamFocus; the mode flip via the effect below.
    settings.subscribe(() => updateRadarCenter());
  }
  // Re-aim the online query centre when the 2D/3D view mode flips.
  $effect(() => { void mapViewMode; updateRadarCenter(); });

  // Airspace Manager: fetch the aero layers for a 500 km region around the reference (UAV/GCS, else the
  // map centre) while enabled. The backend caches the region; we only re-request when the rounded centre,
  // provider or key changes.
  let lastAeroFetchKey = '';
  $effect(() => {
    const a = airspaceSettings;
    const ref = radarReference; // re-fetch when the UAV/GCS reference moves
    if (!a.enabled || a.provider === 'none' || (a.provider === 'openaip' && !a.apiKey)) { lastAeroFetchKey = ''; return; }
    const c = ref ?? { lat: get(settings).map.center[0], lon: get(settings).map.center[1] };
    const key = `${a.provider}|${a.apiKey}|${c.lat.toFixed(1)},${c.lon.toFixed(1)}`;
    if (key === lastAeroFetchKey) return;
    lastAeroFetchKey = key;
    // 200 km airspace radius (few polygons); obstacles/airports/RC are capped to a short range backend-side.
    void fetchAero(a.provider, a.apiKey, c.lat, c.lon, 200, ['airspaces', 'obstacles', 'airports', 'rc']);
  });
  // Re-push the radar config when ADS-B-via-MSP support changes (connect/disconnect an INAV 8.0+ FC),
  // so the scheduler's MSP-ADSB polling flag tracks it. Guarded against the initial duplicate.
  let lastMspSupported = false;
  $effect(() => {
    const s = mspAdsbSupported;
    if (s === lastMspSupported) return;
    lastMspSupported = s;
    if (radarSettings.enabled && radarSettings.adsb.enabled) pushRadarConfig();
  });
  // FormationFlight: push the GCS node position we advertise as the emulated FC — the GCS marker
  // location (+ terrain ground altitude). Live; the running source reads it when answering MSP_RAW_GPS.
  $effect(() => {
    if (!radarSettings.enabled || !radarSettings.formationFlight.enabled) return;
    const g = $gcsLocation;
    if (!g) return;
    void setRadarNode(g.lat, g.lon, gcsGroundAltM ?? 0);
  });

  // Auto-start video with the last settings if it was running at last close.
  if (typeof window !== 'undefined') void initVideo();

  function toggleNavPanel() {
    navPanelOpen = !navPanelOpen;
    // The X hides all panels — including the terrain overlay
    if (!navPanelOpen) {
      editMode.set(false);
      patchTerrainAnalysis({ open: false });
    }
    settings.patch({ navPanelOpen });
    // Let the map recalculate its size after panel animation
    setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
  }

  function minimizeLogbook() {
    if (logbookHasFlightOnMap && !logbookMinimized) {
      logbookMinimized = true;
      setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
    }
  }

  function expandLogbook() {
    if (logbookMinimized) {
      logbookMinimized = false;
      setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
    }
  }

  // Persist a settings patch + mirror it into the local reactive vars the page binds. Shared by
  // the legacy SettingsPanel and the new SettingsPanel.
  function applySettingsPatch(patch: Partial<AppSettings>) {
    settings.patch(patch);
    if (patch.attitudeRateHz != null) attitudeRateHz = patch.attitudeRateHz;
    if (patch.positionRateHz != null) positionRateHz = patch.positionRateHz;
    if (patch.airspeedEnabled != null) airspeedEnabled = patch.airspeedEnabled;
    if (patch.mavlinkFullTelemetry != null) mavlinkFullTelemetry = patch.mavlinkFullTelemetry;
    if (patch.flightLoggingEnabled != null) flightLoggingEnabled = patch.flightLoggingEnabled;
    if (patch.flightRecordingEnabled != null) flightRecordingEnabled = patch.flightRecordingEnabled;
    if (patch.flightLogRawEnabled != null) flightLogRawEnabled = patch.flightLogRawEnabled;
    if (patch.flightLogRawAlways != null) flightLogRawAlways = patch.flightLogRawAlways;
    if (patch.flightLogDbPath != null) flightLogDbPath = patch.flightLogDbPath;
    if (patch.flightLogRawPath != null) flightLogRawPath = patch.flightLogRawPath;
    if (patch.mapProvider != null) mapProvider = patch.mapProvider;
    if (patch.mapCacheMaxMB != null) mapCacheMaxMB = patch.mapCacheMaxMB;
    if (patch.cesiumIonToken != null) cesiumIonToken = patch.cesiumIonToken;
    if (patch.altitudeCurtain3D != null) altitudeCurtain3D = patch.altitudeCurtain3D;
    if (patch.realLighting3D != null) realLighting3D = patch.realLighting3D;
    if (patch.logReplayTime != null) logReplayTime = patch.logReplayTime;
    if (patch.nightMode2D != null) nightMode2D = patch.nightMode2D;
    if (patch.gcsMode != null) gcsMode = patch.gcsMode;
    if (patch.radar != null) {
      radarSettings = patch.radar;
      pushRadarConfig(); // start/stop the backend pipeline on any radar settings change
    }
    if (patch.airspace != null) airspaceSettings = patch.airspace;
    if (patch.defaultWpAltitudeM != null) defaultWpAltitudeM = patch.defaultWpAltitudeM;
    if (patch.defaultPhTimeSec != null) defaultPhTimeSec = patch.defaultPhTimeSec;
    if (patch.warnAltitudeM != null) warnAltitudeM = patch.warnAltitudeM;
    if (patch.systemMessages != null) systemMessages = patch.systemMessages;
    if (patch.uiScale != null) uiScale = patch.uiScale;
    if (patch.interface != null) {
      interfaceSettings = { ...interfaceSettings, ...patch.interface };
      if (selectedFlight) {
        weatherTempC = weatherTempDisplayFromC(
          selectedFlight.weather_temp_c != null ? String(selectedFlight.weather_temp_c) : '',
          { ...interfaceSettings, ...patch.interface },
        );
        weatherWindMs = weatherWindDisplayFromMs(
          selectedFlight.weather_wind_ms != null ? String(selectedFlight.weather_wind_ms) : '',
          { ...interfaceSettings, ...patch.interface },
        );
      }
    }
  }

  function selectTab(tabId: string) {
    // Terrain Analysis is a full-width overlay shown in place of the panel content.
    // Like every other nav-rail button it only ever OPENS/selects (re-clicking the active
    // button does not close it) — closing happens by closing the whole nav rail (the
    // hamburger X) or by selecting another tab.
    if (tabId === 'terrain') {
      patchTerrainAnalysis({ open: true });
      return;
    }
    // Selecting another tab switches away from the terrain overlay
    patchTerrainAnalysis({ open: false });
    if (tabId !== 'mission') editMode.set(false);
    activeTab = tabId;
    settings.patch({ activeTab });
    if (tabId === 'logbook') {
      logbookMinimized = false;
      void loadLogbook();
    }
    if (!navPanelOpen) {
      navPanelOpen = true;
      settings.patch({ navPanelOpen: true });
      setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
    }
  }

  async function chooseFlightLogPath() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: flightLogDbPath || defaultFlightLogPath || undefined,
      });
      if (typeof selected === 'string' && selected.length > 0) {
        flightLogDbPath = selected;
        settings.patch({ flightLogDbPath });
      }
    } catch (e) {
      console.error('Failed to choose flight log path', e);
    }
  }

  function resetFlightLogPath() {
    flightLogDbPath = '';
    settings.patch({ flightLogDbPath: '' });
  }

  async function chooseRawLogPath() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: flightLogRawPath || defaultRawLogPath || undefined,
      });
      if (typeof selected === 'string' && selected.length > 0) {
        flightLogRawPath = selected;
        settings.patch({ flightLogRawPath });
      }
    } catch (e) {
      console.error('Failed to choose raw log path', e);
    }
  }

  function resetRawLogPath() {
    flightLogRawPath = '';
    settings.patch({ flightLogRawPath: '' });
  }

  async function loadLogbook() {
    if (!flightLoggingEnabled) {
      resetPlayback();
      flightSummaries = [];
      selectedFlight = null;
      selectedFlightTrack = [];
      selectedFlightId = null;
      selectedFlightTrackCount = 0;
      return;
    }

    logbookLoading = true;
    try {
      flightSummaries = await logbookCtrl.loadFlights(flightLogDbPath);
      if (selectedFlightId != null) {
        const found = flightSummaries.find((f) => f.id === selectedFlightId);
        if (!found) {
          selectedFlight = null;
          selectedFlightId = null;
          selectedFlightTrackCount = 0;
        }
      }
    } catch (e) {
      errorMsg = String(e);
    } finally {
      logbookLoading = false;
    }
  }

  function stopPlayback() {
    playbackCtrl.stop();
    playbackPlaying = false;
  }

  function resetPlayback() {
    playbackCtrl.stop();
    playbackActive = false;
    playbackPlaying = false;
    playbackIndex = 0;
    playbackSpeed = 1;
  }

  function startPlayback() {
    if (selectedTrackWithPosition.length <= 1) return;
    playbackActive = true;
    stopPlayback();
    playbackPlaying = true;
    playbackIndex = playbackCtrl.start(
      selectedTrackWithPosition,
      playbackIndex,
      playbackSpeed,
      (idx) => { playbackIndex = idx; },
      () => { playbackPlaying = false; },
    );
  }

  function togglePlayPause() {
    if (playbackPlaying) stopPlayback();
    else startPlayback();
  }

  function cyclePlaybackSpeed() {
    playbackSpeed = PlaybackController.cycleSpeed(playbackSpeed);
    if (playbackPlaying) stopPlayback();
  }

  function seekPlayback(deltaMs: number) {
    if (selectedTrackWithPosition.length === 0) return;
    const wasPlaying = playbackPlaying;
    if (wasPlaying) stopPlayback();
    playbackActive = true;
    playbackIndex = PlaybackController.seek(selectedTrackWithPosition, playbackIndex, deltaMs);
    if (wasPlaying) startPlayback();
  }

  function seekToStart() {
    if (selectedTrackWithPosition.length === 0) return;
    const wasPlaying = playbackPlaying;
    if (wasPlaying) stopPlayback();
    playbackActive = true;
    playbackIndex = 0;
    if (wasPlaying) startPlayback();
  }

  function closePlayer() {
    resetPlayback();
    homePosition.set({ lat: 0, lon: 0, alt: 0, set: false, source: 'manual' });
    selectedFlight = null;
    selectedFlightTrack = [];
    selectedFlightId = null;
    selectedFlightTrackCount = 0;
  }

  function scrubPlayback(index: number) {
    playbackActive = true;
    playbackIndex = index;
  }

  let wasPlayingBeforeScrub = false;

  function scrubStart() {
    wasPlayingBeforeScrub = playbackPlaying;
    if (playbackPlaying) stopPlayback();
  }

  function scrubEnd() {
    if (wasPlayingBeforeScrub) startPlayback();
  }

  /** Route one log file to the right importer by extension. Single source of truth for the
   *  one-button import and drag-drop. New formats (e.g. radio CSV later) just add a branch. */
  async function dispatchImport(filePath: string) {
    const ext = filePath.split('.').pop()?.toLowerCase() ?? '';
    if (ext === 'bin') {
      await performArdupilotImport(filePath, false); // ArduPilot DataFlash
    } else if (ext === 'kflight') {
      await performKflightImport(filePath); // KiteGC exchange file
    } else if (ext === 'rawmsp' || ext === 'tlog') {
      await performRawImport(filePath); // raw serial log (ADR-049)
    } else {
      await performImport(filePath, undefined, false); // INAV Blackbox text (.txt/.bbl/.bfl)
    }
  }

  async function performRawImport(filePath: string) {
    const result = await logbookCtrl.importRaw(filePath, flightLogDbPath);
    await loadLogbook();
    if (result.flightIds.length > 0) {
      await selectFlight(result.flightIds[result.flightIds.length - 1]);
    }
  }

  async function performKflightImport(filePath: string) {
    const result = await logbookCtrl.importFromKflight(filePath, flightLogDbPath);
    await loadLogbook();
    let msg = $t('logbook.importKflightResult', {
      values: { imported: result.imported, skipped: result.skipped },
    });
    if (result.errors.length > 0) msg += '\n' + result.errors.join('\n');
    await showInfo($t('logbook.importKflightTitle'), msg);
  }

  function baseName(p: string): string {
    return p.split(/[\\/]/).pop() ?? p;
  }

  /** Import a batch of files, isolating each so one bad/corrupt/non-log file doesn't abort the rest;
   *  failures (with the per-importer reason) are collected and surfaced together. */
  async function importFiles(files: string[]) {
    blackboxImporting = true;
    const failures: string[] = [];
    for (const filePath of files) {
      try {
        await dispatchImport(filePath);
      } catch (e) {
        failures.push(`${baseName(filePath)}: ${String(e)}`);
      }
    }
    blackboxImporting = false;
    blackboxImportProgress = null;
    if (failures.length > 0) {
      errorMsg = $t('logbook.importErrors', { values: { errors: failures.join('\n') } });
    }
  }

  /** One import action: pick any supported log file(s); the importer is chosen per file by extension. */
  async function importFlightLog() {
    if (blackboxImporting) return;
    let files: string[];
    try {
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: $t('logbook.allLogsFilter'),
            extensions: ['txt', 'bbl', 'bfl', 'bin', 'kflight', 'rawmsp', 'tlog'],
          },
        ],
      });
      if (!selected) return;
      files = Array.isArray(selected) ? selected : [selected];
    } catch (e) {
      errorMsg = String(e);
      return;
    }
    if (files.length === 0) return;
    await importFiles(files);
  }

  async function importDroppedFiles(paths: string[]) {
    // Guard against concurrent imports (drag-drop can fire multiple times on Windows)
    if (blackboxImporting) {
      console.warn('[IMPORT] Skipping — import already in progress');
      return;
    }

    console.log('[IMPORT] importDroppedFiles called with', paths.length, 'files');

    const supported = paths.filter((p) => /\.(txt|bbl|bfl|bin|kflight|rawmsp|tlog)$/i.test(p));
    if (supported.length === 0) return;
    await importFiles(supported);
  }

  async function exportFlightsToKflight(flightIds: number[]) {
    if (flightIds.length === 0) return;
    try {
      // Auto-include linked partner flights
      const allIds = new Set(flightIds);
      for (const id of flightIds) {
        const summary = flightSummaries.find((f) => f.id === id);
        if (summary?.linked_flight_id) allIds.add(summary.linked_flight_id);
      }
      const exportIds = [...allIds];

      const outputPath = await save({
        filters: [{ name: $t('logbook.kflightFileFilter'), extensions: ['kflight'] }],
        defaultPath: exportIds.length === 1 ? `flight_${exportIds[0]}.kflight` : `flights_export.kflight`,
      });
      if (!outputPath) return;
      const count = await logbookCtrl.exportSelectedFlights(exportIds, outputPath, flightLogDbPath);
      await showInfo($t('logbook.exportTitle'), $t('logbook.exportSuccess', { values: { count } }));
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function exportBlackbox() {
    if (!selectedFlightId || !selectedFlight) return;
    const src = selectedFlight.source;
    if (src !== 'blackbox' && src !== 'both') return;
    try {
      const defaultName = `blackbox_flight_${selectedFlightId}.TXT`;
      const outputPath = await save({
        filters: [{ name: $t('logbook.blackboxFileFilter'), extensions: ['TXT', 'BBL', 'BFL'] }],
        defaultPath: defaultName,
      });
      if (!outputPath) return;
      const originalFilename = await logbookCtrl.exportBlackbox(selectedFlightId, outputPath, flightLogDbPath);
      await showInfo($t('logbook.exportBlackboxTitle'), $t('logbook.exportBlackboxSuccess', { values: { filename: originalFilename } }));
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function exportTrack() {
    if (!selectedFlightId || !selectedFlight) return;
    try {
      const craft = selectedFlight.craft_name || 'flight';
      const date = selectedFlight.start_time ? new Date(selectedFlight.start_time).toISOString().slice(0, 10) : '';
      const defaultName = `${craft}_${date}`.replace(/\s+/g, '_');
      const outputPath = await save({
        filters: [
          { name: 'KMZ (Google Earth)', extensions: ['kmz'] },
          { name: 'KML (Google Earth)', extensions: ['kml'] },
          { name: 'GPX (GPS Exchange)', extensions: ['gpx'] },
          { name: 'CSV (Spreadsheet)', extensions: ['csv'] },
        ],
        defaultPath: `${defaultName}.kmz`,
      });
      if (!outputPath) return;
      await logbookCtrl.exportTrack(selectedFlightId, outputPath, flightLogDbPath);
      await showInfo($t('logbook.exportTrackTitle'), $t('logbook.exportTrackSuccess'));
    } catch (e) {
      errorMsg = String(e);
    }
  }


  async function performImport(filePath: string, logIndex: number | undefined, forceImport: boolean) {
    const result = await logbookCtrl.importBlackbox(filePath, flightLogDbPath, logIndex, forceImport, $locale ?? 'en');
    
    if (result.type === 'duplicate') {
      const answer = await showDialog({
        title: $t('logbook.duplicateTitle'),
        message: $t('logbook.duplicateMessage', {
          values: {
            craft: result.duplicate_craft_name,
            time: new Date(result.duplicate_start_time).toLocaleString(),
          },
        }),
        buttons: [{ label: $t('logbook.importAnyway'), value: 'force', danger: true }],
      });
      
      if (answer === 'force') {
        await performImport(filePath, logIndex, true);
      }
    } else {
      if (result.type === 'success_linkable') {
        const answer = await showDialog({
          title: $t('logbook.linkableTitle'),
          message: $t('logbook.linkableFound', { values: { id: result.linkable_flight_id } }),
          buttons: [{ label: $t('logbook.linkYes'), value: 'link', primary: true }],
        });
        if (answer === 'link') {
          await logbookCtrl.linkFlights(result.flight_id, result.linkable_flight_id, flightLogDbPath);
        }
      }
      await loadLogbook();
      await selectFlight(result.flight_id);
    }
  }

  async function performArdupilotImport(filePath: string, forceImport: boolean) {
    const result = await logbookCtrl.importArdupilot(filePath, flightLogDbPath, forceImport, $locale ?? 'en');
    
    if (result.type === 'duplicate') {
      const answer = await showDialog({
        title: $t('logbook.duplicateTitle'),
        message: $t('logbook.duplicateMessage', {
          values: {
            craft: result.duplicate_craft_name,
            time: new Date(result.duplicate_start_time).toLocaleString(),
          },
        }),
        buttons: [{ label: $t('logbook.importAnyway'), value: 'force', danger: true }],
      });
      
      if (answer === 'force') {
        await performArdupilotImport(filePath, true);
      }
    } else {
      if (result.type === 'success_linkable') {
        const answer = await showDialog({
          title: $t('logbook.linkableTitle'),
          message: $t('logbook.linkableFound', { values: { id: result.linkable_flight_id } }),
          buttons: [{ label: $t('logbook.linkYes'), value: 'link', primary: true }],
        });
        if (answer === 'link') {
          await logbookCtrl.linkFlights(result.flight_id, result.linkable_flight_id, flightLogDbPath);
        }
      }
      await loadLogbook();
      await selectFlight(result.flight_id);
    }
  }

  async function selectFlight(flightId: number) {
    selectedFlightId = flightId;
    const data = await logbookCtrl.selectFlightData(flightId, flightLogDbPath, $locale ?? 'en');
    selectedFlight = data.flight;
    selectedFlightTrack = data.track;
    selectedFlightTrackCount = data.trackCount;
    selectedFlightNotes = data.notes;
    weatherTempC = weatherTempDisplayFromC(data.weatherTempC, interfaceSettings);
    weatherWindMs = weatherWindDisplayFromMs(data.weatherWindMs, interfaceSettings);
    weatherWindDir = data.weatherWindDir;
    weatherDesc = canonicalWeatherDescription(data.weatherDesc);
    weatherEditing = false;
    replaySource = 'live';
    linkedPartnerTrack = [];
    resetPlayback();

    // While connected to a UAV, selecting a logbook entry shows DETAILS ONLY — nothing is loaded
    // onto the map (no mission, home, launch or playback), so the live FC mission/home stay
    // authoritative (this was the source of the FC↔map desync). To fly a logbook flight's mission,
    // open it from the detail's linked-mission chip → Mission Manager.
    if (connStatus === 'connected') {
      replayWpTotal.set(null);
      await loadLogbook();
      return;
    }

    if (data.hasGpsData) playbackActive = true;

    // Resolve the replay WP total (X for the WP N/X readout) and load the flown mission.
    // If the flight has a linked library mission, load it onto the map (so the mission overlay
    // + active-WP highlight show what was actually flown — hideable via the player MISSION
    // toggle). X = linked mission's WP count, else the Blackbox-header count, else null.
    replayWpTotal.set(null);
    try {
      const linked = await missionDbForFlight(flightId, flightLogDbPath);
      if (linked) {
        try {
          const linkedSys = linked.format === 'ardupilot' || linked.format === 'px4' ? linked.format : 'inav';
          if (linkedSys === 'inav') {
            switchAutopilotSystemForReplay('inav');
            await missionSetWaypoints(JSON.parse(linked.waypoints_json));
            loadedMissionId.set(linked.id);
            markMissionSynced('db'); // it's the library mission → trusted for the highlight
            // Launch/home reference for the replay mission (REL waypoint altitudes + 3D height):
            // the real flown start if known, else the mission's saved home, else its first waypoint.
            const fl = data.flight;
            if (fl?.start_lat != null && fl?.start_lon != null) {
              launchPoint.set({ lat: fl.start_lat, lng: fl.start_lon });
            } else if (linked.home_lat != null && linked.home_lon != null) {
              launchPoint.set({ lat: linked.home_lat, lng: linked.home_lon });
            } else {
              const fw = get(mission).waypoints.find((w) => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
              if (fw) launchPoint.set({ lat: toDeg(fw.lat), lng: toDeg(fw.lon) });
            }
          } else {
            // ArduPilot/PX4 linked mission → render via the AP layer (the mission layer switches on
            // the active autopilot system).
            switchAutopilotSystemForReplay(linkedSys);
            arduMission.set(JSON.parse(linked.waypoints_json) as ArduWaypoint[]);
            arduSelectedWpIndex.set(-1);
            arduLoadedMissionId.set(linked.id);
          }
        } catch (e) {
          console.warn('[replay] failed to load linked mission', e);
        }
        replayWpTotal.set(linked.wp_count);
      } else {
        replayWpTotal.set(await flightLoggedWpCount(flightId, flightLogDbPath));
      }
    } catch {
      replayWpTotal.set(null);
    }

    // Pre-load linked partner track for source switching
    if (data.flight?.linked_flight_id) {
      const partnerTrack = await logbookCtrl.getPartnerTrack(data.flight.linked_flight_id, flightLogDbPath);
      linkedPartnerTrack = partnerTrack;
    }

    // Set home position for replay (used by HomeWidget)
    if (data.flight?.start_lat != null && data.flight?.start_lon != null) {
      homePosition.set({ lat: data.flight.start_lat, lon: data.flight.start_lon, alt: 0, set: true, source: 'replay' });
    }

    await loadLogbook();
  }

  function switchReplaySource(source: 'live' | 'blackbox') {
    if (source === replaySource) return;
    replaySource = source;
    resetPlayback();
    if (activeReplayTrack.length > 0) playbackActive = true;
  }

  async function saveSelectedFlightNotes() {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.saveNotes(selectedFlightId, selectedFlightNotes, flightLogDbPath);
    await loadLogbook();
  }

  async function saveSelectedFlightWeather() {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.saveWeather(
      selectedFlightId,
      weatherTempCFromDisplay(weatherTempC, interfaceSettings),
      weatherWindMsFromDisplay(weatherWindMs, interfaceSettings),
      weatherWindDir,
      canonicalWeatherDescription(weatherDesc),
      flightLogDbPath,
    );
    // Keep editor/display values in selected UI units after save refresh
    weatherTempC = weatherTempDisplayFromC(selectedFlight?.weather_temp_c != null ? String(selectedFlight.weather_temp_c) : '', interfaceSettings);
    weatherWindMs = weatherWindDisplayFromMs(selectedFlight?.weather_wind_ms != null ? String(selectedFlight.weather_wind_ms) : '', interfaceSettings);
    weatherDesc = canonicalWeatherDescription(selectedFlight?.weather_desc ?? '');
    weatherEditing = false;
  }

  async function saveSelectedFlightCraftName(name: string) {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.saveCraftName(selectedFlightId, name, flightLogDbPath);
    await loadLogbook();
  }

  async function saveSelectedFlightPilot(pilotName: string, pilotId: string) {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.savePilot(selectedFlightId, pilotName, pilotId, flightLogDbPath);
  }

  async function saveSelectedFlightPlatformType(platformType: number) {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.savePlatformType(selectedFlightId, platformType, flightLogDbPath);
    await loadLogbook();
  }

  async function removeSelectedFlight() {
    if (!selectedFlightId || !selectedFlight) return;

    let buttons: DialogButton[];
    if (selectedFlight.linked_flight_id) {
      buttons = [
        { label: $t('logbook.deleteLiveOnly'), value: 'live', danger: true },
        { label: $t('logbook.deleteBlackboxOnly'), value: 'blackbox', danger: true },
        { label: $t('logbook.deleteBoth'), value: 'both', danger: true },
      ];
    } else {
      buttons = [
        { label: $t('logbook.deleteFlight'), value: 'single', danger: true },
      ];
    }

    // If a battery is linked, offer to consolidate this flight's usage into the pack's
    // persistent totals before deleting (opt-in — otherwise its contribution just drops).
    const linkedPack = selectedFlight.battery_serial
      ? await batteryDbFindBySerial(selectedFlight.battery_serial, flightLogDbPath).catch(() => null)
      : null;

    const value = await showDialog({
      title: $t('logbook.deleteTitle'),
      message: $t('logbook.deleteWarning'),
      buttons,
      checkbox: linkedPack ? { label: $t('logbook.deleteConsolidateBattery') } : undefined,
    });
    if (!value || !selectedFlightId || !selectedFlight) return;
    const consolidateBattery = linkedPack != null && confirmDialog.checkboxResult();

    const flightId = selectedFlightId;
    const linkedId = selectedFlight.linked_flight_id;

    let idsToDelete: number[] = [];
    if (value === 'single' || value === 'both') {
      idsToDelete.push(flightId);
      if (linkedId) idsToDelete.push(linkedId);
    } else if (value === 'live') {
      // Delete the live flight (lower id = created first = live recording)
      const liveId = linkedId && linkedId < flightId ? linkedId : flightId;
      idsToDelete.push(liveId);
    } else if (value === 'blackbox') {
      // Delete the blackbox flight (higher id = imported after = blackbox)
      const bbxId = linkedId && linkedId > flightId ? linkedId : flightId;
      idsToDelete.push(bbxId);
    }

    // Consolidate the deleted flights' battery usage into their packs first (opt-in).
    if (consolidateBattery) {
      for (const id of idsToDelete) {
        const f = id === flightId ? selectedFlight : await getFlight(id, flightLogDbPath).catch(() => null);
        if (!f?.battery_serial) continue;
        const pack = f.battery_serial === linkedPack?.serial
          ? linkedPack
          : await batteryDbFindBySerial(f.battery_serial, flightLogDbPath).catch(() => null);
        if (!pack) continue;
        const mah = f.battery_used_mah ?? 0;
        const cycles = pack.capacity_mah ? mah / pack.capacity_mah : 0;
        await batteryDbAddUsage(pack.id, f.duration_sec ?? 0, mah, cycles, 0, flightLogDbPath);
      }
    }

    for (const id of idsToDelete) {
      await logbookCtrl.removeFlight(id, flightLogDbPath);
    }

    resetPlayback();
    selectedFlight = null;
    selectedFlightTrack = [];
    selectedFlightId = null;
    selectedFlightTrackCount = 0;
    selectedFlightNotes = '';
    linkedPartnerTrack = [];
    weatherTempC = '';
    weatherWindMs = '';
    weatherWindDir = '';
    weatherDesc = '';
    weatherEditing = false;
    await loadLogbook();
  }

  async function loadInfo() {
    appVersion = await invoke("get_app_version");
    selectedPort = await refreshSerialPorts(selectedPort);
  }

  async function refreshPorts() {
    selectedPort = await refreshSerialPorts(selectedPort);
  }

  // ── Auto-discovery while disconnected (Quick Note: auto-discover changed COM ports + auto-scan BLE) ──
  // Only runs while a connection is neither active nor being established. The kickoff is wrapped in
  // `untrack` so the effect tracks only the transport/status it gates on — never the port/scan state it
  // writes (which would self-trigger).
  //
  // Serial: cheap port enumeration polled every 1 s; newly plugged adapters are auto-selected and
  // unplugged ones disappear (diff logic in refreshSerialPorts) — no manual ⟳ needed.
  $effect(() => {
    if (selectedTransport !== 'serial' || connStatus === 'connected' || connStatus === 'connecting') return;
    const id = setInterval(() => { void refreshPorts(); }, 1000);
    untrack(() => { void refreshPorts(); }); // immediate, then poll
    return () => clearInterval(id);
  });

  // BLE: a continuous backend scan streams discovered/updated devices via the `ble-device` event
  // (listener set up in initPage), so the list populates in real time. Start it on entering BLE,
  // stop it on leaving / connecting.
  $effect(() => {
    if (selectedTransport !== 'ble' || connStatus === 'connected' || connStatus === 'connecting') return;
    isBleScanning = true;
    untrack(() => { clearBleDevices(); void startBleScan(); });
    return () => {
      isBleScanning = false;
      void stopBleScan();
    };
  });

  async function handleConnect() {
    if (connStatus === "connected") {
      // Disconnect while a flight is being recorded (armed) → confirm first and let the user decide
      // what happens to the in-progress recording (ADR-042) — we do NOT disconnect immediately.
      const tnow = get(telemetry);
      const armed = isArmed(tnow.armingFlags, tnow.lastUpdate);
      const recordingActive = flightLoggingEnabled && flightRecordingEnabled;
      if (armed && recordingActive) {
        const choice = await disconnectArmedDialog.show({
          durationSec: armStartMs ? Math.round((Date.now() - armStartMs) / 1000) : null,
        });
        if (choice === 'cancel') return; // stay connected
        // Capture the flown mission now (still connected + FC-synced) for a Save/Continue commit.
        captureEndedMission();
        try {
          await disconnectFC(selectedBaud); // backend stashes the active flight as the pending session
        } catch (e) {
          errorMsg = String(e);
          return;
        }
        try {
          if (choice === 'discard') {
            await flightlogDiscardPending();
          } else if (choice === 'save') {
            const flightId = await flightlogCommitPending();
            await linkEndedMission(flightId, false);
            void loadLogbook();
          } else if (choice === 'continue') {
            await flightlogContinuePending();
            awaitingResumeReconnect = true;
          }
        } catch (e) {
          console.warn('[disconnect-armed] action failed', e);
        }
        errorMsg = "";
        return;
      }
      try {
        await disconnectFC(selectedBaud);
        errorMsg = "";
      } catch (e) {
        errorMsg = String(e);
      }
      return;
    }

    // Validate required fields per transport type
    if (selectedTransport === 'serial' && !selectedPort) {
      errorMsg = $t('connection.noPortSelected');
      return;
    }
    if ((selectedTransport === 'tcp' || selectedTransport === 'udp') && !tcpHost) {
      errorMsg = $t('connection.noHostSpecified');
      return;
    }
    if (selectedTransport === 'ble' && !selectedBleDevice) {
      errorMsg = $t('connection.noBleDeviceSelected');
      return;
    }

    // Stop the live BLE scan before connecting — the adapter can't scan and open a GATT link at once.
    if (selectedTransport === 'ble') await stopBleScan();

    isConnecting = true;
    errorMsg = "";
    connection.update((c) => ({ ...c, status: "connecting" }));
    settings.patch({ lastPort: selectedPort, lastBaud: selectedBaud, lastProtocol: selectedProtocol, lastTransport: selectedTransport, lastHost: tcpHost, lastTcpPort: tcpPort, lastBleDevice: selectedBleDevice, flightLoggingEnabled, flightRecordingEnabled, flightLogDbPath, flightLogRawPath, flightLogRawEnabled, flightLogRawAlways });

    try {
      await connectFC({
        protocolType: selectedProtocol,
        transportType: selectedTransport,
        port: selectedTransport === 'serial' ? selectedPort : undefined,
        baudRate: selectedTransport === 'serial' ? selectedBaud : undefined,
        host: (selectedTransport === 'tcp' || selectedTransport === 'udp') ? tcpHost : undefined,
        tcpPort: (selectedTransport === 'tcp' || selectedTransport === 'udp') ? tcpPort : undefined,
        bleDeviceId: selectedTransport === 'ble' ? selectedBleDevice : undefined,
        attitudeRateHz,
        positionRateHz,
        airspeedEnabled,
        mavlinkFullTelemetry,
        flightLogEnabled: flightRecordingEnabled,
        flightLogDbEnabled: flightLoggingEnabled && flightRecordingEnabled,
        flightLogPath: flightLogDbPath,
        flightLogRawPath,
        flightLogRaw: flightRecordingEnabled && (!flightLoggingEnabled || flightLogRawEnabled),
        flightLogRawAlways: flightRecordingEnabled && flightLogRawAlways,
      });
    } catch (e) {
      errorMsg = String(e);
      connection.set({ status: "error", protocolType: selectedProtocol, transportType: selectedTransport, port: "", baudRate: selectedBaud, errorMessage: String(e), fcInfo: null });
    } finally {
      isConnecting = false;
    }
  }

  async function initPage() {
    await startBleDeviceListener(); // live BLE discovery → bleDevices store
    await loadInfo();
    try {
      defaultFlightLogPath = await getDefaultFlightlogPath();
    } catch {
      defaultFlightLogPath = '';
    }
    try {
      defaultRawLogPath = await getDefaultRawLogPath();
    } catch {
      defaultRawLogPath = '';
    }
    if (activeTab === 'logbook') {
      await loadLogbook();
    }
  }

  initPage();

  function handleReorder(panelId: string, newIds: string[]) {
    panels = widgetCtrl.reorderPanel(panels, panelId, newIds);
    settings.patch({ panels });
  }

  function handleReceive(targetPanel: string, widgetId: string, index: number) {
    panels = widgetCtrl.receiveWidget(panels, targetPanel, widgetId, index);
    settings.patch({ panels });
  }

  function toggleWidget(widgetId: string) {
    panels = widgetCtrl.toggleWidgetVisibility(panels, widgetId);
    settings.patch({ panels });
  }

  function isWidgetActive(widgetId: string): boolean {
    return widgetCtrl.isWidgetActive(panels, widgetId);
  }

  function getWidgetPanelLabel(widgetId: string): string {
    const panel = widgetCtrl.getWidgetPanel(panels, widgetId);
    if (panel === 'bottom') return $t('widgets.bottom');
    if (panel === 'right') return $t('widgets.right');
    return $t('widgets.off');
  }

  const isPrimaryConnected = $derived(connStatus === 'connected');

  // Active replay track: switches between live telemetry and linked blackbox track
  const activeReplayTrack = $derived(
    replaySource === 'blackbox' && linkedPartnerTrack.length > 0
      ? linkedPartnerTrack
      : selectedFlightTrack,
  );

  const selectedTrackWithPosition = $derived(
    activeReplayTrack.filter((point) => isValidGpsCoordinate(point.lat, point.lon))
  );
  const mapTrack = $derived(isPrimaryConnected ? [] : selectedTrackWithPosition);
  const playbackPoint = $derived(
    playbackActive && !isPrimaryConnected && selectedTrackWithPosition.length > 0
      ? selectedTrackWithPosition[Math.min(playbackIndex, selectedTrackWithPosition.length - 1)]
      : null,
  );
  const showPlayer = $derived(playbackActive && !isPrimaryConnected && selectedFlight != null);
  // Mirror replay-mode state to the store so the map layers can gate mission
  // visibility (replay → follow the MISSION toggle; planning/live → always show).
  $effect(() => { replayActive.set(showPlayer); });
  const playbackBaseMs = $derived(
    selectedTrackWithPosition.length > 0 ? selectedTrackWithPosition[0].timestamp_ms : 0,
  );
  const playbackCurrentMs = $derived(
    selectedTrackWithPosition.length > 0
      ? selectedTrackWithPosition[Math.min(playbackIndex, selectedTrackWithPosition.length - 1)].timestamp_ms - playbackBaseMs
      : 0,
  );
  const playbackTotalMs = $derived(
    selectedTrackWithPosition.length > 0
      ? selectedTrackWithPosition[selectedTrackWithPosition.length - 1].timestamp_ms - playbackBaseMs
      : 0,
  );
  const logbookHasFlightOnMap = $derived(activeTab === 'logbook' && selectedFlight != null && !isPrimaryConnected);

  // Platform type for the UAV map symbol. Live and replay are mutually exclusive on the map
  // (mapTrack/playbackPoint gate on !isPrimaryConnected), so: connected → live FC type;
  // otherwise → the replayed flight's type (even if stale fcInfo still lingers after disconnect).
  const mapPlatformType = $derived(
    isPrimaryConnected
      ? ((fcInfo as FcInfo | null)?.platform_type ?? 0)
      : ((selectedFlight as Flight | null)?.platform_type
          ?? (fcInfo as FcInfo | null)?.platform_type
          ?? 0),
  );

  // FC variant for the selected flight (used by mode widgets + map coloring)
  const replayFcVariant = $derived((selectedFlight as Flight | null)?.fc_variant ?? 'INAV');

  // Absolute flight-start epoch (ms) — telemetry timestamp_ms is flight-relative, so the
  // 3D sky clock needs this origin to reconstruct the real instant for sun positioning.
  const replayStartEpochMs = $derived.by(() => {
    const s = (selectedFlight as Flight | null)?.start_time;
    if (!s) return null;
    const t = new Date(s).getTime();
    return Number.isFinite(t) ? t : null;
  });

  // Unified telemetry: live data when connected, playback data when replaying
  const telem = $derived(
    playbackActive && !isPrimaryConnected && playbackPoint
      ? toTelemetryData(playbackPoint, replayFcVariant)
      : liveTelem,
  );

  // ── Active-WP highlight trust gating (see MISSION_TRACKING_AND_PROVENANCE.md) ──
  // The highlight only shows when the loaded mission is trusted for the active context:
  //  - replay: the mission has the DB flag (or the user confirmed once for this log/file)
  //  - live:   the mission has the FC flag and the UAV is armed (or the user confirmed at arm)
  let replayTrackConfirmed = $state(false);
  let liveTrackConfirmed = $state(false);
  let replayAskedFlightId: number | null = null;
  let prevArmedForTrack = false;
  let prevFileFlagForTrack = false;

  async function promptTrackMission(kind: 'replay' | 'flight'): Promise<boolean> {
    const ans = await showDialog({
      title: $t('mission.trackTitle'),
      message: kind === 'replay' ? $t('mission.trackReplayMsg') : $t('mission.trackFlightMsg'),
      buttons: [{ label: $t('mission.trackYes'), value: 'track', primary: true }],
    });
    return ans === 'track';
  }

  // Gate: surface the active target WP only when in NAV_WP mode AND the mission is trusted.
  $effect(() => {
    const wp = telem.activeWpNumber ?? 0;
    const inWpMode = modeCategory(telem.flightMode.primary) === 'mission';
    const isReplay = playbackActive && !isPrimaryConnected;
    const armed = isArmed(telem.armingFlags, telem.lastUpdate);
    const f = $missionFlags;
    let trusted = false;
    if (isReplay) trusted = f.db || replayTrackConfirmed;
    else if (isPrimaryConnected) {
      // ArduPilot/MAVLink reports its own current mission item (MISSION_CURRENT) — that is the FC's
      // own truth, so trust it whenever armed + in a mission mode. INAV needs the mission to be FC-
      // synced (or operator-confirmed) since the active WP is matched against the loaded planner mission.
      const fcOwnsActiveWp = get(connection).protocolType === 'mavlink';
      trusted = armed && (fcOwnsActiveWp || f.fc || liveTrackConfirmed);
    }
    activeWpNumber.set(inWpMode && trusted ? wp : 0);
  });

  // Replay prompt: once per loaded log, if a mission is on the map but not DB-linked.
  $effect(() => {
    const id = selectedFlightId;
    if (id == null) { replayAskedFlightId = null; replayTrackConfirmed = false; return; }
    if (playbackActive && id !== replayAskedFlightId) {
      replayAskedFlightId = id;
      replayTrackConfirmed = false;
      if (get(mission).waypoints.length > 0 && !get(missionFlags).db) {
        void promptTrackMission('replay').then((ok) => { replayTrackConfirmed = ok; });
      }
    }
  });

  // Replay prompt: also when a mission file is loaded during a replay (FILE flag rising edge).
  $effect(() => {
    const fileFlag = $missionFlags.file;
    if (fileFlag && !prevFileFlagForTrack && playbackActive && !isPrimaryConnected && !$missionFlags.db) {
      replayTrackConfirmed = false;
      void promptTrackMission('replay').then((ok) => { replayTrackConfirmed = ok; });
    }
    prevFileFlagForTrack = fileFlag;
  });

  // Live prompt: once at arm, if connected and the mission isn't FC-synced.
  // Suppressed while a system switch is pending (e.g. an INAV mission is still loaded when an
  // ArduPilot FC connects): the Clear-or-Disconnect dialog handles that mission, so tracking the
  // soon-to-be-cleared INAV waypoints during the new flight is meaningless.
  $effect(() => {
    const armed = isArmed(telem.armingFlags, telem.lastUpdate);
    if (isPrimaryConnected && armed && !prevArmedForTrack && !get(pendingSystemSwitch)) {
      liveTrackConfirmed = false;
      if (get(mission).waypoints.length > 0 && !get(missionFlags).fc) {
        void promptTrackMission('flight').then((ok) => { liveTrackConfirmed = ok; });
      }
    }
    if (!armed) liveTrackConfirmed = false;
    prevArmedForTrack = armed;
  });

  // ── Connect prompt: offer to sync the mission with the FC on a fresh connection ──
  let prevConnForPrompt = false;
  $effect(() => {
    const connected = isPrimaryConnected;
    if (connected && !prevConnForPrompt) void onConnectMissionPrompt();
    prevConnForPrompt = connected;
  });

  async function onConnectMissionPrompt() {
    // INAV/MSP only for now (ArduPilot/MAVLink mission sync is a separate path).
    if (get(connection).protocolType !== 'msp') return;
    let fcWpCount = 0;
    try { fcWpCount = (await missionFcInfo()).wp_count; } catch { /* FC may not answer — treat as none */ }
    const mapHasMission = get(mission).waypoints.length > 0;
    if (fcWpCount === 0 && !mapHasMission) return; // nothing to offer

    const buttons: DialogButton[] = [];
    if (fcWpCount > 0) buttons.push({ label: $t('mission.connDownload'), value: 'download', primary: true });
    if (mapHasMission) buttons.push({ label: $t('mission.connUpload'), value: 'upload' });

    const msg = fcWpCount > 0
      ? $t('mission.connMsgFcHas', { values: { count: fcWpCount } })
      : $t('mission.connMsgUploadOnly');
    const ans = await showDialog({ title: $t('mission.connTitle'), message: msg, buttons });

    try {
      if (ans === 'download') await missionDownload();
      else if (ans === 'upload') await missionUpload();
    } catch (e) {
      await showInfo($t('mission.connTitle'), String(e));
    }
  }

  // When primary connection is established, clear playback
  $effect(() => {
    if (isPrimaryConnected && playbackActive) {
      resetPlayback();
    }
  });

  // ── Mission recording link (deferred commit, ADR-041) ────────────────
  // The flown mission is captured at disarm (while FC-sync still reflects what the FC flew) and
  // linked when the pending session is committed (Save, or a grace-lapsed re-arm auto-commit).
  // Captured per active autopilot system: INAV carries an FC-sync flag (links silently when synced);
  // ArduPilot/PX4 have no provenance flag yet (Phase 2) → linked only on explicit user opt-in.
  interface EndedMissionSnapshot {
    system: 'inav' | 'ardupilot' | 'px4';
    inavWps: Waypoint[];
    arduWps: ArduWaypoint[];
    fc: boolean;
  }
  let endedMission: EndedMissionSnapshot = { system: 'inav', inavWps: [], arduWps: [], fc: false };

  /** Snapshot the flown mission from the active autopilot system (at disarm / interrupt). */
  function captureEndedMission(): void {
    const sys = get(autopilotSystem);
    if (sys === 'inav') {
      endedMission = { system: 'inav', inavWps: [...get(mission).waypoints], arduWps: [], fc: get(missionFlags).fc };
    } else {
      endedMission = { system: sys, inavWps: [], arduWps: [...get(arduMission)], fc: false };
    }
  }

  function endedMissionHasWps(snap: EndedMissionSnapshot): boolean {
    return snap.system === 'inav' ? snap.inavWps.length > 0 : snap.arduWps.length > 0;
  }

  /** Save the captured mission to the library (dedup) and link it to the committed flight. */
  async function linkCapturedMission(flightId: number, snap: EndedMissionSnapshot): Promise<void> {
    let id: number;
    if (snap.system === 'inav') {
      id = await missionDbSave(await buildMissionInput(snap.inavWps), flightLogDbPath);
      markMissionSynced('db');
      loadedMissionId.set(id);
    } else {
      const fmt = snap.system === 'px4' ? 'px4' : 'ardupilot';
      id = await missionDbSave(await buildArduMissionInput(snap.arduWps, { format: fmt }), flightLogDbPath);
      arduLoadedMissionId.set(id);
    }
    await flightLinkMission(flightId, id, flightLogDbPath);
    void missionDbGeocode(id, $locale ?? 'en', flightLogDbPath).catch(() => {});
  }

  /** Link the captured flown mission to a freshly committed flight: FC-synced → silently;
   *  otherwise only when the user opted in via the dialog checkbox. */
  async function linkEndedMission(flightId: number, userOptedIn: boolean): Promise<void> {
    if (!endedMissionHasWps(endedMission)) return;
    if (!endedMission.fc && !userOptedIn) return;
    try { await linkCapturedMission(flightId, endedMission); }
    catch (e) { console.warn('[end-flight] mission link failed', e); }
  }

  // Fresh recording started — clear any stale captured-mission snapshot (defensive).
  function onRecordingStarted(): void {
    endedMission = { system: 'inav', inavWps: [], arduWps: [], fc: false };
  }

  /** Switch the active autopilot system for replay rendering (the mission layer switches on it),
   *  keeping any in-editor mission in memory — no destructive clear, no global switch dialog.
   *  No-op when already on that system or connected (locked). */
  function switchAutopilotSystemForReplay(sys: 'inav' | 'ardupilot' | 'px4'): void {
    if (get(autopilotSystem) === sys) return;
    setAutopilotSystem(sys);
    if (get(pendingSystemSwitch)) confirmSystemSwitch('keep');
  }

  async function onRecordingEnded(stats: EndFlightStats): Promise<void> {
    awaitingResumeReconnect = false; // a recovered session that came back disarmed is now in the dialog
    // Capture the flown mission at disarm, while FC-sync still reflects what the FC flew.
    captureEndedMission();
    const missionConfirm = endedMissionHasWps(endedMission) && !endedMission.fc;
    try {
      const res = await endFlightDialog.show({ stats, recorded: true, missionConfirm });
      // null = the dialog was force-closed by a re-arm (resumed) or a grace auto-commit — the
      // backend already resolved the pending session, so there is nothing to do here.
      if (res === null) return;
      if (res.discard) {
        await flightlogDiscardPending();
        return;
      }
      // Save → commit the pending session, then link mission + battery/notes against the new id.
      const flightId = await flightlogCommitPending();
      if (res.batterySerial) await flightSetBatterySerial(flightId, res.batterySerial, flightLogDbPath);
      if (res.notes) await updateFlightNotes(flightId, res.notes, flightLogDbPath);
      await linkEndedMission(flightId, res.linkMission);
      void loadLogbook();
    } catch (e) {
      console.warn('[end-flight] disarm dialog failed', e);
    }
  }

  // Disconnect while the UAV was still armed (ADR-042): the recovery prompt (Discard / Save /
  // Continue on Reconnect), NOT the End-Flight dialog — the flight may not be over (port change,
  // switch to telemetry). The session is already stashed as pending in the backend.
  async function onRecordingInterrupted(info: { temp_path: string; craft_name: string; start_time: string; duration_sec: number; sample_count: number }): Promise<void> {
    // Capture the flown mission (FC-sync) for a later commit.
    captureEndedMission();
    awaitingResumeReconnect = false;
    try {
      const choice = await recoveryPrompt.show(info, { reason: 'lost' });
      if (choice === 'discard') {
        await flightlogDiscardPending();
      } else if (choice === 'save') {
        const flightId = await flightlogCommitPending();
        await linkEndedMission(flightId, false);
        void loadLogbook();
      } else if (choice === 'continue') {
        await flightlogContinuePending();
        awaitingResumeReconnect = true; // resolved by the next connection's first poll
      }
    } catch (e) {
      console.warn('[interrupted] recovery failed', e);
    }
  }

  // Startup recovery (ADR-042): if a crash/close left an orphan temp session, prompt for it.
  async function runStartupRecovery(): Promise<void> {
    try {
      const orphan = await scanOrphanSessions(flightLogDbPath);
      if (!orphan) return;
      const choice = await recoveryPrompt.show(orphan);
      if (choice === 'discard') {
        await recoverDiscard(orphan.temp_path);
      } else if (choice === 'save') {
        await recoverSaveIncomplete(orphan.temp_path, flightLogDbPath);
        void loadLogbook();
      } else if (choice === 'continue') {
        await recoverContinue(orphan.temp_path, flightLogDbPath);
        awaitingResumeReconnect = true; // resolved by the next connection's first poll
      }
    } catch (e) {
      console.warn('[recovery] startup recovery failed', e);
    }
  }

  onMount(() => { void runStartupRecovery(); });

  // Jump to a flight in the Logbook when requested (e.g. from the Mission Manager's
  // "flights with this mission" list).
  $effect(() => {
    const id = $requestOpenFlightId;
    if (id == null) return;
    requestOpenFlightId.set(null);
    activeTab = 'logbook';
    batteryManagerOpen.set(false); // leave the Battery Manager so the flight detail is shown
    void selectFlight(id);
  });

  // Jump to a library mission in the Mission Manager (from a flight's linked-mission chip).
  $effect(() => {
    const id = $requestOpenMissionId;
    if (id == null) return;
    requestOpenMissionId.set(null);
    activeTab = 'mission';
    missionManagerOpen.set(true);
    missionManagerSelectedId.set(id);
  });

  if (typeof window !== 'undefined') {
    void listen('flight-recording-started', () => {
      onRecordingStarted(); // id-less signal — recording started (deferred commit, ADR-041)
    });
    // Disarm → the summary dialog (stats arrive in the payload; no flight_id yet under deferred
    // commit). Save commits the pending session, Discard drops it.
    void listen<{ duration_sec: number; max_alt_m: number; max_speed_ms: number; max_distance_m: number; battery_used_mah: number | null }>(
      'flight-recording-ended',
      (event) => {
        const p = event.payload;
        void onRecordingEnded({
          durationSec: p.duration_sec,
          maxAltM: p.max_alt_m,
          maxSpeedMs: p.max_speed_ms,
          maxDistM: p.max_distance_m,
          batteryUsedMah: p.battery_used_mah,
        });
      },
    );
    // Grace-lapsed re-arm auto-committed the previous flight: close the (now stale) summary and link
    // the mission captured at that disarm.
    void listen<{ flight_id: number }>('flight-recording-committed', async (event) => {
      const snap = endedMission; // snapshot before any await
      endFlightDialog?.close();
      if (endedMissionHasWps(snap) && snap.fc) {
        try { await linkCapturedMission(event.payload.flight_id, snap); } catch (e) { console.warn('[auto-commit] link failed', e); }
      }
      void loadLogbook();
    });
    // Re-arm within grace (or a recovered session resumed on reconnect) continues the same flight —
    // drop the stale summary dialog and exit the awaiting-reconnect state.
    void listen('flight-recording-resumed', () => {
      awaitingResumeReconnect = false;
      endFlightDialog?.close();
    });
    // Connection lost while recording (device gone, e.g. USB unplugged) → recovery prompt.
    void listen<{ temp_path: string; craft_name: string; start_time: string; duration_sec: number; sample_count: number }>(
      'flight-recording-interrupted',
      (event) => { void onRecordingInterrupted(event.payload); },
    );
    // The device vanished (fatal transport error) — the backend tore the scheduler down. Clean up the
    // connection state so the UI shows disconnected and the user can simply reconnect.
    void listen('connection-lost', () => {
      void disconnectFC(selectedBaud).catch(() => {});
    });
    void listen<BlackboxImportProgress>('flightlog-import-progress', (event) => {
      blackboxImportProgress = event.payload;
    });
    void listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
      if (activeTab === 'logbook' && event.payload.paths?.length) {
        importDroppedFiles(event.payload.paths);
      }
    });
    // Home from the FC (MSP_WP 0), pushed once at connect — recovers Home on a mid-flight connect /
    // app restart. The live arm-transition path (Map.svelte) overwrites it on the next arm.
    void listen<{ lat: number; lon: number; alt: number }>('home-position', (event) => {
      const { lat, lon, alt } = event.payload;
      if (lat === 0 && lon === 0) return;
      // The FC re-broadcasts HOME_POSITION (ArduPilot ~0.2 Hz), often with sub-metre jitter. Re-setting
      // the stores on every tick churns every subscriber — writable stores emit even on an identical
      // value — and the 3D mission overlay rebuilds, flickering its polylines. Only update on a real move.
      const h = get(homePosition);
      const unchanged = h.set && h.source === 'fc'
        && Math.abs(h.lat - lat) < 5e-6   // ≈ 0.55 m
        && Math.abs(h.lon - lon) < 5e-6
        && Math.abs(h.alt - alt) < 1;
      if (unchanged) return;
      homePosition.set({ lat, lon, alt, set: true, source: 'fc' }); // authoritative → locked green "H"
      launchPoint.set({ lat, lng: lon }); // pin the planning reference to the real home
    });
  }

  onDestroy(() => {
    playbackCtrl.destroy();
    stopBleDeviceListener();
    void stopBleScan();
  });
</script>

<svelte:window bind:innerWidth={winW} bind:innerHeight={winH} />

<div class="ui-root" style:--ui-scale={uiScale}>
  <!-- Window resize grips — outside `.ui-scale` so position:fixed stays viewport-relative.
       Re-adds edge resizing lost when the native decorations are disabled. -->
  <WindowResizeBorders />

  <!-- ======= MAP LAYER — unzoomed / native resolution (see docs/archive/UI_SCALING.md) =======
       The map must stay crisp, so it lives OUTSIDE the zoomed `.ui-scale` layer. It is the
       same single Map/Map3D instance (no re-mount). Normally it sits behind the chrome; when
       video is primary it flips above the chrome into the floating window's body (.in-frame). -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="layer-map"
    class:in-frame={mapInFrame}
    style={mapInFrame ? mapFrameStyle : ''}
    onclick={minimizeLogbook}
  >
    {#if mapViewMode === '2d'}
      <Map
        playbackTrack={mapTrack}
        playbackPoint={playbackPoint}
        {nightMode2D}
        {trackColorMode}
        platformType={mapPlatformType}
        {modelOverride}
        {uiScale}
        fcVariant={replayFcVariant}
        {mapViewMode}
        onToggleMapView={toggleMapView}
        bind:viewMode={map2dViewMode}
        radarActive={radarSettings.enabled}
        radarMapSettings={radarSettings.map}
        {radarReference}
        {radarRefAltM}
      />
    {/if}
    <!-- 3D stays mounted (hidden) once opened, so toggling back is instant. -->
    {#if map3dEverOpened}
      <div class="map3d-layer" class:active={mapViewMode === '3d'}>
        <Map3D
          bind:this={map3dRef}
          active={mapViewMode === '3d'}
          playbackTrack={mapTrack}
          playbackPoint={playbackPoint}
          {replayStartEpochMs}
          {trackColorMode}
          platformType={mapPlatformType}
          {modelOverride}
          fcVariant={replayFcVariant}
          {mapViewMode}
          onToggleMapView={toggleMapView}
          onCamFocus={() => updateRadarCenter()}
          radarActive={radarSettings.enabled}
          radarMapSettings={radarSettings.map}
          {radarRefAltM}
          {radarReference}
        />
      </div>
    {/if}

    <!-- Conflict-alert banner — pinned to the top of the map, above 2D/3D (renders nothing when idle). -->
    <RadarAlertBanner {interfaceSettings} />

    <!-- FC system messages (MAVLink STATUSTEXT) as top-edge toasts (renders nothing when idle). -->
    <StatusTextToasts />
  </div>

  <!-- ======= UI CHROME LAYER — zoomed by --ui-scale ======= -->
  <div class="ui-scale">

<ConfirmDialog bind:this={confirmDialog} />
<EndFlightDialog bind:this={endFlightDialog} {interfaceSettings} />
<RecoveryPrompt bind:this={recoveryPrompt} />
<DisconnectArmedDialog bind:this={disconnectArmedDialog} />

{#if awaitingResumeReconnect}
  <div class="resume-banner">{$t('recovery.waitingBanner')}</div>
{/if}

<main
  class="app"
  style:--grid-bottom-height={gridBottomHeight}
  style:--grid-side-width={gridSideWidth}
>
  <!-- ======= TOOLBAR ======= -->
  <div class="zone-toolbar">
    <Toolbar
    {appVersion}
    {telem}
    {ports}
    {bleDeviceList}
    {isBleScanning}
    {connStatus}
    {isConnecting}
    bind:selectedTransport
    bind:selectedProtocol
    bind:selectedPort
    bind:selectedBaud
    bind:tcpHost
    bind:tcpPort
    bind:selectedBleDevice
    {baudRates}
    onConnect={handleConnect}
    relayOpen={relayPanelOpen}
    onToggleRelay={() => (relayPanelOpen = !relayPanelOpen)}
  />
    <RelayPanel open={relayPanelOpen} />
  </div>

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- Full-size video shown in the map area when the view is swapped (videoPrimary).
       Double-click to swap back. -->
  {#if mapInFrame}
    <!-- Wrapper carries the inset + black backdrop; the video fills it with object-fit: contain so
         it scales to the window (full height/width) without distortion — bars where aspect differs. -->
    <div class="map-video-wrap">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        class="map-video"
        class:mirror={$videoState.mirror}
        bind:this={mapVideoEl}
        autoplay
        muted
        playsinline
        ondblclick={() => setVideoPrimary(false)}
      ></video>
    </div>
  {/if}

  <!-- Map lives in the unzoomed `.layer-map` above (see docs/archive/UI_SCALING.md). -->

  <LogPlayer
    {showPlayer}
    {selectedFlight}
    {playbackPlaying}
    {playbackSpeed}
    {playbackCurrentMs}
    {playbackTotalMs}
    trackLength={selectedTrackWithPosition.length}
    {playbackIndex}
    onClose={closePlayer}
    onSeekToStart={seekToStart}
    onSeek={seekPlayback}
    onTogglePlayPause={togglePlayPause}
    onCycleSpeed={cyclePlaybackSpeed}
    onScrub={scrubPlayback}
    onScrubStart={scrubStart}
    onScrubEnd={scrubEnd}
    {trackColorMode}
    onTrackColorModeChange={(mode) => { trackColorMode = mode; }}
    {modelOverride}
    onModelOverrideChange={(v) => { modelOverride = v; }}
    playbackTrack={mapTrack}
    {warnAltitudeM}
    {replaySource}
    hasLinkedPartner={selectedFlight?.linked_flight_id != null && linkedPartnerTrack.length > 0}
    onSwitchSource={switchReplaySource}
  />

  <!-- ======= FLOATING NAV PANEL SYSTEM ======= -->
  <NavRail
    open={navPanelOpen}
    activeTab={railActiveTab}
    tabs={railTabs}
    onToggle={toggleNavPanel}
    onSelectTab={selectTab}
  />

  <!-- Floating panels — all on the panel framework (docs/active/PANEL_FRAMEWORK.md). Each is a
       self-positioned PanelShell; terrain is its own overlay below. -->
  {#if navPanelOpen && !terrainOpen}
    {#if activeTab === 'uav-info'}
      <UavInfoPanel {connStatus} {fcInfo} />
    {:else if activeTab === 'settings'}
      <SettingsPanel
        localeValue={$locale ?? 'en'}
        {uiScale}
        {mapProvider}
        {mapCacheMaxMB}
        {cacheStats}
        {cesiumIonToken}
        {altitudeCurtain3D}
        {realLighting3D}
        {logReplayTime}
        {nightMode2D}
        {gcsMode}
        userLocation={$userGeoLocation}
        onGeoCheck={requestUserLocation}
        {attitudeRateHz}
        {positionRateHz}
        {airspeedEnabled}
        directionLines={$settings.directionLines}
        {mavlinkFullTelemetry}
        {flightLoggingEnabled}
        {flightRecordingEnabled}
        {flightLogRawEnabled}
        {flightLogRawAlways}
        {flightLogDbPath}
        {defaultFlightLogPath}
        {flightLogRawPath}
        {defaultRawLogPath}
        {defaultWpAltitudeM}
        {defaultPhTimeSec}
        {warnAltitudeM}
        {systemMessages}
        {interfaceSettings}
        radar={radarSettings}
        airspace={airspaceSettings}
        {isWidgetActive}
        {getWidgetPanelLabel}
        onPatch={applySettingsPatch}
        onSetCacheMaxMB={setCacheMaxMB}
        onClearCache={clearCache}
        onChooseFlightLogPath={chooseFlightLogPath}
        onResetFlightLogPath={resetFlightLogPath}
        onChooseRawLogPath={chooseRawLogPath}
        onResetRawLogPath={resetRawLogPath}
        onToggleWidget={toggleWidget}
      />
    {:else if activeTab === 'logbook'}
      <LogbookPanel
        {flightLoggingEnabled}
        {logbookMinimized}
        {logbookLoading}
        {blackboxImporting}
        {blackboxImportProgress}
        {flightSummaries}
        {selectedFlight}
        {selectedFlightId}
        {selectedFlightTrackCount}
        {interfaceSettings}
        bind:selectedFlightNotes
        bind:weatherTempC
        bind:weatherWindMs
        bind:weatherWindDir
        bind:weatherDesc
        bind:weatherEditing
        onExpand={expandLogbook}
        onLoadLogbook={loadLogbook}
        onImport={importFlightLog}
        onSelectFlight={selectFlight}
        onSaveNotes={saveSelectedFlightNotes}
        onSaveWeather={saveSelectedFlightWeather}
        onSaveCraftName={saveSelectedFlightCraftName}
        onSavePlatformType={saveSelectedFlightPlatformType}
        onSavePilot={saveSelectedFlightPilot}
        onDeleteFlight={removeSelectedFlight}
        onExportFlights={exportFlightsToKflight}
        onExportBlackbox={exportBlackbox}
        onExportTrack={exportTrack}
      />
    {:else if activeTab === 'mission'}
      <MissionPanel />
    {:else if activeTab === 'control'}
      <MavCommandPanel />
    {:else if activeTab === 'radar'}
      <RadarPanel radar={radarSettings} {interfaceSettings} referencePoint={radarReference} mspSupported={mspAdsbSupported} onPatch={applySettingsPatch} />
    {:else if activeTab === 'airspace'}
      <AirspaceManagerPanel reference={radarReference} distanceUnit={interfaceSettings.distanceUnit} />
    {:else if activeTab === 'video'}
      <VideoPanel />
    {:else if DEV_MODE && activeTab === 'dev-playground'}
      <PanelPlayground initial="compact" label="DEV Playground" />
    {/if}
  {/if}

  <!-- ======= TERRAIN ANALYSIS OVERLAY ======= -->
  {#if terrainOpen}
    <TerrainAnalysisPanel track={selectedTrackWithPosition} live={isPrimaryConnected} {interfaceSettings} confirm={showDialog} />
  {/if}

  <!-- ======= BOTTOM WIDGET PANEL ======= -->
  <div class="zone-bottom-dock" class:zone-hidden={!$layout.bottomDock.visible} class:panel-editing={widgetEditMode} bind:clientWidth={bottomDockW} bind:clientHeight={bottomDockH} style:padding-left="{videoReserve}px">
    <div class="panel-bottom-wrap">
      <button
        class="widget-edit-btn widget-edit-btn--panel"
        class:active={widgetEditMode}
        onclick={() => widgetEditMode = !widgetEditMode}
        title={widgetEditMode ? $t('widgets.exitEdit') : $t('widgets.editLayout')}
      >
        ✎
      </button>

      <WidgetPanel
        widgetIds={panels.bottom}
        orientation="horizontal"
        availableVmin={bottomAvailUnits}
        pxPerVmin={bottomPxPerUnit}
        {telem}
        editing={widgetEditMode}
        {interfaceSettings}
        onreorder={handleReorder}
        onreceive={handleReceive}
        panelId="bottom"
      />
    </div>
  </div>

  <!-- Persistent hidden source for native Picture-in-Picture (survives panel close) -->
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={pipVideoEl} class="pip-source" autoplay muted playsinline></video>

  <!-- ======= FLOATING VIDEO WINDOW ======= -->
  <FloatingVideoWindow />

  <!-- ======= RIGHT WIDGET PANEL ======= -->
  <div class="zone-side-dock" class:zone-hidden={!$layout.sideDock.visible} class:panel-editing={widgetEditMode} bind:clientWidth={sideDockW} bind:clientHeight={sideDockH}>
    <WidgetPanel
      widgetIds={panels.right}
      orientation="vertical"
      availableVmin={rightAvailUnits}
      pxPerVmin={sidePxPerUnit}
      {telem}
      editing={widgetEditMode}
      {interfaceSettings}
      onreorder={handleReorder}
      onreceive={handleReceive}
      panelId="right"
    />
  </div>

  <!-- ======= MAP CONTROLS RESERVED AREA ======= -->
  <div class="zone-map-controls">
    <!-- reserved for map control buttons (zoom, 3D toggle etc.) -->
  </div>

  <!-- ======= DEBUG PANEL (dev only) ======= -->
  {#if DEV_MODE && debugOpen && DebugPanelCmp}
    <DebugPanelCmp onclose={() => debugOpen = false} />
  {/if}

  <!-- ======= ERROR BAR ======= -->
  {#if errorMsg}
    <div class="error-bar">
      <span>{errorMsg}</span>
      <button class="error-dismiss" onclick={() => (errorMsg = "")}>✕</button>
    </div>
  {/if}

  <!-- ======= STATUS BAR ======= -->
  <div class="zone-status-bar">
    <StatusBar
      {connStatus}
      {fcInfo}
      {telem}
      connectionPort={$connection.port}
      devMode={DEV_MODE}
      bind:debugOpen
    />
  </div>
</main>
  </div><!-- .ui-scale -->

  <!-- Cursor-positioned overlays stay OUTSIDE the zoom so their fixed clientX/clientY
       coordinates are not multiplied by --ui-scale (they render unscaled but in the
       correct place; see docs/archive/UI_SCALING.md). -->
  <ContextMenu />
  <BatchEditPopup {interfaceSettings} />
</div><!-- .ui-root -->

<style>
  /* ============================================================
     Kite Ground Control Theme — Floating Panel Layout
     Color palette derived from INAV Configurator
     https://github.com/iNavFlight/inav-configurator
     ============================================================ */

  /* Continue-on-reconnect status banner (recovery, ADR-042) */
  .resume-banner {
    position: fixed;
    bottom: 36px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 9000;
    padding: 8px 16px;
    font-size: 12px;
    font-weight: 600;
    color: #cfe8f5;
    background: rgba(26, 107, 148, 0.92);
    border: 1px solid #2590c8;
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45);
    pointer-events: none;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    background-color: #3d3f3e;
    color: #e0e0e0;
    overflow: hidden;
    /* Block accidental text selection on drag everywhere (UI is app-like, not a document) */
    user-select: none;
    -webkit-user-select: none;
  }

  /* …but keep text selectable in real text-entry controls */
  :global(input),
  :global(textarea),
  :global([contenteditable="true"]) {
    user-select: text;
    -webkit-user-select: text;
  }

  /* Leaflet map tooltips (hover hints / "toasts") live in the unzoomed map. Scale them
     with the global UI scale via font-size + padding (Leaflet sets an inline transform
     for positioning, so a CSS transform would be overridden — em/px scaling reflows the
     box instead). --ui-scale inherits from `.ui-root`. Base values match Leaflet defaults. */
  :global(.leaflet-tooltip) {
    font-size: calc(12px * var(--ui-scale, 1));
    padding: calc(6px * var(--ui-scale, 1)) calc(8px * var(--ui-scale, 1));
  }

  /* ── Global UI scaling (see docs/archive/UI_SCALING.md) ──────────
     `.ui-root` fills the viewport. `.ui-scale` holds all chrome and is zoomed by
     --ui-scale (sized /scale so it fills exactly the viewport after the zoom).
     `.layer-map` holds the single Map/Map3D instance UNZOOMED so it stays crisp. */
  .ui-root {
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
  }
  .ui-scale {
    position: absolute;
    top: 0;
    left: 0;
    width: calc(100vw / var(--ui-scale, 1));
    height: calc(100vh / var(--ui-scale, 1));
    zoom: var(--ui-scale, 1);
    z-index: 1;
  }

  .app {
    display: grid;
    height: 100%;
    position: relative;
    grid-template-rows: 53px 1fr var(--grid-bottom-height) 24px;
    grid-template-columns: 62px 1fr var(--grid-side-width) 54px;
    grid-template-areas:
      "toolbar      toolbar      toolbar      toolbar"
      "nav-rail     panel        side-dock    side-dock"
      "nav-rail     bottom-dock  bottom-dock  map-controls"
      "status-bar   status-bar   status-bar   status-bar";
  }

  /* The chrome layer sits ABOVE the unzoomed map, so its empty centre must let pointer
     events fall through to the map (pan/zoom/Leaflet controls + the WP editor popup,
     which is part of the map). BOTH `.ui-scale` (the parent covering the viewport) and
     `.app` must be click-through, or the parent eats events the moment `.app` passes them
     on. Solid children re-capture; the widget docks + map-controls stay click-through so
     the map is draggable under/around them. See docs/archive/UI_SCALING.md. */
  .ui-scale {
    pointer-events: none;
  }
  .ui-scale > :global(*) {
    pointer-events: auto; /* dialogs (and .app, immediately overridden below) */
  }
  .app {
    pointer-events: none;
  }
  .app > :global(*) {
    pointer-events: auto;
  }
  .app > :global(.zone-bottom-dock),
  .app > :global(.zone-side-dock),
  .app > :global(.zone-map-controls) {
    pointer-events: none;
  }

  /* ── Grid zone wrappers ─────────────────────────────────── */
  .zone-toolbar {
    grid-area: toolbar;
    z-index: 200;
  }

  /* Map layer — UNZOOMED overlay over the content area. The toolbar (53px) and status
     bar (24px) live in the zoomed `.ui-scale`, so their visual heights are *--ui-scale;
     the map offsets track that. z-index 0 keeps it behind the chrome normally. When the
     view is swapped into the floating window it flips above the chrome (.in-frame) and
     uses the inline rect (already *--ui-scale in mapFrameStyle). */
  .layer-map {
    position: absolute;
    top: calc(53px * var(--ui-scale, 1));
    left: 0;
    right: 0;
    bottom: calc(24px * var(--ui-scale, 1));
    z-index: 0;
    overflow: hidden;
  }
  /* 3D map overlay — kept mounted but hidden (visibility, not display, so Cesium
     keeps a sized canvas) while 2D is shown. */
  .map3d-layer {
    position: absolute;
    inset: 0;
  }
  .map3d-layer:not(.active) {
    visibility: hidden;
    pointer-events: none;
  }
  .layer-map.in-frame {
    top: auto;
    right: auto;
    bottom: auto; /* left/top/width/height come from the inline rect */
    z-index: 2; /* above .ui-scale (z:1): into the floating frame body; frame draws the border */
    border-radius: 0 0 7px 7px;
  }
  /* Full-size video shown in the content area when swapped (videoPrimary). The wrapper holds the
     chrome inset + black backdrop; the video fills it. */
  .map-video-wrap {
    position: absolute;
    top: 53px;
    left: 0;
    right: 0;
    bottom: 24px;
    background: #000;
    z-index: 0;
  }
  /* width/height 100% (not auto) so the replaced <video> stretches to the wrapper instead of using
     its intrinsic stream resolution; object-fit: contain keeps the aspect ratio (letterbox bars). */
  .map-video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .map-video.mirror {
    transform: scaleX(-1);
  }
  /* PiP source: rendered + playing but visually out of the way (must not be
     display:none, or it produces no frames for Picture-in-Picture). */
  .pip-source {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
    z-index: -1;
  }

  .zone-bottom-dock {
    grid-area: bottom-dock;
    z-index: 100;
    display: flex;
    justify-content: center;
    align-items: center;
    pointer-events: none;
    overflow: hidden;
    padding: 6px 0;
  }

  .zone-bottom-dock.panel-editing {
    pointer-events: auto;
  }

  .zone-bottom-dock > * {
    pointer-events: auto;
  }

  .zone-side-dock {
    grid-area: side-dock;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    pointer-events: none;
    overflow: hidden;
    padding: 0 6px;
  }

  .zone-side-dock.panel-editing {
    pointer-events: auto;
  }

  .zone-side-dock > :global(*) {
    pointer-events: auto;
  }

  .zone-map-controls {
    grid-area: map-controls;
    z-index: 90;
    pointer-events: none;
  }

  .zone-status-bar {
    grid-area: status-bar;
    z-index: 200;
  }

  /* Zone hidden toggle — collapses zone content */
  .zone-hidden {
    visibility: hidden;
    pointer-events: none !important;
  }

  /* --- Bottom Widget Panel (inside .zone-bottom-dock) --- */

  .panel-bottom-wrap {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    pointer-events: auto;
  }

  /* --- Widget edit toggle button --- */
  .widget-edit-btn {
    width: 28px;
    height: 28px;
    background: rgba(46, 46, 46, 0.85);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #949494;
    font-size: 13px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(8px);
    transition: background-color 0.2s, border-color 0.2s, color 0.2s;
  }

  .widget-edit-btn--panel {
    flex: 0 0 auto;
    z-index: 110;
  }

  .widget-edit-btn:hover {
    background: rgba(55, 168, 219, 0.2);
    color: #e0e0e0;
  }

  .widget-edit-btn.active {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
    color: #37a8db;
  }


  /* --- Error Bar --- */
  .error-bar {
    position: absolute;
    bottom: 24px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 12px;
    background: #d40000;
    color: #fff;
    font-size: 12px;
    z-index: 300;
  }

  .error-dismiss {
    background: none;
    border: none;
    color: #fff;
    font-size: 14px;
    cursor: pointer;
    padding: 0 4px;
  }

</style>
