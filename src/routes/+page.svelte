<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { connection, availablePorts } from "$lib/stores/connection";
  import type { FcInfo, PortInfo, FeatureSet } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import type { AppSettings } from "$lib/stores/settings";
  import { telemetry, startTelemetryListeners, stopTelemetryListeners, resetTelemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";
  import { t, locale } from 'svelte-i18n';
  import { SUPPORTED_LOCALES } from '$lib/i18n';
  import Map from "$lib/components/Map.svelte";
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import MissionPanel from "$lib/components/MissionPanel.svelte";
  import { editMode } from "$lib/stores/mission";
  import { WIDGET_DEFS, LARGE_BASE_VMIN } from "$lib/config/widgetRegistry";
  import type { PanelConfig } from "$lib/stores/settings";
  import {
    listFlights,
    getFlight,
    getFlightTrack,
    deleteFlight,
    updateFlightNotes,
    updateFlightWeather,
    geocodeFlight,
    fetchFlightWeather,
    getDefaultFlightlogPath,
    importBlackboxLog,
    buildFlightTree,
    formatDurationSec,
    type BlackboxImportProgress,
    type Flight,
    type FlightTree,
    type FlightSummary,
    type LogbookSortMode,
    type TelemetryRecord,
  } from "$lib/stores/flightlog";

  // Reactive window dimensions for dynamic panel sizing
  let winW = $state(typeof window !== 'undefined' ? window.innerWidth : 1920);
  let winH = $state(typeof window !== 'undefined' ? window.innerHeight : 1080);
  $effect(() => {
    const onResize = () => { winW = window.innerWidth; winH = window.innerHeight; };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  // Available vmin for each panel (screen space minus reserved areas)
  const vminPx = $derived(Math.min(winW, winH) / 100);
  // Bottom: left margin (2vmin) + right reserved corner (22.5vmin + 3vmin)
  const bottomAvailVmin = $derived(winW / vminPx - 2 - 22.5 - 3);
  // Right: toolbar (~60px top) + statusbar (~30px) + bottom reserved (22.5vmin)
  const rightAvailVmin = $derived((winH - 60 - 30) / vminPx - 22.5);

  let appVersion = $state("...");
  let selectedPort = $state("");
  let selectedBaud = $state(115200);
  let isConnecting = $state(false);
  let errorMsg = $state("");
  let navPanelOpen = $state(false);
  let activeTab = $state("uav-info");

  // Dev-only debug panel (tree-shaken in production builds)
  const DEV_MODE = import.meta.env.DEV;
  let debugOpen = $state(false);
  let DebugPanelCmp: any = $state(null);
  if (DEV_MODE) {
    import('$lib/components/DebugPanel.svelte').then(m => { DebugPanelCmp = m.default; });
  }

  // Reactive telemetry subscription
  let telem = $state(get(telemetry));
  telemetry.subscribe((t) => { telem = t; });

  // Settings state for the settings panel
  let attitudeRateHz = $state(5);
  let positionRateHz = $state(2);
  let airspeedEnabled = $state(false);
  let flightLoggingEnabled = $state(false);
  let flightLogDbPath = $state("");
  let flightLogRawEnabled = $state(false);
  let defaultFlightLogPath = $state("");
  let mapProvider = $state("osm");
  let mapCacheMaxMB = $state(200);
  let defaultWpAltitudeM = $state(50);
  let defaultPhTimeSec = $state(30);

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
  let logbookSortMode = $state<LogbookSortMode>("aircraft-location-date");
  let logbookTreeOpenTop = $state<Set<string>>(new Set());
  let logbookTreeOpenSecond = $state<Set<string>>(new Set());
  let prevLogbookSortMode = $state<LogbookSortMode>("aircraft-location-date");
  let playbackActive = $state(false);
  let playbackPlaying = $state(false);
  let playbackIndex = $state(0);
  let playbackTimer: ReturnType<typeof window.setInterval> | null = null;
  let logbookMinimized = $state(false);

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

  // INAV arming flags (bit positions from INAV source)
  const ARMING_FLAG_ARMED = 2; // bit 2 = ARMED

  function isArmed(): boolean {
    return telem.lastUpdate > 0 && (telem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  }

  function getArmingLabel(): string {
    if (!telem.lastUpdate) return "";
    return isArmed() ? $t('arming.armed') : $t('arming.disarmed');
  }

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[telem.fixType] || `FIX:${telem.fixType}`;
  }

  // Platform type names from flyingPlatformType_e
  function getPlatformLabel(type: number): string {
    const keys: Record<number, string> = {
      0: 'platform.multirotor', 1: 'platform.airplane', 2: 'platform.helicopter',
      3: 'platform.tricopter', 4: 'platform.rover', 5: 'platform.boat', 6: 'platform.other',
    };
    return keys[type] ? $t(keys[type]) : $t('platform.unknown', { values: { type } });
  }

  const baudRates = [115200, 57600, 38400, 19200, 9600, 230400, 460800, 921600];

  const tabs = [
    { id: "uav-info", label: () => $t('nav.uavInfo'), icon: "✈" },
    { id: "settings", label: () => $t('nav.settings'), icon: "⚙" },
    { id: "logbook", label: () => $t('nav.logbook'), icon: "📒" },
    { id: "mission", label: () => $t('nav.mission'), icon: "◎" },
  ];

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
  navPanelOpen = saved.navPanelOpen;
  activeTab = saved.activeTab;
  attitudeRateHz = saved.attitudeRateHz;
  positionRateHz = saved.positionRateHz;
  airspeedEnabled = saved.airspeedEnabled;
  flightLoggingEnabled = saved.flightLoggingEnabled;
  flightLogDbPath = saved.flightLogDbPath;
  flightLogRawEnabled = saved.flightLogRawEnabled;
  mapProvider = saved.mapProvider;
  mapCacheMaxMB = saved.mapCacheMaxMB;
  defaultWpAltitudeM = saved.defaultWpAltitudeM;
  defaultPhTimeSec = saved.defaultPhTimeSec;
  panels = saved.panels ?? defaultPanels;

  function toggleNavPanel() {
    navPanelOpen = !navPanelOpen;
    if (!navPanelOpen) editMode.set(false);
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

  function autoResizeNotes(el: HTMLTextAreaElement) {
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 140) + 'px';
  }

  function notesAutoSize(el: HTMLTextAreaElement) {
    autoResizeNotes(el);
    return { update() { autoResizeNotes(el); } };
  }

  function selectTab(tabId: string) {
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
      flightSummaries = await listFlights(flightLogDbPath);
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

  function formatFlightSource(source: string): string {
    if (source === 'blackbox') return $t('logbook.sourceBlackbox');
    if (source === 'both') return $t('logbook.sourceBoth');
    return $t('logbook.sourceLive');
  }

  function windDegToLabel(deg: number): string {
    const dirs = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
    return dirs[Math.round(deg / 45) % 8];
  }

  function flightListMarker(source: string): string {
    if (source === 'blackbox') return '◈ ';
    if (source === 'both') return '◉ ';
    return '';
  }

  function stopPlayback() {
    playbackPlaying = false;
    if (playbackTimer != null) {
      window.clearInterval(playbackTimer);
      playbackTimer = null;
    }
  }

  function resetPlayback() {
    stopPlayback();
    playbackActive = false;
    playbackIndex = 0;
  }

  function startPlayback() {
    if (selectedTrackWithPosition.length <= 1) return;
    playbackActive = true;
    if (playbackIndex >= selectedTrackWithPosition.length - 1) {
      playbackIndex = 0;
    }
    stopPlayback();
    playbackPlaying = true;
    playbackTimer = window.setInterval(() => {
      if (playbackIndex >= selectedTrackWithPosition.length - 1) {
        stopPlayback();
        return;
      }
      playbackIndex += 1;
    }, 120);
  }

  async function importBlackbox() {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: $t('logbook.blackboxFileFilter'), extensions: ['bbl', 'bfl', 'csv', 'txt'] }],
      });
      if (!selected) return;
      const files = Array.isArray(selected) ? selected : [selected];
      if (files.length === 0) return;

      blackboxImporting = true;
      for (const filePath of files) {
        await performImport(filePath, undefined, false);
      }
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
    } finally {
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
  }

  async function importDroppedFiles(paths: string[]) {
    const bbFiles = paths.filter((p) => /\.(bbl|bfl|csv|txt)$/i.test(p));
    if (bbFiles.length === 0) return;
    try {
      blackboxImporting = true;
      for (const filePath of bbFiles) {
        await performImport(filePath, undefined, false);
      }
    } catch (e: any) {
      errorMsg = e?.toString?.() ?? String(e);
    } finally {
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
  }

  async function performImport(filePath: string, logIndex: number | undefined, forceImport: boolean) {
    const result = await importBlackboxLog(filePath, flightLogDbPath, logIndex, forceImport, $locale ?? 'en');
    
    if (result.type === 'duplicate') {
      const confirmImport = await confirm(
        $t('logbook.duplicateMessage', {
          values: {
            craft: result.duplicate_craft_name,
            time: new Date(result.duplicate_start_time).toLocaleString(),
          },
        }),
        { title: $t('logbook.duplicateTitle'), kind: 'warning' }
      );
      
      if (confirmImport) {
        // Retry import with force_import: true
        await performImport(filePath, logIndex, true);
      }
    } else {
      // Success case
      await loadLogbook();
      await selectFlight(result.flight_id);
    }
  }

  async function selectFlight(flightId: number) {
    selectedFlightId = flightId;
    selectedFlight = await getFlight(flightId, flightLogDbPath);
    selectedFlightNotes = selectedFlight?.notes ?? '';
    weatherTempC = selectedFlight?.weather_temp_c != null ? String(selectedFlight.weather_temp_c) : '';
    weatherWindMs = selectedFlight?.weather_wind_ms != null ? String(selectedFlight.weather_wind_ms) : '';
    weatherWindDir = selectedFlight?.weather_wind_deg != null ? String(selectedFlight.weather_wind_deg) : '';
    weatherDesc = selectedFlight?.weather_desc ?? '';
    weatherEditing = false;

    const track = await getFlightTrack(flightId, flightLogDbPath);
    selectedFlightTrack = track;
    selectedFlightTrackCount = track.length;
    resetPlayback();

    // Lazy enrich metadata if still missing
    if (selectedFlight && !hasKnownLocation(selectedFlight.location_name) && selectedFlight.start_lat != null && selectedFlight.start_lon != null) {
      const geocoded = await geocodeFlight(flightId, flightLogDbPath, $locale ?? 'en');
      if (geocoded) {
        selectedFlight = { ...selectedFlight, location_name: geocoded };
      }
    }
    if (
      selectedFlight &&
      selectedFlight.source !== 'blackbox' &&
      selectedFlight.weather_temp_c == null &&
      selectedFlight.start_lat != null &&
      selectedFlight.start_lon != null
    ) {
      await fetchFlightWeather(flightId, flightLogDbPath);
      selectedFlight = await getFlight(flightId, flightLogDbPath);
    }

    // Keep list in sync if metadata got enriched
    await loadLogbook();
  }

  async function saveSelectedFlightNotes() {
    if (!selectedFlightId) return;
    await updateFlightNotes(selectedFlightId, selectedFlightNotes, flightLogDbPath);
    selectedFlight = await getFlight(selectedFlightId, flightLogDbPath);
    await loadLogbook();
  }

  async function saveSelectedFlightWeather() {
    if (!selectedFlightId) return;
    const temp = weatherTempC !== '' && weatherTempC != null && !isNaN(Number(weatherTempC)) ? Number(weatherTempC) : null;
    const wind = weatherWindMs !== '' && weatherWindMs != null && !isNaN(Number(weatherWindMs)) ? Number(weatherWindMs) : null;
    const dir = weatherWindDir !== '' && weatherWindDir != null ? parseInt(String(weatherWindDir), 10) : null;
    const desc = typeof weatherDesc === 'string' && weatherDesc.trim() ? weatherDesc.trim() : null;
    await updateFlightWeather(selectedFlightId, temp, wind, dir, desc, flightLogDbPath);
    selectedFlight = await getFlight(selectedFlightId, flightLogDbPath);
    weatherEditing = false;
  }

  async function removeSelectedFlight() {
    if (!selectedFlightId) return;
    const ok = await deleteFlight(selectedFlightId, flightLogDbPath);
    if (!ok) return;
    resetPlayback();
    selectedFlight = null;
    selectedFlightTrack = [];
    selectedFlightId = null;
    selectedFlightTrackCount = 0;
    selectedFlightNotes = '';
    weatherTempC = '';
    weatherWindMs = '';
    weatherWindDir = '';
    weatherDesc = '';
    weatherEditing = false;
    await loadLogbook();
  }

  function formatDateTime(value: string): string {
    const d = new Date(value);
    return d.toLocaleString();
  }

  function hasKnownLocation(name: string | null | undefined): boolean {
    const n = (name ?? '').trim();
    if (!n) return false;
    return n !== 'Unknown location' && n !== 'Unbekannter Ort';
  }

  function isValidGpsCoordinate(lat: number | null | undefined, lon: number | null | undefined): boolean {
    if (lat == null || lon == null) return false;
    if (!Number.isFinite(lat) || !Number.isFinite(lon)) return false;
    if (lat < -90 || lat > 90 || lon < -180 || lon > 180) return false;
    return !(lat === 0 && lon === 0);
  }

  async function loadInfo() {
    appVersion = await invoke("get_app_version");
    await refreshPorts();
  }

  async function refreshPorts() {
    try {
      const result = await invoke<PortInfo[]>("list_serial_ports");
      availablePorts.set(result);
      if (result.length > 0 && !selectedPort) {
        selectedPort = result[0].path;
      }
      if (selectedPort && result.some((p) => p.path === selectedPort)) {
        // port still valid
      } else if (result.length > 0) {
        selectedPort = result[0].path;
      }
    } catch (e) {
      console.error("Failed to list ports:", e);
    }
  }

  async function handleConnect() {
    if (connStatus === "connected") {
      try {
        stopTelemetryListeners();
        resetTelemetry();
        await invoke("disconnect");
        connection.set({
          status: "disconnected",
          port: "",
          baudRate: selectedBaud,
          errorMessage: "",
          fcInfo: null,
        });
        errorMsg = "";
      } catch (e: any) {
        errorMsg = e.toString();
      }
      return;
    }

    if (!selectedPort) {
      errorMsg = $t('connection.noPortSelected');
      return;
    }

    isConnecting = true;
    errorMsg = "";
    connection.update((c) => ({ ...c, status: "connecting" }));

    settings.patch({
      lastPort: selectedPort,
      lastBaud: selectedBaud,
      flightLoggingEnabled,
      flightLogDbPath,
      flightLogRawEnabled,
    });

    try {
      const info = await invoke<FcInfo>("connect", {
        port: selectedPort,
        baudRate: selectedBaud,
        attitudeRateHz: attitudeRateHz,
        positionRateHz: positionRateHz,
        airspeedEnabled: airspeedEnabled,
        flightLogEnabled: flightLoggingEnabled,
        flightLogPath: flightLogDbPath,
        flightLogRaw: flightLogRawEnabled,
      });
      connection.set({
        status: "connected",
        port: selectedPort,
        baudRate: selectedBaud,
        errorMessage: "",
        fcInfo: info,
      });
      await startTelemetryListeners();
    } catch (e: any) {
      errorMsg = e.toString();
      connection.set({
        status: "error",
        port: "",
        baudRate: selectedBaud,
        errorMessage: e.toString(),
        fcInfo: null,
      });
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

  // Widget panel management
  function handleReorder(panelId: string, newIds: string[]) {
    panels = { ...panels, [panelId]: newIds };
    settings.patch({ panels });
  }

  function handleReceive(targetPanel: string, widgetId: string, index: number) {
    // Remove from source panel
    const newPanels = { ...panels };
    for (const key of ['bottom', 'right'] as const) {
      newPanels[key] = newPanels[key].filter(id => id !== widgetId);
    }
    // Insert into target
    const targetList = [...newPanels[targetPanel as 'bottom' | 'right']];
    targetList.splice(index, 0, widgetId);
    newPanels[targetPanel as 'bottom' | 'right'] = targetList;
    // Remember position
    newPanels.positions = { ...newPanels.positions, [widgetId]: targetPanel as 'bottom' | 'right' };
    panels = newPanels;
    settings.patch({ panels });
  }

  function toggleWidget(widgetId: string) {
    const allAssigned = [...panels.bottom, ...panels.right];
    if (allAssigned.includes(widgetId)) {
      // Remember current panel before removing
      const currentPanel = panels.bottom.includes(widgetId) ? 'bottom' : 'right';
      const newPositions = { ...panels.positions, [widgetId]: currentPanel as 'bottom' | 'right' };
      panels = {
        bottom: panels.bottom.filter(id => id !== widgetId),
        right: panels.right.filter(id => id !== widgetId),
        positions: newPositions,
      };
    } else {
      // Restore to last known panel, default to bottom
      const target = panels.positions?.[widgetId] ?? 'bottom';
      panels = { ...panels, [target]: [...panels[target], widgetId] };
    }
    settings.patch({ panels });
  }

  function isWidgetActive(widgetId: string): boolean {
    return panels.bottom.includes(widgetId) || panels.right.includes(widgetId);
  }

  function getWidgetPanelLabel(widgetId: string): string {
    if (panels.bottom.includes(widgetId)) return $t('widgets.bottom');
    if (panels.right.includes(widgetId)) return $t('widgets.right');
    return $t('widgets.off');
  }

  function treeTopId(topKey: string): string {
    return `${logbookSortMode}::${topKey}`;
  }

  function treeSecondId(topKey: string, secondKey: string): string {
    return `${treeTopId(topKey)}::${secondKey}`;
  }

  function isTopOpen(topKey: string): boolean {
    return logbookTreeOpenTop.has(treeTopId(topKey));
  }

  function isSecondOpen(topKey: string, secondKey: string): boolean {
    return logbookTreeOpenSecond.has(treeSecondId(topKey, secondKey));
  }

  function toggleTop(topKey: string) {
    const id = treeTopId(topKey);
    const nextTop = new Set(logbookTreeOpenTop);
    const nextSecond = new Set(logbookTreeOpenSecond);
    if (nextTop.has(id)) {
      nextTop.delete(id);
      for (const secondId of nextSecond) {
        if (secondId.startsWith(`${id}::`)) {
          nextSecond.delete(secondId);
        }
      }
    } else {
      nextTop.add(id);
    }
    logbookTreeOpenTop = nextTop;
    logbookTreeOpenSecond = nextSecond;
  }

  function toggleSecond(topKey: string, secondKey: string) {
    const id = treeSecondId(topKey, secondKey);
    const next = new Set(logbookTreeOpenSecond);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    logbookTreeOpenSecond = next;
  }

  $effect(() => {
    if (prevLogbookSortMode === logbookSortMode) return;
    prevLogbookSortMode = logbookSortMode;
    logbookTreeOpenTop = new Set();
    logbookTreeOpenSecond = new Set();
  });

  const flightTree = $derived<FlightTree>(buildFlightTree(flightSummaries, logbookSortMode));
  const selectedTrackWithPosition = $derived(
    selectedFlightTrack.filter((point) => isValidGpsCoordinate(point.lat, point.lon))
  );
  const playbackPoint = $derived(
    playbackActive && selectedTrackWithPosition.length > 0
      ? selectedTrackWithPosition[Math.min(playbackIndex, selectedTrackWithPosition.length - 1)]
      : null,
  );
  const logbookDetailOpen = $derived(activeTab === 'logbook' && selectedFlight != null && !logbookMinimized);
  const logbookHasFlightOnMap = $derived(activeTab === 'logbook' && selectedFlight != null);

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
    stopPlayback();
  });
</script>

<main class="app">
  <!-- ======= TOOLBAR ======= -->
  <header class="toolbar">
    <div class="toolbar-left">
      <div class="logo">{$t('app.brand')}</div>
      <span class="version">v{appVersion}</span>
    </div>
    <div class="toolbar-center">
      <div class="sensor-bar">
        <div class="sensor" class:active={telem.sensorGyro === 1} class:error={telem.sensorGyro === 3} title={$t('sensors.gyroTooltip')}>{$t('sensors.gyro')}</div>
        <div class="sensor" class:active={telem.sensorAcc === 1} class:error={telem.sensorAcc === 3} title={$t('sensors.accTooltip')}>{$t('sensors.acc')}</div>
        <div class="sensor" class:active={telem.sensorMag === 1} class:error={telem.sensorMag === 3} title={$t('sensors.magTooltip')}>{$t('sensors.mag')}</div>
        <div class="sensor" class:active={telem.sensorBaro === 1} class:error={telem.sensorBaro === 3} title={$t('sensors.baroTooltip')}>{$t('sensors.baro')}</div>
        <div class="sensor" class:active={telem.sensorGps === 1 && telem.fixType >= 2} class:warning={telem.sensorGps === 1 && telem.fixType < 2} class:error={telem.sensorGps === 3} title="GPS: {getGpsFixLabel()} {telem.numSat}S">{$t('sensors.gps')}</div>
      </div>
    </div>
    <div class="toolbar-right">
      <div class="port-controls">
        {#if connStatus !== "connected"}
          <select class="port-select" bind:value={selectedPort}>
            {#if ports.length === 0}
              <option value="">{$t('connection.noPortsFound')}</option>
            {:else}
              {#each ports as port}
                <option value={port.path}>{port.label}</option>
              {/each}
            {/if}
          </select>
          <select class="baud-select" bind:value={selectedBaud}>
            {#each baudRates as baud}
              <option value={baud}>{baud}</option>
            {/each}
          </select>
          <button class="refresh-btn" onclick={refreshPorts} title={$t('connection.refreshPorts')}>⟳</button>
        {/if}
      </div>
      <button
        class="connect-btn"
        class:connected={connStatus === "connected"}
        class:connecting={isConnecting}
        onclick={handleConnect}
        disabled={isConnecting}
      >
        {#if isConnecting}
          {$t('connection.connecting')}
        {:else if connStatus === "connected"}
          {$t('connection.disconnect')}
        {:else}
          {$t('connection.connect')}
        {/if}
      </button>
    </div>
  </header>

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="map-fullscreen" onclick={minimizeLogbook}>
    <Map playbackTrack={selectedTrackWithPosition} playbackPoint={playbackPoint} />
  </div>

  <!-- ======= FLOATING NAV PANEL SYSTEM ======= -->
  <div class="nav-rail" class:open={navPanelOpen}>
    <!-- Hamburger button -->
    <button class="hamburger-btn" onclick={toggleNavPanel} title={navPanelOpen ? $t('nav.closePanel') : $t('nav.openPanel')}>
      <span class="hamburger-icon" class:open={navPanelOpen}>
        <span></span>
        <span></span>
        <span></span>
      </span>
    </button>

    <!-- Tab buttons (visible only when panel is open) -->
    {#if navPanelOpen}
      <div class="tab-buttons">
        {#each tabs as tab}
          <button
            class="tab-btn"
            class:active={activeTab === tab.id}
            onclick={() => selectTab(tab.id)}
            title={tab.label()}
          >
            <span class="tab-icon">{tab.icon}</span>
            <span class="tab-label">{tab.label()}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Floating panel content -->
  {#if navPanelOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="nav-panel" class:nav-panel-wide={logbookDetailOpen} class:nav-panel-minimized={logbookMinimized && logbookHasFlightOnMap} onclick={() => { if (logbookMinimized) expandLogbook(); }}>
      <div class="panel-content">
        <!-- UAV Info Tab -->
        {#if activeTab === "uav-info"}
          {#if connStatus === "connected" && fcInfo}
            <section class="panel-section">
              <h4 class="section-heading">{$t('uavInfo.flightController')}</h4>
              <div class="fc-info-grid">
                <span class="fc-label">{$t('uavInfo.craftName')}</span>
                <span class="fc-value">{fcInfo.craft_name || $t('uavInfo.craftNameUnset')}</span>
                <span class="fc-label">{$t('uavInfo.variant')}</span>
                <span class="fc-value">{fcInfo.fc_variant}</span>
                <span class="fc-label">{$t('uavInfo.version')}</span>
                <span class="fc-value">{fcInfo.fc_version}</span>
                <span class="fc-label">{$t('uavInfo.board')}</span>
                <span class="fc-value">{fcInfo.board_id}</span>
                <span class="fc-label">{$t('uavInfo.type')}</span>
                <span class="fc-value">{getPlatformLabel(fcInfo.platform_type)}</span>
                <span class="fc-label">{$t('uavInfo.api')}</span>
                <span class="fc-value">{fcInfo.api_version}</span>
                {#if fcInfo.hardware_revision > 0}
                  <span class="fc-label">{$t('uavInfo.hwRev')}</span>
                  <span class="fc-value">{fcInfo.hardware_revision}</span>
                {/if}
              </div>
            </section>

            {#if fcInfo.features}
              <section class="panel-section">
                <h4 class="section-heading">{$t('uavInfo.features')}</h4>
                <div class="feature-list">
                  <span class="feature-badge available">{$t('uavInfo.telemetry')}</span>
                  <span class="feature-badge" class:available={fcInfo.features.autoland_config} class:unavailable={!fcInfo.features.autoland_config} title="INAV 7.1+">{$t('uavInfo.autoland')}</span>
                  <span class="feature-badge" class:available={fcInfo.features.geozones} class:unavailable={!fcInfo.features.geozones} title="INAV 8.0+">{$t('uavInfo.geozones')}</span>
                  <span class="feature-badge" class:available={fcInfo.features.msp_rc} class:unavailable={!fcInfo.features.msp_rc} title="INAV 8.0+">{$t('uavInfo.mspRc')}</span>
                  <span class="feature-badge" class:available={fcInfo.features.aux_rc} class:unavailable={!fcInfo.features.aux_rc} title="INAV 9.1+">{$t('uavInfo.auxRc')}</span>
                </div>
              </section>
            {/if}
          {:else}
            <div class="panel-empty">
              <span class="panel-empty-icon">⊘</span>
              <span>{$t('uavInfo.notConnected')}</span>
            </div>
          {/if}

        <!-- Settings Tab -->
        {:else if activeTab === "settings"}
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.language')}</h4>
            <div class="setting-row">
              <label class="setting-label" for="lang-select">{$t('settings.language')}</label>
              <select id="lang-select" class="setting-select" value={$locale}
                      onchange={(e) => { const v = (e.target as HTMLSelectElement).value; locale.set(v); settings.patch({ locale: v }); }}>
                {#each SUPPORTED_LOCALES as loc}
                  <option value={loc.code}>{loc.label}</option>
                {/each}
              </select>
            </div>
          </section>

          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.map')}</h4>
            <div class="setting-row">
              <label class="setting-label" for="map-provider">{$t('settings.tileProvider')}</label>
              <select id="map-provider" class="setting-select" bind:value={mapProvider} onchange={() => settings.patch({ mapProvider })}>
                {#each MAP_PROVIDERS as p}
                  <option value={p.id}>{p.label}</option>
                {/each}
              </select>
            </div>
            <div class="setting-row">
              <label class="setting-label" for="map-cache">{$t('settings.tileCache')}</label>
              <select id="map-cache" class="setting-select" bind:value={mapCacheMaxMB} onchange={() => { settings.patch({ mapCacheMaxMB }); setCacheMaxMB(mapCacheMaxMB); }}>
                <option value={0}>{$t('settings.noCache')}</option>
                <option value={100}>100 MB</option>
                <option value={200}>200 MB</option>
                <option value={500}>500 MB</option>
                <option value={1000}>1000 MB</option>
              </select>
            </div>
            {#if mapCacheMaxMB > 0}
              <div class="cache-bar-container">
                <div class="cache-bar-track">
                  <div
                    class="cache-bar-fill"
                    class:cache-bar-warning={cacheStats.maxBytes > 0 && cacheStats.usedBytes / cacheStats.maxBytes > 0.85}
                    style="width: {cacheStats.maxBytes > 0 ? Math.min(100, cacheStats.usedBytes / cacheStats.maxBytes * 100).toFixed(1) : 0}%"
                  ></div>
                </div>
                <span class="cache-bar-label">
                  {(cacheStats.usedBytes / 1024 / 1024).toFixed(1)} / {mapCacheMaxMB} MB · {cacheStats.tileCount} tiles
                </span>
                <button class="cache-clear-btn" onclick={() => clearCache()} title={$t('settings.clear')}>{$t('settings.clear')}</button>
              </div>
            {/if}
          </section>
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.telemetryRates')}</h4>
            <div class="setting-row">
              <label class="setting-label" for="attitude-rate">{$t('settings.attitude')}</label>
              <select id="attitude-rate" class="setting-select" bind:value={attitudeRateHz} onchange={() => settings.patch({ attitudeRateHz })}>
                <option value={1}>1 Hz</option>
                <option value={2}>2 Hz</option>
                <option value={3}>3 Hz</option>
                <option value={5}>5 Hz</option>
              </select>
            </div>
            <div class="setting-row">
              <label class="setting-label" for="position-rate">{$t('settings.gpsPosition')}</label>
              <select id="position-rate" class="setting-select" bind:value={positionRateHz} onchange={() => settings.patch({ positionRateHz })}>
                <option value={1}>1 Hz</option>
                <option value={2}>2 Hz</option>
                <option value={3}>3 Hz</option>
                <option value={5}>5 Hz</option>
              </select>
            </div>
          </section>
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.optionalModules')}</h4>
            <div class="setting-row">
              <label class="setting-label" for="airspeed-toggle">{$t('settings.airspeed')}</label>
              <label class="toggle-switch">
                <input type="checkbox" id="airspeed-toggle" bind:checked={airspeedEnabled} onchange={() => settings.patch({ airspeedEnabled })} />
                <span class="toggle-slider"></span>
              </label>
            </div>
          </section>
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.flightLogging')}</h4>
            <div class="setting-row">
              <label class="setting-label" for="flightlog-enabled">{$t('settings.enableFlightLogging')}</label>
              <label class="toggle-switch">
                <input
                  type="checkbox"
                  id="flightlog-enabled"
                  bind:checked={flightLoggingEnabled}
                  onchange={() => settings.patch({ flightLoggingEnabled })}
                />
                <span class="toggle-slider"></span>
              </label>
            </div>
            <div class="setting-row">
              <label class="setting-label" for="flightlog-raw">{$t('settings.rawFlightLogs')}</label>
              <label class="toggle-switch">
                <input
                  type="checkbox"
                  id="flightlog-raw"
                  bind:checked={flightLogRawEnabled}
                  onchange={() => settings.patch({ flightLogRawEnabled })}
                />
                <span class="toggle-slider"></span>
              </label>
            </div>
            <div class="setting-row setting-row-stack">
              <span class="setting-label">{$t('settings.flightLogDbPath')}</span>
              <div class="path-picker-row">
                <input
                  class="setting-input path-input"
                  type="text"
                  readonly
                  value={flightLogDbPath || defaultFlightLogPath || $t('settings.defaultPathUnknown')}
                />
                <button class="cache-clear-btn" onclick={chooseFlightLogPath}>{$t('settings.choose')}</button>
                <button class="cache-clear-btn" onclick={resetFlightLogPath}>{$t('settings.useDefault')}</button>
              </div>
            </div>
            <p class="setting-hint">{$t('settings.flightLogHint')}</p>
          </section>
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.missionControl')}</h4>
            <div class="setting-row">
              <label class="setting-label">{$t('settings.defaultWpAlt')}</label>
              <div class="setting-stepper">
                <button class="stepper-btn" onclick={() => { defaultWpAltitudeM = Math.max(1, defaultWpAltitudeM - 1); settings.patch({ defaultWpAltitudeM }); }}>−</button>
                <input type="number" class="stepper-input" min="1" max="1000" bind:value={defaultWpAltitudeM}
                       onchange={() => { defaultWpAltitudeM = Math.max(1, Math.min(1000, defaultWpAltitudeM)); settings.patch({ defaultWpAltitudeM }); }} />
                <button class="stepper-btn" onclick={() => { defaultWpAltitudeM = Math.min(1000, defaultWpAltitudeM + 1); settings.patch({ defaultWpAltitudeM }); }}>+</button>
                <span class="setting-unit">m</span>
              </div>
            </div>
            <div class="setting-row">
              <label class="setting-label">{$t('settings.defaultPhTime')}</label>
              <div class="setting-stepper">
                <button class="stepper-btn" onclick={() => { defaultPhTimeSec = Math.max(1, defaultPhTimeSec - 1); settings.patch({ defaultPhTimeSec }); }}>−</button>
                <input type="number" class="stepper-input" min="1" max="600" bind:value={defaultPhTimeSec}
                       onchange={() => { defaultPhTimeSec = Math.max(1, Math.min(600, defaultPhTimeSec)); settings.patch({ defaultPhTimeSec }); }} />
                <button class="stepper-btn" onclick={() => { defaultPhTimeSec = Math.min(600, defaultPhTimeSec + 1); settings.patch({ defaultPhTimeSec }); }}>+</button>
                <span class="setting-unit">s</span>
              </div>
            </div>
          </section>
          <section class="panel-section">
            <h4 class="section-heading">{$t('settings.hudWidgets')}</h4>
            {#each WIDGET_DEFS as wdef}
              <div class="setting-row">
                <label class="setting-label">{$t(wdef.labelKey)}</label>
                <div class="widget-toggle-group">
                  <span class="widget-panel-indicator">{getWidgetPanelLabel(wdef.id)}</span>
                  <label class="toggle-switch">
                    <input type="checkbox" checked={isWidgetActive(wdef.id)} onchange={() => toggleWidget(wdef.id)} />
                    <span class="toggle-slider"></span>
                  </label>
                </div>
              </div>
            {/each}
          </section>
          <section class="panel-section">
            <p class="setting-hint">{$t('settings.rateHint')}</p>
          </section>

        <!-- Logbook Tab -->
        {:else if activeTab === "logbook"}
          <section class="panel-section">
            <h4 class="section-heading">{$t('logbook.title')}</h4>

            {#if !flightLoggingEnabled}
              <div class="panel-empty">
                <span class="panel-empty-icon">⊘</span>
                <span>{$t('logbook.disabled')}</span>
              </div>
            {:else if logbookMinimized && selectedFlight}
              <!-- Minimized: metadata + notes only -->
              <div class="logbook-detail logbook-detail-minimized">
                <div class="fc-info-grid">
                  <span class="fc-label">{$t('logbook.craft')}</span>
                  <span class="fc-value">{selectedFlight.craft_name || $t('logbook.unnamedCraft')}</span>
                  <span class="fc-label">{$t('logbook.firmware')}</span>
                  <span class="fc-value">{selectedFlight.fc_version || `${selectedFlight.fc_variant || '—'}`}</span>
                  <span class="fc-label">{$t('logbook.source')}</span>
                  <span class="fc-value">{formatFlightSource(selectedFlight.source)}</span>
                  <span class="fc-label">{$t('logbook.started')}</span>
                  <span class="fc-value">{formatDateTime(selectedFlight.start_time)}</span>
                  <span class="fc-label">{$t('logbook.duration')}</span>
                  <span class="fc-value">{formatDurationSec(selectedFlight.duration_sec)}</span>
                  <span class="fc-label">{$t('logbook.location')}</span>
                  <span class="fc-value">{selectedFlight.location_name || $t('logbook.unknownLocation')}</span>
                  <span class="fc-label">{$t('logbook.maxAlt')}</span>
                  <span class="fc-value">{selectedFlight.max_alt_m?.toFixed(1) ?? '—'} m</span>
                  <span class="fc-label">{$t('logbook.maxSpeed')}</span>
                  <span class="fc-value">{selectedFlight.max_speed_ms?.toFixed(1) ?? '—'} m/s</span>
                  <span class="fc-label">{$t('logbook.totalDistance')}</span>
                  <span class="fc-value">{selectedFlight.total_distance_m?.toFixed(0) ?? '—'} m</span>
                  <span class="fc-label">{$t('logbook.maxDistance')}</span>
                  <span class="fc-value">{selectedFlight.max_distance_m?.toFixed(0) ?? '—'} m</span>
                  <span class="fc-label">{$t('logbook.batteryUsed')}</span>
                  <span class="fc-value">{selectedFlight.battery_used_mah ?? '—'} mAh</span>
                  <span class="fc-label">{$t('logbook.trackPoints')}</span>
                  <span class="fc-value">{selectedFlightTrackCount}</span>
                  <span class="fc-label">{$t('logbook.weather')}</span>
                  <span class="fc-value">
                    {#if selectedFlight.weather_temp_c != null || selectedFlight.weather_desc}
                      {selectedFlight.weather_temp_c != null ? selectedFlight.weather_temp_c.toFixed(1) + ' °C' : ''}
                      {selectedFlight.weather_wind_ms != null ? ', ' + selectedFlight.weather_wind_ms.toFixed(1) + ' m/s' : ''}
                      {selectedFlight.weather_wind_deg != null ? ' ' + windDegToLabel(selectedFlight.weather_wind_deg) : ''}
                      {selectedFlight.weather_desc ? ', ' + selectedFlight.weather_desc : ''}
                    {:else}
                      {$t('logbook.weatherUnavailable')}
                    {/if}
                  </span>
                </div>

                <div class="setting-row setting-row-stack">
                  <label class="setting-label" for="flight-notes-min">{$t('logbook.notes')}</label>
                  <textarea
                    id="flight-notes-min"
                    class="setting-input notes-input notes-input-auto"
                    rows="2"
                    readonly
                    bind:value={selectedFlightNotes}
                    use:notesAutoSize
                  ></textarea>
                </div>
                <div class="setting-row">
                  <button class="cache-clear-btn" onclick={saveSelectedFlightNotes}>{$t('logbook.saveNotes')}</button>
                  <button class="cache-clear-btn logbook-danger" onclick={removeSelectedFlight}>{$t('logbook.deleteFlight')}</button>
                </div>
              </div>
            {:else}
              <div class="setting-row">
                <label class="setting-label" for="logbook-sort">{$t('logbook.sortMode')}</label>
                <select id="logbook-sort" class="setting-select" bind:value={logbookSortMode}>
                  <option value="aircraft-location-date">{$t('logbook.sortAircraftLocationDate')}</option>
                  <option value="location-date-aircraft">{$t('logbook.sortLocationDateAircraft')}</option>
                  <option value="date-location-aircraft">{$t('logbook.sortDateLocationAircraft')}</option>
                  <option value="aircraft-date-location">{$t('logbook.sortAircraftDateLocation')}</option>
                </select>
              </div>

              <div class="setting-row">
                <button class="cache-clear-btn" onclick={loadLogbook} disabled={logbookLoading}>
                  {#if logbookLoading}
                    {$t('logbook.loading')}
                  {:else}
                    {$t('logbook.refresh')}
                  {/if}
                </button>
                <button class="cache-clear-btn" onclick={importBlackbox} disabled={blackboxImporting}>
                  {#if blackboxImporting}
                    {$t('logbook.importingBlackbox')}
                  {:else}
                    {$t('logbook.importBlackbox')}
                  {/if}
                </button>
              </div>

              {#if blackboxImportProgress}
                <div class="logbook-progress">
                  <div class="logbook-progress-head">
                    <span>{$t('logbook.importProgress')}</span>
                    <span>{blackboxImportProgress.progress}%</span>
                  </div>
                  <div class="logbook-progress-bar">
                    <div class="logbook-progress-fill" style={`width: ${blackboxImportProgress.progress}%`}></div>
                  </div>
                  <div class="logbook-progress-message">{blackboxImportProgress.message}</div>
                </div>
              {/if}

              {#if flightSummaries.length === 0}
                <div class="panel-empty">
                  <span class="panel-empty-icon">🗂</span>
                  <span>{$t('logbook.empty')}</span>
                </div>
              {:else}
                <div class="logbook-layout">
                  <div class="logbook-list">
                    {#each flightTree.groups as top}
                      <div class="logbook-tree-node">
                        <button class="logbook-tree-toggle logbook-tree-toggle-top" onclick={() => toggleTop(top.key)}>
                          <span class="logbook-tree-caret">{isTopOpen(top.key) ? '▾' : '▸'}</span>
                          <span class="logbook-tree-label">{top.key}</span>
                          <span class="logbook-tree-count">{top.flight_count}</span>
                        </button>

                        {#if isTopOpen(top.key)}
                          <div class="logbook-tree-children">
                            {#each top.children as second}
                              <div class="logbook-tree-node">
                                <button class="logbook-tree-toggle logbook-tree-toggle-second" onclick={() => toggleSecond(top.key, second.key)}>
                                  <span class="logbook-tree-caret">{isSecondOpen(top.key, second.key) ? '▾' : '▸'}</span>
                                  <span class="logbook-tree-label">{second.key}</span>
                                  <span class="logbook-tree-count">{second.flights.length}</span>
                                </button>

                                {#if isSecondOpen(top.key, second.key)}
                                  <div class="logbook-tree-flights">
                                    {#each second.flights as f}
                                      <button
                                        class="logbook-item"
                                        class:selected={selectedFlightId === f.id}
                                        onclick={() => selectFlight(f.id)}
                                      >
                                        <div class="logbook-item-title">{flightListMarker(f.source)}{formatDateTime(f.start_time)}</div>
                                        <div class="logbook-item-meta">
                                          <span>{f.craft_name || $t('logbook.unnamedCraft')}</span>
                                          <span>{f.location_name || $t('logbook.unknownLocation')}</span>
                                          <span>{formatDurationSec(f.duration_sec)}</span>
                                        </div>
                                      </button>
                                    {/each}
                                  </div>
                                {/if}
                              </div>
                            {/each}
                          </div>
                        {/if}
                      </div>
                    {/each}
                  </div>

                  <div class="logbook-detail">
                    {#if selectedFlight}
                      <div class="fc-info-grid">
                        <span class="fc-label">{$t('logbook.craft')}</span>
                        <span class="fc-value">{selectedFlight.craft_name || $t('logbook.unnamedCraft')}</span>
                        <span class="fc-label">{$t('logbook.firmware')}</span>
                        <span class="fc-value">{selectedFlight.fc_version || `${selectedFlight.fc_variant || '—'}`}</span>
                        <span class="fc-label">{$t('logbook.source')}</span>
                        <span class="fc-value">{formatFlightSource(selectedFlight.source)}</span>
                        <span class="fc-label">{$t('logbook.started')}</span>
                        <span class="fc-value">{formatDateTime(selectedFlight.start_time)}</span>
                        <span class="fc-label">{$t('logbook.duration')}</span>
                        <span class="fc-value">{formatDurationSec(selectedFlight.duration_sec)}</span>
                        <span class="fc-label">{$t('logbook.location')}</span>
                        <span class="fc-value">{selectedFlight.location_name || $t('logbook.unknownLocation')}</span>
                        <span class="fc-label">{$t('logbook.maxAlt')}</span>
                        <span class="fc-value">{selectedFlight.max_alt_m?.toFixed(1) ?? '—'} m</span>
                        <span class="fc-label">{$t('logbook.maxSpeed')}</span>
                        <span class="fc-value">{selectedFlight.max_speed_ms?.toFixed(1) ?? '—'} m/s</span>
                        <span class="fc-label">{$t('logbook.totalDistance')}</span>
                        <span class="fc-value">{selectedFlight.total_distance_m?.toFixed(0) ?? '—'} m</span>
                        <span class="fc-label">{$t('logbook.maxDistance')}</span>
                        <span class="fc-value">{selectedFlight.max_distance_m?.toFixed(0) ?? '—'} m</span>
                        <span class="fc-label">{$t('logbook.batteryUsed')}</span>
                        <span class="fc-value">{selectedFlight.battery_used_mah ?? '—'} mAh</span>
                        <span class="fc-label">{$t('logbook.trackPoints')}</span>
                        <span class="fc-value">{selectedFlightTrackCount}</span>
                        <span class="fc-label">{$t('logbook.weather')}</span>
                        <span class="fc-value weather-value-row">
                          <span>
                            {#if selectedFlight.weather_temp_c != null || selectedFlight.weather_desc}
                              {selectedFlight.weather_temp_c != null ? selectedFlight.weather_temp_c.toFixed(1) + ' °C' : ''}
                              {selectedFlight.weather_wind_ms != null ? ', ' + selectedFlight.weather_wind_ms.toFixed(1) + ' m/s' : ''}
                              {selectedFlight.weather_wind_deg != null ? ' ' + windDegToLabel(selectedFlight.weather_wind_deg) : ''}
                              {selectedFlight.weather_desc ? ', ' + selectedFlight.weather_desc : ''}
                            {:else}
                              {$t('logbook.weatherUnavailable')}
                            {/if}
                          </span>
                          <button class="weather-edit-btn" onclick={() => { weatherEditing = !weatherEditing; }} title={$t('logbook.editWeather')}>✎</button>
                        </span>
                      </div>

                      {#if weatherEditing}
                        <div class="weather-editor">
                          <div class="weather-fields">
                            <label class="weather-field">
                              <span class="weather-field-label">{$t('logbook.weatherTemp')}</span>
                              <div class="setting-stepper">
                                <button class="stepper-btn" onclick={() => { weatherTempC = String(Math.round((Number(weatherTempC || 0) - 0.5) * 10) / 10); }}>−</button>
                                <input type="number" step="0.5" class="stepper-input" bind:value={weatherTempC} placeholder="—" />
                                <button class="stepper-btn" onclick={() => { weatherTempC = String(Math.round((Number(weatherTempC || 0) + 0.5) * 10) / 10); }}>+</button>
                                <span class="setting-unit">°C</span>
                              </div>
                            </label>
                            <label class="weather-field">
                              <span class="weather-field-label">{$t('logbook.weatherWind')}</span>
                              <div class="setting-stepper">
                                <button class="stepper-btn" onclick={() => { weatherWindMs = String(Math.max(0, Math.round((Number(weatherWindMs || 0) - 0.5) * 10) / 10)); }}>−</button>
                                <input type="number" step="0.5" min="0" class="stepper-input" bind:value={weatherWindMs} placeholder="—" />
                                <button class="stepper-btn" onclick={() => { weatherWindMs = String(Math.round((Number(weatherWindMs || 0) + 0.5) * 10) / 10); }}>+</button>
                                <span class="setting-unit">m/s</span>
                              </div>
                            </label>
                            <label class="weather-field">
                              <span class="weather-field-label">{$t('logbook.weatherWindDir')}</span>
                              <select class="setting-select weather-select" bind:value={weatherWindDir}>
                                <option value="">—</option>
                                <option value="0">N</option>
                                <option value="45">NE</option>
                                <option value="90">E</option>
                                <option value="135">SE</option>
                                <option value="180">S</option>
                                <option value="225">SW</option>
                                <option value="270">W</option>
                                <option value="315">NW</option>
                              </select>
                            </label>
                            <label class="weather-field">
                              <span class="weather-field-label">{$t('logbook.weatherConditions')}</span>
                              <select class="setting-select weather-select" bind:value={weatherDesc}>
                                <option value="">—</option>
                                <option value="Clear">{$t('logbook.weatherClear')}</option>
                                <option value="Partly Cloudy">{$t('logbook.weatherPartlyCloudy')}</option>
                                <option value="Overcast">{$t('logbook.weatherOvercast')}</option>
                                <option value="Light Rain">{$t('logbook.weatherLightRain')}</option>
                                <option value="Rain">{$t('logbook.weatherRain')}</option>
                                <option value="Snow">{$t('logbook.weatherSnow')}</option>
                                <option value="Fog">{$t('logbook.weatherFog')}</option>
                                <option value="Stormy">{$t('logbook.weatherStormy')}</option>
                              </select>
                            </label>
                          </div>
                          <button class="cache-clear-btn weather-save-btn" onclick={saveSelectedFlightWeather}>{$t('logbook.saveWeather')}</button>
                        </div>
                      {/if}

                      <div class="setting-row setting-row-stack">
                        <label class="setting-label" for="flight-notes">{$t('logbook.notes')}</label>
                        <textarea
                          id="flight-notes"
                          class="setting-input notes-input notes-input-auto"
                          rows="2"
                          bind:value={selectedFlightNotes}
                          oninput={(e: Event) => autoResizeNotes(e.target as HTMLTextAreaElement)}
                          use:notesAutoSize
                        ></textarea>
                      </div>
                      <div class="setting-row">
                        <button class="cache-clear-btn" onclick={saveSelectedFlightNotes}>{$t('logbook.saveNotes')}</button>
                        <button class="cache-clear-btn logbook-danger" onclick={removeSelectedFlight}>{$t('logbook.deleteFlight')}</button>
                      </div>
                    {:else}
                      <div class="panel-empty">
                        <span class="panel-empty-icon">✈</span>
                        <span>{$t('logbook.selectFlight')}</span>
                      </div>
                    {/if}
                  </div>
                </div>
              {/if}
            {/if}
          </section>

        <!-- Mission Tab -->
        {:else if activeTab === "mission"}
          <MissionPanel />
        {/if}
      </div>
    </div>
  {/if}

  <!-- ======= BOTTOM WIDGET PANEL ======= -->
  <div class="panel-bottom">
    <WidgetPanel
      widgetIds={panels.bottom}
      orientation="horizontal"
      availableVmin={bottomAvailVmin}
      {telem}
      editing={widgetEditMode}
      onreorder={handleReorder}
      onreceive={handleReceive}
      panelId="bottom"
    />
  </div>

  <!-- ======= RIGHT WIDGET PANEL ======= -->
  <div class="panel-right">
    <WidgetPanel
      widgetIds={panels.right}
      orientation="vertical"
      availableVmin={rightAvailVmin}
      {telem}
      editing={widgetEditMode}
      onreorder={handleReorder}
      onreceive={handleReceive}
      panelId="right"
    />
  </div>

  <!-- ======= BOTTOM-RIGHT RESERVED AREA (controls placeholder) ======= -->
  <div class="reserved-corner">
    <!-- reserved for future control buttons -->
  </div>

  <!-- ======= WIDGET EDIT TOGGLE ======= -->
  <button
    class="widget-edit-btn"
    class:active={widgetEditMode}
    onclick={() => widgetEditMode = !widgetEditMode}
    title={widgetEditMode ? $t('widgets.exitEdit') : $t('widgets.editLayout')}
  >
    ✎
  </button>

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
  <footer class="statusbar">
    <div class="statusbar-left">
      <span
        class="status-indicator"
        class:connected={connStatus === "connected"}
        class:disconnected={connStatus !== "connected"}
      ></span>
      <span>
        {#if connStatus === "connected" && fcInfo}
          {$t('connection.connectedOn', { values: { variant: fcInfo.fc_variant, version: fcInfo.fc_version, port: $connection.port } })}
        {:else if connStatus === "connecting"}
          {$t('connection.connecting')}
        {:else}
          {$t('connection.disconnected')}
        {/if}
      </span>
      {#if DEV_MODE}
        <button class="debug-btn" class:open={debugOpen} onclick={() => debugOpen = !debugOpen} title="MSP Debug Monitor">
          🔧 {$t('statusBar.debug')}
        </button>
      {/if}
    </div>
    <div class="statusbar-right">
      {#if connStatus === "connected" && telem.lastUpdate > 0}
        <span class="status-arming" class:armed={isArmed()} class:disarmed={!isArmed()}>
          {getArmingLabel()}
        </span>
        <span class="status-sep">|</span>
      {/if}
      <span>{$t('app.brand')}</span>
    </div>
  </footer>
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
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
  }

  /* --- Header / Toolbar --- */
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 16px;
    height: 50px;
    background: #2e2e2e;
    border-bottom: 3px solid #37a8db;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
    position: relative;
    z-index: 200;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .toolbar-center {
    display: flex;
    align-items: center;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .logo {
    font-size: 20px;
    font-weight: 700;
    color: #37a8db;
    letter-spacing: 0.5px;
  }

  .version {
    font-size: 11px;
    color: #949494;
  }

  /* --- Sensor Status Bar --- */
  .sensor-bar {
    display: flex;
    gap: 1px;
    background: #434343;
    border-radius: 5px;
    border: 1px solid #272727;
    box-shadow: 0 2px 0 rgba(92, 92, 92, 0.5);
    overflow: hidden;
  }

  .sensor {
    padding: 6px 12px;
    font-size: 10px;
    font-weight: 600;
    color: #4f4f4f;
    text-shadow: 0 1px rgba(0, 0, 0, 1.0);
    background: #434343 linear-gradient(to bottom, transparent, rgba(0, 0, 0, 0.45));
    border-right: 1px solid #373737;
    text-align: center;
    min-width: 36px;
  }

  .sensor:last-child {
    border-right: none;
  }

  /* --- Port Controls --- */
  .port-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .port-select,
  .baud-select {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 12px;
  }

  .port-select {
    min-width: 160px;
  }

  .baud-select {
    min-width: 80px;
  }

  .refresh-btn {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 14px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .refresh-btn:hover {
    background: #555;
  }

  /* --- Connect Button --- */
  .connect-btn {
    padding: 6px 16px;
    background: #37a8db;
    border: 1px solid #339cc1;
    border-radius: 3px;
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    text-shadow: 0 1px rgba(0, 0, 0, 0.25);
    cursor: pointer;
    transition: background-color 0.2s ease;
    min-width: 90px;
  }

  .connect-btn:hover:not(:disabled) {
    background: #45bce5;
  }

  .connect-btn:disabled {
    opacity: 0.7;
    cursor: wait;
  }

  .connect-btn.connected {
    background: #e60000;
    border-color: #fe0000;
  }

  .connect-btn.connected:hover {
    background: #f21212;
  }

  .connect-btn.connecting {
    background: #f5a623;
    border-color: #e09a1e;
  }

  /* --- Full-screen Map --- */
  .map-fullscreen {
    position: absolute;
    top: 53px; /* toolbar height + border */
    left: 0;
    right: 0;
    bottom: 24px; /* statusbar height */
    z-index: 0;
  }

  /* --- Floating Navigation Rail (hamburger + tab buttons) --- */
  .nav-rail {
    position: absolute;
    top: 65px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 0;
    z-index: 100;
    transition: left 0.3s ease;
  }

  .nav-rail.open {
    left: 12px;
  }

  /* --- Hamburger Button --- */
  .hamburger-btn {
    width: 42px;
    height: 42px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.2s;
    backdrop-filter: blur(8px);
  }

  .hamburger-btn:hover {
    background: rgba(55, 168, 219, 0.25);
  }

  .hamburger-icon {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 20px;
    transition: transform 0.3s ease;
  }

  .hamburger-icon span {
    display: block;
    height: 2px;
    background: #37a8db;
    border-radius: 1px;
    transition: transform 0.3s ease, opacity 0.2s ease;
  }

  .hamburger-icon.open span:nth-child(1) {
    transform: translateY(6px) rotate(45deg);
  }

  .hamburger-icon.open span:nth-child(2) {
    opacity: 0;
  }

  .hamburger-icon.open span:nth-child(3) {
    transform: translateY(-6px) rotate(-45deg);
  }

  /* --- Tab Buttons --- */
  .tab-buttons {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
  }

  .tab-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 42px;
    height: 38px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #949494;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
    justify-content: center;
    overflow: hidden;
    transition: width 0.3s ease, background-color 0.2s;
    backdrop-filter: blur(8px);
    white-space: nowrap;
  }

  .tab-btn:hover {
    background: rgba(55, 168, 219, 0.15);
    color: #e0e0e0;
  }

  .tab-btn.active {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
    color: #37a8db;
  }

  .tab-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .tab-label {
    display: none;
  }

  /* --- Floating Nav Panel --- */
  .nav-panel {
    position: absolute;
    top: 65px;
    left: 62px; /* after the rail buttons */
    width: min(360px, calc(100vw - 86px));
    max-height: calc(100vh - 53px - 24px - 80px); /* toolbar - statusbar - margins */
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

  .nav-panel.nav-panel-wide {
    width: min(920px, calc(100vw - 86px));
  }

  .nav-panel.nav-panel-minimized {
    width: min(280px, calc(100vw - 86px));
    cursor: pointer;
  }

  .logbook-detail-minimized {
    border: none;
    background: none;
    padding: 0;
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

  .panel-section {
    margin-bottom: 16px;
  }

  .section-heading {
    margin: 0 0 8px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 32px 0;
    color: #555;
    font-size: 12px;
  }

  .panel-empty-icon {
    font-size: 28px;
    opacity: 0.4;
  }

  .fc-info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    font-size: 12px;
  }

  .fc-label {
    color: #949494;
  }

  .fc-value {
    color: #e0e0e0;
    font-weight: 600;
  }

  .feature-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .feature-badge {
    padding: 3px 8px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
  }

  .feature-badge.available {
    background: rgba(89, 170, 41, 0.2);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.4);
  }

  .feature-badge.unavailable {
    background: rgba(80, 80, 80, 0.2);
    color: #555;
    border: 1px solid #444;
    text-decoration: line-through;
  }

  /* --- Bottom Widget Panel --- */
  .panel-bottom {
    position: absolute;
    bottom: 30px;
    left: 2vmin;
    right: calc(22.5vmin + 3vmin); /* LARGE_BASE_VMIN + margin */
    z-index: 100;
    display: flex;
    justify-content: center;
    pointer-events: none;
  }

  .panel-bottom > :global(*) {
    pointer-events: auto;
  }

  /* --- Right Widget Panel --- */
  .panel-right {
    position: absolute;
    top: 60px;
    right: 0.5vmin;
    bottom: calc(22.5vmin + 36px); /* LARGE_BASE_VMIN + statusbar + margin */
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    pointer-events: none;
  }

  .panel-right > :global(*) {
    pointer-events: auto;
  }

  /* --- Bottom-right reserved corner --- */
  .reserved-corner {
    position: absolute;
    bottom: 30px;
    right: 0.5vmin;
    width: 22.5vmin;
    height: 22.5vmin;
    z-index: 90;
    pointer-events: none;
    /* Visible only in debug — uncomment to see reserved area */
    /* outline: 1px dashed rgba(255,0,0,0.3); */
  }

  /* --- Widget edit toggle button --- */
  .widget-edit-btn {
    position: absolute;
    bottom: 32px;
    right: calc(0.5vmin + 1vmin);
    width: 3.5vmin;
    height: 3.5vmin;
    min-width: 28px;
    min-height: 28px;
    background: rgba(46, 46, 46, 0.85);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #949494;
    font-size: 1.5vmin;
    cursor: pointer;
    z-index: 110;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(8px);
    transition: background-color 0.2s, border-color 0.2s, color 0.2s;
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

  /* --- Widget toggle group in settings --- */
  .widget-toggle-group {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .widget-panel-indicator {
    font-size: 9px;
    color: #888;
    min-width: 38px;
    text-align: right;
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

  /* --- Status Bar --- */
  .statusbar {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 10px;
    height: 24px;
    background: #2e2e2e;
    border-top: 1px solid #272727;
    font-size: 11px;
    color: #949494;
    z-index: 200;
  }

  .statusbar-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .status-indicator.disconnected {
    background: #d40000;
    box-shadow: 0 0 4px rgba(212, 0, 0, 0.5);
  }

  .status-indicator.connected {
    background: #59aa29;
    box-shadow: 0 0 4px rgba(89, 170, 41, 0.5);
  }

  /* --- Sensor bar active state --- */
  .sensor.active {
    color: #59aa29;
    text-shadow: 0 0 4px rgba(89, 170, 41, 0.3);
  }

  .sensor.warning {
    color: #f5a623;
    text-shadow: 0 0 4px rgba(245, 166, 35, 0.3);
  }

  .sensor.error {
    color: #d40000;
    text-shadow: 0 0 4px rgba(212, 0, 0, 0.3);
  }

  /* --- Arming status in status bar --- */
  .statusbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-arming {
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.5px;
  }

  .status-arming.armed {
    color: #ff4444;
  }

  .status-arming.disarmed {
    color: #59aa29;
  }

  .status-sep {
    color: #555;
  }

  /* --- Settings panel controls --- */
  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
  }

  .setting-label {
    font-size: 12px;
    color: #e0e0e0;
  }

  .setting-select {
    padding: 3px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
    min-width: 70px;
  }

  .setting-input {
    padding: 3px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
  }

  .setting-row-stack {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }

  .path-picker-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .path-input {
    flex: 1;
    min-width: 0;
  }

  .notes-input {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    min-height: 44px;
  }

  .notes-input-auto {
    overflow: hidden;
    max-height: 140px;
  }

  .notes-input-auto[readonly] {
    resize: none;
    cursor: pointer;
    opacity: 0.85;
  }

  .weather-value-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .weather-edit-btn {
    background: none;
    border: none;
    color: #777;
    cursor: pointer;
    font-size: 13px;
    padding: 0 2px;
    line-height: 1;
    flex-shrink: 0;
  }

  .weather-edit-btn:hover {
    color: #37a8db;
  }

  .weather-editor {
    margin-top: 8px;
    padding: 10px;
    border: 1px solid rgba(55, 168, 219, 0.25);
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.03);
  }

  .weather-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .weather-field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .weather-field-label {
    font-size: 10px;
    color: #949494;
  }

  .weather-select {
    width: 100%;
    box-sizing: border-box;
  }

  .weather-save-btn {
    margin-top: 8px;
    width: 100%;
  }

  .setting-stepper {
    display: flex;
    align-items: stretch;
    gap: 4px;
  }
  .stepper-btn {
    background: #333;
    color: #aaa;
    border: 1px solid #555;
    border-radius: 3px;
    width: 24px;
    cursor: pointer;
    font-size: 14px;
    font-weight: bold;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    user-select: none;
  }
  .stepper-btn:hover {
    background: #37a8db;
    color: #fff;
  }
  .stepper-btn:active {
    background: #2d8ab8;
  }
  .stepper-input {
    padding: 3px 4px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
    width: 52px;
    text-align: center;
    color-scheme: dark;
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .stepper-input::-webkit-inner-spin-button,
  .stepper-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  .setting-unit {
    font-size: 11px;
    color: #888;
    margin-left: 2px;
    align-self: center;
  }

  .setting-hint {
    font-size: 10px;
    color: #666;
    margin: 4px 0 0 0;
    font-style: italic;
  }

  /* --- Tile cache bar --- */
  .cache-bar-container {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0 2px 0;
  }

  .cache-bar-track {
    flex: 1;
    height: 6px;
    background: #333;
    border-radius: 3px;
    overflow: hidden;
  }

  .cache-bar-fill {
    height: 100%;
    background: #37a8db;
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .cache-bar-fill.cache-bar-warning {
    background: #e8a317;
  }

  .cache-bar-label {
    font-size: 9px;
    color: #888;
    white-space: nowrap;
  }

  .cache-clear-btn {
    font-size: 9px;
    padding: 1px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #ccc;
    cursor: pointer;
    transition: background 0.15s;
  }

  .cache-clear-btn:hover {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }

  .cache-clear-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .cache-clear-btn:disabled:hover {
    background: #434343;
    border-color: #555;
    color: #ccc;
  }

  .logbook-layout {
    display: grid;
    grid-template-columns: minmax(240px, 0.85fr) minmax(0, 1.35fr);
    gap: 12px;
    min-height: 420px;
  }

  .logbook-list {
    max-height: 560px;
    overflow: auto;
    border: 1px solid #555;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.12);
    padding: 6px;
  }

  .logbook-tree-node {
    margin-bottom: 4px;
  }

  .logbook-tree-toggle {
    width: 100%;
    text-align: left;
    border: 1px solid #555;
    border-radius: 4px;
    background: #353535;
    color: #ddd;
    cursor: pointer;
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto;
    align-items: center;
    gap: 6px;
    padding: 5px 7px;
  }

  .logbook-tree-toggle:hover {
    border-color: #37a8db;
  }

  .logbook-tree-toggle-top {
    font-size: 12px;
    font-weight: 600;
  }

  .logbook-tree-toggle-second {
    margin-top: 4px;
    margin-left: 12px;
    width: calc(100% - 12px);
    font-size: 11px;
    background: #303030;
  }

  .logbook-tree-children {
    margin-top: 3px;
  }

  .logbook-tree-flights {
    margin-top: 4px;
    margin-left: 24px;
    width: calc(100% - 24px);
  }

  .logbook-tree-caret {
    color: #9cc6d9;
    font-size: 11px;
    line-height: 1;
  }

  .logbook-tree-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .logbook-tree-count {
    font-size: 10px;
    color: #8fb4c5;
    background: rgba(55, 168, 219, 0.12);
    border: 1px solid rgba(55, 168, 219, 0.32);
    border-radius: 999px;
    padding: 1px 6px;
  }

  .logbook-item {
    width: 100%;
    text-align: left;
    border: 1px solid #555;
    border-radius: 4px;
    background: #383838;
    color: #ddd;
    margin-bottom: 4px;
    padding: 6px;
    cursor: pointer;
  }

  .logbook-item:hover {
    border-color: #37a8db;
  }

  .logbook-item.selected {
    border-color: #37a8db;
    background: rgba(55, 168, 219, 0.18);
  }

  .logbook-item-title {
    font-size: 12px;
    color: #fff;
    font-weight: 600;
  }

  .logbook-item-meta {
    margin-top: 2px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: 10px;
    color: #aaa;
  }

  .logbook-detail {
    border: 1px solid #555;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.12);
    padding: 10px;
    overflow: auto;
    max-height: 560px;
  }

  .logbook-progress {
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 6px;
    background: rgba(55, 168, 219, 0.08);
    padding: 8px;
    margin-bottom: 10px;
  }

  .logbook-progress-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }

  .logbook-progress-bar {
    margin-top: 6px;
    height: 8px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 999px;
    overflow: hidden;
  }

  .logbook-progress-fill {
    height: 100%;
    background: linear-gradient(90deg, #2d8ab8, #37a8db);
    transition: width 0.2s ease;
  }

  .logbook-progress-message {
    margin-top: 6px;
    font-size: 11px;
    color: #b8c7cf;
  }

  .logbook-danger {
    background: #7a2020;
    border-color: #8b2525;
    color: #e8c0c0;
  }

  .logbook-danger:hover {
    background: #9b1f1f;
    border-color: #9b1f1f;
    color: #fff;
  }

  @media (max-width: 980px) {
    .logbook-layout {
      grid-template-columns: 1fr;
    }
  }

  /* --- Toggle switch --- */
  .toggle-switch {
    position: relative;
    display: inline-block;
    width: 36px;
    height: 20px;
  }

  .toggle-switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-slider {
    position: absolute;
    cursor: pointer;
    top: 0; left: 0; right: 0; bottom: 0;
    background-color: #434343;
    border: 1px solid #555;
    border-radius: 20px;
    transition: background-color 0.2s;
  }

  .toggle-slider::before {
    content: "";
    position: absolute;
    height: 14px;
    width: 14px;
    left: 2px;
    bottom: 2px;
    background-color: #949494;
    border-radius: 50%;
    transition: transform 0.2s, background-color 0.2s;
  }

  .toggle-switch input:checked + .toggle-slider {
    background-color: rgba(55, 168, 219, 0.3);
    border-color: #37a8db;
  }

  .toggle-switch input:checked + .toggle-slider::before {
    transform: translateX(16px);
    background-color: #37a8db;
  }

  /* --- Dev Debug Button (status bar) --- */
  .debug-btn {
    background: none;
    border: 1px solid transparent;
    color: #666;
    font-size: 11px;
    cursor: pointer;
    padding: 0 6px;
    border-radius: 3px;
    margin-left: 8px;
    transition: color 0.2s, border-color 0.2s, background-color 0.2s;
  }

  .debug-btn:hover {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.3);
  }

  .debug-btn.open {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.4);
    background: rgba(245, 166, 35, 0.1);
  }
</style>
