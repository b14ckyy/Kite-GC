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
  import Toolbar from "$lib/components/Toolbar.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import NavRail from "$lib/components/NavRail.svelte";
  import PanelPlayground from "$lib/components/panel/PanelPlayground.svelte";
  import UavInfoPanel from "$lib/components/UavInfoPanel.svelte";
  import LogbookPanel from "$lib/components/logbook/LogbookPanel.svelte";
  import MissionPanel from "$lib/components/mission/MissionPanel.svelte";
  import VideoPanel from "$lib/components/video/VideoPanel.svelte";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import { PlaybackController } from '$lib/controllers/playbackController';
  import { refreshSerialPorts, connectFC, disconnectFC, scanBleDevices } from '$lib/controllers/connectionController';
  import * as logbookCtrl from '$lib/controllers/logbookController';
  import * as widgetCtrl from '$lib/controllers/widgetController';
  import { isValidGpsCoordinate, isArmed } from '$lib/helpers/telemetry';
  import { liveTrack, appendLivePoint, clearLiveTrack } from '$lib/stores/liveTrack';
  import { toTelemetryData } from '$lib/adapters/telemetryAdapter';
  import { activeWpNumber, replayWpTotal } from '$lib/stores/navStatus';
  import { missionManagerOpen, missionManagerSelectedId, requestOpenFlightId, requestOpenMissionId } from '$lib/stores/missionManager';
  import { batteryManagerOpen } from '$lib/stores/batteryManager';
  import { missionDbForFlight, flightLoggedWpCount, missionDbSave, flightLinkMission, missionDbGeocode, flightSetBatterySerial, updateFlightNotes, getFlight, batteryDbFindBySerial, batteryDbAddUsage } from '$lib/stores/flightlog';
  import EndFlightDialog from "$lib/components/logbook/EndFlightDialog.svelte";
  import type { EndFlightStats } from "$lib/components/logbook/EndFlightDialog.svelte";
  import { haversineDistance } from '$lib/utils/geo';
  import { buildMissionInput, missionContentHash } from '$lib/helpers/missionLibrary';
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
  import { editMode, replayActive, mission, missionFlags, missionDownload, missionUpload, missionFcInfo, markMissionSynced, loadedMissionId, missionSetWaypoints } from "$lib/stores/mission";
  import { terrainAnalysis, patchTerrainAnalysis } from "$lib/stores/terrainAnalysis";
  import type { AppSettings, InterfaceSettings, PanelConfig } from "$lib/stores/settings";
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

  // Global UI scale (1 = 100%, up to 2). Zooms the chrome via `.ui-scale`; the map
  // (`.layer-map`) stays unzoomed/native. See docs/active/UI_SCALING.md.
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
  // Live flight-stats accumulator (armed period) — drives the End-Flight summary when there is
  // no DB recording (the recorded case reads the finalized stats from the flight row instead).
  let armStartMs = 0;
  let accMaxAlt = 0, accMaxSpeed = 0, accMaxDist = 0, accMah = 0;
  let accStartLat: number | null = null, accStartLon: number | null = null;
  telemetry.subscribe((t) => {
    liveTelem = t;
    // Accumulate the live flown track (RAM) for the Terrain Analyzer
    const armed = isArmed(t.armingFlags, t.lastUpdate);
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
    if (armed && isValidGpsCoordinate(t.lat, t.lon)) {
      appendLivePoint(t.lat, t.lon, t.altMsl, t.lastUpdate || Date.now());
    }
    if (!armed && prevArmed) {
      void handleDisarm(t.lastUpdate || Date.now());
    }
    prevArmed = armed;
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
  let endFlightDialog: ReturnType<typeof EndFlightDialog>;

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

  const allTabs = [
    { id: "uav-info", label: () => $t('nav.uavInfo'), icon: ICON_UAV_INFO },
    { id: "mission", label: () => $t('nav.mission'), icon: ICON_MISSION },
    { id: "terrain", label: () => $t('nav.terrain'), icon: ICON_TERRAIN },
    { id: "logbook", label: () => $t('nav.logbook'), icon: ICON_LOGBOOK },
    { id: "video", label: () => $t('nav.video'), icon: ICON_VIDEO },
    { id: "settings", label: () => $t('nav.settings'), icon: ICON_SETTINGS },
  ];
  const tabs = $derived(
    flightLoggingEnabled ? allTabs : allTabs.filter(t => t.id !== 'logbook')
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
  // Drop any legacy "-v2" suffix from a persisted tab (the migration scaffolding is gone now).
  activeTab = (saved.activeTab ?? 'uav-info').replace(/-v2$/, '');
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
  uiScale = saved.uiScale ?? 1;
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

  // Persist a settings patch + mirror it into the local reactive vars the page binds. Shared by
  // the legacy SettingsPanel and the new SettingsPanel.
  function applySettingsPatch(patch: Partial<AppSettings>) {
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
  // mission uploaded), offer to update the link. See docs/active/MISSION_LIBRARY_AND_DB.md.
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
      const fc = get(missionFlags).fc;
      // FC-synced mission is trusted (it's what the FC flew) → link it silently. Covers a
      // mid-flight re-upload (relink to the new one); unchanged since arm is already linked.
      if (wps.length > 0 && fc) {
        const curHash = await missionContentHash(wps);
        if (curHash !== flightMissionHash) {
          try { await linkMissionToFlight(flightId, wps); } catch (e) { console.warn('[end-flight] fc relink failed', e); }
        }
      }
      // A non-FC-synced mission is uncertain (may not be what was flown) → offer to link it
      // with explicit confirmation. linkMissionToFlight upserts (saves if not in the DB) + links.
      const missionConfirm = wps.length > 0 && !fc;

      // Summary from the finalized flight row.
      const flight = await getFlight(flightId, flightLogDbPath);
      const stats: EndFlightStats = {
        durationSec: flight?.duration_sec ?? null,
        maxAltM: flight?.max_alt_m ?? null,
        maxSpeedMs: flight?.max_speed_ms ?? null,
        maxDistM: flight?.max_distance_m ?? null,
        batteryUsedMah: flight?.battery_used_mah ?? null,
        locationName: flight?.location_name ?? null,
      };
      const res = await endFlightDialog.show({ stats, recorded: true, missionConfirm });
      if (res) {
        if (res.batterySerial) await flightSetBatterySerial(flightId, res.batterySerial, flightLogDbPath);
        if (res.notes) await updateFlightNotes(flightId, res.notes, flightLogDbPath);
        if (res.linkMission) await linkMissionToFlight(flightId, wps);
      }
    } catch (e) {
      console.warn('[end-flight] disarm dialog failed', e);
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
    void listen<{ flight_id: number }>('flight-recording-ended', async (event) => {
      await onRecordingEnded(event.payload.flight_id); // End-Flight dialog (summary + battery/notes/mission)
      void loadLogbook(); // refresh the list with the just-recorded (and linked) flight
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

<div class="ui-root" style:--ui-scale={uiScale}>
  <!-- ======= MAP LAYER — unzoomed / native resolution (see docs/active/UI_SCALING.md) =======
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

  <!-- ======= UI CHROME LAYER — zoomed by --ui-scale ======= -->
  <div class="ui-scale">

<ConfirmDialog bind:this={confirmDialog} />
<EndFlightDialog bind:this={endFlightDialog} {interfaceSettings} />

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

  <!-- Map lives in the unzoomed `.layer-map` above (see docs/active/UI_SCALING.md). -->

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
        onPatch={applySettingsPatch}
        onSetCacheMaxMB={setCacheMaxMB}
        onClearCache={clearCache}
        onChooseFlightLogPath={chooseFlightLogPath}
        onResetFlightLogPath={resetFlightLogPath}
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
        onImportBlackbox={importBlackbox}
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
        onImportKflight={importKflightFile}
      />
    {:else if activeTab === 'mission'}
      <MissionPanel />
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
       correct place; see docs/active/UI_SCALING.md). -->
  <ContextMenu />
  <BatchEditPopup {interfaceSettings} />
</div><!-- .ui-root -->

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

  /* Leaflet map tooltips (hover hints / "toasts") live in the unzoomed map. Scale them
     with the global UI scale via font-size + padding (Leaflet sets an inline transform
     for positioning, so a CSS transform would be overridden — em/px scaling reflows the
     box instead). --ui-scale inherits from `.ui-root`. Base values match Leaflet defaults. */
  :global(.leaflet-tooltip) {
    font-size: calc(12px * var(--ui-scale, 1));
    padding: calc(6px * var(--ui-scale, 1)) calc(8px * var(--ui-scale, 1));
  }

  /* ── Global UI scaling (see docs/active/UI_SCALING.md) ──────────
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
     the map is draggable under/around them. See docs/active/UI_SCALING.md. */
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
  .layer-map.in-frame {
    top: auto;
    right: auto;
    bottom: auto; /* left/top/width/height come from the inline rect */
    z-index: 2; /* above .ui-scale (z:1): into the floating frame body; frame draws the border */
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
