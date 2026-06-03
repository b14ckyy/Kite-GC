<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { connection, availablePorts, bleDevices } from "$lib/stores/connection";
  import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import { telemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";
  import { t, locale } from 'svelte-i18n';
  import Map from "$lib/components/Map.svelte";
  import Map3D from "$lib/components/Map3D.svelte";
  import LogPlayer from "$lib/components/logbook/LogPlayer.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import BatchEditPopup from "$lib/components/mission/BatchEditPopup.svelte";
  import type { DialogButton, DialogOptions } from "$lib/components/ConfirmDialog.svelte";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import LogbookPanel from "$lib/components/logbook/LogbookPanel.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import UavInfoPanel from "$lib/components/UavInfoPanel.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import NavRail from "$lib/components/NavRail.svelte";
  import { PlaybackController } from '$lib/controllers/playbackController';
  import { refreshSerialPorts, connectFC, disconnectFC, scanBleDevices } from '$lib/controllers/connectionController';
  import * as logbookCtrl from '$lib/controllers/logbookController';
  import * as widgetCtrl from '$lib/controllers/widgetController';
  import { isValidGpsCoordinate, isArmed } from '$lib/helpers/telemetry';
  import { liveTrack, appendLivePoint, clearLiveTrack } from '$lib/stores/liveTrack';
  import { toTelemetryData } from '$lib/adapters/telemetryAdapter';
  import { activeWpNumber, replayWpTotal } from '$lib/stores/navStatus';
  import { missionManagerOpen, missionManagerSelectedId, requestOpenFlightId, requestOpenMissionId } from '$lib/stores/missionManager';
  import { batteryManagerOpen, batteryManagerSelectedId } from '$lib/stores/batteryManager';
  import { missionDbForFlight, flightLoggedWpCount, missionDbSave, flightLinkMission, missionDbGeocode } from '$lib/stores/flightlog';
  import { buildMissionInput, missionContentHash } from '$lib/helpers/missionLibrary';
  import { homePosition } from '$lib/stores/home';
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import { weatherTempDisplayFromC, weatherWindDisplayFromMs, weatherTempCFromDisplay, weatherWindMsFromDisplay, canonicalWeatherDescription } from "$lib/helpers/weather";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import { LARGE_BASE_VMIN } from "$lib/config/widgetRegistry";
  import MissionPanel from "$lib/components/mission/MissionPanel.svelte";
  import VideoPanel from "$lib/components/video/VideoPanel.svelte";
  import FloatingVideoWindow from "$lib/components/video/FloatingVideoWindow.svelte";
  import { initVideo, videoState, videoStream, setVideoPrimary, registerPiPElement } from "$lib/stores/video";
  import TerrainAnalysisPanel from "$lib/components/terrain/TerrainAnalysisPanel.svelte";
  import { editMode, replayActive, mission, missionFlags, missionDownload, missionUpload, missionFcInfo, markMissionSynced, loadedMissionId, missionSetWaypoints } from "$lib/stores/mission";
  import { terrainAnalysis, patchTerrainAnalysis } from "$lib/stores/terrainAnalysis";
  import type { InterfaceSettings, PanelConfig } from "$lib/stores/settings";
  import { layout, GRID_DEFAULTS } from '$lib/stores/layout';
  import {
    getDefaultFlightlogPath,
    type BlackboxImportProgress,
    type Flight,
    type FlightSummary,
    type TelemetryRecord,
  } from "$lib/stores/flightlog";
  import type { TrackColorMode } from "$lib/helpers/trackColors";
  import { FLIGHT_MODE } from "$lib/helpers/trackColors";

  // ── Layout zone CSS custom properties (driven by layout store) ──
  const gridBottomHeight = $derived(
    $layout.bottomDock.sizeOverride ?? GRID_DEFAULTS.bottomDockHeight
  );
  const gridSideWidth = $derived(
    $layout.sideDock.sizeOverride ?? GRID_DEFAULTS.sideDockWidth
  );

  // Map view mode: 2D (Leaflet) or 3D (CesiumJS)
  let mapViewMode = $state<'2d' | '3d'>('2d');

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

  // Floating-window rect (must match FloatingVideoWindow's own computation) — used
  // to place the map inside the window's frame when the view is swapped.
  const FLOAT_HDR = 20; // floating-window header height
  const floatH = $derived(Math.round($videoState.floatHeightFrac * winH));
  const floatW = $derived(Math.min(Math.round(floatH * ($videoState.aspect || 16 / 9)), Math.round(winW * 0.7)));
  const floatLeft = $derived($videoState.floatSnapped ? 8 : $videoState.floatX);
  const floatTop = $derived($videoState.floatSnapped ? winH - floatH - 30 : $videoState.floatY);
  // When video is primary, the map occupies the window's body (below the header).
  const mapInFrame = $derived($videoState.videoPrimary && $videoState.status === 'live');
  const mapFrameStyle = $derived(
    `left:${floatLeft}px; top:${floatTop + FLOAT_HDR}px; width:${floatW}px; height:${floatH - FLOAT_HDR}px;`,
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
  telemetry.subscribe((t) => {
    liveTelem = t;
    // Accumulate the live flown track (RAM) for the Terrain Analyzer
    const armed = isArmed(t.armingFlags, t.lastUpdate);
    if (armed && !prevArmed) {
      clearLiveTrack();
      // warm the Copernicus tile for the current area so it's ready
      if (isValidGpsCoordinate(t.lat, t.lon)) {
        void invoke('terrain_elevation', { lat: t.lat, lon: t.lon }).catch(() => {});
      }
    }
    if (armed && isValidGpsCoordinate(t.lat, t.lon)) {
      appendLivePoint(t.lat, t.lon, t.altMsl, t.lastUpdate || Date.now());
    }
    prevArmed = armed;
  });

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
  bleDevices.subscribe((d) => { bleDeviceList = d; });

  // Settings state for the settings panel
  let attitudeRateHz = $state(5);
  let positionRateHz = $state(2);
  let airspeedEnabled = $state(false);
  let flightLoggingEnabled = $state(false);
  let flightRecordingEnabled = $state(false);
  let flightLogDbPath = $state("");
  let flightLogRawEnabled = $state(false);
  let flightLogRawAlways = $state(false);
  let defaultFlightLogPath = $state("");
  let mapProvider = $state("osm");
  let mapCacheMaxMB = $state(200);
  let cesiumIonToken = $state("");
  let altitudeCurtain3D = $state(true);
  let defaultWpAltitudeM = $state(50);
  let defaultPhTimeSec = $state(30);
  let warnAltitudeM = $state(120);
  let interfaceSettings = $state<InterfaceSettings>({
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  });
  let trackColorMode = $state<TrackColorMode>('flightmode');

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

  // Remember last selected blackbox filter type ('inav' or 'ardupilot')
  let lastBlackboxFilter = $state<string>(
    (typeof localStorage !== 'undefined' && localStorage.getItem('lastBlackboxFilter')) || 'inav'
  );
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

  const allTabs = [
    { id: "uav-info", label: () => $t('nav.uavInfo'), icon: "✈" },
    { id: "settings", label: () => $t('nav.settings'), icon: "⚙" },
    { id: "logbook", label: () => $t('nav.logbook'), icon: "📒" },
    { id: "mission", label: () => $t('nav.mission'), icon: "◎" },
    { id: "terrain", label: () => $t('nav.terrain'), icon: "⛰" },
    { id: "video", label: () => $t('nav.video'), icon: "🎥" },
  ];
  const tabs = $derived(
    flightLoggingEnabled ? allTabs : allTabs.filter(t => t.id !== 'logbook')
  );
  // Highlight the terrain rail button while its overlay is open
  const railActiveTab = $derived(terrainOpen ? 'terrain' : activeTab);

  let ports: PortInfo[] = $state([]);
  let connStatus: string = $state("disconnected");
  let fcInfo: FcInfo | null = $state(null);

  // Subscribe to stores
  connection.subscribe((c) => {
    const wasConnected = connStatus === 'connected';
    connStatus = c.status;
    fcInfo = c.fcInfo;
    // Auto-refresh the logbook on disconnect (picks up a just-recorded live flight) — replaces
    // the manual Refresh button. Disarm is covered by the flight-recording-ended listener.
    if (wasConnected && c.status !== 'connected') void loadLogbook();
  });
  availablePorts.subscribe((p) => {
    ports = p;
  });

  // Restore persisted settings
  const saved = get(settings);
  selectedPort = saved.lastPort;
  selectedBaud = saved.lastBaud;
  selectedProtocol = (saved.lastProtocol === 'mavlink' ? 'mavlink' : 'msp') as ProtocolType;
  navPanelOpen = saved.navPanelOpen;
  activeTab = saved.activeTab;
  attitudeRateHz = saved.attitudeRateHz;
  positionRateHz = saved.positionRateHz;
  airspeedEnabled = saved.airspeedEnabled;
  flightLoggingEnabled = saved.flightLoggingEnabled;
  flightRecordingEnabled = saved.flightRecordingEnabled ?? false;
  flightLogDbPath = saved.flightLogDbPath;
  flightLogRawEnabled = saved.flightLogRawEnabled;
  flightLogRawAlways = saved.flightLogRawAlways ?? false;
  mapProvider = saved.mapProvider;
  mapCacheMaxMB = saved.mapCacheMaxMB;
  cesiumIonToken = saved.cesiumIonToken ?? '';
  altitudeCurtain3D = saved.altitudeCurtain3D ?? true;
  defaultWpAltitudeM = saved.defaultWpAltitudeM;
  defaultPhTimeSec = saved.defaultPhTimeSec;
  warnAltitudeM = saved.warnAltitudeM;
  interfaceSettings = saved.interface ?? {
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  };
  panels = saved.panels ?? defaultPanels;

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

  function selectTab(tabId: string) {
    // Terrain Analysis is a full-width overlay that behaves like a floating
    // panel: toggling its tab opens/closes it, the nav rail stays open.
    if (tabId === 'terrain') {
      patchTerrainAnalysis({ open: !get(terrainAnalysis).open });
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
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
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
    homePosition.set({ lat: 0, lon: 0, alt: 0, set: false });
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

  async function importBlackbox() {
    if (blackboxImporting) return;
    try {
      const inavFilter = { name: $t('logbook.inavBlackboxFilter'), extensions: ['txt'] };
      const apFilter = { name: $t('logbook.ardupilotFilter'), extensions: ['bin'] };
      const filters = lastBlackboxFilter === 'ardupilot' ? [apFilter, inavFilter] : [inavFilter, apFilter];

      const selected = await open({
        multiple: true,
        filters,
      });
      if (!selected) return;
      const files = Array.isArray(selected) ? selected : [selected];
      if (files.length === 0) return;

      // Remember which format was picked based on file extension
      const firstExt = files[0].split('.').pop()?.toLowerCase();
      if (firstExt === 'bin') {
        lastBlackboxFilter = 'ardupilot';
      } else {
        lastBlackboxFilter = 'inav';
      }
      localStorage.setItem('lastBlackboxFilter', lastBlackboxFilter);

      blackboxImporting = true;
      for (const filePath of files) {
        if (/\.bin$/i.test(filePath)) {
          await performArdupilotImport(filePath, false);
        } else {
          await performImport(filePath, undefined, false);
        }
      }
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
    } finally {
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
  }

  async function importDroppedFiles(paths: string[]) {
    // Guard against concurrent imports (drag-drop can fire multiple times on Windows)
    if (blackboxImporting) {
      console.warn('[IMPORT] Skipping — import already in progress');
      return;
    }

    console.log('[IMPORT] importDroppedFiles called with', paths.length, 'files');

    // Set guard early to prevent concurrent calls
    const hasBbFiles = paths.some((p) => /\.(txt|bin)$/i.test(p));
    if (hasBbFiles) blackboxImporting = true;

    try {
    // Handle .kflight files
    const kflightFiles = paths.filter((p) => /\.kflight$/i.test(p));
    for (const filePath of kflightFiles) {
      try {
        const result = await logbookCtrl.importFromKflight(filePath, flightLogDbPath);
        await loadLogbook();
        let msg = $t('logbook.importKflightResult', {
          values: { imported: result.imported, skipped: result.skipped },
        });
        if (result.errors.length > 0) {
          msg += '\n' + result.errors.join('\n');
        }
        await showInfo($t('logbook.importKflightTitle'), msg);
      } catch (e: any) {
        errorMsg = e?.toString?.() ?? String(e);
      }
    }

    // Handle ArduPilot .bin files
    const binFiles = paths.filter((p) => /\.bin$/i.test(p));
    for (const filePath of binFiles) {
      await performArdupilotImport(filePath, false);
    }

    // Handle blackbox files
    const bbFiles = paths.filter((p) => /\.txt$/i.test(p));
    for (const filePath of bbFiles) {
      await performImport(filePath, undefined, false);
    }

    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
    } finally {
      if (hasBbFiles) {
        blackboxImporting = false;
        blackboxImportProgress = null;
      }
    }
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
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
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
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
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
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
    }
  }

  async function importKflightFile() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: $t('logbook.kflightFileFilter'), extensions: ['kflight'] }],
      });
      if (!selected) return;
      const filePath = Array.isArray(selected) ? selected[0] : selected;
      if (!filePath) return;
      const result = await logbookCtrl.importFromKflight(filePath, flightLogDbPath);
      await loadLogbook();
      let msg = $t('logbook.importKflightResult', {
        values: { imported: result.imported, skipped: result.skipped },
      });
      if (result.errors.length > 0) {
        msg += '\n' + result.errors.join('\n');
      }
      await showInfo($t('logbook.importKflightTitle'), msg);
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
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
          await missionSetWaypoints(JSON.parse(linked.waypoints_json));
          loadedMissionId.set(linked.id);
          markMissionSynced('db'); // it's the library mission → trusted for the highlight
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
      homePosition.set({ lat: data.flight.start_lat, lon: data.flight.start_lon, alt: 0, set: true });
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

    const value = await showDialog({
      title: $t('logbook.deleteTitle'),
      message: $t('logbook.deleteWarning'),
      buttons,
    });
    if (!value || !selectedFlightId || !selectedFlight) return;

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

  async function handleScanBle() {
    isBleScanning = true;
    try {
      const devices = await scanBleDevices();
      if (devices.length > 0 && !selectedBleDevice) {
        selectedBleDevice = devices[0].id;
      }
    } catch (e: any) {
      errorMsg = e.toString();
    } finally {
      isBleScanning = false;
    }
  }

  async function handleConnect() {
    if (connStatus === "connected") {
      try {
        await disconnectFC(selectedBaud);
        errorMsg = "";
      } catch (e: any) {
        errorMsg = e.toString();
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

    isConnecting = true;
    errorMsg = "";
    connection.update((c) => ({ ...c, status: "connecting" }));
    settings.patch({ lastPort: selectedPort, lastBaud: selectedBaud, lastProtocol: selectedProtocol, flightLoggingEnabled, flightRecordingEnabled, flightLogDbPath, flightLogRawEnabled, flightLogRawAlways });

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
        flightLogEnabled: flightRecordingEnabled,
        flightLogDbEnabled: flightLoggingEnabled && flightRecordingEnabled,
        flightLogPath: flightLogDbPath,
        flightLogRaw: flightRecordingEnabled && (!flightLoggingEnabled || flightLogRawEnabled),
        flightLogRawAlways: flightRecordingEnabled && flightLogRawAlways,
      });
    } catch (e: any) {
      errorMsg = e.toString();
      connection.set({ status: "error", protocolType: selectedProtocol, transportType: selectedTransport, port: "", baudRate: selectedBaud, errorMessage: e.toString(), fcInfo: null });
    } finally {
      isConnecting = false;
    }
  }

  async function initPage() {
    await loadInfo();
    try {
      defaultFlightLogPath = await getDefaultFlightlogPath();
    } catch {
      defaultFlightLogPath = '';
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
  const logbookDetailOpen = $derived(activeTab === 'logbook' && selectedFlight != null && !logbookMinimized);
  // The logbook panel goes wide for a selected detail entry. In the Battery Manager view the
  // width depends ONLY on the battery selection (a still-selected flight must not keep it wide);
  // in the flight view it depends on the selected flight.
  const logbookWide = $derived(
    activeTab === 'logbook' &&
    ($batteryManagerOpen ? $batteryManagerSelectedId != null : (selectedFlight != null && !logbookMinimized))
  );
  const logbookHasFlightOnMap = $derived(activeTab === 'logbook' && selectedFlight != null && !isPrimaryConnected);

  // Platform type: from live connection or selected flight log
  const mapPlatformType = $derived(
    (fcInfo as FcInfo | null)?.platform_type
      ?? (selectedFlight as Flight | null)?.platform_type
      ?? 0,
  );

  // FC variant for the selected flight (used by mode widgets + map coloring)
  const replayFcVariant = $derived((selectedFlight as Flight | null)?.fc_variant ?? 'INAV');

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
    const inWpMode = (telem.activeFlightModeFlags & FLIGHT_MODE.NAV_WP) !== 0;
    const isReplay = playbackActive && !isPrimaryConnected;
    const armed = isArmed(telem.armingFlags, telem.lastUpdate);
    const f = $missionFlags;
    let trusted = false;
    if (isReplay) trusted = f.db || replayTrackConfirmed;
    else if (isPrimaryConnected) trusted = armed && (f.fc || liveTrackConfirmed);
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
  $effect(() => {
    const armed = isArmed(telem.armingFlags, telem.lastUpdate);
    if (isPrimaryConnected && armed && !prevArmedForTrack) {
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

  // ── Mission recording link (arm-save / disarm-link) ──────────────────
  // On arm (with DB recording), save the displayed mission to the library and link it to the
  // just-created flight. On disarm, if the mission changed during the flight (e.g. a new
  // mission uploaded), offer to update the link. See docs/dev/MISSION_LIBRARY_AND_DB.md.
  let flightMissionHash: string | null = null;

  async function linkMissionToFlight(flightId: number, wps = get(mission).waypoints): Promise<void> {
    const input = await buildMissionInput(wps);
    const id = await missionDbSave(input, flightLogDbPath);
    await flightLinkMission(flightId, id, flightLogDbPath);
    markMissionSynced('db');
    loadedMissionId.set(id);
    flightMissionHash = input.content_hash;
    void missionDbGeocode(id, $locale ?? 'en', flightLogDbPath).catch(() => {});
  }

  async function onRecordingStarted(flightId: number): Promise<void> {
    flightMissionHash = null;
    const wps = get(mission).waypoints;
    // Only link the mission that is actually being flown: it must be in sync with the FC
    // (FC flag). A stale plan, or an edited-but-not-reuploaded mission, is NOT what the FC
    // flies, so we don't link it. (This also keeps arm-save INAV-only — ArduPilot uses a
    // separate mission path without this flag.)
    if (wps.length === 0 || !get(missionFlags).fc) return;
    try {
      await linkMissionToFlight(flightId, wps);
    } catch (e) {
      console.warn('[mission-link] arm-save failed', e);
    }
  }

  async function onRecordingEnded(flightId: number): Promise<void> {
    try {
      const wps = get(mission).waypoints;
      // Only consider the FC-synced mission (what was actually flown). If it isn't FC-synced
      // now, leave any arm-time link untouched.
      if (wps.length === 0 || !get(missionFlags).fc) return;
      const curHash = await missionContentHash(wps);
      if (curHash === flightMissionHash) return; // unchanged since arm → keep the link
      // Changed since arm (e.g. a new mission uploaded in-flight) or never linked at arm →
      // offer to (re)link to the current version.
      const ans = await showDialog({
        title: $t('mission.linkUpdateTitle'),
        message: $t('mission.linkUpdateMsg'),
        buttons: [{ label: $t('mission.linkUpdateYes'), value: 'update', primary: true }],
      });
      if (ans === 'update') await linkMissionToFlight(flightId, wps);
    } catch (e) {
      console.warn('[mission-link] disarm-link failed', e);
    } finally {
      flightMissionHash = null;
    }
  }

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
    void listen<{ flight_id: number }>('flight-recording-started', (event) => {
      void onRecordingStarted(event.payload.flight_id);
    });
    void listen<{ flight_id: number }>('flight-recording-ended', (event) => {
      void onRecordingEnded(event.payload.flight_id);
      void loadLogbook(); // auto-refresh the list with the just-recorded flight (replaces Refresh)
    });
    void listen<BlackboxImportProgress>('flightlog-import-progress', (event) => {
      blackboxImportProgress = event.payload;
    });
    void listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
      if (activeTab === 'logbook' && event.payload.paths?.length) {
        importDroppedFiles(event.payload.paths);
      }
    });
  }

  onDestroy(() => {
    playbackCtrl.destroy();
  });
</script>

<svelte:window bind:innerWidth={winW} bind:innerHeight={winH} />

<ConfirmDialog bind:this={confirmDialog} />
<ContextMenu />
<BatchEditPopup {interfaceSettings} />

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
    onRefreshPorts={refreshPorts}
    onScanBle={handleScanBle}
    onConnect={handleConnect}
  />
  </div>

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- Full-size video shown in the map area when the view is swapped (videoPrimary).
       Double-click to swap back. -->
  {#if mapInFrame}
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
  {/if}

  <!-- Map holder — top-level so it can sit inside the floating window's frame
       (above the docks) when swapped, without re-mounting the map. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="map-holder"
    class:in-frame={mapInFrame}
    style={mapInFrame ? mapFrameStyle : ''}
    onclick={minimizeLogbook}
  >
    {#if mapViewMode === '2d'}
      <Map
        playbackTrack={mapTrack}
        playbackPoint={playbackPoint}
        {trackColorMode}
        platformType={mapPlatformType}
        fcVariant={replayFcVariant}
        {mapViewMode}
        onToggleMapView={() => mapViewMode = mapViewMode === '2d' ? '3d' : '2d'}
      />
    {:else}
      <Map3D
        playbackTrack={mapTrack}
        playbackPoint={playbackPoint}
        {trackColorMode}
        platformType={mapPlatformType}
        fcVariant={replayFcVariant}
        {mapViewMode}
        onToggleMapView={() => mapViewMode = mapViewMode === '2d' ? '3d' : '2d'}
      />
    {/if}
  </div>

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
    {tabs}
    onToggle={toggleNavPanel}
    onSelectTab={selectTab}
  />

  <!-- Floating panel content (hidden while the terrain overlay is open) -->
  {#if navPanelOpen && !terrainOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="nav-panel" class:nav-panel-mission={activeTab === 'mission' && !$missionManagerOpen} class:nav-panel-logbook={(activeTab === 'logbook' && !logbookWide) || (activeTab === 'mission' && $missionManagerOpen && $missionManagerSelectedId == null)} class:nav-panel-wide={logbookWide || (activeTab === 'mission' && $missionManagerOpen && $missionManagerSelectedId != null)} class:nav-panel-minimized={logbookMinimized && logbookHasFlightOnMap} onclick={() => { if (logbookMinimized) expandLogbook(); }}>
      <div class="panel-content">
        <!-- UAV Info Tab -->
        {#if activeTab === "uav-info"}
          <UavInfoPanel {connStatus} {fcInfo} />

        <!-- Settings Tab -->
        {:else if activeTab === "settings"}
          <SettingsPanel
            localeValue={$locale ?? 'en'}
            {mapProvider}
            {mapCacheMaxMB}
            {cacheStats}
            {cesiumIonToken}
            {altitudeCurtain3D}
            {attitudeRateHz}
            {positionRateHz}
            {airspeedEnabled}
            {flightLoggingEnabled}
            {flightRecordingEnabled}
            {flightLogRawEnabled}
            {flightLogRawAlways}
            {flightLogDbPath}
            {defaultFlightLogPath}
            {defaultWpAltitudeM}
            {defaultPhTimeSec}
            {warnAltitudeM}
            {interfaceSettings}
            {isWidgetActive}
            {getWidgetPanelLabel}
            onPatch={(patch) => {
              settings.patch(patch);
              if (patch.attitudeRateHz != null) attitudeRateHz = patch.attitudeRateHz;
              if (patch.positionRateHz != null) positionRateHz = patch.positionRateHz;
              if (patch.airspeedEnabled != null) airspeedEnabled = patch.airspeedEnabled;
              if (patch.flightLoggingEnabled != null) flightLoggingEnabled = patch.flightLoggingEnabled;
              if (patch.flightRecordingEnabled != null) flightRecordingEnabled = patch.flightRecordingEnabled;
              if (patch.flightLogRawEnabled != null) flightLogRawEnabled = patch.flightLogRawEnabled;
              if (patch.flightLogRawAlways != null) flightLogRawAlways = patch.flightLogRawAlways;
              if (patch.flightLogDbPath != null) flightLogDbPath = patch.flightLogDbPath;
              if (patch.mapProvider != null) mapProvider = patch.mapProvider;
              if (patch.mapCacheMaxMB != null) mapCacheMaxMB = patch.mapCacheMaxMB;
              if (patch.cesiumIonToken != null) cesiumIonToken = patch.cesiumIonToken;
              if (patch.altitudeCurtain3D != null) altitudeCurtain3D = patch.altitudeCurtain3D;
              if (patch.defaultWpAltitudeM != null) defaultWpAltitudeM = patch.defaultWpAltitudeM;
              if (patch.defaultPhTimeSec != null) defaultPhTimeSec = patch.defaultPhTimeSec;
              if (patch.warnAltitudeM != null) warnAltitudeM = patch.warnAltitudeM;
              if (patch.interface != null) {
                interfaceSettings = {
                  ...interfaceSettings,
                  ...patch.interface,
                };
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
            }}
            onSetCacheMaxMB={setCacheMaxMB}
            onClearCache={clearCache}
            onChooseFlightLogPath={chooseFlightLogPath}
            onResetFlightLogPath={resetFlightLogPath}
            onToggleWidget={toggleWidget}
          />

        <!-- Logbook Tab -->
        {:else if activeTab === "logbook"}
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
            onLoadLogbook={loadLogbook}
            onImportBlackbox={importBlackbox}
            onSelectFlight={selectFlight}
            onSaveNotes={saveSelectedFlightNotes}
            onSaveWeather={saveSelectedFlightWeather}
            onSaveCraftName={saveSelectedFlightCraftName}
            onSavePilot={saveSelectedFlightPilot}
            onDeleteFlight={removeSelectedFlight}
            onExportFlights={exportFlightsToKflight}
            onExportBlackbox={exportBlackbox}
            onExportTrack={exportTrack}
            onImportKflight={importKflightFile}
          />

        <!-- Mission Tab -->
        {:else if activeTab === "mission"}
          <MissionPanel />

        <!-- Video Tab -->
        {:else if activeTab === "video"}
          <VideoPanel />
        {/if}
      </div>
    </div>
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

<style>
  /* ============================================================
     Kite Ground Control Theme — Floating Panel Layout
     Color palette derived from INAV Configurator
     https://github.com/iNavFlight/inav-configurator
     ============================================================ */

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

  .app {
    display: grid;
    height: 100vh;
    position: relative;
    grid-template-rows: 53px 1fr var(--grid-bottom-height) 24px;
    grid-template-columns: 62px 1fr var(--grid-side-width) 54px;
    grid-template-areas:
      "toolbar      toolbar      toolbar      toolbar"
      "nav-rail     panel        side-dock    side-dock"
      "nav-rail     bottom-dock  bottom-dock  map-controls"
      "status-bar   status-bar   status-bar   status-bar";
  }

  /* ── Grid zone wrappers ─────────────────────────────────── */
  .zone-toolbar {
    grid-area: toolbar;
    z-index: 200;
  }

  /* Map holder — top-level overlay over the content area (toolbar 53px / status
     bar 24px). Fills the area normally; sits inside the floating window's frame
     (rect via inline style) when the view is swapped. */
  .map-holder {
    position: absolute;
    top: 53px;
    left: 0;
    right: 0;
    bottom: 24px;
    z-index: 0;
    overflow: hidden;
  }
  .map-holder.in-frame {
    top: auto;
    right: auto;
    bottom: auto; /* left/top/width/height come from the inline rect */
    z-index: 61; /* in the floating frame, above its body; the frame draws the border */
    border-radius: 0 0 7px 7px;
  }
  /* Full-size video shown in the content area when swapped (videoPrimary) */
  .map-video {
    position: absolute;
    top: 53px;
    left: 0;
    right: 0;
    bottom: 24px;
    object-fit: cover;
    background: #000;
    z-index: 0;
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

  /* --- Floating Nav Panel --- */
  .nav-panel {
    position: absolute;
    top: 65px;
    left: 62px; /* after the rail buttons */
    width: min(360px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
    max-height: calc(100vh - 53px - var(--grid-bottom-height) - 24px - 12px);
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    z-index: 150;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(12px);
    animation: panel-slide-in 0.25s ease-out;
    transition: width 0.25s ease;
  }

  .nav-panel.nav-panel-mission {
    /* +15% over the 360px base — fits all toolbar buttons on one row and
       leaves headroom for richer WP-list entries. */
    width: min(414px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
  }

  .nav-panel.nav-panel-logbook {
    width: min(430px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
  }

  .nav-panel.nav-panel-wide {
    width: min(920px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
  }

  .nav-panel.nav-panel-minimized {
    width: min(280px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
    cursor: pointer;
  }

  @keyframes panel-slide-in {
    from {
      opacity: 0;
      transform: translateX(-16px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .panel-content {
    padding: 14px;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
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
