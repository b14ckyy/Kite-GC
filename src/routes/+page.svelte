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
  import { homePosition } from '$lib/stores/home';
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import { weatherTempDisplayFromC, weatherWindDisplayFromMs, weatherTempCFromDisplay, weatherWindMsFromDisplay, canonicalWeatherDescription } from "$lib/helpers/weather";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import { LARGE_BASE_VMIN } from "$lib/config/widgetRegistry";
  import MissionPanel from "$lib/components/mission/MissionPanel.svelte";
  import TerrainAnalysisPanel from "$lib/components/terrain/TerrainAnalysisPanel.svelte";
  import { editMode } from "$lib/stores/mission";
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

  // Per-container px-per-unit: 1 unit = cross-axis fraction so that
  // LARGE_BASE_VMIN units == cross-axis px (widget fills dock height/width).
  // This fully decouples bottom dock and side dock scaling.
  // Subtract zone padding (6px each side) from cross-axis measurement.
  const DOCK_PAD = 6;
  const bottomPxPerUnit = $derived((bottomDockH - 2 * DOCK_PAD) / LARGE_BASE_VMIN);
  const sidePxPerUnit   = $derived((sideDockW  - 2 * DOCK_PAD) / LARGE_BASE_VMIN);

  // Available space expressed in abstract units (container px / pxPerUnit)
  // Bottom: subtract edit button (28px) + wrapper gap (6px) + zone padding (12px)
  const bottomAvailUnits = $derived(Math.max(0, (bottomDockW - 34 - 2 * DOCK_PAD) / bottomPxPerUnit));
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
    connStatus = c.status;
    fcInfo = c.fcInfo;
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

  // When primary connection is established, clear playback
  $effect(() => {
    if (isPrimaryConnected && playbackActive) {
      resetPlayback();
    }
  });

  if (typeof window !== 'undefined') {
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

<ConfirmDialog bind:this={confirmDialog} />

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
  <div class="zone-map" onclick={minimizeLogbook}>
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
    <div class="nav-panel" class:nav-panel-logbook={activeTab === 'logbook' && !logbookDetailOpen} class:nav-panel-wide={logbookDetailOpen} class:nav-panel-minimized={logbookMinimized && logbookHasFlightOnMap} onclick={() => { if (logbookMinimized) expandLogbook(); }}>
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
            onDeleteFlight={removeSelectedFlight}
            onExportFlights={exportFlightsToKflight}
            onExportBlackbox={exportBlackbox}
            onExportTrack={exportTrack}
            onImportKflight={importKflightFile}
          />

        <!-- Mission Tab -->
        {:else if activeTab === "mission"}
          <MissionPanel />
        {/if}
      </div>
    </div>
  {/if}

  <!-- ======= TERRAIN ANALYSIS OVERLAY ======= -->
  {#if terrainOpen}
    <TerrainAnalysisPanel track={selectedTrackWithPosition} live={isPrimaryConnected} {interfaceSettings} confirm={showDialog} />
  {/if}

  <!-- ======= BOTTOM WIDGET PANEL ======= -->
  <div class="zone-bottom-dock" class:zone-hidden={!$layout.bottomDock.visible} class:panel-editing={widgetEditMode} bind:clientWidth={bottomDockW} bind:clientHeight={bottomDockH}>
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

  .zone-map {
    /* Map spans the full content area behind all other zones */
    grid-row: 2 / 4;
    grid-column: 1 / -1;
    z-index: 0;
    position: relative;
    overflow: hidden;
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
