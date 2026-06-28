<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import { t } from 'svelte-i18n';
  import { get } from "svelte/store";
  import { radarAlertDebug, type AlertLevel } from "$lib/controllers/radarAlerts";
  import { gpsInject } from "$lib/stores/telemetry";
  import { settings } from "$lib/stores/settings";
  import { invoke } from "@tauri-apps/api/core";
  import { hidSnapshot } from "$lib/stores/hid";
  import { currentChannels } from "$lib/stores/rcProfiles";
  import { channelValues } from "$lib/stores/rcEngine";
  import { fcChannels } from "$lib/stores/rcMirror";
  import { boxName } from "$lib/helpers/inavModes";
  import { getPerf3dViewer, perf3dFps, perf3dForceContinuous } from "$lib/stores/perf3d";

  let { onclose }: { onclose: () => void } = $props();

  type Tab = 'msp' | 'mavlink' | 'alerts' | 'telemetry' | 'rc' | 'performance';
  let tab = $state<Tab>('msp');

  // ── 3D performance live-tuning (dev) — mutate the running Cesium scene to localise the
  // Linux/WebKitGTK bottleneck. Values are loaded from the live scene when the tab opens; each
  // edit writes straight back and forces a render (the scene runs in requestRenderMode). ──
  let perfReady = $state(false);
  let pFog = $state(true);
  let pFogDensity = $state(2.5e-4);
  let pFogSse = $state(2);
  let pMaxSse = $state(2);
  let pLighting = $state(false);
  let pSkyAtmo = $state(true);
  let pSkyBox = $state(true);
  let pSun = $state(true);
  let pMsaa = $state(1);
  let pFxaa = $state(false);
  let pResScale = $state(1);
  let pShowGlobe = $state(true);
  let pFpsOverlay = $state(false);
  // Content-independent full-screen passes — prime suspects for the per-pixel idle cost.
  let pOit = $state(true);
  let pLogDepth = $state(true);
  let pHdr = $state(false);

  function loadPerf(): void {
    const v = getPerf3dViewer();
    if (!v) { perfReady = false; return; }
    const s = v.scene;
    pFog = s.fog.enabled;
    pFogDensity = s.fog.density;
    pFogSse = s.fog.screenSpaceErrorFactor;
    pMaxSse = s.globe.maximumScreenSpaceError;
    pLighting = s.globe.enableLighting;
    pSkyAtmo = s.skyAtmosphere?.show ?? false;
    // Cesium's SkyBox .d.ts is missing `show` (present at runtime) — narrow cast, not `any`.
    pSkyBox = (s.skyBox as unknown as { show: boolean } | undefined)?.show ?? false;
    pSun = s.sun?.show ?? false;
    pMsaa = s.msaaSamples;
    pFxaa = s.postProcessStages.fxaa.enabled;
    pResScale = v.resolutionScale;
    pShowGlobe = s.globe.show;
    pFpsOverlay = s.debugShowFramesPerSecond;
    pOit = s.orderIndependentTranslucency;
    pLogDepth = s.logarithmicDepthBuffer;
    pHdr = s.highDynamicRange;
    perfReady = true;
  }

  function applyPerf(): void {
    const v = getPerf3dViewer();
    if (!v) { perfReady = false; return; }
    const s = v.scene;
    s.fog.enabled = pFog;
    s.fog.density = pFogDensity;
    s.fog.screenSpaceErrorFactor = pFogSse;
    s.globe.maximumScreenSpaceError = pMaxSse;
    s.globe.enableLighting = pLighting;
    if (s.skyAtmosphere) s.skyAtmosphere.show = pSkyAtmo;
    if (s.skyBox) (s.skyBox as unknown as { show: boolean }).show = pSkyBox;
    if (s.sun) s.sun.show = pSun;
    s.msaaSamples = pMsaa;
    s.postProcessStages.fxaa.enabled = pFxaa;
    v.resolutionScale = pResScale;
    s.globe.show = pShowGlobe;
    s.logarithmicDepthBuffer = pLogDepth;
    s.highDynamicRange = pHdr;
    s.debugShowFramesPerSecond = pFpsOverlay;
    // While the fps overlay is on, force continuous rendering — otherwise requestRenderMode renders
    // only on change, so the counter freezes when the map is idle and is skewed when it only ticks on
    // a load/update. Set it directly for immediate effect AND flag it in the store so Map3D's own
    // requestRenderMode reverters (alert/WP pulses, radar clear) keep honouring it. Restored when the
    // overlay (or the panel) closes.
    perf3dForceContinuous.set(pFpsOverlay);
    s.requestRenderMode = !pFpsOverlay;
    s.requestRender();
  }

  // OIT is a constructor-only option (read-only at runtime), so toggling it persists a dev flag that
  // Map3D reads on creation and reloads the window to rebuild the viewer with the new setting.
  function applyOit(): void {
    localStorage.setItem('kite_perf_oit', pOit ? 'on' : 'off');
    location.reload();
  }

  // Revert the dev render-mode override + overlay when the panel closes, so we don't leave the 3D
  // view rendering continuously (battery/GPU) or an orphaned overlay the user can't toggle off.
  function restorePerfRenderState(): void {
    perf3dForceContinuous.set(false);
    const v = getPerf3dViewer();
    if (!v) return;
    v.scene.debugShowFramesPerSecond = false;
    v.scene.requestRenderMode = true;
    v.scene.requestRender();
  }

  // Load live values whenever the Performance tab is opened.
  $effect(() => {
    if (tab === 'performance') loadPerf();
  });

  // ── RC control (MSP) diagnostics ──
  let rcFc = $state<{
    receiver_type: number;
    msp_override_channels: number | null;
    mode_ranges: { permanent_id: number; channel: number; range_min: number; range_max: number }[];
  } | null>(null);
  let rcErr = $state('');
  async function readRcFc() {
    rcErr = '';
    try {
      rcFc = await invoke('rc_read_fc_config');
    } catch (e) {
      rcErr = String(e);
      rcFc = null;
    }
  }
  const rxName = (t: number) => (t === 2 ? 'MSP' : t === 1 ? 'SERIAL' : t === 0 ? 'NONE' : `?(${t})`);
  function bitChannels(mask: number): string {
    const ch: string[] = [];
    for (let i = 0; i < 32; i++) if (mask & (1 << i)) ch.push(`CH${i + 1}`);
    return ch.length ? ch.join(', ') : '—';
  }
  function channelSummary(map: Record<number, unknown>, vals: Record<number, number>): string {
    const ks = Object.keys(map).map(Number).sort((a, b) => a - b);
    return ks.length ? ks.map((c) => `CH${c}=${vals[c] ?? '—'}`).join('  ') : '—';
  }

  // Dev GPS injection (global — visible in both tabs). Mirror the store so reopening reflects the
  // current override; write back on every change.
  let inj = $state({ ...get(gpsInject) });
  function applyInject() {
    gpsInject.set({ ...inj });
  }
  function fillFromView() {
    const c = get(settings).map.center;
    inj.lat = Number(c[0].toFixed(6));
    inj.lon = Number(c[1].toFixed(6));
    applyInject();
  }

  interface MspCodeDebug {
    code: number;
    name: string;
    is_polling: boolean;
    request_count: number;
    response_count: number;
    timeout_count: number;
    last_status: string;
    target_rate_hz: number;
    actual_rate_hz: number;
    latency_ms: number;
  }

  interface DebugSnapshot {
    messages: MspCodeDebug[];
    msg_per_sec_tx: number;
    msg_per_sec_rx: number;
    bytes_per_sec_tx: number;
    bytes_per_sec_rx: number;
  }

  let snapshot = $state<DebugSnapshot>({
    messages: [],
    msg_per_sec_tx: 0,
    msg_per_sec_rx: 0,
    bytes_per_sec_tx: 0,
    bytes_per_sec_rx: 0,
  });

  // MAVLink is push-based: no request/response/timeout — just per-message-ID counts, a measured
  // rate and a "last seen" age, separated by direction (RX from FC, TX from us).
  interface MavMsgDebug {
    id: number;
    name: string;
    dir: string;        // "RX" | "TX"
    count: number;
    rate_hz: number;
    last_seen_s: number;
  }

  interface MavlinkDebugSnapshot {
    messages: MavMsgDebug[];
    msg_per_sec_rx: number;
    msg_per_sec_tx: number;
    bytes_per_sec_rx: number;
    bytes_per_sec_tx: number;
  }

  let mavSnapshot = $state<MavlinkDebugSnapshot>({
    messages: [],
    msg_per_sec_rx: 0,
    msg_per_sec_tx: 0,
    bytes_per_sec_rx: 0,
    bytes_per_sec_tx: 0,
  });

  // Passive telemetry (listen-only) raw-stream sniffer: detection guess, byte rate and a live hex
  // tail. No request/response — it never transmits. See docs/active/RADIO_TELEMETRY.md.
  interface TelemProtoHit {
    name: string;
    count: number;
  }

  interface TelemSnapshot {
    locked: string;
    best_guess: string;
    total_bytes: number;
    bytes_per_sec: number;
    chunk_count: number;
    hex_tail: string;
    capture_file: string;
    crsf_frames: number;
    ltm_frames: number;
    msp_probe: MspProbeStats | null;
    hits: TelemProtoHit[];
  }

  // Experimental MSP-over-SmartPort probe diagnostics (dev-only; present once SmartPort is locked).
  interface MspProbeStats {
    tx_count: number;
    rx_chunks: number;
    replies: number;
    probe_replies: number;
    cmds_seen: string;
    last_reply_cmd: number;
    last_reply_hex: string;
    last_tx_hex: string;
    last_tx_variant: string;
    last_rx_hex: string;
    last_rx_note: string;
  }

  let telemSnapshot = $state<TelemSnapshot>({
    locked: "",
    best_guess: "",
    total_bytes: 0,
    bytes_per_sec: 0,
    chunk_count: 0,
    hex_tail: "",
    capture_file: "",
    crsf_frames: 0,
    ltm_frames: 0,
    msp_probe: null,
    hits: [],
  });

  // BLE GATT explorer (dev): the full service/characteristic table of the connected device, plus live
  // per-characteristic byte activity — so we can identify which characteristic carries the telemetry.
  interface GattCharInfo {
    uuid: string;
    properties: string[];
    subscribed: boolean;
  }
  interface GattServiceInfo {
    uuid: string;
    characteristics: GattCharInfo[];
  }
  interface GattTable {
    device: string;
    services: GattServiceInfo[];
  }

  let gattTable = $state<GattTable | null>(null);
  // Per-characteristic UUID → accumulated bytes + notification count.
  let gattActivity = $state<Record<string, { bytes: number; count: number }>>({});

  /** Short 16-bit form for standard BLE UUIDs (0000XXXX-0000-1000-8000-00805f9b34fb → 0xXXXX). */
  function shortUuid(uuid: string): string {
    const m = uuid.toLowerCase().match(/^0000([0-9a-f]{4})-0000-1000-8000-00805f9b34fb$/);
    return m ? `0x${m[1].toUpperCase()}` : uuid;
  }

  // Alerts tab reads the controller's live debug snapshot directly (frontend store).
  const alerts = $derived($radarAlertDebug);

  let unlisten: (() => void) | null = null;
  let unlistenMav: (() => void) | null = null;
  let unlistenTelem: (() => void) | null = null;
  let unlistenGatt: (() => void) | null = null;
  let unlistenGattData: (() => void) | null = null;

  onMount(async () => {
    unlisten = await listen<DebugSnapshot>("debug-msp-stats", (event) => {
      snapshot = event.payload;
    });
    unlistenMav = await listen<MavlinkDebugSnapshot>("debug-mavlink-stats", (event) => {
      mavSnapshot = event.payload;
    });
    unlistenTelem = await listen<TelemSnapshot>("debug-telemetry-stats", (event) => {
      telemSnapshot = event.payload;
    });
    unlistenGatt = await listen<GattTable>("ble-gatt-services", (event) => {
      gattTable = event.payload;
      gattActivity = {}; // reset per (re)connection
    });
    unlistenGattData = await listen<{ uuid: string; len: number }>("ble-gatt-char-data", (event) => {
      const { uuid, len } = event.payload;
      const cur = gattActivity[uuid] ?? { bytes: 0, count: 0 };
      gattActivity[uuid] = { bytes: cur.bytes + len, count: cur.count + 1 };
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
    if (unlistenMav) unlistenMav();
    if (unlistenTelem) unlistenTelem();
    if (unlistenGatt) unlistenGatt();
    if (unlistenGattData) unlistenGattData();
    restorePerfRenderState();
  });

  function ledColor(status: string): string {
    switch (status) {
      case "request": return "#f5a623";
      case "response": return "#59aa29";
      case "timeout": return "#d40000";
      default: return "#555";
    }
  }

  // MAVLink LED: green = fresh, amber = slowing, grey = stale (push-based, so age-driven
  // rather than the MSP request/response status).
  function mavLedColor(lastSeenS: number): string {
    if (lastSeenS < 1) return "#59aa29";
    if (lastSeenS < 3) return "#f5a623";
    return "#555";
  }

  function levelColor(level: AlertLevel | null): string {
    switch (level) {
      case "warning": return "#d40000";
      case "caution": return "#f5a623";
      default: return "#555";
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B/s`;
    return `${(bytes / 1024).toFixed(1)} KB/s`;
  }

  /** Absolute byte size (no "/s"), for the telemetry sniffer's running total. */
  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  function formatCode(code: number): string {
    if (code > 0xFF) return `0x${code.toString(16).toUpperCase().padStart(4, "0")}`;
    return code.toString();
  }

  /** Compact metres (raw dev readout — no unit conversion). */
  function m(v: number | null): string {
    if (v == null) return "—";
    return v >= 1000 ? `${(v / 1000).toFixed(1)}k` : `${Math.round(v)}`;
  }
</script>

<div class="debug-panel">
  <div class="debug-header">
    <span class="debug-title">{$t('debug.title')}</span>
    <button class="debug-close" onclick={onclose}>✕</button>
  </div>

  <div class="debug-tabs">
    <button class="tab" class:active={tab === 'msp'} onclick={() => tab = 'msp'}>{$t('debug.tabMsp')}</button>
    <button class="tab" class:active={tab === 'mavlink'} onclick={() => tab = 'mavlink'}>{$t('debug.tabMavlink')}</button>
    <button class="tab" class:active={tab === 'telemetry'} onclick={() => tab = 'telemetry'}>{$t('debug.tabTelemetry')}</button>
    <button class="tab" class:active={tab === 'alerts'} onclick={() => tab = 'alerts'}>{$t('debug.tabAlerts')}</button>
    <button class="tab" class:active={tab === 'rc'} onclick={() => tab = 'rc'}>{$t('debug.tabRc')}</button>
    <button class="tab" class:active={tab === 'performance'} onclick={() => tab = 'performance'}>{$t('debug.tabPerformance')}</button>
  </div>

  <div class="inject-row" class:on={inj.active}>
    <label class="inj-toggle" title={$t('debug.injGps')}>
      <input type="checkbox" bind:checked={inj.active} onchange={applyInject} />
      {$t('debug.injGps')}
    </label>
    <input class="inj-num" type="number" step="any" aria-label={$t('debug.injLat')}
      placeholder={$t('debug.injLat')} bind:value={inj.lat} onchange={applyInject} />
    <input class="inj-num" type="number" step="any" aria-label={$t('debug.injLon')}
      placeholder={$t('debug.injLon')} bind:value={inj.lon} onchange={applyInject} />
    <input class="inj-num inj-alt" type="number" step="any" aria-label={$t('debug.injAlt')}
      placeholder={$t('debug.injAlt')} bind:value={inj.altMsl} onchange={applyInject} />
    <button class="inj-btn" onclick={fillFromView} title={$t('debug.injFromView')}>⌖</button>
  </div>

  {#if tab === 'msp'}
    <div class="debug-stats">
      <div class="stat-group">
        <span class="stat-label">{$t('debug.msgPerSec')}</span>
        <span class="stat-value">TX {snapshot.msg_per_sec_tx.toFixed(1)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {snapshot.msg_per_sec_rx.toFixed(1)}</span>
      </div>
      <div class="stat-group">
        <span class="stat-label">{$t('debug.throughput')}</span>
        <span class="stat-value">TX {formatBytes(snapshot.bytes_per_sec_tx)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {formatBytes(snapshot.bytes_per_sec_rx)}</span>
      </div>
    </div>

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-led"></th>
            <th class="col-code">{$t('debug.colCode')}</th>
            <th class="col-name">{$t('debug.colName')}</th>
            <th class="col-status">{$t('debug.colStatus')}</th>
            <th class="col-num">{$t('debug.colReq')}</th>
            <th class="col-num">{$t('debug.colResp')}</th>
            <th class="col-num">{$t('debug.colTimeout')}</th>
            <th class="col-rate">{$t('debug.colTarget')}</th>
            <th class="col-rate">{$t('debug.colActual')}</th>
            <th class="col-rate">{$t('debug.colLatency')}</th>
          </tr>
        </thead>
        <tbody>
          {#each snapshot.messages as msg}
            <tr class:inactive={!msg.is_polling}>
              <td class="col-led">
                <span
                  class="led"
                  style="background: {ledColor(msg.last_status)}; box-shadow: 0 0 4px {ledColor(msg.last_status)};"
                ></span>
              </td>
              <td class="col-code">{formatCode(msg.code)}</td>
              <td class="col-name" title={msg.name}>{msg.name}</td>
              <td class="col-status">
                <span class="status-badge" class:polling={msg.is_polling} class:init={!msg.is_polling}>
                  {msg.is_polling ? "POLL" : "INIT"}
                </span>
              </td>
              <td class="col-num">{msg.request_count}</td>
              <td class="col-num">{msg.response_count}</td>
              <td class="col-num" class:has-timeouts={msg.timeout_count > 0}>{msg.timeout_count}</td>
              <td class="col-rate">{msg.target_rate_hz > 0 ? `${msg.target_rate_hz} Hz` : '—'}</td>
              <td class="col-rate" class:throttled={msg.is_polling && msg.target_rate_hz > 0 && msg.actual_rate_hz < msg.target_rate_hz * 0.85}>
                {msg.actual_rate_hz > 0 ? `${msg.actual_rate_hz} Hz` : '—'}
              </td>
              <td class="col-rate">{msg.latency_ms > 0 ? `${msg.latency_ms} ms` : '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if tab === 'mavlink'}
    <div class="debug-stats">
      <div class="stat-group">
        <span class="stat-label">{$t('debug.msgPerSec')}</span>
        <span class="stat-value">TX {mavSnapshot.msg_per_sec_tx.toFixed(1)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {mavSnapshot.msg_per_sec_rx.toFixed(1)}</span>
      </div>
      <div class="stat-group">
        <span class="stat-label">{$t('debug.throughput')}</span>
        <span class="stat-value">TX {formatBytes(mavSnapshot.bytes_per_sec_tx)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {formatBytes(mavSnapshot.bytes_per_sec_rx)}</span>
      </div>
    </div>

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-led"></th>
            <th class="col-code">{$t('debug.colId')}</th>
            <th class="col-name">{$t('debug.colName')}</th>
            <th class="col-status">{$t('debug.colDir')}</th>
            <th class="col-num">{$t('debug.colCount')}</th>
            <th class="col-rate">{$t('debug.colRate')}</th>
            <th class="col-rate">{$t('debug.colLast')}</th>
          </tr>
        </thead>
        <tbody>
          {#if mavSnapshot.messages.length === 0}
            <tr class="inactive"><td colspan="7" class="empty-cell">{$t('debug.mavNoData')}</td></tr>
          {/if}
          {#each mavSnapshot.messages as msg (msg.dir + msg.id)}
            <tr class:inactive={msg.last_seen_s >= 3}>
              <td class="col-led">
                <span
                  class="led"
                  style="background: {mavLedColor(msg.last_seen_s)}; box-shadow: 0 0 4px {mavLedColor(msg.last_seen_s)};"
                ></span>
              </td>
              <td class="col-code">{msg.id}</td>
              <td class="col-name" title={msg.name}>{msg.name}</td>
              <td class="col-status">
                <span class="status-badge" class:polling={msg.dir === 'RX'} class:init={msg.dir === 'TX'}>
                  {msg.dir}
                </span>
              </td>
              <td class="col-num">{msg.count}</td>
              <td class="col-rate">{msg.rate_hz > 0 ? `${msg.rate_hz} Hz` : '—'}</td>
              <td class="col-rate">{msg.last_seen_s.toFixed(1)}s</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if tab === 'telemetry'}
    <div class="debug-stats">
      <div class="stat-group">
        <span class="stat-label">{$t('debug.telDetected')}</span>
        <span class="stat-value">{telemSnapshot.locked || telemSnapshot.best_guess || $t('debug.telSearching')}</span>
        {#if telemSnapshot.locked}
          <span class="gate-badge stable">{$t('debug.telLocked')}</span>
        {/if}
      </div>
      <div class="stat-group">
        <span class="stat-label">{$t('debug.throughput')}</span>
        <span class="stat-value">{formatBytes(telemSnapshot.bytes_per_sec)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-label">{$t('debug.telTotal')}</span>
        <span class="stat-value">{formatSize(telemSnapshot.total_bytes)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-label">{$t('debug.telChunks')}</span>
        <span class="stat-value">{telemSnapshot.chunk_count}</span>
        {#if telemSnapshot.crsf_frames > 0}
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.telCrsfFrames')}</span>
          <span class="stat-value">{telemSnapshot.crsf_frames}</span>
        {/if}
        {#if telemSnapshot.ltm_frames > 0}
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.telLtmFrames')}</span>
          <span class="stat-value">{telemSnapshot.ltm_frames}</span>
        {/if}
      </div>
    </div>

    {#if telemSnapshot.msp_probe}
      {@const probe = telemSnapshot.msp_probe}
      <div class="debug-stats">
        <div class="stat-group">
          <span class="stat-label">{$t('debug.mspProbe')}</span>
          {#if probe.probe_replies > 0}
            <span class="gate-badge stable">{$t('debug.mspProbeOurReply')}</span>
          {:else if probe.replies > 0}
            <span class="gate-badge stable">{$t('debug.mspProbeSniffed')}</span>
          {:else}
            <span class="gate-badge unstable">{$t('debug.mspProbeNoReply')}</span>
          {/if}
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.mspProbeTx')}</span>
          <span class="stat-value">{probe.tx_count}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.mspProbeRx')}</span>
          <span class="stat-value">{probe.rx_chunks}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.mspProbeReplies')}</span>
          <span class="stat-value">{probe.replies}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.mspProbeOurs')}</span>
          <span class="stat-value">{probe.probe_replies}</span>
        </div>
      </div>
      {#if probe.cmds_seen}
        <div class="cap-row" title={probe.cmds_seen}>
          <span class="stat-label">{$t('debug.mspProbeCmds')}</span>
          <span class="cap-path">{probe.cmds_seen}</span>
        </div>
      {/if}
      {#if probe.last_reply_hex}
        <div class="cap-row" title={probe.last_reply_hex}>
          <span class="stat-label">{$t('debug.mspProbeLast')}</span>
          <span class="cap-path">cmd {probe.last_reply_cmd}: {probe.last_reply_hex}</span>
        </div>
      {/if}
      {#if probe.last_rx_note}
        <div class="cap-row" title={probe.last_rx_note}>
          <span class="stat-label">{$t('debug.mspProbeParse')}</span>
          <span class="cap-path">{probe.last_rx_note}</span>
        </div>
      {/if}
      {#if probe.last_rx_hex}
        <div class="hex-tail">
          <div class="hex-label">{$t('debug.mspProbeRxRaw')}</div>
          <div class="hex-bytes">{probe.last_rx_hex}</div>
        </div>
      {/if}
      {#if probe.last_tx_variant}
        <div class="cap-row" title={probe.last_tx_variant}>
          <span class="stat-label">{$t('debug.mspProbeVariant')}</span>
          <span class="cap-path">{probe.last_tx_variant}</span>
        </div>
      {/if}
      {#if probe.last_tx_hex}
        <div class="hex-tail">
          <div class="hex-label">{$t('debug.mspProbeTxRaw')}</div>
          <div class="hex-bytes">{probe.last_tx_hex}</div>
        </div>
      {/if}
    {/if}

    {#if telemSnapshot.capture_file}
      <div class="cap-row" title={telemSnapshot.capture_file}>
        <span class="stat-label">{$t('debug.telCapture')}</span>
        <span class="cap-path">{telemSnapshot.capture_file}</span>
      </div>
    {/if}

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-name">{$t('debug.telProto')}</th>
            <th class="col-num">{$t('debug.telHits')}</th>
          </tr>
        </thead>
        <tbody>
          {#if telemSnapshot.total_bytes === 0}
            <tr class="inactive"><td colspan="2" class="empty-cell">{$t('debug.telNoData')}</td></tr>
          {/if}
          {#each telemSnapshot.hits as h (h.name)}
            <tr class:inactive={h.count === 0}>
              <td class="col-name">{h.name}</td>
              <td class="col-num">{h.count}</td>
            </tr>
          {/each}
        </tbody>
      </table>

      {#if telemSnapshot.hex_tail}
        <div class="hex-tail">
          <div class="hex-label">{$t('debug.telHexTail')}</div>
          <div class="hex-bytes">{telemSnapshot.hex_tail}</div>
        </div>
      {/if}

      {#if gattTable}
        <div class="gatt-section">
          <div class="hex-label">{$t('debug.telGatt')}: {gattTable.device}</div>
          {#each gattTable.services as svc (svc.uuid)}
            <div class="gatt-svc">{shortUuid(svc.uuid)}</div>
            {#each svc.characteristics as ch (ch.uuid)}
              <div class="gatt-char" class:sub={ch.subscribed}>
                <span class="gatt-uuid">{shortUuid(ch.uuid)}</span>
                <span class="gatt-props">{ch.properties.join(' · ')}</span>
                {#if gattActivity[ch.uuid]}
                  <span class="gatt-act">{gattActivity[ch.uuid].bytes} B / {gattActivity[ch.uuid].count}×</span>
                {/if}
              </div>
            {/each}
          {/each}
        </div>
      {/if}
    </div>
  {:else if tab === 'rc'}
    <div class="debug-stats">
      <div class="stat-group">
        <button class="dbg-btn" onclick={readRcFc}>{$t('debug.rcRead')}</button>
        {#if rcErr}<span class="stat-warn">{rcErr}</span>{/if}
      </div>
      {#if rcFc}
        <div class="stat-group">
          <span class="stat-label">{$t('debug.rcRxType')}</span>
          <span class="stat-value">{rxName(rcFc.receiver_type)}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.rcOverride')}</span>
          <span class="stat-value">{rcFc.msp_override_channels == null ? 'n/a' : '0x' + rcFc.msp_override_channels.toString(16)}</span>
        </div>
        {#if rcFc.msp_override_channels != null}
          <div class="cap-row">
            <span class="stat-label">{$t('debug.rcOverrideCh')}</span>
            <span class="cap-path">{bitChannels(rcFc.msp_override_channels)}</span>
          </div>
        {/if}
        <div class="cap-row">
          <span class="stat-label">{$t('debug.rcModes')}</span>
          <span class="cap-path">{rcFc.mode_ranges.length ? rcFc.mode_ranges.map((m) => `CH${m.channel}:${boxName(m.permanent_id)}`).join('  ') : '—'}</span>
        </div>
      {/if}
      <div class="stat-group">
        <span class="stat-label">{$t('debug.rcHid')}</span>
        <span class="stat-value">{$hidSnapshot ? `id ${$hidSnapshot.id}` : '—'}</span>
        <span class="stat-sep">|</span>
        <span class="stat-label">{$t('debug.rcAxesBtnHat')}</span>
        <span class="stat-value">{$hidSnapshot ? `${$hidSnapshot.axes.length}/${$hidSnapshot.buttons.length}/${$hidSnapshot.hats.length}` : '—'}</span>
      </div>
      <div class="cap-row">
        <span class="stat-label">{$t('debug.rcChannels')}</span>
        <span class="cap-path">{channelSummary($currentChannels, $channelValues)}</span>
      </div>
      <div class="cap-row">
        <span class="stat-label">{$t('debug.rcFcMirror')}</span>
        <span class="cap-path">{$fcChannels.length ? $fcChannels.map((v, i) => `CH${i + 1}:${v}`).join('  ') : '—'}</span>
      </div>
    </div>
  {:else if tab === 'alerts'}
    <div class="debug-stats alerts-stats">
      {#if !alerts.uavValid}
        <span class="stat-warn">{$t('debug.alNoFix')}</span>
      {:else}
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alCourse')}</span>
          <span class="stat-value">{alerts.uavCourseDeg.toFixed(0)}°</span>
          <span class="gate-badge" class:stable={alerts.courseStable} class:unstable={!alerts.courseStable}>
            {alerts.courseStable ? $t('debug.alStable') : $t('debug.alUnstable')}
          </span>
        </div>
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alSpeed')}</span>
          <span class="stat-value">{alerts.uavSpeedMs.toFixed(1)} m/s</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.alVario')}</span>
          <span class="stat-value">{alerts.uavVarioMs.toFixed(1)} m/s</span>
        </div>
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alWorst')}</span>
          <span class="level-badge" style="color: {levelColor(alerts.worst)};">{alerts.worst ?? '—'}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.alEval')}</span>
          <span class="stat-value">{alerts.evaluated}</span>
        </div>
      {/if}
    </div>

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-led"></th>
            <th class="col-name">{$t('debug.alColContact')}</th>
            <th class="col-num">{$t('debug.alColDh')}</th>
            <th class="col-num">{$t('debug.alColDv')}</th>
            <th class="col-num">{$t('debug.alColRate')}</th>
            <th class="col-num">{$t('debug.alColTcpa')}</th>
            <th class="col-num">{$t('debug.alColMissH')}</th>
            <th class="col-num">{$t('debug.alColMissV')}</th>
            <th class="col-stage">{$t('debug.alColS1')}</th>
            <th class="col-stage">{$t('debug.alColS2')}</th>
          </tr>
        </thead>
        <tbody>
          {#each alerts.rows as row (row.id)}
            <tr class:inactive={!row.level}>
              <td class="col-led">
                <span class="led" style="background: {levelColor(row.level)}; box-shadow: 0 0 4px {levelColor(row.level)};"></span>
              </td>
              <td class="col-name">{row.callsign ?? row.id}</td>
              <td class="col-num">{m(row.dH)}</td>
              <td class="col-num">{m(row.dV)}</td>
              <td class="col-num" class:closing={row.rangeRate != null && row.rangeRate < 0}>
                {row.rangeRate == null ? '—' : row.rangeRate.toFixed(0)}
              </td>
              <td class="col-num">{row.tCpa == null ? '—' : `${row.tCpa.toFixed(0)}s`}</td>
              <td class="col-num">{m(row.missH)}</td>
              <td class="col-num">{m(row.missV)}</td>
              <td class="col-stage"><span class="dot" class:on={row.stage1Raw}></span></td>
              <td class="col-stage"><span class="dot" class:on={row.stage2Raw}></span></td>
            </tr>
          {/each}
          {#if alerts.active && alerts.rows.length === 0}
            <tr><td colspan="10" class="empty-row">{$t('debug.alNone')}</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {:else if tab === 'performance'}
    <div class="perf-tab">
      <div class="perf-head">
        <div class="perf-fps">{$t('debug.perf.fps')}: <b>{$perf3dFps}</b></div>
        <label class="perf-check"><input type="checkbox" bind:checked={pFpsOverlay} onchange={applyPerf} /> {$t('debug.perf.fpsOverlay')}</label>
        <button class="perf-refresh" onclick={loadPerf}>{$t('debug.perf.refresh')}</button>
      </div>

      {#if !perfReady}
        <div class="perf-empty">{$t('debug.perf.inactive')}</div>
      {:else}
        <div class="perf-section">{$t('debug.perf.secDistance')}</div>
        <label class="perf-check"><input type="checkbox" bind:checked={pFog} onchange={applyPerf} /> {$t('debug.perf.fog')}</label>
        <div class="perf-row"><span>{$t('debug.perf.fogDensity')}</span><input type="number" step="0.0001" min="0" bind:value={pFogDensity} onchange={applyPerf} /></div>
        <div class="perf-row"><span>{$t('debug.perf.fogSse')}</span><input type="number" step="0.5" min="1" bind:value={pFogSse} onchange={applyPerf} /></div>
        <div class="perf-row"><span>{$t('debug.perf.maxSse')}</span><input type="number" step="0.5" min="1" max="64" bind:value={pMaxSse} onchange={applyPerf} /></div>

        <div class="perf-section">{$t('debug.perf.secScene')}</div>
        <label class="perf-check"><input type="checkbox" bind:checked={pLighting} onchange={applyPerf} /> {$t('debug.perf.lighting')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pSkyAtmo} onchange={applyPerf} /> {$t('debug.perf.skyAtmo')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pSkyBox} onchange={applyPerf} /> {$t('debug.perf.skyBox')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pSun} onchange={applyPerf} /> {$t('debug.perf.sun')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pShowGlobe} onchange={applyPerf} /> {$t('debug.perf.showGlobe')}</label>

        <div class="perf-section">{$t('debug.perf.secQuality')}</div>
        <div class="perf-row"><span>{$t('debug.perf.msaa')}</span>
          <select bind:value={pMsaa} onchange={applyPerf}>
            <option value={0}>0</option><option value={1}>1</option><option value={2}>2</option><option value={4}>4</option><option value={8}>8</option>
          </select>
        </div>
        <label class="perf-check"><input type="checkbox" bind:checked={pFxaa} onchange={applyPerf} /> {$t('debug.perf.fxaa')}</label>
        <div class="perf-row"><span>{$t('debug.perf.resScale')}</span><input type="number" step="0.05" min="0.25" max="2" bind:value={pResScale} onchange={applyPerf} /></div>

        <div class="perf-section">{$t('debug.perf.secPasses')}</div>
        <label class="perf-check"><input type="checkbox" bind:checked={pOit} onchange={applyOit} /> {$t('debug.perf.oit')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pLogDepth} onchange={applyPerf} /> {$t('debug.perf.logDepth')}</label>
        <label class="perf-check"><input type="checkbox" bind:checked={pHdr} onchange={applyPerf} /> {$t('debug.perf.hdr')}</label>

        <div class="perf-hint">{$t('debug.perf.hint')}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .perf-tab { padding: 10px 12px; overflow-y: auto; font-size: 12px; }
  .perf-head {
    display: flex; align-items: center; gap: 14px;
    padding-bottom: 8px; margin-bottom: 8px; border-bottom: 1px solid #272727;
  }
  .perf-fps { font-size: 13px; color: #949494; }
  .perf-fps b { color: #37a8db; font-size: 16px; font-variant-numeric: tabular-nums; }
  .perf-refresh {
    margin-left: auto; padding: 3px 10px; font-size: 11px;
    background: #434343; color: #e0e0e0; border: 1px solid #555; border-radius: 4px; cursor: pointer;
  }
  .perf-refresh:hover { background: #4f4f4f; }
  .perf-section {
    margin: 12px 0 4px; font-size: 11px; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.5px; color: #37a8db;
  }
  .perf-row { display: flex; align-items: center; justify-content: space-between; padding: 3px 0; }
  .perf-row span { color: #c0c0c0; }
  .perf-row input, .perf-row select {
    width: 110px; height: 24px; box-sizing: border-box; padding: 0 6px;
    background: #434343; border: 1px solid #555; border-radius: 4px; color: #e0e0e0; font-size: 12px;
  }
  .perf-check { display: flex; align-items: center; gap: 7px; padding: 3px 0; color: #c0c0c0; cursor: pointer; }
  .perf-empty { padding: 20px 4px; color: #949494; font-style: italic; }
  .perf-hint { margin-top: 12px; padding-top: 8px; border-top: 1px solid #272727; color: #777; font-size: 11px; line-height: 1.4; }

  .debug-panel {
    position: absolute;
    top: 65px;
    right: 12px;
    width: 480px;
    max-height: calc(100vh - 53px - 24px - 30px);
    background: rgba(30, 30, 30, 0.95);
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 8px;
    z-index: 150;
    display: flex;
    flex-direction: column;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(12px);
    animation: debug-slide-in 0.2s ease-out;
    overflow: hidden;
    font-family: "Consolas", "JetBrains Mono", "Fira Code", monospace;
  }

  @keyframes debug-slide-in {
    from { opacity: 0; transform: translateX(16px); }
    to { opacity: 1; transform: translateX(0); }
  }

  .debug-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    background: rgba(55, 168, 219, 0.1);
    border-bottom: 1px solid rgba(55, 168, 219, 0.2);
  }

  .debug-title {
    font-size: 12px;
    font-weight: 600;
    color: #37a8db;
  }

  .debug-close {
    background: none;
    border: none;
    color: #949494;
    font-size: 14px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }

  .debug-close:hover {
    background: rgba(212, 0, 0, 0.3);
    color: #ff4444;
  }

  .debug-tabs {
    display: flex;
    gap: 4px;
    padding: 6px 8px 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }

  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: #949494;
    font-size: 11px;
    font-weight: 600;
    padding: 5px 12px;
    cursor: pointer;
    font-family: inherit;
  }

  .tab:hover {
    color: #e0e0e0;
  }

  .tab.active {
    color: #37a8db;
    border-bottom-color: #37a8db;
  }

  .inject-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    background: rgba(255, 255, 255, 0.02);
  }

  .inject-row.on {
    background: rgba(245, 166, 35, 0.08);
  }

  .inj-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-weight: 700;
    color: #949494;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    cursor: pointer;
    white-space: nowrap;
  }

  .inject-row.on .inj-toggle {
    color: #f5a623;
  }

  .inj-toggle input {
    accent-color: #f5a623;
    cursor: pointer;
  }

  .inj-num {
    width: 0;
    flex: 1;
    min-width: 0;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 3px;
    color: #e0e0e0;
    font-family: inherit;
    font-size: 10px;
    padding: 3px 5px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .inj-num.inj-alt {
    flex: 0 0 52px;
  }

  .inj-num:focus {
    outline: none;
    border-color: #37a8db;
  }

  .inj-btn {
    background: rgba(55, 168, 219, 0.15);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 3px;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 3px 6px;
  }

  .inj-btn:hover {
    background: rgba(55, 168, 219, 0.3);
  }

  .dbg-btn {
    background: rgba(55, 168, 219, 0.15);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 3px;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    padding: 4px 10px;
  }
  .dbg-btn:hover {
    background: rgba(55, 168, 219, 0.3);
  }

  .debug-stats {
    display: flex;
    gap: 16px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    font-size: 11px;
  }

  .alerts-stats {
    flex-wrap: wrap;
    gap: 10px 16px;
  }

  .stat-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .stat-label {
    color: #949494;
    font-weight: 600;
  }

  .stat-value {
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }

  .stat-sep {
    color: #555;
  }

  .stat-warn {
    color: #f5a623;
    font-size: 11px;
  }

  .level-badge {
    font-weight: 700;
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.3px;
  }

  .gate-badge {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
  }

  .gate-badge.stable {
    background: rgba(89, 170, 41, 0.15);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.3);
  }

  .gate-badge.unstable {
    background: rgba(148, 148, 148, 0.1);
    color: #999;
    border: 1px solid rgba(148, 148, 148, 0.2);
  }

  .debug-table-wrap {
    overflow-y: auto;
    flex: 1;
  }

  .debug-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }

  .debug-table thead {
    position: sticky;
    top: 0;
    background: rgba(30, 30, 30, 0.98);
  }

  .debug-table th {
    padding: 6px 8px;
    text-align: left;
    color: #949494;
    font-weight: 600;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }

  .debug-table td {
    padding: 4px 8px;
    color: #e0e0e0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.03);
  }

  .debug-table tr:hover {
    background: rgba(55, 168, 219, 0.05);
  }

  .debug-table tr.inactive td {
    color: #777;
  }

  .empty-cell {
    text-align: center;
    color: #777;
    font-style: italic;
    padding: 12px 8px;
  }

  .col-led {
    width: 20px;
    text-align: center;
  }

  .col-code {
    width: 55px;
    font-variant-numeric: tabular-nums;
  }

  .col-name {
    /* Flexible column that absorbs the remaining width and truncates with an ellipsis instead of
       widening the table (max-width:0 + width:100% is the standard single-flex-column table trick). */
    max-width: 0;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .col-status {
    width: 45px;
  }

  .col-num {
    width: 50px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  th.col-num {
    text-align: right;
  }

  .col-rate {
    width: 55px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-size: 10px;
  }

  th.col-rate {
    text-align: right;
  }

  .col-stage {
    width: 28px;
    text-align: center;
  }

  th.col-stage {
    text-align: center;
  }

  .closing {
    color: #f5a623 !important;
    font-weight: 700;
  }

  .empty-row {
    text-align: center !important;
    color: #777 !important;
    padding: 12px !important;
  }

  .throttled {
    color: #f5a623 !important;
    font-weight: 700;
  }

  .led {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    transition: background-color 0.05s, box-shadow 0.05s;
  }

  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: rgba(148, 148, 148, 0.2);
  }

  .dot.on {
    background: #37a8db;
    box-shadow: 0 0 4px #37a8db;
  }

  .status-badge {
    display: inline-block;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.3px;
  }

  .status-badge.polling {
    background: rgba(89, 170, 41, 0.15);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.3);
  }

  .status-badge.init {
    background: rgba(148, 148, 148, 0.1);
    color: #777;
    border: 1px solid rgba(148, 148, 148, 0.2);
  }

  .has-timeouts {
    color: #ff4444 !important;
    font-weight: 700;
  }

  .cap-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 5px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    font-size: 10px;
  }

  .cap-path {
    color: #37a8db;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .hex-tail {
    padding: 8px 12px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
  }

  .hex-label {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: #949494;
    margin-bottom: 4px;
  }

  .hex-bytes {
    font-size: 10px;
    line-height: 1.5;
    color: #b8d8e8;
    word-break: break-all;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 3px;
    padding: 6px 8px;
    font-variant-numeric: tabular-nums;
  }

  .gatt-section {
    padding: 8px 12px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
  }

  .gatt-svc {
    font-size: 10px;
    font-weight: 700;
    color: #37a8db;
    margin: 6px 0 2px;
    font-variant-numeric: tabular-nums;
  }

  .gatt-char {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 10px;
    padding: 2px 0 2px 12px;
    color: #949494;
  }

  .gatt-char.sub {
    color: #59aa29;
  }

  .gatt-uuid {
    font-variant-numeric: tabular-nums;
    min-width: 56px;
    color: #e0e0e0;
  }

  .gatt-char.sub .gatt-uuid {
    color: #59aa29;
  }

  .gatt-props {
    flex: 1;
    color: #949494;
  }

  .gatt-act {
    color: #f5a623;
    font-variant-numeric: tabular-nums;
  }
</style>
