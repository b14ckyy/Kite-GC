<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { connection, availablePorts } from "$lib/stores/connection";
  import type { FcInfo, PortInfo } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import { telemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";
  import { t, locale } from 'svelte-i18n';
  import Map from "$lib/components/Map.svelte";
  import LogPlayer from "$lib/components/LogPlayer.svelte";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import LogbookPanel from "$lib/components/LogbookPanel.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import UavInfoPanel from "$lib/components/UavInfoPanel.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import NavRail from "$lib/components/NavRail.svelte";
  import { PlaybackController } from '$lib/controllers/playbackController';
  import { refreshSerialPorts, connectFC, disconnectFC } from '$lib/controllers/connectionController';
  import * as logbookCtrl from '$lib/controllers/logbookController';
  import * as widgetCtrl from '$lib/controllers/widgetController';
  import { isValidGpsCoordinate } from '$lib/helpers/telemetry';
  import { toTelemetryData } from '$lib/adapters/telemetryAdapter';
  import { homePosition } from '$lib/stores/home';
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import MissionPanel from "$lib/components/MissionPanel.svelte";
  import { editMode } from "$lib/stores/mission";
  import type { PanelConfig } from "$lib/stores/settings";
  import {
    getDefaultFlightlogPath,
    type BlackboxImportProgress,
    type Flight,
    type FlightSummary,
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
  let liveTelem = $state(get(telemetry));
  telemetry.subscribe((t) => { liveTelem = t; });

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
  let playbackActive = $state(false);
  let playbackPlaying = $state(false);
  let playbackIndex = $state(0);
  let playbackSpeed = $state(1);
  const playbackCtrl = new PlaybackController();
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
    playbackActive = true;
    playbackIndex = PlaybackController.seek(selectedTrackWithPosition, playbackIndex, deltaMs);
  }

  function seekToStart() {
    if (selectedTrackWithPosition.length === 0) return;
    playbackActive = true;
    playbackIndex = 0;
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
    const result = await logbookCtrl.importBlackbox(filePath, flightLogDbPath, logIndex, forceImport, $locale ?? 'en');
    
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
    const data = await logbookCtrl.selectFlightData(flightId, flightLogDbPath, $locale ?? 'en');
    selectedFlight = data.flight;
    selectedFlightTrack = data.track;
    selectedFlightTrackCount = data.trackCount;
    selectedFlightNotes = data.notes;
    weatherTempC = data.weatherTempC;
    weatherWindMs = data.weatherWindMs;
    weatherWindDir = data.weatherWindDir;
    weatherDesc = data.weatherDesc;
    weatherEditing = false;
    resetPlayback();
    if (data.hasGpsData) playbackActive = true;

    // Set home position for replay (used by HomeWidget)
    if (data.flight?.start_lat != null && data.flight?.start_lon != null) {
      homePosition.set({ lat: data.flight.start_lat, lon: data.flight.start_lon, alt: 0, set: true });
    }

    await loadLogbook();
  }

  async function saveSelectedFlightNotes() {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.saveNotes(selectedFlightId, selectedFlightNotes, flightLogDbPath);
    await loadLogbook();
  }

  async function saveSelectedFlightWeather() {
    if (!selectedFlightId) return;
    selectedFlight = await logbookCtrl.saveWeather(
      selectedFlightId, weatherTempC, weatherWindMs, weatherWindDir, weatherDesc, flightLogDbPath,
    );
    weatherEditing = false;
  }

  async function removeSelectedFlight() {
    if (!selectedFlightId) return;
    const ok = await logbookCtrl.removeFlight(selectedFlightId, flightLogDbPath);
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

  async function loadInfo() {
    appVersion = await invoke("get_app_version");
    selectedPort = await refreshSerialPorts(selectedPort);
  }

  async function refreshPorts() {
    selectedPort = await refreshSerialPorts(selectedPort);
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

    if (!selectedPort) {
      errorMsg = $t('connection.noPortSelected');
      return;
    }

    isConnecting = true;
    errorMsg = "";
    connection.update((c) => ({ ...c, status: "connecting" }));
    settings.patch({ lastPort: selectedPort, lastBaud: selectedBaud, flightLoggingEnabled, flightLogDbPath, flightLogRawEnabled });

    try {
      await connectFC({
        port: selectedPort,
        baudRate: selectedBaud,
        attitudeRateHz,
        positionRateHz,
        airspeedEnabled,
        flightLogEnabled: flightLoggingEnabled,
        flightLogPath: flightLogDbPath,
        flightLogRaw: flightLogRawEnabled,
      });
    } catch (e: any) {
      errorMsg = e.toString();
      connection.set({ status: "error", port: "", baudRate: selectedBaud, errorMessage: e.toString(), fcInfo: null });
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
  const selectedTrackWithPosition = $derived(
    selectedFlightTrack.filter((point) => isValidGpsCoordinate(point.lat, point.lon))
  );
  const mapTrack = $derived(isPrimaryConnected ? [] : selectedTrackWithPosition);
  const playbackPoint = $derived(
    playbackActive && !isPrimaryConnected && selectedTrackWithPosition.length > 0
      ? selectedTrackWithPosition[Math.min(playbackIndex, selectedTrackWithPosition.length - 1)]
      : null,
  );
  const showPlayer = $derived(playbackActive && !isPrimaryConnected && selectedFlight != null);
  const playbackCurrentMs = $derived(
    selectedTrackWithPosition.length > 0 ? selectedTrackWithPosition[Math.min(playbackIndex, selectedTrackWithPosition.length - 1)].timestamp_ms : 0,
  );
  const playbackTotalMs = $derived(
    selectedTrackWithPosition.length > 0 ? selectedTrackWithPosition[selectedTrackWithPosition.length - 1].timestamp_ms : 0,
  );
  const logbookDetailOpen = $derived(activeTab === 'logbook' && selectedFlight != null && !logbookMinimized);
  const logbookHasFlightOnMap = $derived(activeTab === 'logbook' && selectedFlight != null && !isPrimaryConnected);

  // Unified telemetry: live data when connected, playback data when replaying
  const telem = $derived(
    playbackActive && !isPrimaryConnected && playbackPoint
      ? toTelemetryData(playbackPoint)
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

<main class="app">
  <!-- ======= TOOLBAR ======= -->
  <Toolbar
    {appVersion}
    {telem}
    {ports}
    {connStatus}
    {isConnecting}
    bind:selectedPort
    bind:selectedBaud
    {baudRates}
    onRefreshPorts={refreshPorts}
    onConnect={handleConnect}
  />

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="map-fullscreen" onclick={minimizeLogbook}>
    <Map playbackTrack={mapTrack} playbackPoint={playbackPoint} />
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
  />

  <!-- ======= FLOATING NAV PANEL SYSTEM ======= -->
  <NavRail
    open={navPanelOpen}
    {activeTab}
    {tabs}
    onToggle={toggleNavPanel}
    onSelectTab={selectTab}
  />

  <!-- Floating panel content -->
  {#if navPanelOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="nav-panel" class:nav-panel-wide={logbookDetailOpen} class:nav-panel-minimized={logbookMinimized && logbookHasFlightOnMap} onclick={() => { if (logbookMinimized) expandLogbook(); }}>
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
            {attitudeRateHz}
            {positionRateHz}
            {airspeedEnabled}
            {flightLoggingEnabled}
            {flightLogRawEnabled}
            {flightLogDbPath}
            {defaultFlightLogPath}
            {defaultWpAltitudeM}
            {defaultPhTimeSec}
            {isWidgetActive}
            {getWidgetPanelLabel}
            onPatch={(patch) => settings.patch(patch)}
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
            onDeleteFlight={removeSelectedFlight}
          />

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
  <StatusBar
    {connStatus}
    {fcInfo}
    {telem}
    connectionPort={$connection.port}
    devMode={DEV_MODE}
    bind:debugOpen
  />
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

  /* --- Full-screen Map --- */
  .map-fullscreen {
    position: absolute;
    top: 53px; /* toolbar height + border */
    left: 0;
    right: 0;
    bottom: 24px; /* statusbar height */
    z-index: 0;
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
