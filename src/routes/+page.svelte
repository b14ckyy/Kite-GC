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
  import { isAndroid, isMobile, isTablet, isPhone as isPhoneDevice, hasSerialPorts, logPlayerWidth } from "$lib/platform";
  import { isDebugMode } from "$lib/stores/debug";
  import { telemetry } from "$lib/stores/telemetry";
  import { startRadarListeners, configureRadar, setRadarCenter, setRadarNode } from "$lib/stores/radarTracking";
  import { startRadarAlerts } from "$lib/controllers/radarAlerts";
  import { startBreachMonitor } from "$lib/controllers/breachMonitor";
  import { startAlertAudio } from "$lib/controllers/alertAudio";
  import { get } from "svelte/store";
  import { t, locale } from 'svelte-i18n';
  import Map from "$lib/components/Map.svelte";
  import Map3D from "$lib/components/Map3D.svelte";
  import CesiumKeyPrompt from "$lib/components/CesiumKeyPrompt.svelte";
  import LogPlayer from "$lib/components/logbook/LogPlayer.svelte";
  import HiresParseModal from "$lib/components/logbook/HiresParseModal.svelte";
  import RawTelemetryModal from "$lib/components/RawTelemetryModal.svelte";
  import PhoneBottomChips from "$lib/components/phone/PhoneBottomChips.svelte";
  import PhoneDebugButton from "$lib/components/phone/PhoneDebugButton.svelte";
  import ConnectionPopout from "$lib/components/phone/ConnectionPopout.svelte";
  import PhoneWidgetPanel from "$lib/components/phone/PhoneWidgetPanel.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import UpdateDialog from "$lib/components/UpdateDialog.svelte";
  import { runUpdateCheck } from "$lib/controllers/updateCheck";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import BatchEditPopup from "$lib/components/mission/BatchEditPopup.svelte";
  import ArduBatchEditPopup from "$lib/components/mission/ArduBatchEditPopup.svelte";
  import type { DialogButton, DialogOptions } from "$lib/components/ConfirmDialog.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import RelayPanel from "$lib/components/RelayPanel.svelte";
  import WindowResizeBorders from "$lib/components/WindowResizeBorders.svelte";
  import StatusBar from "$lib/components/StatusBar.svelte";
  import "$lib/stores/rcSession"; // global RC session (keeps engage alive across navigation; involuntary-loss guard)
  import NavRail from "$lib/components/NavRail.svelte";
  import PanelPlayground from "$lib/components/panel/PanelPlayground.svelte";
  import UavInfoPanel from "$lib/components/UavInfoPanel.svelte";
  import LogbookPanel from "$lib/components/logbook/LogbookPanel.svelte";
  import MissionPanel from "$lib/components/mission/MissionPanel.svelte";
  import MavCommandPanel from "$lib/components/control/MavCommandPanel.svelte";
  import RcControlPanel from "$lib/components/control/RcControlPanel.svelte";
  import VirtualSticks from "$lib/components/control/VirtualSticks.svelte";
  import VideoPanel from "$lib/components/video/VideoPanel.svelte";
  import RadarPanel from "$lib/components/RadarPanel.svelte";
  import AirspaceManagerPanel from "$lib/components/AirspaceManagerPanel.svelte";
  import { geozoneWorking } from "$lib/stores/geozone";
  import { fenceWorking } from "$lib/stores/fence";
  import { rallyWorking } from "$lib/stores/rally";
  import RadarAlertBanner from "$lib/components/RadarAlertBanner.svelte";
  import StatusTextToasts from "$lib/components/StatusTextToasts.svelte";
  import { startStatusText } from "$lib/stores/statusText";
  import { panelDockRight } from "$lib/stores/panelDock";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import { ensureUserLocation, requestUserLocation, userGeoLocation } from "$lib/helpers/userLocation";
  import { gcsLocation, gcsAccuracyM } from "$lib/stores/gcsLocation";
  import { fetchAero } from "$lib/stores/airspace";
  import { PlaybackController } from '$lib/controllers/playbackController';
  import { refreshSerialPorts, connectFC, disconnectFC, startBleScan, stopBleScan, startBleDeviceListener, stopBleDeviceListener, clearBleDevices } from '$lib/controllers/connectionController';
  import * as logbookCtrl from '$lib/controllers/logbookController';
  import * as widgetCtrl from '$lib/controllers/widgetController';
  import * as phoneCtrl from '$lib/controllers/phoneWidgetController';
  import { PHONE_GRID_PAD } from '$lib/config/phoneGrid';
  import type { PhoneWidgetsConfig } from '$lib/controllers/phoneWidgetController';
  import { isValidGpsCoordinate, isArmed } from '$lib/helpers/telemetry';
  import { anyCase } from '$lib/helpers/fileFilters';
  import { liveTrack, appendLivePoint, clearLiveTrack, backfillLivePoints } from '$lib/stores/liveTrack';
  import { toTelemetryData } from '$lib/adapters/telemetryAdapter';
  import { activeWpNumber, replayWpTotal } from '$lib/stores/navStatus';
  import { missionManagerOpen, missionManagerSelectedId, requestOpenFlightId, requestOpenMissionId } from '$lib/stores/missionManager';
  import { batteryManagerOpen, batteryManagerCreateSerial, normalizeSerial } from '$lib/stores/batteryManager';
  import { vehicleManagerOpen, vehicleManagerCreateCraft } from '$lib/stores/vehicleManager';
  import type { BlackboxImportStatus } from '$lib/stores/flightlog';
  import { missionDbForFlight, flightLoggedWpCount, missionDbSave, flightLinkMission, missionDbGeocode, flightSetBatterySerial, updateFlightNotes, getFlight, flightlogCommitPending, flightlogDiscardPending, flightlogContinuePending, scanOrphanSessions, recoverDiscard, recoverSaveIncomplete, recoverContinue, batteryDbFindBySerial, batteryDbAddUsage, vehicleDbFindByCraftName, blackboxDecoderAvailable, downloadBlackboxDecode, hiresInfo, hiresParse, hiresSample, hiresDrop, hiresCleanup, scratchDir, scratchClear } from '$lib/stores/flightlog';
  import EndFlightDialog from "$lib/components/logbook/EndFlightDialog.svelte";
  import type { EndFlightStats } from "$lib/components/logbook/EndFlightDialog.svelte";
  import RecoveryPrompt from "$lib/components/logbook/RecoveryPrompt.svelte";
  import DisconnectArmedDialog from "$lib/components/logbook/DisconnectArmedDialog.svelte";
  import { haversineDistance, bearing, destinationPoint } from '$lib/utils/geo';
  import { buildMissionInput } from '$lib/helpers/missionLibrary';
  import { buildArduMissionInput } from '$lib/helpers/missionLibraryArdu';
  import { homePosition } from '$lib/stores/home';
  import { ingestFcGuidedTarget } from '$lib/controllers/vehicleControl';
  import { MAP_PROVIDERS } from "$lib/config/mapProviders";
  import { tileCacheStats, setCacheMaxMB, clearCache } from "$lib/cache/tileCache";
  import { weatherTempDisplayFromC, weatherWindDisplayFromMs, weatherTempCFromDisplay, weatherWindMsFromDisplay, canonicalWeatherDescription } from "$lib/helpers/weather";
  import type { TileCacheStats } from "$lib/cache/tileCache";
  import WidgetPanel from "$lib/components/WidgetPanel.svelte";
  import VideoBackdropMap from "$lib/components/video/VideoBackdropMap.svelte";
  import { LARGE_BASE_VMIN } from "$lib/config/widgetRegistry";
  import FloatingVideoWindow from "$lib/components/video/FloatingVideoWindow.svelte";
  import PhoneVideoDock from "$lib/components/phone/PhoneVideoDock.svelte";
  import { setNativeRightBound } from "$lib/controllers/nativeVideo";
  import { doubleTap, mouseDoubleClick } from "$lib/helpers/doubleTap";
  import { initVideo, videoState, videoStream, bindVideoEl, setMapLocation, setFloatHeightFrac, setFloatPos, registerPiPElement, reportMjpegError } from "$lib/stores/video";
  import { canvasSink, mjpegSink } from "$lib/controllers/mjpegSink";
  import { nativeSurface, activeNativeSurface } from "$lib/controllers/nativeVideo";
  import { lowPowerActive } from "$lib/stores/lowPower";
  import { initPulseBlink } from "$lib/stores/pulseBlink";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import TerrainAnalysisPanel from "$lib/components/terrain/TerrainAnalysisPanel.svelte";
  import { editMode, replayActive, mission, missionFlags, missionDownload, missionUpload, missionFcInfo, markMissionSynced, loadedMissionId, missionSetWaypoints, missionImportXml, launchPoint, hasLocation, toDeg, type Waypoint } from "$lib/stores/mission";
  import { pendingSystemSwitch, autopilotSystem, autopilotLocked, setAutopilotSystem, confirmSystemSwitch } from "$lib/stores/autopilotContext";
  import { arduMission, arduSelectedWpIndex, arduLoadedMissionId, parseWaypoints, parsePlanFile, planFirmwareTarget, loadArduMissionFromFile, type ArduWaypoint } from "$lib/stores/missionArdupilot";
  import { frameMissionOnMap } from "$lib/stores/mapCamera";
  import { terrainAnalysis, patchTerrainAnalysis } from "$lib/stores/terrainAnalysis";
  import { DEFAULT_RADAR, DEFAULT_AIRSPACE, BUILTIN_ADSB_PROVIDERS } from "$lib/stores/settings";
  import type { AppSettings, InterfaceSettings, PanelConfig, RadarSettings, GcsMode, AirspaceSettings, SystemMessagesLevel, LogLevel } from "$lib/stores/settings";
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
  // Mission edit mode needs the full map: leave 3D, and bring the map back from a mini frame (the
  // widget tile / the phone's docked frame take no waypoint interaction — Marc, 2026-09-04).
  $effect(() => {
    if ($editMode) untrack(() => {
      if (mapViewMode === '3d') mapViewMode = '2d';
      if (mapInFrame) setMapLocation('main');
    });
  });
  // Map3D instance handle — used to read the 3D camera focus on a 3D→2D switch so
  // the 2D map can re-centre on the same spot (keeping its own zoom).
  /** 2D map instance — for the trail backfill after the page was hidden (BACKGROUND_TELEMETRY.md). */
  let mapRef: { appendTrailPoints?: (points: { lat: number; lon: number; mode: string }[]) => void } | undefined = $state();
  // `connection-lost` while hidden → one reconnect attempt on return (unless the recording-interrupted
  // prompt took over). Not a loop.
  let lostWhileHidden = false;

  let map3dRef: {
    getCamFocus?: () => { lat: number; lon: number; range: number } | null;
    getCamSubpoint?: () => { lat: number; lon: number } | null;
    getCamGeo?: () => { sub: { lat: number; lon: number }; focus: { lat: number; lon: number } | null; headingDeg: number } | null;
    isFreeLook?: () => boolean;
    applyIonToken?: (token: string) => void;
  } | undefined = $state();

  // Missing-Cesium-key prompt: shown on the first switch to 3D when no Ion token is set (unless the
  // user dismissed it for good). `remind later` just closes (re-armed on the next 2D→3D switch).
  let cesiumKeyPromptOpen = $state(false);
  let cesiumPromptArmed = true; // re-armed whenever we leave 3D, so "remind later" re-triggers
  $effect(() => {
    if (mapViewMode === '3d') {
      const s = get(settings);
      if (cesiumPromptArmed && !s.cesiumIonToken && !s.cesiumKeyPromptDismissed) {
        cesiumKeyPromptOpen = true;
        cesiumPromptArmed = false;
      }
    } else {
      cesiumPromptArmed = true;
    }
  });
  function cesiumKeySave(token: string) {
    applySettingsPatch({ cesiumIonToken: token });
    map3dRef?.applyIonToken?.(token);
    cesiumKeyPromptOpen = false;
  }
  function cesiumKeyRemindLater() { cesiumKeyPromptOpen = false; }
  function cesiumKeyIgnore() {
    settings.patch({ cesiumKeyPromptDismissed: true });
    cesiumKeyPromptOpen = false;
  }
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
  // Live toolbar height (exposed as --toolbar-h). The phone toolbar wraps/collapses so its height
  // varies; the absolutely-positioned nav-rail + panels track this so they never hide under it.
  // The phone has no toolbar at all (Dev-Docs active/PHONE_UI.md): its chrome is the burger + chips,
  // the connection popout, the right-hand widget column and a bottom-left status strip.
  const phoneUi = isPhoneDevice;
  let toolbarH = $state(phoneUi ? 0 : 53);
  /** Rendered width of the phone widget column (reported by PhoneWidgetPanel) → the grid column. */
  let phonePanelW = $state(0);
  let sideDockW = $state(200);
  let sideDockH = $state(400);
  // Each dock panel's own cross-axis extent (its largest widget), reported by WidgetPanel — the dock
  // zone stays the reference that defines the L unit, the panel inside hugs the screen edge.
  let bottomPanelCrossPx = $state(0);
  let sidePanelCrossPx = $state(0);

  // Viewport size (for the snapped floating-video reserve)
  let winW = $state(typeof window !== 'undefined' ? window.innerWidth : 1280);
  let winH = $state(typeof window !== 'undefined' ? window.innerHeight : 720);

  /** The replay player's full panel is showing (paused / pinned) — reported by LogPlayer. */
  let playerExpanded = $state(false);
  // Phone: the full player must fit between the burger (12 + 42 + 8) and the chain-link button
  // (8 + 42 + 8 from the column) with 8px on each side. Whatever is missing slides the widget
  // column (and the button) out to the right — never further than the column is wide. 0 on wide
  // phones (21:9) and whenever the player is collapsed or closed, so the column is anchored while
  // a replay runs.
  const PHONE_PLAYER_CHROME_PX = 62 + 58 + 16;
  const phoneShift = $derived(
    phoneUi && playerExpanded
      ? Math.min(phonePanelW, Math.max(0, Math.round(logPlayerWidth + PHONE_PLAYER_CHROME_PX + phonePanelW - winW)))
      : 0,
  );
  // Width the bottom dock must yield to the bottom-left snapped video window.
  const videoReserve = $derived(
    $videoState.floating && $videoState.floatSnapped
      ? Math.min($videoState.floatHeightFrac * winH * ($videoState.aspect || 16 / 9), winW * 0.7) + 16
      : 0,
  );

  // Phone portrait: too narrow to fit the bottom HUD tiles in one row without clipping, so the dock
  // wraps them onto two rows (see the sizing below + the flex-wrap rule in WidgetPanel). Tablets and
  // desktop keep the single row. `winW`/`isMobile` are reactive so a rotate re-evaluates this.
  const isPhone = $derived(isMobile && winW <= 600);
  const bottomRows = $derived(isPhone ? 2 : 1);
  // Phone gets a taller dock (room for two HUD rows); otherwise the layout store override or default.
  const gridBottomHeight = $derived(
    $layout.bottomDock.sizeOverride ??
      (isPhone ? 'clamp(230px, 40vh, 380px)' : GRID_DEFAULTS.bottomDockHeight)
  );

  // Map-swap: the full-size video sink shown in the map zone when videoPrimary.
  let mapVideoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(mapVideoEl, $videoStream);
  });

  // Persistent (always-mounted) source element for native Picture-in-Picture, so
  // the PiP window survives closing the Video panel. Hidden but rendered/playing.
  let pipVideoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(pipVideoEl, $videoStream);
    if (pipVideoEl) registerPiPElement(pipVideoEl);
  });

  // Global UI scale (1 = 100%, up to 2). Zooms the chrome via `.ui-scale`; the map
  // (`.layer-map`) stays unzoomed/native. See docs/archive/UI_SCALING.md.
  let uiScale = $state(1);

  // Fully populated nav rail in logical px: hamburger 44 + 4 top margin + 9 tabs ×40 + 8 gaps ×2
  // (DEV excluded). A fixed reference on purpose — the live rail height varies with connection state.
  const NAV_RAIL_FULL_HEIGHT = 424;
  // Bottom reserve for the floating panels (PanelShell). Panels normally stop above the bottom
  // widget dock, but once that cap would make them shorter than the nav rail, they may overlay the
  // dock instead — the rail already scrolls past it, so the panel just follows. Logical px
  // throughout, so the switch adapts to the UI scale. The 6px keeps the rail's visual gap above
  // the status bar instead of sitting flush on it.
  const panelBottomReserve = $derived(
    winH / uiScale - 53 - bottomDockH - 24 - 12 < NAV_RAIL_FULL_HEIGHT ? '6px' : gridBottomHeight
  );

  // Floating-window rect (must match FloatingVideoWindow's own computation) — used
  // to place the map inside the window's frame when the view is swapped. The window
  // lives in the zoomed `.ui-scale` layer but the map is unzoomed, so the visual rect
  // is the window's logical rect * uiScale.
  // Must match FloatingVideoWindow's geometry exactly (incl. the 200px min-height floor that keeps
  // the mini-map's 4 control buttons from overflowing) so the in-frame map aligns with the frame.
  const FLOAT_MIN_H = 200;
  // Phone (PHONE_VIDEO.md D2): the DOCKED frame instead — the stream aspect fitted into 40 % of the
  // map-area height / 50 % of its width, whichever binds; bottom-right of the map area, left of the
  // corner-control column (8 + 38 + 8), bottom-aligned with the chip row (8 px; the safe inset is
  // 0 on Android, iPhone has no video). Same numbers drive PhoneVideoDock and the in-frame map.
  const phoneMapW = $derived(winW - phonePanelW + phoneShift);
  const dockH = $derived.by(() => {
    const aspect = $videoState.aspect || 16 / 9;
    return Math.round(Math.min(0.4 * winH, (0.5 * phoneMapW) / aspect));
  });
  const dockW = $derived(Math.round(dockH * ($videoState.aspect || 16 / 9)));
  const dockLeft = $derived(phoneMapW - 8 - 38 - 8 - dockW);
  const dockTop = $derived(winH - 8 - dockH);
  const floatH = $derived(
    phoneUi ? dockH : Math.min(Math.round(0.3 * winH), Math.max(FLOAT_MIN_H, Math.round($videoState.floatHeightFrac * winH))),
  );
  const floatW = $derived(phoneUi ? dockW : Math.min(Math.round(floatH * ($videoState.aspect || 16 / 9)), Math.round(winW * 0.7)));
  const floatLeft = $derived(phoneUi ? dockLeft : $videoState.floatSnapped ? 8 : $videoState.floatX);
  const floatTop = $derived(phoneUi ? dockTop : $videoState.floatSnapped ? winH - floatH - 30 : $videoState.floatY);
  // The phone's widget column overlays the map: no native surface may show through it (the docked
  // frame parks behind it) — the surface router clips at its edge.
  $effect(() => {
    setNativeRightBound(phoneUi ? phoneMapW : null);
  });
  // The single map jumps to whichever video surface was double-clicked: `floating` → the chromeless
  // floating window frame, `widget` → the video-widget tile (its published rect). Every other surface
  // shows video. `main` (default) = the normal full-screen map.
  const mapInFrame = $derived($videoState.mapLocation !== 'main' && $videoState.status === 'live');
  // Map centre offset for the covered right edge (PHONE_UI.md D16) — only while the map is the
  // full-screen layer. In the docked frame / widget tile nothing covers it: a shifted centre put
  // the follow anchor on the frame's left edge (Marc, 2026-09-04).
  const phoneMapInset = $derived(phoneUi && !mapInFrame ? phonePanelW - phoneShift : 0);
  // Full-screen map box, rounded to whole px (issue #52): the CSS fallback `calc(53px * scale)`
  // lands on fractions at uiScale 1.25/1.5 (66.25px / 79.5px), which is what leaked tile seams —
  // see mapFrameStyle above for the mechanism. The map sliding ≤ half a px under the toolbar edge
  // is invisible (chrome z1 covers map z0); Map.svelte's ResizeObserver re-invalidates on the
  // resulting size change by itself.
  const mapLayerStyle = $derived(
    phoneUi
      ? 'top:0; bottom:0;' // the whole screen — the map runs on under the widget column's glass
      : `top:${Math.round(53 * uiScale)}px; bottom:${Math.round(24 * uiScale)}px;`,
  );
  const mapFloating = $derived($videoState.mapLocation === 'floating');
  const mapInWidget = $derived($videoState.mapLocation === 'widget');
  // Rounded to whole px, here and everywhere the unzoomed map layer is placed (issue #52): a
  // fractional box origin (floatLeft × 1.25 …) puts every 256px tile edge on a subpixel boundary,
  // and WebKitGTK antialiases each tile as its own composited layer — hairline seams between tiles,
  // shimmering during heading-follow rotation. Chromium snaps layers to device pixels, which is why
  // Windows never showed it. The frame drawn by the zoomed chrome may sit up to half a px off the
  // rounded map rect — invisible, and .miniframe-ctl uses this same string so it stays aligned.
  const mapFrameStyle = $derived(
    `left:${Math.round(floatLeft * uiScale)}px; top:${Math.round(floatTop * uiScale)}px; width:${Math.round(floatW * uiScale)}px; height:${Math.round(floatH * uiScale)}px;`,
  );
  // The rect the in-frame map is positioned into (screen px): the floating frame, or the widget tile.
  const inFrameStyle = $derived(
    mapFloating
      ? mapFrameStyle
      : $videoState.widgetRect
        ? `left:${Math.round($videoState.widgetRect.x)}px; top:${Math.round($videoState.widgetRect.y)}px; width:${Math.round($videoState.widgetRect.w)}px; height:${Math.round($videoState.widgetRect.h)}px;`
        : '',
  );

  // Safety: if the video feed drops while the map is parked on a video surface, bring it back.
  $effect(() => {
    if ($videoState.mapLocation !== 'main' && $videoState.status !== 'live') {
      untrack(() => setMapLocation('main'));
    }
  });

  // ── Mini-map frame controls (videoPrimary) ──────────────────────────
  // Rendered top-level (unzoomed, above the in-frame map at z2) because the float-win's own corners
  // live in the .ui-scale layer (z1) and would sit *behind* the map. Close swaps back; the grip
  // resizes (top-right, bottom-left anchored), mirroring FloatingVideoWindow.
  let miniResizing = false;
  let mrStartY = 0;
  let mrStartFrac = 0;
  let mrStartBottom = 0;
  let mrSnapped = false;
  function miniResizeDown(e: PointerEvent) {
    e.stopPropagation();
    e.preventDefault();
    miniResizing = true;
    mrStartY = e.clientY;
    mrStartFrac = $videoState.floatHeightFrac;
    mrStartBottom = floatTop + floatH;
    mrSnapped = $videoState.floatSnapped;
    window.addEventListener('pointermove', miniResizeMove);
    window.addEventListener('pointerup', miniResizeUp);
  }
  function miniResizeMove(e: PointerEvent) {
    if (!miniResizing) return;
    const delta = (mrStartY - e.clientY) / winH; // drag up → larger
    const fracMin = Math.max(0.1, FLOAT_MIN_H / winH);
    const newFrac = Math.min(0.3, Math.max(fracMin, mrStartFrac + delta));
    setFloatHeightFrac(newFrac);
    if (!mrSnapped) setFloatPos($videoState.floatX, mrStartBottom - newFrac * winH);
  }
  function miniResizeUp() {
    miniResizing = false;
    window.removeEventListener('pointermove', miniResizeMove);
    window.removeEventListener('pointerup', miniResizeUp);
  }

  // The WIDGET mini-map is locked to a clean nav view: 2D + heading-follow, zoom-only (3D/mode buttons
  // hidden via `miniControls`). The FLOATING map stays fully operational on the desktop; on the phone
  // the docked frame is a mini map too and takes the same lock (PHONE_VIDEO.md D6). Restore the view
  // on release.
  // Only while the map is actually in a frame (mapInFrame includes `status === 'live'`): a stale
  // mapLocation with the video off must not put the FULL map into mini mode (half-size markers).
  const miniMapLocked = $derived(mapInFrame && (mapInWidget || phoneUi));
  let miniLockActive = false;
  let savedMapViewMode: '2d' | '3d' = '2d';
  let savedMode2d: 'free' | 'follow' | 'heading-follow' = 'free';
  $effect(() => {
    const lock = miniMapLocked;
    untrack(() => {
      if (lock && !miniLockActive) {
        miniLockActive = true;
        savedMapViewMode = mapViewMode;
        savedMode2d = map2dViewMode;
        mapViewMode = '2d';
        map2dViewMode = 'heading-follow';
      } else if (!lock && miniLockActive) {
        miniLockActive = false;
        mapViewMode = savedMapViewMode;
        map2dViewMode = savedMode2d;
      }
    });
  });

  // Per-container px-per-unit: 1 unit = cross-axis fraction so that
  // LARGE_BASE_VMIN units == cross-axis px (widget fills dock height/width).
  // This fully decouples bottom dock and side dock scaling.
  // Subtract zone padding (6px each side) from cross-axis measurement.
  const DOCK_PAD = 6;
  // On phone the dock is two rows tall, so a tile is sized to ONE row (dock height / rows). Desktop
  // and tablet keep the full-height single-row tile.
  const bottomPxPerUnit = $derived((bottomDockH / bottomRows - 2 * DOCK_PAD) / LARGE_BASE_VMIN);
  const sidePxPerUnit   = $derived((sideDockW  - 2 * DOCK_PAD) / LARGE_BASE_VMIN);

  // Available space expressed in abstract units (container px / pxPerUnit)
  // Bottom: subtract edit button (28px) + wrapper gap (6px) + zone padding (12px). Multiply by the row
  // count so the sizing algorithm (which lays out in one line) budgets for two rows on phone and keeps
  // the tiles a readable size; flex-wrap then splits them across the rows.
  const bottomAvailUnits = $derived(Math.max(0, ((bottomDockW - 34 - 2 * DOCK_PAD - videoReserve) / bottomPxPerUnit) * bottomRows));
  const rightAvailUnits  = $derived(Math.max(0, (sideDockH - 2 * DOCK_PAD) / sidePxPerUnit));

  let appVersion = $state("...");
  // iOS has no serial/BLE, so the iPad build defaults to Wi-Fi MAVLink (UDP 14550, the MAVLink
  // convention). Desktop keeps its serial/MSP defaults.
  let selectedTransport = $state<TransportType>(isMobile ? 'udp' : 'serial');
  let selectedProtocol = $state<ProtocolType>(isMobile ? 'mavlink' : 'msp');
  let selectedPort = $state("");
  let selectedBaud = $state(115200);
  let tcpHost = $state("192.168.1.1");
  let tcpPort = $state(isMobile ? 14550 : 5761);
  let selectedBleDevice = $state("");
  let bleDeviceList = $state<BleDeviceInfo[]>([]);
  let isBleScanning = $state(false);
  let isConnecting = $state(false);
  let errorMsg = $state("");
  let navPanelOpen = $state(false);
  let activeTab = $state("uav-info");
  /** The active panel is slid out of view (state intact) — re-click of its rail button. */
  let panelHidden = $state(false);
  // Telemetry Relay dropdown (under the connection bar).
  let relayPanelOpen = $state(false);

  // Terrain Analysis overlay (NavRail-triggered, full-width over the map)
  let terrainOpen = $state(false);
  terrainAnalysis.subscribe((s) => { terrainOpen = s.open; });

  // Debug Monitor + dev UI. In dev builds `import.meta.env.DEV` is true; in a RELEASE build it's enabled
  // at runtime when started with `--debug` (backend `is_debug_mode`, fetched in initPage → debugMode).
  // Because DEV_MODE now depends on a runtime value, Vite can no longer prove the DebugPanel import dead,
  // so the chunk stays in the release bundle (loaded lazily only when debug mode is actually on).
  let debugMode = $state(false);
  const DEV_MODE = $derived(import.meta.env.DEV || debugMode);
  let debugOpen = $state(false);
  let DebugPanelCmp: any = $state(null);
  $effect(() => {
    if (DEV_MODE && !DebugPanelCmp) {
      void import('$lib/components/DebugPanel.svelte').then(m => { DebugPanelCmp = m.default; });
    }
  });

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
  let accMaxAlt = 0, accMaxSpeed = 0, accMaxDist = 0, accMah = 0, accTotalDist = 0;
  let accStartLat: number | null = null, accStartLon: number | null = null;
  let accLastLat: number | null = null, accLastLon: number | null = null;
  telemetry.subscribe((t) => {
    liveTelem = t;
    // Accumulate the live flown track (RAM) for the Terrain Analyzer
    const armed = isArmed(t.armingFlags, t.lastUpdate);
    // Baseline the armed state on the first frame that actually CARRIES it (statusSeen) → no false
    // edge on reconnect. Gating on any first frame (lastUpdate) raced: attitude/GPS at 5–10 Hz beat
    // the 1 Hz status after a reconnect, seeding prevArmed=false from a frame without arming info —
    // the first real status then looked like an arm edge and moved Home to wherever the aircraft was.
    if (!armEdgeInit && t.statusSeen) { armEdgeInit = true; prevArmed = armed; }
    if (armed && !prevArmed) {
      clearLiveTrack();
      // reset the flight-stats accumulator for the new flight
      armStartMs = t.lastUpdate || Date.now();
      accMaxAlt = 0; accMaxSpeed = 0; accMaxDist = 0; accMah = 0; accTotalDist = 0;
      accStartLat = null; accStartLon = null; accLastLat = null; accLastLon = null;
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
        // Total flown distance: sum of segments between consecutive fixes (matches the recorder).
        if (accLastLat != null) accTotalDist += haversineDistance(accLastLat, accLastLon as number, t.lat, t.lon);
        accLastLat = t.lat; accLastLon = t.lon;
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
        // altMsl, NOT t.altitude (relative): Home consumers treat alt as AMSL (the HOME_POSITION /
        // MSP_WP0 paths store AMSL, Map3D places the "H" absolutely) — the relative alt (~0 at a
        // ground arm) sank the 3D marker below the terrain by the local elevation.
        homePosition.set({ lat: t.lat, lon: t.lon, alt: t.altMsl, set: true, source: 'fc' });
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
          totalDistM: accTotalDist || null,
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
  let windEnabled = $state(false);
  let mavlinkFullTelemetry = $state(false);
  let flightLoggingEnabled = $state(false);
  let flightRecordingEnabled = $state(false);
  let flightLogDbPath = $state("");
  // Open a log WITHOUT importing it (Dev-Docs active/OPEN_LOG_WITHOUT_IMPORT.md): the file is parsed
  // by the ordinary importers into a throwaway scratch DB dir; while one is open, every logbook /
  // replay READ goes there instead of the main DB (writes are unreachable — the panel is read-only).
  // Several files can be open at once (a multi-file drop, or files dropped one after another): the
  // scratch DB collects them, each scratch flight remembers its source file (+ log index inside a
  // multi-log flash dump) so it can be imported for real on its own.
  type OpenedFlight = { id: number; sourcePath: string; logIndex?: number };
  let openedLogs = $state<{ dir: string; flights: OpenedFlight[] } | null>(null);
  const activeDbPath = $derived(openedLogs?.dir ?? flightLogDbPath);
  const openedFileNames = $derived(
    openedLogs ? [...new Set(openedLogs.flights.map((f) => baseName(f.sourcePath)))] : [],
  );
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
  let buildings3D = $state(false);
  let logReplayTime = $state(false);
  let nightMode2D = $state<'off' | 'auto' | 'on'>('off');
  let gcsMode = $state<GcsMode>('manual');
  let radarSettings = $state<RadarSettings>({ ...DEFAULT_RADAR });
  let airspaceSettings = $state<AirspaceSettings>({ ...DEFAULT_AIRSPACE });
  let defaultWpAltitudeM = $state(50);
  let defaultPhTimeSec = $state(30);
  let warnAltitudeM = $state(120);
  let systemMessages = $state<SystemMessagesLevel>('all');
  let logLevel = $state<LogLevel>('warning');
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

  // ── Hi-res replay (Dev-Docs active/HIRES_REPLAY.md) ──────────────────────────────────────
  // Full-rate re-parse of the archived log into a disposable cache DB. The 10 Hz track stays the
  // master timeline (scrubber/map/index); hi-res only overrides the sampled instrument values.
  let hiresAvailable = $state(false); // archived blob exists + format parseable
  let hiresActive = $state(false);
  let hiresParsing = $state(false);
  let hiresProgress = $state<BlackboxImportProgress | null>(null);
  let hiresCachePath = $state<string | null>(null);
  let hiresEstimateBytes = $state<number | null>(null);
  let hiresSamplePoint = $state<TelemetryRecord | null>(null);
  let hiresVirtualMs = $state<number | null>(null); // continuous playback clock (onTime callback)
  let hiresOwnerFlightId: number | null = null; // whose cache file exists (for the drop on switch)
  let hiresOwnerDbPath = ''; // …and in which DB dir (main, or an opened file's scratch dir)

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
    bottom: ['battery', 'speed', 'ahi', 'altitude', 'compass'],
    right: ['home', 'rcLink', 'gps'],
  };
  let panels = $state<PanelConfig>(defaultPanels);
  let widgetEditMode = $state(false);
  // Raw telemetry popup (toolbar button next to Relay; connected only).
  let rawTelemetryOpen = $state(false);

  // Unobstructed fullscreen (Video panel toggle): the map-swap video box retreats from the
  // nav-rail column (always, in this mode) and from OCCUPIED widget panels, so widgets never
  // overlay the picture. A panel counts as occupied only while visible AND holding widgets —
  // an empty or hidden panel contributes 0 and the video keeps that edge. Values are logical
  // px (the wrapper lives in the zoomed .app layer); the measured dock sizes (clientWidth/
  // clientHeight binds above) track user resizes and the phone/tablet overrides for free.
  // The reserve is the panel's OWN extent (its largest widget + the zone padding), not the whole
  // dock zone: a dock of small tiles hugs the screen edge and the video gets the rest
  // (WIDGET_OVERHAUL.md D7). Capped at the zone in case a bind lags a frame.
  // Not on the phone: there is no dock to retreat from, the video runs under the widget column.
  const ufActive = $derived($videoState.unobstructedFullscreen && mapInFrame && !phoneUi);
  const ufRight = $derived(
    ufActive && $layout.sideDock.visible && panels.right.length > 0
      ? Math.min(sideDockW, sidePanelCrossPx + 2 * DOCK_PAD)
      : 0
  );
  const ufBottomExtra = $derived(
    ufActive && $layout.bottomDock.visible && panels.bottom.length > 0
      ? Math.min(bottomDockH, bottomPanelCrossPx + 2 * DOCK_PAD)
      : 0
  );
  // The wrapper itself stays FULL-SIZE (so the blurred backdrop map fills the whole zone,
  // widgets and nav rail float on it); the reserves only shrink the AVAILABLE AREA the
  // video box is fitted into: 62px nav-rail column left (always in this mode), the occupied
  // docks right/bottom. Inside that area the box is cut to the stream's aspect ratio and
  // centred — no letterbox bars at all (the native sink's own black letterbox never becomes
  // visible because box = picture). Null until the wrapper is measured (or when the mode is
  // off) — the box then falls back to filling the wrapper via CSS.
  let ufWrapW = $state(0);
  let ufWrapH = $state(0);
  const ufBox = $derived.by(() => {
    if (!ufActive) return null;
    const availW = ufWrapW - 62 - ufRight;
    const availH = ufWrapH - ufBottomExtra;
    if (availW <= 0 || availH <= 0) return null;
    const aspect = $videoState.aspect || 16 / 9;
    const w = Math.min(availW, availH * aspect);
    return {
      left: Math.round(62 + (availW - w) / 2),
      top: Math.round((availH - w / aspect) / 2),
      w: Math.round(w),
      h: Math.round(w / aspect),
    };
  });

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

  // RC control (INAV RC over MSP) — two stacked sticks + a signal arc; opt-in via settings.
  const ICON_RC = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="3" width="17" height="18" rx="2.5"/><circle cx="8" cy="9" r="2.1"/><circle cx="16" cy="9" r="2.1"/><path d="M6.5 15.5h11"/><path d="M6.5 18h6"/></svg>';

  // The vehicle-control panel is MAVLink-only (ArduPilot/PX4) and only meaningful while connected.
  const isMavlinkConnected = $derived(
    $connection.status === 'connected' && $connection.protocolType === 'mavlink'
  );

  // Passive telemetry (listen-only) has no uplink — there's no way to send RC channels — so the RC tab
  // is hidden while connected that way. Available = master switch on AND not telemetry-connected.
  const isTelemetryConnected = $derived(
    $connection.status === 'connected' && $connection.protocolType === 'telemetry'
  );
  const rcTabAvailable = $derived($settings.rcControl.enabled && !isTelemetryConnected);
  // On mobile there is no joystick, but the on-screen touch sticks (VirtualSticks) can drive RC over
  // Wi-Fi. RC is safety-relevant and barely field-tested, so touch control is opt-in behind the SAME
  // master switch as the joystick path (`rcTabAvailable`) rather than appearing whenever an FC is
  // connected — one switch governs both, no matter the input device. On top of that, mobile also needs a
  // control-capable connection (MSP or MAVLink, not passive telemetry).
  const mobileRcAvailable = $derived(
    isMobile && rcTabAvailable && $connection.status === 'connected'
  );

  // If the RC tab is open when it becomes unavailable (e.g. a telemetry connection comes up), fall back
  // to the UAV-info tab so the now-hidden panel isn't left rendered.
  $effect(() => {
    if (!rcTabAvailable && !mobileRcAvailable && activeTab === 'rc-control') activeTab = 'uav-info';
  });

  const allTabs = [
    { id: "uav-info", label: () => $t('nav.uavInfo'), icon: ICON_UAV_INFO },
    { id: "mission", label: () => $t('nav.mission'), icon: ICON_MISSION },
    { id: "control", label: () => $t('nav.control'), icon: ICON_CONTROL },
    { id: "rc-control", label: () => $t('nav.rc'), icon: ICON_RC },
    { id: "terrain", label: () => $t('nav.terrain'), icon: ICON_TERRAIN },
    { id: "logbook", label: () => $t('nav.logbook'), icon: ICON_LOGBOOK },
    { id: "radar", label: () => $t('nav.radar'), icon: ICON_RADAR },
    { id: "airspace", label: () => $t('nav.airspace'), icon: ICON_AIRSPACE },
    { id: "video", label: () => $t('nav.video'), icon: ICON_VIDEO },
    { id: "settings", label: () => $t('nav.settings'), icon: ICON_SETTINGS },
  ];
  // Airspace tab shows when its master switch is on, OR when a geozone-capable INAV FC is connected (so
  // the panel can host the geozone editor even with the OpenAIP overlay disabled). (Ardu/PX4 geofence later.)
  const geozonesAvailable = $derived($geozoneWorking?.has_geozones ?? false);
  const fenceAvailable = $derived($fenceWorking?.has_fence ?? false);
  const rallyAvailable = $derived($rallyWorking?.has_rally ?? false);
  const tabs = $derived(
    allTabs.filter(t =>
      (t.id !== 'logbook' || flightLoggingEnabled) &&
      (t.id !== 'control' || isMavlinkConnected) && // control tab only when connected via MAVLink
      (t.id !== 'rc-control' || (rcTabAvailable && !isMobile) || mobileRcAvailable) && // RC tab: desktop needs the master switch + a joystick; mobile uses on-screen sticks when a FC is connected
      (t.id !== 'radar' || radarSettings.enabled) && // radar tab only when the master switch is on
      (t.id !== 'airspace' || airspaceSettings.enabled || geozonesAvailable || fenceAvailable || rallyAvailable) // airspace: master switch, or geozone (INAV) / fence+rally (MAVLink) capable FC
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
  // Restore the full last-used connection path so nothing has to be re-entered. A serial value is only
  // honoured where serial ports exist (iOS has none — a value synced over from a desktop is ignored);
  // TCP/UDP/BLE are valid everywhere.
  if (saved.lastTransport === 'tcp' || saved.lastTransport === 'udp' || saved.lastTransport === 'ble'
      || (hasSerialPorts && saved.lastTransport === 'serial')) {
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
  windEnabled = saved.windEnabled;
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
  buildings3D = saved.buildings3D ?? false;
  logReplayTime = saved.logReplayTime ?? false;
  nightMode2D = saved.nightMode2D ?? 'off';
  gcsMode = saved.gcsMode ?? 'manual';
  if (saved.radar) radarSettings = saved.radar;
  if (saved.airspace) airspaceSettings = saved.airspace;
  defaultWpAltitudeM = saved.defaultWpAltitudeM;
  defaultPhTimeSec = saved.defaultPhTimeSec;
  warnAltitudeM = saved.warnAltitudeM;
  systemMessages = saved.systemMessages ?? 'all';
  // Apply the persisted diagnostic log level to the backend logger (it starts at Warning by default).
  // When the app runs in debug mode (release `--debug` or any debug build) surface the Debug Monitor
  // and force the log to Debug regardless of the saved level.
  const savedLogLevel: LogLevel = saved.logLevel ?? 'warning';
  logLevel = savedLogLevel;
  void invoke<boolean>('is_debug_mode')
    .then((dbg) => {
      debugMode = !!dbg;
      // Mirror the runtime debug flag into the shared store so non-page components (e.g. Map3D's
      // Performance tab hooks) can gate on --debug without each fetching is_debug_mode themselves.
      isDebugMode.set(import.meta.env.DEV || !!dbg);
      if (dbg) logLevel = 'debug'; // reflect the forced level in the Settings dropdown
      void invoke('set_log_level', { level: dbg ? 'debug' : savedLogLevel }).catch(() => {});
    })
    .catch(() => {
      void invoke('set_log_level', { level: savedLogLevel }).catch(() => {});
    });
  // Record a curated settings snapshot in the log's session header (support-relevant config only —
  // not the full blob with widget layout / map center / cache size). See logging::log_session_settings.
  {
    const s = saved;
    const summary = JSON.stringify({
      protocol: s.lastProtocol, transport: s.lastTransport, baud: s.lastBaud,
      attitudeHz: s.attitudeRateHz, positionHz: s.positionRateHz,
      airspeed: s.airspeedEnabled, mavlinkFull: s.mavlinkFullTelemetry,
      flightLog: s.flightLoggingEnabled, rawLog: s.flightLogRawEnabled,
      batteryAlertPct: s.batteryAlertPct, mapProvider: s.mapProvider,
      uiScale: s.uiScale, logLevel: savedLogLevel,
    });
    void invoke('log_session_settings', { summary }).catch(() => {});
  }
  uiScale = saved.uiScale ?? 1;
  interfaceSettings = saved.interface ?? {
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  };
  // Sanitised: a stored layout may still name a widget the registry no longer has.
  panels = widgetCtrl.sanitizePanels(saved.panels ?? defaultPanels);
  if (phoneUi) {
    // Same for the phone grid: registry drift, sizes, overflow → deactivate.
    const normalized = phoneCtrl.normalizePhoneWidgets(saved.phoneWidgets ?? phoneCtrl.DEFAULT_PHONE_WIDGETS);
    if (normalized !== saved.phoneWidgets) settings.patch({ phoneWidgets: normalized });
  }

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
    startBreachMonitor();
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

  // Telemetry API (Dev-Docs active/TELEMETRY_API.md): push the persisted config on start + every change;
  // the backend reconfigures in place (unchanged config = no restart, clients keep their stream).
  $effect(() => {
    const cfg = $settings.telemetryApi;
    void invoke('telemetry_api_configure', { config: cfg }).catch((e) => console.warn('[telemetry-api] configure failed:', e));
  });
  // ...and the one value only the frontend knows: the resolved GCS marker position (null = marker off).
  $effect(() => {
    const g = $gcsLocation;
    const acc = $gcsAccuracyM;
    const alt = gcsGroundAltM;
    void invoke('telemetry_api_set_gcs', { gcs: g ? { lat: g.lat, lon: g.lon, altMsl: alt, accuracyM: acc } : null }).catch(() => {});
  });

  // Auto-start video with the last settings if it was running at last close.
  if (typeof window !== 'undefined') void initVideo();

  function toggleNavPanel() {
    navPanelOpen = !navPanelOpen;
    panelHidden = false;
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

  // Collapse-anywhere (Dev-Docs active/REPLAY_PANEL_COMPACT.md): a click/tap on any surface other
  // than the logbook itself or a dialog collapses the flight view to its info card — the mini-map
  // click used to be the only way, awkward in fullscreen video. Capture phase, so a surface that
  // stops propagation still counts. The handler reads state at event time (not tracked).
  $effect(() => {
    const onDown = (e: PointerEvent) => {
      if (!logbookHasFlightOnMap || logbookMinimized) return;
      const target = e.target as Element | null;
      if (target?.closest('.logbook-host, .dialog-backdrop')) return;
      minimizeLogbook();
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
  });

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
    if (patch.windEnabled != null) windEnabled = patch.windEnabled;
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
    if (patch.buildings3D != null) buildings3D = patch.buildings3D;
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
    if (patch.logLevel != null) {
      logLevel = patch.logLevel;
      void invoke('set_log_level', { level: patch.logLevel }).catch(() => {});
    }
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
    // Re-clicking the ACTIVE tab's button hides its panel without touching its state (the mission
    // edit mode stays armed, a half-typed form survives): the panel slides out to the left and the
    // map gets the whole screen; the next click brings it back. Only switching to another tab or
    // closing the rail (the hamburger X) deactivates as before. Same for the terrain overlay.
    const isActive = terrainOpen ? tabId === 'terrain' : tabId === activeTab;
    if (navPanelOpen && isActive) {
      panelHidden = !panelHidden;
      setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
      return;
    }
    panelHidden = false;
    // Terrain Analysis is a full-width overlay shown in place of the panel content.
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

  /** Folder picker for the storage-location settings. Desktop: the dialog plugin, a real path.
   *  Android: the system tree picker — the user grants ONE folder (scoped storage, no permission),
   *  the setting stores the grant's content:// tree URI, and a session-end mirror copies the
   *  artefacts into it (the app itself keeps writing app-private; SQLite and the raw writers need
   *  real paths, which a SAF grant does not provide). */
  async function pickStorageFolder(defaultPath?: string): Promise<string | null> {
    if (isAndroid) {
      return await invoke<string | null>('storage_pick_folder');
    }
    const selected = await open({ directory: true, multiple: false, defaultPath });
    return typeof selected === 'string' && selected.length > 0 ? selected : null;
  }

  async function chooseFlightLogPath() {
    try {
      const selected = await pickStorageFolder(flightLogDbPath || defaultFlightLogPath || undefined);
      if (selected) {
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
      const selected = await pickStorageFolder(flightLogRawPath || defaultRawLogPath || undefined);
      if (selected) {
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

  // Set when the flight DB was written by a NEWER Kite (downgrade guard, `db-newer:` marker from
  // the backend) — the logbook shows a targeted "restore a backup or update Kite" notice instead
  // of a generic error and stays disabled; the DB file is never touched. Cleared on a successful
  // load (e.g. after the user pointed the DB path at a compatible copy). The explanatory popup
  // fires ONCE per session, and ONLY on a real schema mismatch — every other DB error keeps the
  // generic error bar.
  let logbookDbIncompatible = $state<string | null>(null);
  let dbNewerPopupShown = false;

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
      flightSummaries = await logbookCtrl.loadFlights(activeDbPath);
      logbookDbIncompatible = null;
      if (selectedFlightId != null) {
        const found = flightSummaries.find((f) => f.id === selectedFlightId);
        if (!found) {
          selectedFlight = null;
          selectedFlightId = null;
          selectedFlightTrackCount = 0;
        }
      }
    } catch (e) {
      const msg = String(e);
      if (msg.includes('db-newer:')) {
        logbookDbIncompatible = msg;
        flightSummaries = [];
        if (!dbNewerPopupShown) {
          dbNewerPopupShown = true;
          void showInfo($t('logbook.dbNewerTitle'), $t('logbook.dbNewerPopup'));
        }
      } else {
        errorMsg = msg;
      }
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
      // Hi-res: drive the clock at screen refresh (rAF) and expose the continuous virtual time so
      // the sampler can pull sub-100ms values; the 10 Hz index ticks stay the master timeline.
      { raf: hiresActive, onTime: (t) => { hiresVirtualMs = t; } },
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
    resetHires(true);
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

  // ── Open logs WITHOUT importing them (Dev-Docs active/OPEN_LOG_WITHOUT_IMPORT.md) ──────────
  // Desktop only, and only while nothing is connected (the player is replay-only anyway). Files run
  // through the ordinary importers into the scratch dir — force_import (nothing there to be a
  // duplicate of), no linking prompt (no live flights there), a multi-log .bbl imports all its logs
  // without the prompt. `.kflight` stays import-only: it IS a logbook export.
  const OPEN_EXTS = ['txt', 'bbl', 'bfl', 'bin', 'ulg', 'rawmsp', 'tlog'];
  const canOpenLogFile = $derived(!isMobile && flightLoggingEnabled);
  const isOpenableLog = (p: string) => new RegExp(`\\.(${OPEN_EXTS.join('|')})$`, 'i').test(p);

  /** Parse one file into the scratch dir; returns the scratch flights it produced. */
  async function parseIntoScratch(filePath: string, dir: string): Promise<OpenedFlight[]> {
    const ext = filePath.split('.').pop()?.toLowerCase() ?? '';
    const lang = $locale ?? 'en';
    const idOf = (r: BlackboxImportStatus) => (r.type === 'duplicate' ? null : r.flight_id);
    const out: OpenedFlight[] = [];
    if (ext === 'bin') {
      const id = idOf(await logbookCtrl.importArdupilot(filePath, dir, true, lang));
      if (id != null) out.push({ id, sourcePath: filePath });
    } else if (ext === 'ulg') {
      const id = idOf(await logbookCtrl.importUlog(filePath, dir, true, lang));
      if (id != null) out.push({ id, sourcePath: filePath });
    } else if (ext === 'rawmsp' || ext === 'tlog') {
      for (const id of (await logbookCtrl.importRaw(filePath, dir)).flightIds) out.push({ id, sourcePath: filePath });
    } else {
      const logCount = await logbookCtrl.countBlackboxLogs(filePath);
      if (logCount <= 1) {
        const id = idOf(await logbookCtrl.importBlackbox(filePath, dir, undefined, true, lang));
        if (id != null) out.push({ id, sourcePath: filePath });
      } else {
        for (let index = 0; index < logCount; index++) {
          const id = idOf(await logbookCtrl.importBlackbox(filePath, dir, index, true, lang));
          if (id != null) out.push({ id, sourcePath: filePath, logIndex: index });
        }
      }
    }
    return out;
  }

  /** Open one or more log files for replay. Appends to an already open set; a selected main-DB
   *  flight gives way. Failures are collected per file, the rest still opens. */
  async function openLogFiles(paths: string[]) {
    if (!canOpenLogFile || blackboxImporting) return;
    if (connStatus === 'connected') {
      errorMsg = $t('logbook.openLogConnected');
      return;
    }
    const files = paths.filter(isOpenableLog);
    const rejected = paths.filter((p) => !isOpenableLog(p));
    if (rejected.length > 0) {
      errorMsg = $t('logbook.openLogUnsupported', { values: { file: rejected.map(baseName).join(', ') } });
    }
    if (files.length === 0) return;
    if (files.some((f) => BLACKBOX_EXTS.test(f)) && !(await ensureBlackboxDecoder())) return;

    if (selectedFlight != null) closePlayer();
    const dir = openedLogs?.dir ?? (await scratchDir(flightLogDbPath));
    if (!openedLogs) await scratchClear(flightLogDbPath); // fresh set → start from an empty scratch
    blackboxImporting = true;
    const failures: string[] = [];
    const added: OpenedFlight[] = [];
    try {
      for (const filePath of files) {
        try {
          added.push(...(await parseIntoScratch(filePath, dir)));
        } catch (e) {
          failures.push(`${baseName(filePath)}: ${String(e)}`);
        }
      }
      if (added.length > 0) {
        openedLogs = { dir, flights: [...(openedLogs?.flights ?? []), ...added] };
        activeTab = 'logbook';
        await loadLogbook();
        await selectFlight(added[0].id);
      } else if (!openedLogs) {
        void scratchClear(flightLogDbPath).catch(() => {});
      }
    } finally {
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
    if (failures.length > 0) {
      errorMsg = $t('logbook.importErrors', { values: { errors: failures.join('\n') } });
    }
  }

  /** Leave opened-file mode entirely: drop the player/selection, wipe the scratch dir, back to the
   *  main DB. */
  async function closeOpenedLog(reload = true) {
    if (selectedFlight != null) closePlayer();
    if (!openedLogs) return;
    openedLogs = null;
    await scratchClear(flightLogDbPath).catch(() => {});
    if (reload) await loadLogbook();
  }

  /** Dismiss one opened flight (no import): remove it from the scratch DB and the list; the last one
   *  going closes the set. */
  async function dismissOpenedFlight(id: number) {
    if (!openedLogs || blackboxImporting) return;
    if (selectedFlightId === id) closePlayer();
    try {
      await logbookCtrl.removeFlight(id, openedLogs.dir);
    } catch (e) {
      console.warn('[open-log] dismiss failed', e);
    }
    const remaining = openedLogs.flights.filter((f) => f.id !== id);
    if (remaining.length === 0) {
      await closeOpenedLog();
      return;
    }
    openedLogs = { dir: openedLogs.dir, flights: remaining };
    await loadLogbook();
    await selectFlight(remaining[0].id);
  }

  // Real imports out of the opened set. The standard import flows (performImport & co.) end in
  // afterRealImport(); these two flags tell it whether the import came from the opened set.
  let openedImportSingle: number | null = null;   // scratch flight id being imported on its own
  let openedImportBatch: number[] | null = null;  // main-DB ids collected during "Import Logs"

  /** Every successful real import lands here (instead of the plain reload+select). */
  async function afterRealImport(flightId: number) {
    if (openedImportBatch) {
      openedImportBatch.push(flightId); // the batch decides what to do once it is through
      return;
    }
    if (openedImportSingle != null && openedLogs) {
      // The single flight is in the main DB now → drop it from the opened set, stay in the set
      // while anything is left in it.
      const id = openedImportSingle;
      openedImportSingle = null;
      try {
        await logbookCtrl.removeFlight(id, openedLogs.dir);
      } catch (e) {
        console.warn('[open-log] remove imported scratch flight failed', e);
      }
      const remaining = openedLogs.flights.filter((f) => f.id !== id);
      if (remaining.length > 0) {
        openedLogs = { dir: openedLogs.dir, flights: remaining };
        if (selectedFlightId === id) closePlayer();
        await loadLogbook();
        await selectFlight(remaining[0].id);
        return;
      }
      await closeOpenedLog(false);
    }
    await loadLogbook();
    await selectFlight(flightId);
  }

  /** "Import" in the detail view: the standard import (duplicate check, linking dialog) of just the
   *  selected opened flight — its own log index for a flash dump. */
  async function importOpenedFlight(id: number) {
    if (!openedLogs || blackboxImporting) return;
    const entry = openedLogs.flights.find((f) => f.id === id);
    if (!entry) return;
    if (BLACKBOX_EXTS.test(entry.sourcePath) && !(await ensureBlackboxDecoder())) return;
    const ext = entry.sourcePath.split('.').pop()?.toLowerCase() ?? '';
    openedImportSingle = id;
    blackboxImporting = true;
    try {
      if (ext === 'bin') await performArdupilotImport(entry.sourcePath, false);
      else if (ext === 'ulg') await performUlogImport(entry.sourcePath, false);
      else if (ext === 'rawmsp' || ext === 'tlog') await performRawImport(entry.sourcePath);
      else await performImport(entry.sourcePath, entry.logIndex, false);
    } catch (e) {
      errorMsg = $t('logbook.importErrors', { values: { errors: `${baseName(entry.sourcePath)}: ${String(e)}` } });
    } finally {
      openedImportSingle = null; // a cancelled duplicate prompt leaves the opened set as it was
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
  }

  /** "Import Logs": the standard import of every opened file into the main DB — duplicate check,
   *  multi-log prompt and linking dialog all apply. Once at least one import succeeded the opened
   *  set is closed and the logbook switches to the main DB; if everything was cancelled it stays. */
  async function importOpenedLogs() {
    if (!openedLogs || blackboxImporting) return;
    const paths = [...new Set(openedLogs.flights.map((f) => f.sourcePath))];
    openedImportBatch = [];
    try {
      await importFiles(paths);
    } finally {
      const imported = openedImportBatch;
      openedImportBatch = null;
      if (imported && imported.length > 0) {
        await closeOpenedLog(false);
        await loadLogbook();
        await selectFlight(imported[imported.length - 1]);
      }
    }
  }

  async function openLogFileDialog() {
    if (!canOpenLogFile || blackboxImporting) return;
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: $t('logbook.allLogsFilter'), extensions: anyCase(OPEN_EXTS) }],
      });
      if (!selected) return;
      await openLogFiles(Array.isArray(selected) ? selected : [selected]);
    } catch (e) {
      errorMsg = String(e);
    }
  }

  /** Route one log file to the right importer by extension. Single source of truth for the
   *  one-button import and drag-drop. New formats (e.g. radio CSV later) just add a branch. */
  async function dispatchImport(filePath: string) {
    const ext = filePath.split('.').pop()?.toLowerCase() ?? '';
    if (ext === 'bin') {
      await performArdupilotImport(filePath, false); // ArduPilot DataFlash
    } else if (ext === 'ulg') {
      await performUlogImport(filePath, false); // PX4 ULog
    } else if (ext === 'kflight') {
      await performKflightImport(filePath); // KiteGC exchange file
    } else if (ext === 'rawmsp' || ext === 'tlog') {
      await performRawImport(filePath); // raw serial log (ADR-049)
    } else {
      await performBlackboxImport(filePath); // INAV Blackbox text (.txt/.bbl/.bfl)
    }
  }

  /** A Configurator flash download holds one log per arm/disarm cycle, and `blackbox_decode --stdout`
   *  refuses such a file without `--index`. Import every log as its own flight; a single-log file
   *  keeps passing no index at all. */
  async function performBlackboxImport(filePath: string) {
    const logCount = await logbookCtrl.countBlackboxLogs(filePath);
    if (logCount <= 1) {
      await performImport(filePath, undefined, false);
      return;
    }
    const answer = await showDialog({
      title: $t('logbook.multiLogTitle'),
      message: $t('logbook.multiLogMessage', { values: { count: logCount } }),
      buttons: [{ label: $t('logbook.multiLogImportAll'), value: 'all', primary: true }],
    });
    if (answer !== 'all') return;
    for (let index = 0; index < logCount; index++) {
      await performImport(filePath, index, false);
    }
  }

  async function performRawImport(filePath: string) {
    const result = await logbookCtrl.importRaw(filePath, flightLogDbPath);
    if (result.flightIds.length > 0) {
      await afterRealImport(result.flightIds[result.flightIds.length - 1]);
    } else {
      await loadLogbook();
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

  /** INAV blackbox text logs need the external `blackbox_decode`. If it's missing, offer to fetch it
   *  from GitHub (Windows auto-download) and report progress on the shared import progress bar.
   *  Returns true when the decoder is available afterwards. */
  async function ensureBlackboxDecoder(): Promise<boolean> {
    try {
      if (await blackboxDecoderAvailable()) return true;
    } catch {
      // Availability probe failed — fall through to the offer; the download surfaces a real error.
    }
    const answer = await showDialog({
      title: $t('logbook.decoderMissingTitle'),
      message: $t('logbook.decoderMissingMsg'),
      buttons: [{ label: $t('logbook.decoderDownload'), value: 'download', primary: true }],
    });
    if (answer !== 'download') return false;
    blackboxImporting = true; // drives the progress bar; the backend emits decoder-download progress
    try {
      await downloadBlackboxDecode();
      return true;
    } catch (e) {
      errorMsg = $t('logbook.decoderDownloadFailed', { values: { error: String(e) } });
      return false;
    } finally {
      blackboxImporting = false;
      blackboxImportProgress = null;
    }
  }

  /** The INAV blackbox formats — the only ones that need the external `blackbox_decode`. Everything
   *  else the importer accepts (.kflight archives, .rawmsp, .tlog, ArduPilot .bin, PX4 .ulg) is parsed
   *  in-process by the Rust backend and works on every platform. */
  const BLACKBOX_EXTS = /\.(txt|bbl|bfl)$/i;
  /** Extensions offered in the file picker / accepted from a drop. Mobile keeps what the device can
   *  produce itself or receive from a desktop — .kflight archives and the raw links (.rawmsp / .tlog)
   *  — and drops the rest: the three INAV blackbox formats need `blackbox_decode`, a separate native
   *  executable neither mobile OS allows to run (the backend refuses too — decoder_impossible), and
   *  ArduPilot dataflash (.bin) and PX4 ULog (.ulg) grow to tens or hundreds of megabytes for a tablet database that never
   *  archives originals (ANDROID_SUPPORT.md §4). Recording, replay and export are unaffected. */
  const IMPORT_EXTS = isMobile
    ? ['kflight', 'rawmsp', 'tlog']
    : ['txt', 'bbl', 'bfl', 'bin', 'ulg', 'kflight', 'rawmsp', 'tlog'];

  /** Import a batch of files, isolating each so one bad/corrupt/non-log file doesn't abort the rest;
   *  failures (with the per-importer reason) are collected and surfaced together. */
  async function importFiles(files: string[]) {
    // Second line of defence for mobile: the picker no longer offers blackbox formats, but a file can
    // still arrive by another route (a drop, a picker that ignores the filter). Say why rather than
    // letting it fail somewhere in the decoder lookup.
    if (isMobile) {
      const rejected = files.filter((f) => BLACKBOX_EXTS.test(f));
      files = files.filter((f) => !BLACKBOX_EXTS.test(f));
      if (rejected.length > 0) {
        errorMsg = $t('logbook.blackboxUnsupportedMobile', {
          values: { files: rejected.map(baseName).join(', ') },
        });
      }
      if (files.length === 0) return;
    }
    // INAV blackbox text logs (.txt/.bbl/.bfl) need blackbox_decode — ensure it once before the batch.
    if (files.some((f) => BLACKBOX_EXTS.test(f)) && !(await ensureBlackboxDecoder())) {
      return;
    }
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
            extensions: anyCase(IMPORT_EXTS),
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

    const supported = paths.filter((p) => new RegExp(`\\.(${IMPORT_EXTS.join('|')})$`, 'i').test(p));
    if (supported.length === 0) return;
    await importFiles(supported);
  }

  /** Import a mission file dropped anywhere over the app (the docs' "drag a mission file onto the
   *  map") — the ONLY working drop path is Tauri's native drag-drop event: with `dragDropEnabled`
   *  the WebView never sees DOM drop events, so element-scoped drop zones cannot exist. Routed by
   *  extension: .mission → INAV, .plan/.waypoints → the ArduPilot/PX4 stack. `.txt` stays with the
   *  logbook (SD blackbox logs use it too). If the file belongs to the other mission family, the
   *  editor switches — with 'keep': INAV and Ardu keep separate stores, so nothing is lost — unless
   *  a connected FC locks the system, which reports instead of silently importing into a hidden
   *  store. */
  async function importDroppedMission(path: string): Promise<void> {
    console.log('[IMPORT] mission file dropped:', path);
    try {
      const content = await invoke<string>('read_text_file', { path });
      if (/\.mission$/i.test(path)) {
        if (get(autopilotSystem) !== 'inav') {
          if (get(autopilotLocked)) { errorMsg = $t('missionMgr.systemLocked'); return; }
          setAutopilotSystem('inav');
          if (get(pendingSystemSwitch)) confirmSystemSwitch('keep');
          if (get(autopilotSystem) !== 'inav') return;
        }
        await missionImportXml(content);
      } else {
        const isPlan = /\.plan$/i.test(path);
        const wps = isPlan ? parsePlanFile(content) : parseWaypoints(content);
        if (get(autopilotSystem) === 'inav') {
          if (get(autopilotLocked)) { errorMsg = $t('missionMgr.systemLocked'); return; }
          setAutopilotSystem(isPlan ? planFirmwareTarget(content) : 'ardupilot');
          if (get(pendingSystemSwitch)) confirmSystemSwitch('keep');
          if (get(autopilotSystem) === 'inav') return;
        }
        loadArduMissionFromFile(wps);
      }
      frameMissionOnMap();
    } catch (e) {
      errorMsg = $t('mission.importFailed', { values: { error: String(e) } });
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
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function exportBlackbox() {
    if (!selectedFlightId || !selectedFlight) return;
    const src = selectedFlight.source;
    if (src !== 'blackbox' && src !== 'both') return;
    try {
      // Keep the stored original file's extension (INAV .txt/.bbl, ArduPilot .bin, .tlog, raw-MSP, …)
      // and build a descriptive default name: <craft>_<date>_<flightId>.<ext>. The user can rename in
      // the dialog. (Previously hardcoded to blackbox_flight_<id>.TXT — wrong extension for non-INAV.)
      const orig = blackboxFileInfo?.filename ?? '';
      const dot = orig.lastIndexOf('.');
      const ext = (dot > 0 ? orig.slice(dot + 1) : 'log').toLowerCase();
      const craft = (selectedFlight.craft_name || 'flight')
        .trim().replace(/[^A-Za-z0-9._-]+/g, '_').replace(/^_+|_+$/g, '') || 'flight';
      const date = (selectedFlight.start_time ?? '').slice(0, 10); // YYYY-MM-DD
      const base = date ? `${craft}_${date}_${selectedFlightId}` : `${craft}_${selectedFlightId}`;
      const outputPath = await save({
        filters: [{ name: $t('logbook.blackboxFileFilter'), extensions: [ext] }],
        defaultPath: `${base}.${ext}`,
      });
      if (!outputPath) return;
      await logbookCtrl.exportBlackbox(selectedFlightId, outputPath, activeDbPath);
      const savedName = outputPath.split(/[\\/]/).pop() ?? `${base}.${ext}`;
      await showInfo($t('logbook.exportBlackboxTitle'), $t('logbook.exportBlackboxSuccess', { values: { filename: savedName } }));
    } catch (e) {
      errorMsg = String(e);
    }
  }

  // The selected flight's stored original blackbox file (filename + size), or null. Accurate BLOB
  // presence (not just the source proxy — goes null after the file is deleted). Gates export + delete
  // and supplies the size shown inline next to the Source line.
  let blackboxFileInfo = $state<logbookCtrl.BlackboxFileInfo | null>(null);
  $effect(() => {
    const id = selectedFlightId;
    const src = selectedFlight?.source;
    if (id && (src === 'blackbox' || src === 'both')) {
      logbookCtrl
        .getBlackboxInfo(id, activeDbPath)
        .then((v) => (blackboxFileInfo = v))
        .catch(() => (blackboxFileInfo = null));
    } else {
      blackboxFileInfo = null;
    }
  });

  async function deleteBlackbox() {
    if (!selectedFlightId || !selectedFlight || !blackboxFileInfo) return;
    const value = await showDialog({
      title: $t('logbook.deleteBlackboxTitle'),
      message: $t('logbook.deleteBlackboxWarning'),
      buttons: [{ label: $t('logbook.deleteBlackboxConfirm'), value: 'delete', danger: true }],
    });
    if (value !== 'delete' || !selectedFlightId) return;
    try {
      const filename = await logbookCtrl.deleteBlackbox(selectedFlightId, flightLogDbPath);
      blackboxFileInfo = null;
      await showInfo(
        $t('logbook.deleteBlackboxTitle'),
        $t('logbook.deleteBlackboxSuccess', { values: { filename: filename ?? '' } }),
      );
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function compactDb() {
    try {
      const sizeBytes = await logbookCtrl.compactDb(flightLogDbPath);
      const mb = (sizeBytes / (1024 * 1024)).toFixed(1);
      await showInfo($t('settings.compactDb'), $t('settings.compactDbDone', { values: { size: `${mb} MB` } }));
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
      await logbookCtrl.exportTrack(selectedFlightId, outputPath, activeDbPath);
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
      await afterRealImport(result.flight_id);
    }
  }

  /** Shared import flow for the self-describing binary flash logs (ArduPilot .bin, PX4 .ulg):
   *  duplicate-confirm -> force re-run, then offer linking against a live recording. */
  async function performBinaryLogImport(
    filePath: string,
    forceImport: boolean,
    importer: (fp: string, force: boolean) => Promise<BlackboxImportStatus>,
  ) {
    const result = await importer(filePath, forceImport);
    
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
        await performBinaryLogImport(filePath, true, importer);
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
      await afterRealImport(result.flight_id);
    }
  }

  async function performArdupilotImport(filePath: string, forceImport: boolean) {
    await performBinaryLogImport(filePath, forceImport, (fp, force) =>
      logbookCtrl.importArdupilot(fp, flightLogDbPath, force, $locale ?? 'en'));
  }

  async function performUlogImport(filePath: string, forceImport: boolean) {
    await performBinaryLogImport(filePath, forceImport, (fp, force) =>
      logbookCtrl.importUlog(fp, flightLogDbPath, force, $locale ?? 'en'));
  }

  async function selectFlight(flightId: number) {
    selectedFlightId = flightId;
    const data = await logbookCtrl.selectFlightData(flightId, activeDbPath, $locale ?? 'en');
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
    resetHires(true); // a different flight's cache is stale — drop it

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
      const linked = await missionDbForFlight(flightId, activeDbPath);
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
        replayWpTotal.set(await flightLoggedWpCount(flightId, activeDbPath));
      }
    } catch {
      replayWpTotal.set(null);
    }

    // Pre-load linked partner track for source switching
    if (data.flight?.linked_flight_id) {
      const partnerTrack = await logbookCtrl.getPartnerTrack(data.flight.linked_flight_id, activeDbPath);
      linkedPartnerTrack = partnerTrack;
    }

    // Hi-res availability: an archived original log in a parseable format (partner fallback
    // included, so a linked REC flight still finds the BBX blob).
    try {
      const info = await hiresInfo(flightId, activeDbPath);
      hiresAvailable = info.available;
      hiresCachePath = info.cache_path;
      if (info.cache_path) { hiresOwnerFlightId = flightId; hiresOwnerDbPath = activeDbPath; }
      // The cache is roughly 5–6× the archived log (measured on real INAV blackbox CSV decodes).
      hiresEstimateBytes = info.blob_size_bytes != null ? info.blob_size_bytes * 6 : null;
    } catch (e) {
      console.warn('[hires] info failed', e);
      hiresAvailable = false;
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
    // Hi-res rows share the blackbox track's clock — on the live track they would be misaligned,
    // so switching to REC turns hi-res off (the cache file stays for a later re-enable).
    if (source === 'live' && hiresActive) {
      hiresActive = false;
      hiresSamplePoint = null;
    }
    resetPlayback();
    if (activeReplayTrack.length > 0) playbackActive = true;
  }

  // ── Hi-res replay (Dev-Docs active/HIRES_REPLAY.md) ──────────────────────────────────────

  /** Deactivate hi-res; `drop = true` also deletes the cache file (deselect/close). */
  function resetHires(drop: boolean) {
    hiresActive = false;
    hiresSamplePoint = null;
    hiresVirtualMs = null;
    if (drop) {
      if (hiresOwnerFlightId != null) void hiresDrop(hiresOwnerFlightId, hiresOwnerDbPath);
      hiresOwnerFlightId = null;
      hiresCachePath = null;
      hiresAvailable = false;
      hiresEstimateBytes = null;
    }
  }

  async function toggleHires(active: boolean) {
    if (!active) {
      // Toggle off: back to the 10 Hz samples; the cache file stays for an instant re-enable.
      if (!hiresActive) return;
      hiresActive = false;
      hiresSamplePoint = null;
      if (playbackPlaying) { stopPlayback(); startPlayback(); } // leave the rAF clock
      return;
    }
    if (hiresActive || hiresParsing || selectedFlightId == null) return;
    if (!hiresCachePath) {
      const fid = selectedFlightId; // guard against a flight switch while the parse runs
      const fidDb = activeDbPath;
      hiresParsing = true;
      hiresProgress = null;
      try {
        const out = await hiresParse(fid, fidDb);
        if (fid !== selectedFlightId) {
          void hiresDrop(fid, fidDb); // stale — the user moved on mid-parse
          return;
        }
        hiresCachePath = out.cache_path;
        hiresOwnerFlightId = fid;
        hiresOwnerDbPath = fidDb;
        console.log(`[hires] cache ready: ${out.rows} rows @ ${out.rate_hz.toFixed(0)} Hz, ${out.size_bytes} bytes`);
      } catch (e) {
        console.warn('[hires] parse failed', e);
        void showInfo($t('player.hiresFailedTitle'), String(e));
        return;
      } finally {
        hiresParsing = false;
        hiresProgress = null;
      }
    }
    hiresActive = true;
    if (playbackPlaying) { stopPlayback(); startPlayback(); } // switch the clock to rAF
  }

  // Per-tick sampler: pull the hi-res row nearest the playhead. Serialized — one IPC call in
  // flight, the latest requested timestamp wins (a slow query never queues up a backlog).
  let hiresFetchBusy = false;
  let hiresPendingTs: number | null = null;
  async function fetchHiresSample(ts: number) {
    if (hiresFetchBusy) {
      hiresPendingTs = ts;
      return;
    }
    hiresFetchBusy = true;
    try {
      const path = hiresCachePath;
      if (path) {
        const rec = await hiresSample(path, Math.round(ts));
        if (hiresActive) hiresSamplePoint = rec;
      }
    } catch (e) {
      console.warn('[hires] sample failed', e);
    } finally {
      hiresFetchBusy = false;
      if (hiresPendingTs != null) {
        const next = hiresPendingTs;
        hiresPendingTs = null;
        void fetchHiresSample(next);
      }
    }
  }

  // While hi-res is on: sample at the continuous clock when playing, at the scrub/seek position
  // otherwise. The write goes to hiresSamplePoint (not read here), so no effect self-loop.
  $effect(() => {
    if (!hiresActive) return;
    const ts = playbackPlaying ? hiresVirtualMs : playbackPoint?.timestamp_ms;
    if (ts != null) void fetchHiresSample(ts);
  });

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

  // BLE discovery is a firehose: the backend scan puts the adapter into CONTINUOUS discovery, and
  // BlueZ emits a D-Bus signal per advertisement (hundreds/s in a busy RF area) for as long as it
  // runs — pegging dbus-daemon + bluetoothd, not Kite itself. So we never hold it open: each scan is
  // a short bounded WINDOW. One fires on entering BLE (list ready), and one each time the device
  // dropdown is opened (fresh RSSI) via onRescanBle. Devices stream in through the `ble-device` event
  // (listener set up in initPage) during the window.
  let bleScanTimer: ReturnType<typeof setTimeout> | undefined;
  const BLE_SCAN_WINDOW_MS = 6000;
  function bleScanWindow() {
    if (selectedTransport !== 'ble' || connStatus === 'connected' || connStatus === 'connecting') return;
    clearTimeout(bleScanTimer);
    isBleScanning = true;
    void startBleScan(); // backend restarts any running session
    bleScanTimer = setTimeout(() => { isBleScanning = false; void stopBleScan(); }, BLE_SCAN_WINDOW_MS);
  }
  $effect(() => {
    if (selectedTransport !== 'ble' || connStatus === 'connected' || connStatus === 'connecting') return;
    untrack(() => { clearBleDevices(); bleScanWindow(); });
    return () => {
      clearTimeout(bleScanTimer);
      isBleScanning = false;
      void stopBleScan();
    };
  });

  function onVisibilityChange(): void {
    if (document.hidden) return;
    void resyncTrack();
    if (lostWhileHidden) {
      lostWhileHidden = false;
      if (connStatus === 'disconnected' && !isConnecting) void handleConnect();
    }
  }

  /** Pull the track points the backend buffered since our last one and merge them into the map
   *  trail and `liveTrack` — a no-op when nothing was missed. */
  async function resyncTrack(): Promise<void> {
    if (connStatus !== 'connected') return;
    const cur = get(liveTrack);
    const sinceMs = cur.length > 0 ? cur[cur.length - 1].timestamp_ms : 0;
    try {
      const res = await invoke<{ flight_start_ms: number; points: { lat: number; lon: number; alt_msl: number; mode: string; ts_ms: number }[] }>(
        'telemetry_track_since', { sinceMs },
      );
      if (res.points.length === 0) return;
      console.log(`[track] backfilled ${res.points.length} points after the page was hidden`);
      backfillLivePoints(
        res.points.map((p) => ({ lat: p.lat, lon: p.lon, alt_m: p.alt_msl, mode_primary: p.mode, timestamp_ms: p.ts_ms })),
        res.flight_start_ms,
      );
      mapRef?.appendTrailPoints?.(res.points.map((p) => ({ lat: p.lat, lon: p.lon, mode: p.mode })));
    } catch (e) {
      console.warn('[track] backfill failed', e);
    }
  }

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
        windEnabled,
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

  function handleResize(widgetId: string) {
    panels = widgetCtrl.cycleWidgetSize(panels, widgetId);
    settings.patch({ panels });
  }

  // ── Phone widget grid (Dev-Docs active/PHONE_UI.md D13) — its own config, its own rules ──
  const phoneWidgets = $derived($settings.phoneWidgets);
  function patchPhoneWidgets(next: PhoneWidgetsConfig) {
    if (next !== $settings.phoneWidgets) settings.patch({ phoneWidgets: next });
  }

  function toggleWidget(widgetId: string) {
    if (phoneUi) {
      const next = phoneCtrl.togglePhoneWidget(phoneWidgets, widgetId);
      if (next === null) {
        void showInfo($t('widgets.phoneNoSpaceTitle'), $t('widgets.phoneNoSpace'));
        return;
      }
      patchPhoneWidgets(next);
      return;
    }
    panels = widgetCtrl.toggleWidgetVisibility(panels, widgetId);
    settings.patch({ panels });
  }

  function isWidgetActive(widgetId: string): boolean {
    if (phoneUi) return phoneCtrl.isPhoneWidgetActive(phoneWidgets, widgetId);
    return widgetCtrl.isWidgetActive(panels, widgetId);
  }

  function getWidgetPanelLabel(widgetId: string): string {
    if (phoneUi) {
      const page = phoneCtrl.phoneWidgetPage(phoneWidgets, widgetId);
      return page == null ? $t('widgets.off') : $t('widgets.phonePage', { values: { n: page + 1 } });
    }
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
  // Blackbox-replay position for the video-backdrop map — a replayed model is a valid UAV
  // position, so the backdrop follows it exactly like the mini map does. Null outside a
  // replay or when the record carries no usable coordinates (the track is already
  // GPS-filtered above, so the null checks are belt-and-braces).
  const ufReplayPos = $derived(
    playbackPoint != null && playbackPoint.lat != null && playbackPoint.lon != null
      ? { lat: playbackPoint.lat, lon: playbackPoint.lon }
      : null
  );
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

  // Recording protocol of the selected flight — the 3D map needs it to tell a true-MSL track from an
  // arming-relative one (CRSF/LTM), which decides how its altitude anchor is derived.
  const replayProtocol = $derived((selectedFlight as Flight | null)?.protocol ?? null);

  // Absolute flight-start epoch (ms) — telemetry timestamp_ms is flight-relative, so the
  // 3D sky clock needs this origin to reconstruct the real instant for sun positioning.
  const replayStartEpochMs = $derived.by(() => {
    const s = (selectedFlight as Flight | null)?.start_time;
    if (!s) return null;
    const t = new Date(s).getTime();
    return Number.isFinite(t) ? t : null;
  });

  // Hi-res only aligns with the blackbox track's clock — a linked pair must have BBX selected,
  // a blackbox-only flight always replays its own track (HIRES_REPLAY plan).
  const hiresAllowed = $derived(
    hiresAvailable &&
      (replaySource === 'blackbox' || (selectedFlight as Flight | null)?.source === 'blackbox'),
  );

  // Unified telemetry: live data when connected, playback data when replaying. While hi-res is
  // active the full-rate sample overrides the 10 Hz row (same shape, same clock, denser rows).
  const telem = $derived(
    playbackActive && !isPrimaryConnected && playbackPoint
      ? toTelemetryData(hiresActive && hiresSamplePoint ? hiresSamplePoint : playbackPoint, replayFcVariant)
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
      if (res.batterySerial) {
        await flightSetBatterySerial(flightId, res.batterySerial, flightLogDbPath);
        // A flight may link several packs (comma-separated). Offer to create each unknown serial; on the
        // first "create" we open the Manager pre-filled for that pack (the rest can be added there).
        const serials = res.batterySerial.split(',').map((s) => normalizeSerial(s)).filter(Boolean);
        for (const serial of serials) {
          const existing = await batteryDbFindBySerial(serial, flightLogDbPath).catch(() => null);
          if (existing) continue;
          const choice = await showDialog({
            title: $t('endFlight.newBatteryTitle'),
            message: $t('endFlight.newBatteryMessage', { values: { serial } }),
            buttons: [
              { label: $t('endFlight.newBatteryCreate'), value: 'create' },
              { label: $t('endFlight.newBatterySkip'), value: 'skip' },
            ],
          });
          if (choice === 'create') {
            activeTab = 'logbook';
            settings.patch({ activeTab: 'logbook' });
            batteryManagerOpen.set(true);
            batteryManagerCreateSerial.set(serial);
            break;
          }
        }
      }
      // Unknown craft name → offer to create a vehicle (opens the Vehicle Manager create form
      // pre-filled). The FC's craft name is the one just recorded for this flight.
      const craft = (fcInfo?.craft_name ?? '').trim();
      if (craft) {
        const veh = await vehicleDbFindByCraftName(craft, flightLogDbPath).catch(() => null);
        if (!veh) {
          const choice = await showDialog({
            title: $t('endFlight.newVehicleTitle'),
            message: $t('endFlight.newVehicleMessage', { values: { craft } }),
            buttons: [
              { label: $t('endFlight.newVehicleCreate'), value: 'create' },
              { label: $t('endFlight.newVehicleSkip'), value: 'skip' },
            ],
          });
          if (choice === 'create') {
            activeTab = 'logbook';
            settings.patch({ activeTab: 'logbook' });
            vehicleManagerOpen.set(true);
            vehicleManagerCreateCraft.set(craft);
          }
        }
      }
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
    // Hi-res caches and the opened-file scratch store left by a crash are worthless (always
    // reproducible) — wipe them in passing.
    void hiresCleanup(flightLogDbPath).catch(() => {});
    void scratchClear(flightLogDbPath).catch(() => {});
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

  // The Windows main window starts hidden (`visible: false` in tauri.windows.conf.json): it is
  // `transparent: true` for the native-video hole punch, so before the WebView's first paint the
  // whole app area was see-through to the desktop with only the frame visible. Show it once the
  // UI is actually mounted. No-op on platforms whose window starts visible.
  onMount(() => {
    void import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      const win = getCurrentWebviewWindow();
      void win.show().then(() => win.setFocus()).catch(() => {});
    });
  });

  // Keep the low-power state resolved for the whole app lifetime: the store mirrors it onto a root
  // class that CSS gates the expensive widget-bar transitions off (see stores/lowPower.ts). It is a
  // readable store, so it only runs while something subscribes — this is that subscription.
  onMount(() => lowPowerActive.subscribe(() => {}));

  // Hard-blink indicator mode on WebKitGTK and Android — see stores/pulseBlink.ts for why a looping
  // CSS animation costs a large fraction of a core there (per-frame fixed cost, not per pixel).
  // No-op elsewhere.
  onMount(() => initPulseBlink());

  // Startup update check (GitHub releases). Deferred a few seconds so it never competes with launch work;
  // failures are swallowed inside the controller — it must never disrupt the app.
  onMount(() => {
    const id = setTimeout(() => { void runUpdateCheck(); }, 4000);
    return () => clearTimeout(id);
  });

  // External links (e.g. the Leaflet/OSM map attribution) must open in the system browser — the
  // webview has no tabs/back, so navigating it to an external page traps the user with no way out.
  // Intercept clicks on absolute http(s) anchors (cross-origin) and hand them to the OS browser.
  onMount(() => {
    const onClick = (e: MouseEvent) => {
      const a = (e.target as HTMLElement | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
      if (!a) return;
      const href = a.getAttribute('href') ?? '';
      if (/^https?:\/\//i.test(href) && !href.startsWith(location.origin)) {
        e.preventDefault();
        void openUrl(href).catch(() => {});
      }
    };
    document.addEventListener('click', onClick, true);
    return () => document.removeEventListener('click', onClick, true);
  });

  // Jump to a flight in the Logbook when requested (e.g. from the Mission Manager's
  // "flights with this mission" list).
  $effect(() => {
    const id = $requestOpenFlightId;
    if (id == null) return;
    requestOpenFlightId.set(null);
    activeTab = 'logbook';
    batteryManagerOpen.set(false); // leave the Battery Manager so the flight detail is shown
    vehicleManagerOpen.set(false); // leave the Vehicle Manager too (same reason)
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
    void listen<{ duration_sec: number; max_alt_m: number; max_speed_ms: number; max_distance_m: number; total_distance_m: number; battery_used_mah: number | null }>(
      'flight-recording-ended',
      (event) => {
        const p = event.payload;
        void onRecordingEnded({
          durationSec: p.duration_sec,
          maxAltM: p.max_alt_m,
          maxSpeedMs: p.max_speed_ms,
          maxDistM: p.max_distance_m,
          totalDistM: p.total_distance_m,
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
      (event) => { lostWhileHidden = false; void onRecordingInterrupted(event.payload); },
    );
    // The device vanished (fatal transport error) — the backend tore the scheduler down. Clean up the
    // connection state so the UI shows disconnected and the user can simply reconnect.
    void listen('connection-lost', () => {
      if (document.hidden) lostWhileHidden = true;
      void disconnectFC(selectedBaud).catch(() => {});
    });
    // Back in front: close the trail gap from the backend's buffer, reconnect once if the link was
    // lost meanwhile (BACKGROUND_TELEMETRY.md).
    document.addEventListener('visibilitychange', onVisibilityChange);
    void listen<BlackboxImportProgress>('flightlog-import-progress', (event) => {
      blackboxImportProgress = event.payload;
    });
    // Hi-res replay parse (own event — it must not fight the import bar over shared state).
    void listen<BlackboxImportProgress>('flightlog-hires-progress', (event) => {
      hiresProgress = event.payload;
    });
    void listen<{ paths: string[]; position?: { x: number; y: number } }>('tauri://drag-drop', (event) => {
      const paths = event.payload.paths ?? [];
      if (!paths.length) return;
      // Mission files import from anywhere (see importDroppedMission). `.txt` deliberately stays on
      // the logbook side (SD blackbox collision).
      const missionFile = paths.find((p) => /\.(mission|plan|waypoints)$/i.test(p));
      if (missionFile) {
        void importDroppedMission(missionFile);
        return;
      }
      // Log files: dropped ON the Logbook panel → import as before. Dropped anywhere else (the map)
      // → open for replay without importing (desktop, disconnected). The native event carries the
      // drop position in physical pixels; hit-test it against the panel root. Without a position
      // (older payloads) the old behaviour stands: logbook tab open = import.
      const pos = event.payload.position;
      let overLogbook = activeTab === 'logbook';
      if (overLogbook && pos) {
        const dpr = window.devicePixelRatio || 1;
        overLogbook = document.elementFromPoint(pos.x / dpr, pos.y / dpr)?.closest('.lbv2') != null;
      }
      if (overLogbook) {
        importDroppedFiles(paths);
        return;
      }
      if (!canOpenLogFile) return;
      if (paths.some(isOpenableLog)) void openLogFiles(paths);
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
    // FC-confirmed Guided target (POSITION_TARGET_GLOBAL_INT, 1 Hz) — the controller gates it on the
    // FC actually being in its guided mode (in AUTO/RTL the same message carries the mission WP/home).
    void listen<{ lat: number; lon: number; alt: number }>('guided-target', (event) => {
      ingestFcGuidedTarget(event.payload.lat, event.payload.lon);
    });
  }

  onDestroy(() => {
    playbackCtrl.destroy();
    stopBleDeviceListener();
    void stopBleScan();
  });
</script>

<svelte:window bind:innerWidth={winW} bind:innerHeight={winH} />

<div
  class="ui-root"
  style:--ui-scale={uiScale}
  style:--toolbar-h="{toolbarH}px"
  style:--phone-panel-w="{phonePanelW}px"
  style:--phone-shift="{phoneShift}px"
>
  <!-- Window resize grips — outside `.ui-scale` so position:fixed stays viewport-relative.
       Re-adds edge resizing lost when the native decorations are disabled. Desktop only: a mobile
       build fills the screen and `startResizeDragging` has nothing to resize. -->
  {#if !isMobile}
    <WindowResizeBorders />
  {/if}

  <!-- ======= MAP LAYER — unzoomed / native resolution (see docs/archive/UI_SCALING.md) =======
       The map must stay crisp, so it lives OUTSIDE the zoomed `.ui-scale` layer. It is the
       same single Map/Map3D instance (no re-mount). Normally it sits behind the chrome; when
       video is primary it flips above the chrome into the floating window's body (.in-frame). -->
  <!-- Phone: the swapped-in mini map is clipped to the box of the frame it sits in — the map area
       (docked frame: it parks BEHIND the widget column like the video does) or the column's tile
       area (widget: it scrolls out with the page instead of hanging over the glass). Desktop: a
       transparent, non-clipping wrapper. Same coordinate system as before (inset 0). -->
  <div
    class="map-clip"
    class:clip-map-area={phoneUi && mapFloating}
    class:clip-column={phoneUi && mapInWidget}
    style:--phone-pad="{PHONE_GRID_PAD}px"
  >
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="layer-map"
    class:in-frame={mapInFrame}
    class:parked={phoneUi && mapFloating && !$videoState.floating}
    data-nv-clip={mapInFrame ? undefined : true}
    style={mapInFrame ? inFrameStyle : mapLayerStyle}
    onclick={minimizeLogbook}
  >
    <!-- 2D stays mounted (hidden) exactly like 3D below: the live track lives in Leaflet layers
         inside the component, so unmounting it on every switch to 3D threw the trail away. -->
    <div class="map2d-layer" class:active={mapViewMode === '2d'}>
      <Map
        bind:this={mapRef}
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
        miniControls={miniMapLocked}
        centerInsetRight={phoneMapInset}
        radarActive={radarSettings.enabled}
        radarMapSettings={radarSettings.map}
        {radarReference}
        {radarRefAltM}
      />
    </div>
    <!-- 3D stays mounted (hidden) once opened, so toggling back is instant. -->
    {#if map3dEverOpened}
      <div class="map3d-layer" class:active={mapViewMode === '3d'}>
        <Map3D
          centerInsetRight={phoneMapInset}
          bind:this={map3dRef}
          active={mapViewMode === '3d'}
          playbackTrack={mapTrack}
          playbackPoint={playbackPoint}
          playbackProtocol={replayProtocol}
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

  </div>
  </div><!-- .map-clip -->

  <!-- Toasts & alerts pinned to the MAIN APP FRAME (not the map): the map can shrink into the
       floating window or a widget tile, and map-bound banners would then cover that tiny tile. This
       container sits above everything in the content area so alerts stay readable. -->
  <!-- --toast-dock-inset = the open left-docked panel's right edge (0 when none); the system-message
       toasts read it to centre in the free area beside the panel (issue #10). The radar banner ignores
       it and stays frame-centred + on top. -->
  <div class="app-toasts" style:--toast-dock-inset="{panelHidden ? 0 : $panelDockRight}px">
    <!-- Conflict-alert banner (renders nothing when idle). -->
    <RadarAlertBanner {interfaceSettings} />
    <!-- FC system messages (MAVLink STATUSTEXT) as top-edge toasts (renders nothing when idle). -->
    <StatusTextToasts />
  </div>

  <!-- Floating-frame map controls — top-level/unzoomed so they sit ABOVE the in-frame map (z2); the
       float-win's own corners live in .ui-scale (z1) and would be hidden behind it. Only for the
       floating frame (resizable); the widget tile is sized by the dock. ✕ sends the map back to main. -->
  {#if mapFloating && $videoState.status === 'live' && !phoneUi}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="miniframe-ctl" style={mapFrameStyle}>
      <button class="mf-corner mf-close" onclick={() => setMapLocation('main')} title={$t('video.close')}>✕</button>
      <div class="mf-corner mf-resize" onpointerdown={miniResizeDown} title="Resize"></div>
    </div>
  {/if}

  <!-- ======= UI CHROME LAYER — zoomed by --ui-scale ======= -->
  <div class="ui-scale">

<!-- Dialogs render in the panels layer below — as .ui-scale children their z-index could
     never beat the in-frame mini-map either (same stacking-context wall as the panels). -->

{#if awaitingResumeReconnect}
  <div class="resume-banner">{$t('recovery.waitingBanner')}</div>
{/if}

<main
  class="app"
  class:rc-sticks-active={isMobile && activeTab === 'rc-control'}
  style:--grid-bottom-height={gridBottomHeight}
  style:--grid-side-width={gridSideWidth}
  style:--panel-bottom-reserve={panelBottomReserve}
>
  {#if phoneUi}
  <!-- ======= PHONE CHROME (Dev-Docs active/PHONE_UI.md) ======= -->
  <div class="phone-conn">
    <ConnectionPopout
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
      onRescanBle={bleScanWindow}
    />
  </div>
  <!-- The column's glass layer (inside PhoneWidgetPanel) carries data-nv-clip for the video
       widget's hole — not this zone: a clip on the hit-testable root would let touches fall through. -->
  <div class="zone-phone-widgets">
    <PhoneWidgetPanel
      config={phoneWidgets}
      {telem}
      {interfaceSettings}
      onresize={(id) => patchPhoneWidgets(phoneCtrl.cyclePhoneWidgetSize(phoneWidgets, id))}
      onmove={(id, page, row, col) => patchPhoneWidgets(phoneCtrl.movePhoneWidget(phoneWidgets, id, page, row, col))}
      bind:widthPx={phonePanelW}
    />
  </div>
  <div class="phone-bottom-chips">
    <PhoneBottomChips {telem} />
  </div>
  <!-- Docked video window + its toggle (PHONE_VIDEO.md) — the phone's floating window. -->
  <PhoneVideoDock
    left={dockLeft}
    top={dockTop}
    width={dockW}
    height={dockH}
    widgetActive={phoneCtrl.isPhoneWidgetActive(phoneWidgets, 'videoFeed')}
  />
  {:else}
  <!-- ======= TOOLBAR ======= -->
  <div class="zone-toolbar" bind:clientHeight={toolbarH}>
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
    onOpenRaw={() => (rawTelemetryOpen = true)}
    onOpenRc={() => selectTab('rc-control')}
    onRescanBle={bleScanWindow}
  />
    <RelayPanel open={relayPanelOpen} />
  </div>
  {/if}

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- Full-size video shown in the main map zone whenever the map has jumped to another surface.
       Double-click brings the map back to the main full-screen view. -->
  {#if mapInFrame}
    <!-- Wrapper carries the inset + black backdrop; the video fills it with object-fit: contain so
         it scales to the window (full height/width) without distortion — bars where aspect differs. -->
    <div
      class="map-video-wrap"
      class:nv-active={$activeNativeSurface === 'main'}
      class:unobstructed={ufActive}
      bind:clientWidth={ufWrapW}
      bind:clientHeight={ufWrapH}
    >
      {#if ufActive}
        <!-- Thematic backdrop: a blurred, bare second map following the UAV — fills the area
             around the video box. Carries data-nv-clip, so the native-sink hole is cut into
             it like into the main map layer. -->
        <VideoBackdropMap replayPos={ufReplayPos} />
      {/if}
      <!-- Aspect-exact inner box: in unobstructed mode this is cut to the stream's aspect and
           centred (no letterbox bars — see ufBox); otherwise it just fills the wrapper. -->
      <div
        class="map-video-box"
        style={ufBox ? `left:${ufBox.left}px;top:${ufBox.top}px;width:${ufBox.w}px;height:${ufBox.h}px;` : ''}
      >
      {#if $videoState.nativeSink}
        <!-- Native decode sink (hole punch): the video is a hardware layer BELOW the WebView; this
             div is the transparent hole it shows through. Highest surface priority — full-screen
             video beats every other surface. See controllers/nativeVideo. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="native-hole"
          class:armed={$activeNativeSurface === 'main'}
          use:nativeSurface={'main'}
          ondblclick={mouseDoubleClick(() => setMapLocation('main'))}
          use:doubleTap={() => setMapLocation('main')}
        >
          {#if $activeNativeSurface !== 'main'}<span>{$t('video.sinkElsewhere')}</span>{/if}
        </div>
      {:else if $videoState.mjpegUrl}
        <!-- Native / MJPEG feed (no MediaStream): drawn by the off-thread reader where the WebView
             allows it, otherwise the plain <img> multipart stream. -->
        {#if $canvasSink}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <canvas
            class="map-video"
            class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}
            use:mjpegSink
            ondblclick={mouseDoubleClick(() => setMapLocation('main'))}
          use:doubleTap={() => setMapLocation('main')}
          ></canvas>
        {:else}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <!-- svelte-ignore a11y_missing_attribute -->
          <img
            class="map-video"
            class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}
            src={$videoState.mjpegUrl}
            ondblclick={mouseDoubleClick(() => setMapLocation('main'))}
          use:doubleTap={() => setMapLocation('main')}
            onerror={reportMjpegError}
          />
        {/if}
      {:else}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          class="map-video"
          class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}
          bind:this={mapVideoEl}
          autoplay
          muted
          playsinline
          ondblclick={mouseDoubleClick(() => setMapLocation('main'))}
          use:doubleTap={() => setMapLocation('main')}
        ></video>
      {/if}
      </div>
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
    onExpandedChange={(v) => { playerExpanded = v; }}
    {modelOverride}
    onModelOverrideChange={(v) => { modelOverride = v; }}
    playbackTrack={mapTrack}
    {warnAltitudeM}
    {replaySource}
    hasLinkedPartner={selectedFlight?.linked_flight_id != null && linkedPartnerTrack.length > 0}
    onSwitchSource={switchReplaySource}
    hiresAvailable={hiresAllowed}
    {hiresActive}
    {hiresParsing}
    onHiresToggle={(active) => { void toggleHires(active); }}
    hiresRecord={hiresActive ? hiresSamplePoint : null}
  />

  {#if hiresParsing}
    <HiresParseModal progress={hiresProgress} estimateBytes={hiresEstimateBytes} />
  {/if}
  {#if rawTelemetryOpen && connStatus === 'connected'}
    <RawTelemetryModal {telem} onclose={() => (rawTelemetryOpen = false)} />
  {/if}

  <!-- ======= FLOATING NAV PANEL SYSTEM ======= -->
  <!-- The rail lives here in .app; the panels themselves render in the panels layer AFTER
       .ui-scale so they stack above the in-frame mini-map — see that layer's comment. -->
  <NavRail
    open={navPanelOpen}
    activeTab={railActiveTab}
    activeHidden={panelHidden}
    tabs={railTabs}
    onToggle={toggleNavPanel}
    onSelectTab={selectTab}
  />

  {#if !phoneUi}
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
        smallBoost={isPhone ? 1.5 : isTablet ? 1.4 : 1}
        sizes={panels.sizes ?? {}}
        bind:crossPx={bottomPanelCrossPx}
        {telem}
        editing={widgetEditMode}
        {interfaceSettings}
        onreorder={handleReorder}
        onreceive={handleReceive}
        onresize={handleResize}
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
      smallBoost={isTablet ? 1.4 : 1}
      sizes={panels.sizes ?? {}}
      bind:crossPx={sidePanelCrossPx}
      {telem}
      editing={widgetEditMode}
      {interfaceSettings}
      onreorder={handleReorder}
      onreceive={handleReceive}
      onresize={handleResize}
      panelId="right"
    />
  </div>

  <!-- ======= MAP CONTROLS RESERVED AREA ======= -->
  <div class="zone-map-controls">
    <!-- reserved for map control buttons (zoom, 3D toggle etc.) -->
  </div>
  {/if}

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
  {#if !phoneUi}
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
  {/if}
</main>
  </div><!-- .ui-scale -->

  <!-- ======= FLOATING PANELS LAYER — scaled like .ui-scale, ABOVE the in-frame mini-map ======= -->
  <!-- .ui-scale is one stacking context (its transform), so nothing inside it can stack over the
       unzoomed in-frame map (z2) / its corner controls (z3). The floating panels + modal dialogs
       therefore live in this second, identically-scaled layer (z4) — panels cover the mini-map
       exactly like they cover the floating video window. The host replicates .app's positioning
       context and grid vars so PanelShell geometry is untouched. -->
  <div class="ui-scale panels-layer">
    <div
      class="panels-host"
      class:panels-hidden={panelHidden}
      style:--grid-bottom-height={gridBottomHeight}
      style:--grid-side-width={gridSideWidth}
      style:--panel-bottom-reserve={panelBottomReserve}
    >
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
            {buildings3D}
            {logReplayTime}
            {nightMode2D}
            lowPower3D={$settings.lowPower3D}
            {gcsMode}
            userLocation={$userGeoLocation}
            onGeoCheck={requestUserLocation}
            {attitudeRateHz}
            {positionRateHz}
            {airspeedEnabled}
            {windEnabled}
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
            batteryAlertPct={$settings.batteryAlertPct}
            {systemMessages}
            {logLevel}
            {interfaceSettings}
            radar={radarSettings}
            airspace={airspaceSettings}
            rcControl={$settings.rcControl}
            telemetryApi={$settings.telemetryApi}
            updateCheck={$settings.updateCheck}
            {isWidgetActive}
            {getWidgetPanelLabel}
            onPatch={applySettingsPatch}
            onSetCacheMaxMB={setCacheMaxMB}
            onClearCache={clearCache}
            onCompactDb={compactDb}
            onChooseFlightLogPath={chooseFlightLogPath}
            onResetFlightLogPath={resetFlightLogPath}
            onChooseRawLogPath={chooseRawLogPath}
            onResetRawLogPath={resetRawLogPath}
            onToggleWidget={toggleWidget}
          />
        {:else if activeTab === 'logbook'}
          <!-- .logbook-host: the collapse-anywhere hit-test's notion of "inside the logbook". -->
          <div class="logbook-host" style="display: contents">
          <LogbookPanel
            {flightLoggingEnabled}
            dbIncompatible={logbookDbIncompatible}
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
            onDeleteBlackbox={deleteBlackbox}
            {blackboxFileInfo}
            onExportTrack={exportTrack}
            dbPath={activeDbPath}
            fileMode={openedLogs ? { fileNames: openedFileNames } : null}
            canOpenLog={canOpenLogFile}
            onOpenLog={() => { void openLogFileDialog(); }}
            onImportOpened={() => { void importOpenedLogs(); }}
            onCloseOpened={() => { void closeOpenedLog(); }}
            onImportOpenedFlight={(id) => { void importOpenedFlight(id); }}
            onDismissOpenedFlight={(id) => { void dismissOpenedFlight(id); }}
          />
          </div>
        {:else if activeTab === 'mission'}
          <MissionPanel />
        {:else if activeTab === 'control'}
          <MavCommandPanel />
        {:else if activeTab === 'rc-control'}
          {#if isMobile}
            <VirtualSticks />
          {:else}
            <RcControlPanel />
          {/if}
        {:else if activeTab === 'radar'}
          <RadarPanel radar={radarSettings} {interfaceSettings} referencePoint={radarReference} mspSupported={mspAdsbSupported} onPatch={applySettingsPatch} />
        {:else if activeTab === 'airspace'}
          <!-- Re-init the panel on connect/disconnect + when the FC's geozone/fence capability resolves
               (loaded async after connect) so an already-open panel reflects the new FC without a tab switch. -->
          {#key `${$connection.status}-${geozonesAvailable}-${fenceAvailable}-${rallyAvailable}`}
            <AirspaceManagerPanel reference={radarReference} distanceUnit={interfaceSettings.distanceUnit} />
          {/key}
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
    </div>
    <!-- Phone: the dev Debug toggle lives in the PANELS layer so it stays reachable over an open
         panel (a developer tool); the arming / sensor chips stay in .app under the panels (Marc:
         a panel may cover them, they still peek out left of it). Dialogs keep their higher z-index. -->
    {#if phoneUi && DEV_MODE}
      <div class="phone-debug-btn">
        <PhoneDebugButton bind:debugOpen />
      </div>
    {/if}
    <ConfirmDialog bind:this={confirmDialog} />
    <UpdateDialog />
    <CesiumKeyPrompt bind:open={cesiumKeyPromptOpen} onSave={cesiumKeySave} onRemindLater={cesiumKeyRemindLater} onIgnore={cesiumKeyIgnore} />
    <EndFlightDialog bind:this={endFlightDialog} {interfaceSettings} />
    <RecoveryPrompt bind:this={recoveryPrompt} />
    <DisconnectArmedDialog bind:this={disconnectArmedDialog} />
  </div>

  <!-- Cursor-positioned overlays stay OUTSIDE the zoom so their fixed clientX/clientY
       coordinates are not multiplied by --ui-scale (they render unscaled but in the
       correct place; see docs/archive/UI_SCALING.md). -->
  <ContextMenu />
  <BatchEditPopup {interfaceSettings} />
  <ArduBatchEditPopup />
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
    /* Scale the chrome up to fill the viewport (the box is sized /scale above, then scaled back from
       the top-left corner). We use transform: scale() rather than `zoom` because WebKitGTK (Linux)
       does not support CSS `zoom` — it left the chrome at the /scale size, i.e. SMALLER than the
       window. transform is geometrically identical here (origin 0 0: logical→viewport = ×scale, same
       mapping the offset/rect math relies on) and works on both WebView2 (Windows) and WebKitGTK. */
    transform: scale(var(--ui-scale, 1));
    transform-origin: 0 0;
    z-index: 1;
  }
  /* Second scaled layer for the floating panels + modal dialogs: above the in-frame mini-map
     (z2) and its top-level corner controls (z3) — see the template comment at the layer. */
  .ui-scale.panels-layer {
    z-index: 4;
  }
  /* The host mirrors .app: a full-size positioning context for the self-positioned PanelShells
     that must NOT eat pointer events itself, or the map/chrome below would go dead. */
  .panels-layer .panels-host {
    position: relative;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .panels-host > :global(*) {
    pointer-events: auto;
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

  /* Mobile top safe-area: the iOS status bar (clock/battery/notch) overlays the toolbar. Grow the
     toolbar grid row by the safe-area inset, pad the toolbar content down into the visible strip, and
     push the top-anchored map/video/toast layers down to match so nothing hides under the status bar. */
  :global(html.is-mobile) .app {
    grid-template-rows: calc(53px + var(--safe-top, 0px)) 1fr var(--grid-bottom-height) 24px;
  }
  :global(html.is-mobile) .zone-toolbar {
    padding-top: var(--safe-top, 0px);
    box-sizing: border-box;
  }
  :global(html.is-mobile) .layer-map {
    /* Track the live toolbar height (includes the status-bar inset) so the map starts exactly at the
       bar's bottom edge, with no grey strip below the blue divider when the bar is collapsed/short. */
    top: calc(var(--toolbar-h, 53px) * var(--ui-scale, 1));
  }
  /* Mobile (phone + tablet): hand all touch gestures on the map to Leaflet (pan + pinch). Without
     touch-action:none the iOS WebView swallows the single-finger drag (only pinch/zoom-buttons work).
     Scoped to the map so panel scrollers keep their native touch scrolling. */
  :global(html.is-mobile) .layer-map,
  :global(html.is-mobile) .layer-map :global(.leaflet-container) {
    touch-action: none;
  }
  :global(html.is-mobile) .map-video-wrap {
    top: var(--toolbar-h, 53px);
  }
  /* Phone: no toolbar / status bar — the swapped-in video fills the MAP AREA (it ends at the widget
     column and rides along when the replay player pushes the column aside), so the picture is
     centred in the uncovered area like the map's follow target, not on the screen. */
  :global(html.is-phone) .map-video-wrap {
    top: 0;
    bottom: 0;
    right: calc(var(--phone-panel-w, 0px) - var(--phone-shift, 0px));
    transition: right 0.3s ease;
  }
  /* Phone, video primary: the mini map in the docked frame parks with it — off to the right,
     behind the column and out (the frame itself unmounts; the map layer is +page's, so it moves).
     Touches: the mini map takes them (pinch zoom — Leaflet's dragging and double-tap zoom are off
     in mini mode, D6) and relays a long-press to the tile underneath (PhoneWidgetPanel); while the
     grid is in EDIT mode (html.phone-editing) the layer goes touch-free so the tile can be dragged. */
  :global(html.is-phone) .layer-map.in-frame {
    transition: transform 0.3s ease;
  }
  :global(html.phone-editing) .layer-map.in-frame,
  :global(html.phone-editing) .layer-map.in-frame :global(*) {
    pointer-events: none !important;
  }
  :global(html.is-phone) .layer-map.in-frame.parked {
    transform: translateX(100vw);
  }
  /* The clipping wrapper: transparent on the desktop; on the phone it bounds the mini map to the
     map area (docked frame — the map slides out BEHIND the column, like the video) or to the
     column's tile box (widget — the map leaves with its page instead of hanging over the glass).
     pointer-events is inherited: the wrapper takes none, the map layer opts back in. */
  .map-clip {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .map-clip > .layer-map {
    pointer-events: auto;
  }
  /* clip-path, NOT a smaller box: the map layer's inline left/top are viewport coordinates, and a
     wrapper with its own offset would move their origin. clip-path creates a stacking context, so
     the wrapper takes the in-frame layer's z-index (2, above .ui-scale) while it clips. */
  :global(html.is-phone) .map-clip.clip-map-area {
    clip-path: inset(0 calc(var(--phone-panel-w, 0px) - var(--phone-shift, 0px)) 0 0);
    z-index: 2;
  }
  :global(html.is-phone) .map-clip.clip-column {
    clip-path: inset(
      var(--phone-pad, 4px)
      calc(var(--phone-pad, 4px) + var(--safe-right, 0px))
      var(--phone-pad, 4px)
      calc(100vw - var(--phone-panel-w, 0px) + var(--phone-shift, 0px) + var(--phone-pad, 4px))
    );
    z-index: 2;
  }
  /* Editing the grid: the mini map drops UNDER the column (dimmed through the glass) so the tile's
     own edit chrome — the resize button — is on top and reachable. */
  :global(html.phone-editing) .map-clip.clip-column {
    z-index: 0;
  }
  :global(html.is-mobile) .app-toasts {
    top: var(--safe-top, 0px);
  }

  /* Phone portrait is too narrow to fit the toolbar on one row, so it wraps to multiple lines
     (see Toolbar.svelte). Size the toolbar grid row to its content instead of the fixed one-row
     height so the wrapped bar is never clipped and the nav-rail / panels below are pushed down
     cleanly rather than tucked under it. Tablets keep the fixed row (wider screen fits one row). */
  @media (max-width: 600px) {
    :global(html.is-mobile) .app {
      /* Bottom bar hidden on phone (redundant with the collapsed top strip; arming moves into it). The
         last row is kept as a thin strip (home-indicator inset + room for the Leaflet label) so the HUD
         sits above it and the label is not covered; the map runs full-height behind it. */
      grid-template-rows: auto 1fr var(--grid-bottom-height) calc(var(--safe-bottom, 0px) + 20px);
      /* Reclaim the left nav-rail column for the bottom dock (NavRail is absolutely positioned, so its
         grid cell is empty): gives the HUD tiles the full left-to-right width on phone and uses the
         empty space on the left. Map controls keep the right column. */
      grid-template-areas:
        "toolbar      toolbar      toolbar      toolbar"
        "nav-rail     panel        side-dock    side-dock"
        "bottom-dock  bottom-dock  bottom-dock  map-controls"
        "status-bar   status-bar   status-bar   status-bar";
    }
    /* Redundant on phone (same status as the top strip); hidden. */
    :global(html.is-mobile) .zone-status-bar {
      display: none;
    }
    /* Map runs all the way to the bottom edge (behind the home indicator) so there is no grey strip. */
    :global(html.is-mobile) .layer-map {
      bottom: 0;
    }
    /* Top-align the right side dock so MODE sits just under the toolbar (the zone centered the whole
       widget block vertically, leaving a big gap above it). The 8px top padding matches the panel/
       nav-rail offset so MODE lines up with the UAV Info panel below the bar. */
    :global(html.is-mobile) .zone-side-dock {
      align-items: flex-start;
      padding-top: 8px;
    }
  }

  /* Map layer — UNZOOMED overlay over the content area. The toolbar (53px) and status
     bar (24px) live in the zoomed `.ui-scale`, so their visual heights are *--ui-scale;
     the map offsets track that. z-index 0 keeps it behind the chrome normally. When the
     view is swapped into the floating window it flips above the chrome (.in-frame) and
     uses the inline rect (already *--ui-scale in mapFrameStyle). */
  .layer-map {
    position: absolute;
    /* top/bottom only as the no-JS fallback: the inline `mapLayerStyle` overrides them with the
       same values rounded to whole px — a fractional box origin (53px × 1.25 = 66.25px) put every
       tile edge on a subpixel boundary and WebKitGTK rendered hairline seams between the tiles
       (issue #52). */
    top: calc(53px * var(--ui-scale, 1));
    left: 0;
    right: 0;
    bottom: calc(24px * var(--ui-scale, 1));
    z-index: 0;
    overflow: hidden;
  }
  /* Both map overlays stay mounted and hide with visibility, not display: that keeps their box
     size, so Leaflet and Cesium come back without a resize dance. */
  .map2d-layer,
  .map3d-layer {
    position: absolute;
    inset: 0;
  }
  .map2d-layer:not(.active),
  .map3d-layer:not(.active) {
    visibility: hidden;
    pointer-events: none;
  }
  .layer-map.in-frame {
    top: auto;
    right: auto;
    bottom: auto; /* left/top/width/height come from the inline rect */
    z-index: 2; /* above .ui-scale (z:1): into the floating frame body; frame draws the border */
    border-radius: 7px;
  }
  /* Hide the Leaflet attribution while the map is in the tiny floating frame: it's illegible there
     and tapping it navigates the whole webview to an inescapable page. (Stays on the full map.) */
  .layer-map.in-frame :global(.leaflet-control-attribution) {
    display: none;
  }
  /* Toasts & alerts container — pinned to the app frame's top, above the map/video layers (z2/z0) so
     banners are never tied to (or clipped by) a shrunken in-frame map. Zero-height (children are
     absolutely positioned) so it never blocks clicks. */
  .app-toasts {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 40;
  }
  /* Mini-map frame controls — overlay the in-frame map's top corners (z above the map). */
  .miniframe-ctl {
    position: absolute;
    z-index: 3;
    pointer-events: none;
  }
  .mf-corner {
    position: absolute;
    top: 0;
    width: 26px;
    height: 26px;
    pointer-events: auto;
    box-sizing: border-box;
    touch-action: none;
  }
  .mf-close {
    left: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    line-height: 1;
    color: #e0e0e0;
    background: rgba(0, 0, 0, 0.5);
    border: none;
    border-radius: 8px 0 8px 0;
    cursor: pointer;
  }
  .mf-close:hover {
    background: rgba(212, 0, 0, 0.7);
    color: #fff;
  }
  .mf-resize {
    right: 0;
    cursor: nesw-resize;
    border-radius: 0 8px 0 8px;
    background: linear-gradient(225deg, rgba(55, 168, 219, 0.85) 42%, transparent 42%);
  }
  .mf-resize:hover {
    background: linear-gradient(225deg, rgba(55, 168, 219, 1) 50%, transparent 50%);
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
  /* Native-sink hole: the wrapper stops painting while it holds the hardware video layer
     (the sink letterboxes on its own black backbuffer). */
  .map-video-wrap.nv-active {
    background: transparent;
  }
  /* Unobstructed fullscreen (Video panel toggle): the wrapper KEEPS its full-zone box (the
     blurred backdrop map inside it fills everything — widgets, docks and nav rail float on
     it); the video box alone retreats from the reserves, computed in ufBox. The wrapper only
     stops painting its own black so the backdrop (or the app ground, without a position)
     shows through. */
  .map-video-wrap.unobstructed {
    background: transparent;
  }
  /* Inner video box: fills the wrapper normally; in unobstructed mode the inline style from
     ufBox cuts it to the stream's aspect (left+width beat the inset's right, top+height its
     bottom) and it reads as a deliberately framed surface — panel-style accent border, black
     only behind the picture itself (≤1px rounding slivers). border-box keeps the frame inside
     the aspect-exact rect. */
  .map-video-box {
    position: absolute;
    inset: 0;
  }
  .map-video-wrap.unobstructed .map-video-box {
    background: #000;
    border: 1px solid rgba(55, 168, 219, 0.35);
    box-sizing: border-box;
    /* Drop shadow — near-black right at the frame, fading out wide and soft: lifts the
       framed video clearly off the blurred backdrop map. */
    box-shadow: 0 0 6px rgba(0, 0, 0, 1), 0 6px 28px rgba(0, 0, 0, 0.85);
  }
  /* Native sink: the box must not paint either, or its black would sit ON TOP of the
     hardware layer below the WebView (same rule as .nv-active on the wrapper). The hole
     child handles its own armed/unarmed background; the border stays. */
  .map-video-wrap.unobstructed.nv-active .map-video-box {
    background: transparent;
  }
  .map-video-wrap .native-hole {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    background: #000;
  }
  .map-video-wrap .native-hole.armed {
    background: transparent;
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
  .map-video.rot180 {
    transform: rotate(180deg);
  }
  .map-video.mirror.rot180 {
    transform: scaleY(-1);
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
    /* The panel hugs the bottom edge: a dock of small tiles frees the space ABOVE it, next to the
       map/video (WIDGET_OVERHAUL.md D7). The zone's own height stays the L-unit reference. */
    align-items: flex-end;
    pointer-events: none;
    overflow: hidden;
    padding: 6px 0;
  }

  .zone-bottom-dock.panel-editing {
    pointer-events: auto;
  }

  /* When the on-screen RC sticks are up (mobile, rc-control tab) they cover the bottom ~46vh as a
     fixed overlay. Lift the HUD dock above the sticks so HOME/SPD/ALT/GPS stay visible instead of
     being obscured. VirtualSticks .vs-root is height: 46vh. */
  .app.rc-sticks-active .zone-bottom-dock {
    transform: translateY(calc(-46vh - 8px));
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

  /* ── Phone chrome (Dev-Docs active/PHONE_UI.md) ──────────────────────────────────────────
     One row: the map area (everything floats over it) and the full-height widget column whose
     width the panel reports (--phone-panel-w). No toolbar, no docks, no status bar. Declared
     after the is-mobile rules so it wins at equal specificity. */
  :global(html.is-phone) .app {
    /* minmax(0, 1fr): a grid row's default min-height is its CONTENT — the widget column's
       two pages are 2× the panel height, so a plain 1fr row grew with them, the panel measured
       the taller row, the slot grew, the pages grew … (measured: slot 419 840 px, the screen
       flickering). The row must be the viewport, never the content. */
    grid-template-rows: minmax(0, 1fr);
    grid-template-columns: 1fr var(--phone-panel-w, 0px);
    grid-template-areas: "main phone-widgets";
  }
  .zone-phone-widgets {
    grid-area: phone-widgets;
    z-index: 100;
    min-width: 0;
    min-height: 0;
    height: 100%;
    overflow: hidden;
    pointer-events: none;
  }
  .zone-phone-widgets > :global(*) {
    pointer-events: auto;
  }
  /* The widget column slides out to the right by --phone-shift (set on .ui-root) while the full
     replay player needs more room than the gap between the burger and the chain-link button offers
     (narrow 16:9 phones); the chain-link button rides along so the gap really grows. The grid
     column itself stays put — a transform only, so the packer and the map centre are untouched. */
  .zone-phone-widgets {
    transform: translateX(var(--phone-shift, 0px));
    transition: transform 0.3s ease;
  }
  .phone-conn {
    position: absolute;
    top: calc(8px + var(--safe-top, 0px));
    right: calc(var(--phone-panel-w, 0px) + 8px - var(--phone-shift, 0px));
    z-index: 110;
    transition: right 0.3s ease;
  }
  /* Bottom-left corner: arming + sensor chips (under the panels — they peek out left of an open
     panel), then the dev Debug button in the panels layer (over them), then the Leaflet
     attribution (--phone-bottom-w / --phone-debug-w, published by the two components); the nav
     rail stops above the row. */
  .phone-bottom-chips {
    position: absolute;
    left: calc(12px + var(--safe-left, 0px));
    bottom: calc(8px + var(--safe-bottom, 0px));
    z-index: 110;
    pointer-events: none;
  }
  .phone-debug-btn {
    position: absolute;
    left: calc(var(--phone-bottom-w, 0px) + 8px);
    bottom: calc(8px + var(--safe-bottom, 0px));
    z-index: 170; /* over every panel (150 / 160), under the dialogs */
  }
  /* Toasts and alerts live in the TOP band between the burger (12 + 42 + 8) and the chain-link
     button (panel-w + 8 + 42 + 8 from the right) — the same gap the replay player uses. The
     container already sits at --safe-top (is-mobile rule), so its children start 8px down, in
     the buttons' row; the right edge rides along when the widget column slides out. */
  :global(html.is-phone) .app-toasts {
    left: calc(62px + var(--safe-left, 0px));
    right: calc(var(--phone-panel-w, 0px) - var(--phone-shift, 0px) + 58px);
    transition: right 0.3s ease;
    /* The system-message banner takes this off --toast-dock-inset (a viewport x) to centre in
       the free band beside an open panel, as it does on the desktop. */
    --toast-band-left: calc(62px + var(--safe-left, 0px));
  }
  /* BOTTOM band: between the chip row (+ Debug) and the map-control column (38 + 8 + 8 from the
     map area's right edge). The error bar sits in the chip row itself, centred and capped to the
     band instead of spanning the screen; a long message ellipsizes, the ✕ stays. */
  :global(html.is-phone) .error-bar {
    left: calc(var(--phone-bottom-w, 0px) + var(--phone-debug-w, 0px) + 8px);
    right: calc(var(--phone-panel-w, 0px) - var(--phone-shift, 0px) + 54px);
    bottom: calc(8px + var(--safe-bottom, 0px));
    margin-inline: auto;
    width: max-content;
    max-width: calc(100% - var(--phone-bottom-w, 0px) - var(--phone-debug-w, 0px) - var(--phone-panel-w, 0px) + var(--phone-shift, 0px) - 62px);
    box-sizing: border-box;
    gap: 8px;
    border-radius: 6px;
    transition: right 0.3s ease;
  }
  :global(html.is-phone) .error-bar > span {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* The resume banner goes one row up (the chip row may hold the error bar at the same time — a
     failed reconnect attempt while waiting for one), centred between the nav rail and the map
     controls and capped to that gap. */
  :global(html.is-phone) .resume-banner {
    --band-l: calc(62px + var(--safe-left, 0px));
    --band-r: calc(100vw - var(--phone-panel-w, 0px) + var(--phone-shift, 0px) - 54px);
    left: calc((var(--band-l) + var(--band-r)) / 2);
    bottom: calc(46px + var(--safe-bottom, 0px));
    max-width: calc(var(--band-r) - var(--band-l) - 16px);
    box-sizing: border-box;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: left 0.3s ease;
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
  /* Phone: stack the edit button ABOVE the HUD (not beside it) so the tiles get the full dock width
     and use the empty left space. Button sits top-right, out of the way. */
  @media (max-width: 600px) {
    :global(html.is-mobile) .panel-bottom-wrap {
      flex-direction: column;
      align-items: flex-end;
      gap: 4px;
      width: 100%;
    }
    :global(html.is-mobile) .panel-bottom-wrap > :global(.widget-panel) {
      width: 100%;
    }
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
