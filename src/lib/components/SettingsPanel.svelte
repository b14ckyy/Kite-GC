<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Settings on the panel framework (docs/active/PANEL_FRAMEWORK.md): a `compact` PanelShell with
  // a SegmentedToggle tab switcher (Interface / Data) in the toolbar; each tab is grouped into
  // labelled subsections. On/off switches use the shared <Toggle>, actions use <Button>, selects
  // match the framework height. All tiny italic hints are dropped except the Cesium-token one.
  import { invoke } from '@tauri-apps/api/core';
  import { t, locale } from 'svelte-i18n';
  import { SUPPORTED_LOCALES } from '$lib/i18n';
  import { MAP_PROVIDERS } from '$lib/config/mapProviders';
  import { WIDGET_DEFS } from '$lib/config/widgetRegistry';
  import { DEFAULT_RADAR, DEFAULT_AIRSPACE, DEFAULT_RC_CONTROL, DEFAULT_UPDATE_CHECK } from '$lib/stores/settings';
  import { panelState } from '$lib/stores/panelState';
  import { resetGcsManual, gcsManuallySet } from '$lib/stores/gcsLocation';
  import type { AppSettings, InterfaceSettings, RadarSettings, GcsMode, AirspaceSettings, AirspaceProvider, SystemMessagesLevel, LogLevel, RcControlSettings, UpdateCheckSettings, UpdateCheckMode } from '$lib/stores/settings';
  import { revealItemInDir } from '@tauri-apps/plugin-opener';
  import { blackboxDecoderVersion, downloadBlackboxDecode } from '$lib/stores/flightlog';
  import type { TileCacheStats } from '$lib/cache/tileCache';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import UnitStepper from '$lib/components/UnitStepper.svelte';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';
  import AboutDialog from '$lib/components/AboutDialog.svelte';

  let aboutOpen = $state(false);

  let {
    localeValue = 'en',
    uiScale = 1,
    mapProvider = 'osm',
    mapCacheMaxMB = 200,
    cacheStats = { usedBytes: 0, maxBytes: 0, tileCount: 0 },
    cesiumIonToken = '',
    altitudeCurtain3D = true,
    realLighting3D = false,
    logReplayTime = false,
    nightMode2D = 'off',
    lowPower3D = 'auto',
    gcsMode = 'manual',
    userLocation = null,
    attitudeRateHz = 5,
    positionRateHz = 2,
    airspeedEnabled = true,
    windEnabled = false,
    directionLines = true,
    mavlinkFullTelemetry = false,
    flightLoggingEnabled = false,
    flightRecordingEnabled = false,
    flightLogRawEnabled = false,
    flightLogRawAlways = false,
    flightLogDbPath = '',
    defaultFlightLogPath = '',
    flightLogRawPath = '',
    defaultRawLogPath = '',
    defaultWpAltitudeM = 50,
    defaultPhTimeSec = 30,
    warnAltitudeM = 120,
    batteryAlertPct = 30,
    systemMessages = 'all',
    logLevel = 'warning',
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
    radar = DEFAULT_RADAR,
    airspace = DEFAULT_AIRSPACE,
    rcControl = DEFAULT_RC_CONTROL,
    updateCheck = DEFAULT_UPDATE_CHECK,
    isWidgetActive = (_widgetId: string) => false,
    getWidgetPanelLabel = (_widgetId: string) => '',
    onPatch = (_patch: Partial<AppSettings>) => {},
    onSetCacheMaxMB = (_maxMB: number) => {},
    onClearCache = () => {},
    onChooseFlightLogPath = () => {},
    onResetFlightLogPath = () => {},
    onCompactDb = () => {},
    onChooseRawLogPath = () => {},
    onResetRawLogPath = () => {},
    onToggleWidget = (_widgetId: string) => {},
    onGeoCheck = () => {},
  }: {
    localeValue?: string;
    uiScale?: number;
    mapProvider?: string;
    mapCacheMaxMB?: number;
    cacheStats?: TileCacheStats;
    cesiumIonToken?: string;
    altitudeCurtain3D?: boolean;
    realLighting3D?: boolean;
    logReplayTime?: boolean;
    nightMode2D?: 'off' | 'auto' | 'on';
    lowPower3D?: 'off' | 'on' | 'auto';
    gcsMode?: GcsMode;
    userLocation?: { lat: number; lon: number } | null;
    attitudeRateHz?: number;
    positionRateHz?: number;
    airspeedEnabled?: boolean;
    windEnabled?: boolean;
    directionLines?: boolean;
    mavlinkFullTelemetry?: boolean;
    flightLoggingEnabled?: boolean;
    flightRecordingEnabled?: boolean;
    flightLogRawEnabled?: boolean;
    flightLogRawAlways?: boolean;
    flightLogDbPath?: string;
    defaultFlightLogPath?: string;
    flightLogRawPath?: string;
    defaultRawLogPath?: string;
    defaultWpAltitudeM?: number;
    defaultPhTimeSec?: number;
    warnAltitudeM?: number;
    batteryAlertPct?: number;
    systemMessages?: SystemMessagesLevel;
    logLevel?: LogLevel;
    interfaceSettings?: InterfaceSettings;
    radar?: RadarSettings;
    airspace?: AirspaceSettings;
    rcControl?: RcControlSettings;
    updateCheck?: UpdateCheckSettings;
    isWidgetActive?: (widgetId: string) => boolean;
    getWidgetPanelLabel?: (widgetId: string) => string;
    onPatch?: (patch: Partial<AppSettings>) => void;
    onSetCacheMaxMB?: (maxMB: number) => void;
    onClearCache?: () => void;
    onChooseFlightLogPath?: () => void;
    onResetFlightLogPath?: () => void;
    onCompactDb?: () => void;
    onChooseRawLogPath?: () => void;
    onResetRawLogPath?: () => void;
    onToggleWidget?: (widgetId: string) => void;
    onGeoCheck?: () => void;
  } = $props();

  // Active tab persists across panel switches + restart (panelState store).
  const tab = $derived($panelState.settingsTab);

  const DEV = import.meta.env.DEV;

  /** Human size: GB for ≥1 GiB, else MB. Used for both cache readouts. */
  function fmtSize(bytes: number): string {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    return `${(bytes / 1024 ** 2).toFixed(0)} MB`;
  }

  // Terrain cache (Copernicus DEM tiles on disk) — unbounded, so we only show size + a clear button.
  let terrainCache = $state<{ bytes: number; count: number }>({ bytes: 0, count: 0 });
  async function loadTerrainCache() {
    try {
      terrainCache = await invoke<{ bytes: number; count: number }>('terrain_cache_stats');
    } catch { /* backend unavailable — leave at zero */ }
  }
  async function clearTerrainCache() {
    try { await invoke('terrain_cache_clear'); } catch { /* non-critical */ }
    await loadTerrainCache();
  }

  // blackbox_decode (the one external dependency, INAV-only). Show its version so the user can tell
  // whether it needs updating for a new INAV version, and offer a one-click download/update.
  let bbVersion = $state<string | null>(null);
  let bbBusy = $state(false);
  let bbError = $state('');
  async function loadBbVersion() {
    try { bbVersion = await blackboxDecoderVersion(); } catch { bbVersion = null; }
  }
  async function updateBbDecoder() {
    bbBusy = true;
    bbError = '';
    try {
      await downloadBlackboxDecode();
      await loadBbVersion();
    } catch (e) {
      bbError = String(e);
    } finally {
      bbBusy = false;
    }
  }

  // Reveal the backend diagnostic log file in the OS file manager (Settings → Diagnostics).
  async function openLogFolder() {
    try {
      const p = await invoke<string | null>('get_log_path');
      if (p) await revealItemInDir(p);
    } catch { /* logging unavailable — nothing to open */ }
  }
  // Refresh the terrain-cache size + blackbox_decode version whenever the Data tab is shown.
  $effect(() => {
    if (tab !== 'interface') {
      void loadTerrainCache();
      void loadBbVersion();
    }
  });

  /** Patch the nested radar settings (onPatch merges shallowly, so pass the whole radar object). */
  function patchRadar(partial: Partial<RadarSettings>) {
    onPatch({ radar: { ...radar, ...partial } });
  }
  function patchRcControl(partial: Partial<RcControlSettings>) {
    onPatch({ rcControl: { ...rcControl, ...partial } });
  }
  function patchAirspace(partial: Partial<AirspaceSettings>) {
    onPatch({ airspace: { ...airspace, ...partial } });
  }

  function handleLocaleChange(event: Event) {
    const value = (event.target as HTMLSelectElement).value;
    locale.set(value);
    onPatch({ locale: value });
  }
  function handleUiScaleChange(event: Event) {
    onPatch({ uiScale: Number((event.target as HTMLSelectElement).value) });
  }
  function handleMapProviderChange(event: Event) {
    onPatch({ mapProvider: (event.target as HTMLSelectElement).value });
  }
  function handleCacheSizeChange(event: Event) {
    const value = Number((event.target as HTMLSelectElement).value);
    onPatch({ mapCacheMaxMB: value });
    onSetCacheMaxMB(value);
  }
  function handleAttitudeRateChange(event: Event) {
    onPatch({ attitudeRateHz: Number((event.target as HTMLSelectElement).value) });
  }
  function handlePositionRateChange(event: Event) {
    onPatch({ positionRateHz: Number((event.target as HTMLSelectElement).value) });
  }

  // Stepper-backed mission/alert settings (metric internal, unit-aware display).
  // svelte-ignore state_referenced_locally
  let wpAlt = $state(defaultWpAltitudeM);
  // svelte-ignore state_referenced_locally
  let phTime = $state(defaultPhTimeSec);
  // svelte-ignore state_referenced_locally
  let warnAlt = $state(warnAltitudeM);
  // svelte-ignore state_referenced_locally
  let batAlert = $state(batteryAlertPct);
  function onWpAltChange() { onPatch({ defaultWpAltitudeM: Math.max(1, wpAlt) }); }
  function onPhTimeChange() { onPatch({ defaultPhTimeSec: Math.max(1, Math.round(phTime)) }); }
  function onWarnAltChange() { onPatch({ warnAltitudeM: Math.max(0, warnAlt) }); }
  function onBatAlertChange() { onPatch({ batteryAlertPct: Math.max(0, Math.min(100, Math.round(batAlert))) }); }

  function patchIface(p: Partial<InterfaceSettings>) {
    onPatch({ interface: { ...interfaceSettings, ...p } });
  }

  // Raw flight logs are gated on recording; the row is force-on + disabled when logging is off
  // but recording is on (raw is the only sink then).
  const rawForced = $derived(!flightLoggingEnabled && flightRecordingEnabled);
</script>

{#snippet toolbar()}
  <div class="settabs">
    <SegmentedToggle
      full
      options={[{ value: 'interface', label: $t('settings.interface') }, { value: 'data', label: $t('settings.tabData') }]}
      value={tab}
      onchange={(v) => panelState.patch({ settingsTab: v as 'interface' | 'data' })}
    />
  </div>
{/snippet}

{#snippet body()}
  {#if tab === 'interface'}
    <!-- ── UI ────────────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.groupUi')}</h4>
      <div class="s-row">
        <label class="s-label" for="lang-select">{$t('settings.language')}</label>
        <select id="lang-select" class="s-select" value={localeValue} onchange={handleLocaleChange}>
          {#each SUPPORTED_LOCALES as loc}<option value={loc.code}>{loc.label}</option>{/each}
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="ui-scale">{$t('settings.uiScale')}</label>
        <select id="ui-scale" class="s-select" value={uiScale} onchange={handleUiScaleChange}>
          <option value={1}>100%</option>
          <option value={1.25}>125%</option>
          <option value={1.5}>150%</option>
        </select>
      </div>
    </div>

    <!-- ── Map ───────────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.map')}</h4>
      <div class="s-row">
        <label class="s-label" for="map-provider">{$t('settings.tileProvider')}</label>
        <select id="map-provider" class="s-select" value={mapProvider} onchange={handleMapProviderChange}>
          {#each MAP_PROVIDERS as p}<option value={p.id}>{p.label}</option>{/each}
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="altitude-curtain">{$t('settings.altitudeCurtain')}</label>
        <Toggle checked={altitudeCurtain3D} id="altitude-curtain" onchange={(c) => onPatch({ altitudeCurtain3D: c })} />
      </div>
      <div class="s-row">
        <label class="s-label" for="real-lighting">{$t('settings.realLighting')}</label>
        <Toggle checked={realLighting3D} id="real-lighting" onchange={(c) => onPatch({ realLighting3D: c })} />
      </div>
      <div class="s-row">
        <label class="s-label" for="log-replay-time" class:s-label-disabled={!realLighting3D}>{$t('settings.logReplayTime')}</label>
        <Toggle checked={realLighting3D && logReplayTime} disabled={!realLighting3D} id="log-replay-time" onchange={(c) => onPatch({ logReplayTime: c })} />
      </div>
      <div class="s-row">
        <label class="s-label" for="night-mode">{$t('settings.nightMode')}</label>
        <select id="night-mode" class="s-select" value={nightMode2D} onchange={(e) => onPatch({ nightMode2D: (e.target as HTMLSelectElement).value as 'off' | 'auto' | 'on' })}>
          <option value="off">{$t('settings.nightModeOff')}</option>
          <option value="auto">{$t('settings.nightModeAuto')}</option>
          <option value="on">{$t('settings.nightModeOn')}</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="low-power-3d">{$t('settings.lowPower3D')}</label>
        <select id="low-power-3d" class="s-select" value={lowPower3D} onchange={(e) => onPatch({ lowPower3D: (e.target as HTMLSelectElement).value as 'off' | 'on' | 'auto' })}>
          <option value="off">{$t('settings.lowPower3DOff')}</option>
          <option value="on">{$t('settings.lowPower3DOn')}</option>
          <option value="auto">{$t('settings.lowPower3DAuto')}</option>
        </select>
      </div>
      <div class="s-row">
        <span class="s-label">{$t('settings.userLocation')}</span>
        <div class="s-loc">
          <span class="s-loc-coords">
            {userLocation ? `${userLocation.lat.toFixed(2)}°, ${userLocation.lon.toFixed(2)}°` : $t('settings.locationNone')}
          </span>
          <Button variant="standard" size="sm" icon="refresh" onclick={onGeoCheck} title={$t('settings.detectLocationHint')}>{$t('settings.detectLocation')}</Button>
        </div>
      </div>
      <div class="s-row">
        <label class="s-label" for="gcs-mode">{$t('settings.gcsLocation')}</label>
        <div class="s-loc">
          {#if gcsMode === 'manual'}
            <Button variant="standard" size="sm" icon="refresh" disabled={!$gcsManuallySet} onclick={resetGcsManual} title={$t('settings.gcsResetHint')}>{$t('settings.gcsReset')}</Button>
          {/if}
          <select id="gcs-mode" class="s-select" value={gcsMode} onchange={(e) => onPatch({ gcsMode: (e.target as HTMLSelectElement).value as GcsMode })}>
            <option value="off">{$t('settings.gcsOff')}</option>
            <option value="manual">{$t('settings.gcsManual')}</option>
            <option value="continuous">{$t('settings.gcsContinuous')}</option>
          </select>
        </div>
      </div>
    </div>

    <!-- ── Units ─────────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.units')}</h4>
      <div class="s-row">
        <label class="s-label" for="speed-unit">{$t('settings.speedUnit')}</label>
        <select id="speed-unit" class="s-select" value={interfaceSettings.speedUnit} onchange={(e) => patchIface({ speedUnit: (e.target as HTMLSelectElement).value as InterfaceSettings['speedUnit'] })}>
          <option value="kmh">km/h</option>
          <option value="mph">mi/h</option>
          <option value="ms">m/s</option>
          <option value="fts">ft/s</option>
          <option value="kt">kt</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="altitude-unit">{$t('settings.altitudeUnit')}</label>
        <select id="altitude-unit" class="s-select" value={interfaceSettings.altitudeUnit} onchange={(e) => patchIface({ altitudeUnit: (e.target as HTMLSelectElement).value as InterfaceSettings['altitudeUnit'] })}>
          <option value="m">m</option>
          <option value="ft">ft</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="distance-unit">{$t('settings.distanceUnit')}</label>
        <select id="distance-unit" class="s-select" value={interfaceSettings.distanceUnit} onchange={(e) => patchIface({ distanceUnit: (e.target as HTMLSelectElement).value as InterfaceSettings['distanceUnit'] })}>
          <option value="metric">m / km</option>
          <option value="imperial">ft / mi</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="vertical-speed-unit">{$t('settings.verticalSpeedUnit')}</label>
        <select id="vertical-speed-unit" class="s-select" value={interfaceSettings.verticalSpeedUnit} onchange={(e) => patchIface({ verticalSpeedUnit: (e.target as HTMLSelectElement).value as InterfaceSettings['verticalSpeedUnit'] })}>
          <option value="ms">m/s</option>
          <option value="fts">ft/s</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="temperature-unit">{$t('settings.temperatureUnit')}</label>
        <select id="temperature-unit" class="s-select" value={interfaceSettings.temperatureUnit} onchange={(e) => patchIface({ temperatureUnit: (e.target as HTMLSelectElement).value as InterfaceSettings['temperatureUnit'] })}>
          <option value="c">°C</option>
          <option value="f">°F</option>
        </select>
      </div>
    </div>

    <!-- ── Widgets ───────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.hudWidgets')}</h4>
      {#each WIDGET_DEFS as wdef}
        <div class="s-row">
          <span class="s-label">{$t(wdef.labelKey)}</span>
          <div class="widget-toggle-group">
            <span class="widget-panel-indicator">{getWidgetPanelLabel(wdef.id)}</span>
            <Toggle checked={isWidgetActive(wdef.id)} onchange={() => onToggleWidget(wdef.id)} />
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <!-- ── Map (cache + 3D token) ────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.map')}</h4>
      <div class="s-row">
        <label class="s-label" for="map-cache">{$t('settings.tileCache')}</label>
        <select id="map-cache" class="s-select" value={mapCacheMaxMB} onchange={handleCacheSizeChange}>
          <option value={0}>{$t('settings.noCache')}</option>
          <option value={100}>100 MB</option>
          <option value={200}>200 MB</option>
          <option value={500}>500 MB</option>
          <option value={1000}>1 GB</option>
          <option value={2000}>2 GB</option>
          <option value={5000}>5 GB</option>
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
            {fmtSize(cacheStats.usedBytes)} / {fmtSize(mapCacheMaxMB * 1024 * 1024)} · {cacheStats.tileCount} tiles
          </span>
          <Button variant="standard" size="sm" onclick={onClearCache} title={$t('settings.clear')}>{$t('settings.clear')}</Button>
        </div>
      {/if}

      <!-- Terrain (Copernicus DEM) cache — unbounded on disk; size readout + clear only. -->
      <div class="s-row">
        <span class="s-label">{$t('settings.terrainCache')}</span>
        <div class="cache-inline">
          <span class="cache-bar-label">{fmtSize(terrainCache.bytes)} · {terrainCache.count} tiles</span>
          <Button variant="standard" size="sm" onclick={clearTerrainCache} title={$t('settings.clear')}>{$t('settings.clear')}</Button>
        </div>
      </div>
      <div class="s-row">
        <label class="s-label" for="cesium-ion-token">Cesium Ion Token</label>
        <input
          id="cesium-ion-token"
          class="s-input"
          type="password"
          placeholder="(optional)"
          value={cesiumIonToken}
          onchange={(e) => onPatch({ cesiumIonToken: (e.target as HTMLInputElement).value.trim() })}
        />
      </div>
      <p class="cesium-hint">
        {$t('settings.cesiumHintPre')} <a href="https://ion.cesium.com/signup" target="_blank" rel="noopener">ion.cesium.com</a> {$t('settings.cesiumHintPost')}
      </p>
      <!-- Airspace Manager (aeronautical data) — global toggle + provider + key; rest lives in the panel. -->
      <div class="s-row">
        <span class="s-label">{$t('settings.airspaceManager')}</span>
        <Toggle checked={airspace.enabled} id="airspace-enabled" onchange={(c) => patchAirspace({ enabled: c })} />
      </div>
      {#if airspace.enabled}
        <div class="s-row">
          <label class="s-label" for="airspace-provider">{$t('settings.airspaceProvider')}</label>
          <select id="airspace-provider" class="s-select" value={airspace.provider} onchange={(e) => patchAirspace({ provider: (e.target as HTMLSelectElement).value as AirspaceProvider })}>
            <option value="none">{$t('settings.airspaceProviderNone')}</option>
            <option value="openaip">OpenAIP</option>
          </select>
        </div>
        {#if airspace.provider === 'openaip'}
          <div class="s-row">
            <label class="s-label" for="airspace-key">{$t('settings.airspaceApiKey')}</label>
            <input
              id="airspace-key"
              class="s-input"
              type="password"
              placeholder="(OpenAIP API key)"
              value={airspace.apiKey}
              onchange={(e) => patchAirspace({ apiKey: (e.target as HTMLInputElement).value.trim() })}
            />
          </div>
          <p class="cesium-hint">
            {$t('settings.airspaceHintPre')} <a href="https://www.openaip.net/" target="_blank" rel="noopener">openaip.net</a> {$t('settings.airspaceHintPost')}
          </p>
        {/if}
      {/if}
    </div>

    <!-- ── Telemetry ─────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.telemetry')}</h4>
      <div class="s-row">
        <label class="s-label" for="attitude-rate">{$t('settings.attitude')}</label>
        <select id="attitude-rate" class="s-select" value={attitudeRateHz} onchange={handleAttitudeRateChange}>
          <option value={1}>1 Hz</option>
          <option value={2}>2 Hz</option>
          <option value={3}>3 Hz</option>
          <option value={5}>5 Hz</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="position-rate">{$t('settings.gpsPosition')}</label>
        <select id="position-rate" class="s-select" value={positionRateHz} onchange={handlePositionRateChange}>
          <option value={1}>1 Hz</option>
          <option value={2}>2 Hz</option>
          <option value={3}>3 Hz</option>
          <option value={5}>5 Hz</option>
        </select>
      </div>
      <div class="s-row">
        <label class="s-label" for="airspeed-toggle">{$t('settings.airspeed')}</label>
        <Toggle checked={airspeedEnabled} id="airspeed-toggle" onchange={(c) => onPatch({ airspeedEnabled: c })} />
      </div>
      <div class="s-row" title={$t('settings.windHint')}>
        <label class="s-label" for="wind-toggle">{$t('settings.wind')}</label>
        <Toggle checked={windEnabled} id="wind-toggle" onchange={(c) => onPatch({ windEnabled: c })} />
      </div>
      <div class="s-row" title={$t('settings.directionLinesHint')}>
        <label class="s-label" for="direction-lines-toggle">{$t('settings.directionLines')}</label>
        <Toggle checked={directionLines} id="direction-lines-toggle" onchange={(c) => onPatch({ directionLines: c })} />
      </div>
      <!-- MAVLink-only: stream everything the FC sends (ignores the two rate knobs above). -->
      <div class="s-row" title={$t('settings.mavlinkFullTelemetryHint')}>
        <label class="s-label" for="mav-full-telem">{$t('settings.mavlinkFullTelemetry')}</label>
        <Toggle checked={mavlinkFullTelemetry} id="mav-full-telem" onchange={(c) => onPatch({ mavlinkFullTelemetry: c })} />
      </div>

      <!-- Radar (foreign-vehicle tracking) — master + per-system enables. -->
      <div class="s-row">
        <label class="s-label" for="radar-enabled">{$t('settings.radarTracking')}</label>
        <Toggle checked={radar.enabled} id="radar-enabled" onchange={(c) => patchRadar({ enabled: c })} />
      </div>
      <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
        <label class="s-label" for="radar-adsb">{$t('settings.radarAdsb')}</label>
        <Toggle checked={radar.adsb.enabled} id="radar-adsb" disabled={!radar.enabled} onchange={(c) => patchRadar({ adsb: { ...radar.adsb, enabled: c } })} />
      </div>
      <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
        <label class="s-label" for="radar-ff">{$t('settings.radarFormationFlight')}</label>
        <Toggle checked={radar.formationFlight.enabled} id="radar-ff" disabled={!radar.enabled} onchange={(c) => patchRadar({ formationFlight: { ...radar.formationFlight, enabled: c } })} />
      </div>
      <!-- Radio Telemetry radar system parked for now (feature cut; may return). Kept the store/type so
           it can be re-enabled by un-commenting this row + the RadarPanel group entry. -->
      <!-- <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
        <label class="s-label" for="radar-radio">{$t('settings.radarRadio')}</label>
        <Toggle checked={radar.radio.enabled} id="radar-radio" disabled={!radar.enabled} onchange={(c) => patchRadar({ radio: { enabled: c } })} />
      </div> -->
      {#if DEV}
        <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
          <label class="s-label" for="radar-sim">{$t('settings.radarSimDev')}</label>
          <Toggle checked={radar.sim} id="radar-sim" disabled={!radar.enabled} onchange={(c) => patchRadar({ sim: c })} />
        </div>
      {/if}

      <!-- RC Control (INAV RC over MSP) — master switch for the RC nav-rail tab. -->
      <div class="s-row">
        <label class="s-label" for="rc-control-enabled">{$t('settings.rcControl')}</label>
        <Toggle checked={rcControl.enabled} id="rc-control-enabled" onchange={(c) => patchRcControl({ enabled: c })} />
      </div>
    </div>

    <!-- ── Flight Logbook ────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.flightLogging')}</h4>
      <div class="s-row">
        <label class="s-label" for="flightlog-enabled">{$t('settings.enableFlightLogging')}</label>
        <Toggle checked={flightLoggingEnabled} id="flightlog-enabled" onchange={(c) => onPatch({ flightLoggingEnabled: c })} />
      </div>
      <div class="s-row">
        <label class="s-label" for="flightrecord-enabled">{$t('settings.enableFlightRecording')}</label>
        <Toggle checked={flightRecordingEnabled} id="flightrecord-enabled" onchange={(c) => { onPatch({ flightRecordingEnabled: c }); if (!c) onPatch({ flightLogRawEnabled: false }); }} />
      </div>
      <div class="s-row" class:s-disabled={!flightRecordingEnabled || rawForced}>
        <label class="s-label" for="flightlog-raw">{$t('settings.rawFlightLogs')}</label>
        <Toggle checked={flightLogRawEnabled || rawForced} id="flightlog-raw" disabled={!flightRecordingEnabled || rawForced} onchange={(c) => onPatch({ flightLogRawEnabled: c })} />
      </div>
      <div class="s-row" class:s-disabled={!flightRecordingEnabled}>
        <label class="s-label" for="flightlog-raw-always">{$t('settings.continuousRawLogging')}</label>
        <Toggle checked={flightLogRawAlways} id="flightlog-raw-always" disabled={!flightRecordingEnabled} onchange={(c) => onPatch({ flightLogRawAlways: c })} />
      </div>
      <div class="s-row s-row-stack">
        <span class="s-label">{$t('settings.flightLogDbPath')}</span>
        <div class="path-picker-row">
          <input class="s-input path-input" type="text" readonly value={flightLogDbPath || defaultFlightLogPath || $t('settings.defaultPathUnknown')} />
          <Button variant="standard" size="sm" onclick={onChooseFlightLogPath}>{$t('settings.choose')}</Button>
          <Button variant="standard" size="sm" onclick={onResetFlightLogPath}>{$t('settings.useDefault')}</Button>
        </div>
      </div>
      <div class="s-row">
        <span class="s-label" title={$t('settings.compactDbHint')}>{$t('settings.compactDb')}</span>
        <Button variant="standard" size="sm" onclick={onCompactDb} title={$t('settings.compactDbHint')}>{$t('settings.compactDbBtn')}</Button>
      </div>
      <div class="s-row s-row-stack">
        <span class="s-label">{$t('settings.rawLogPath')}</span>
        <div class="path-picker-row">
          <input class="s-input path-input" type="text" readonly value={flightLogRawPath || defaultRawLogPath || $t('settings.defaultPathUnknown')} />
          <Button variant="standard" size="sm" onclick={onChooseRawLogPath}>{$t('settings.choose')}</Button>
          <Button variant="standard" size="sm" onclick={onResetRawLogPath}>{$t('settings.useDefault')}</Button>
        </div>
      </div>
      <div class="s-row s-row-stack">
        <span class="s-label">{$t('settings.blackboxDecoder')}</span>
        <div class="path-picker-row">
          <span class="s-readout">{bbVersion ?? $t('settings.bbDecoderMissing')}</span>
          <Button variant="standard" size="sm" disabled={bbBusy} onclick={updateBbDecoder}>
            {bbBusy ? $t('settings.bbDecoderBusy') : bbVersion ? $t('settings.bbDecoderUpdate') : $t('settings.bbDecoderDownload')}
          </Button>
        </div>
        {#if bbError}<span class="s-err">{bbError}</span>{/if}
      </div>
    </div>

    <!-- ── Diagnostics ───────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.diagnostics')}</h4>
      <div class="s-row">
        <span class="s-label">{$t('settings.logLevel')}</span>
        <select id="log-level" class="s-select" value={logLevel}
          onchange={(e) => onPatch({ logLevel: (e.target as HTMLSelectElement).value as LogLevel })}>
          <option value="off">{$t('settings.logOff')}</option>
          <option value="error">{$t('settings.logError')}</option>
          <option value="warning">{$t('settings.logWarning')}</option>
          <option value="debug">{$t('settings.logDebug')}</option>
        </select>
      </div>
      <div class="s-row">
        <span class="s-label">{$t('settings.logFolder')}</span>
        <Button variant="standard" size="sm" onclick={openLogFolder}>{$t('settings.openLogFolder')}</Button>
      </div>
    </div>

    <!-- ── Updates ───────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.updates')}</h4>
      <div class="s-row">
        <span class="s-label">{$t('settings.updateCheck')}</span>
        <select id="update-check" class="s-select" value={updateCheck.mode}
          onchange={(e) => onPatch({ updateCheck: { ...updateCheck, mode: (e.target as HTMLSelectElement).value as UpdateCheckMode } })}>
          <option value="disabled">{$t('settings.updateDisabled')}</option>
          <option value="release">{$t('settings.updateRelease')}</option>
          <option value="prerelease">{$t('settings.updatePrerelease')}</option>
        </select>
      </div>
      <p class="cesium-hint">{$t('settings.updateHint')}</p>
    </div>

    <!-- ── Mission Control ───────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.missionControl')}</h4>
      <div class="s-row">
        <span class="s-label">{$t('settings.defaultWpAlt')}</span>
        <UnitStepper bind:value={wpAlt} kind="altitude" settings={interfaceSettings} min={1} max={1000} step={5} decimals={0} onchange={onWpAltChange} />
      </div>
      <div class="s-row">
        <span class="s-label">{$t('settings.defaultPhTime')}</span>
        <NumberStepper bind:value={phTime} min={1} max={600} step={1} decimals={0} unit="s" onchange={onPhTimeChange} />
      </div>
    </div>

    <!-- ── Alerts ────────────────────────────────────── -->
    <div class="s-group">
      <h4 class="s-head">{$t('settings.alerts')}</h4>
      <div class="s-row">
        <span class="s-label">{$t('settings.altitude')}</span>
        <UnitStepper bind:value={warnAlt} kind="altitude" settings={interfaceSettings} min={0} max={5000} step={10} decimals={0} onchange={onWarnAltChange} />
      </div>
      <div class="s-row">
        <span class="s-label">{$t('settings.batteryAlert')}</span>
        <NumberStepper bind:value={batAlert} min={0} max={100} step={5} decimals={0} unit="%" onchange={onBatAlertChange} />
      </div>
      <div class="s-row">
        <span class="s-label">{$t('settings.systemMessages')}</span>
        <select id="system-messages" class="s-select" value={systemMessages}
          onchange={(e) => onPatch({ systemMessages: (e.target as HTMLSelectElement).value as SystemMessagesLevel })}>
          <option value="off">{$t('settings.sysMsgOff')}</option>
          <option value="error">{$t('settings.sysMsgError')}</option>
          <option value="warning">{$t('settings.sysMsgWarning')}</option>
          <option value="all">{$t('settings.sysMsgAll')}</option>
        </select>
      </div>
    </div>
  {/if}
{/snippet}

{#snippet headerActions()}
  <Button variant="standard" size="sm" icon="info" onclick={() => (aboutOpen = true)}>{$t('about.button')}</Button>
{/snippet}

<div class="spv2">
  <PanelShell variant="compact" title={$t('nav.settings')} {toolbar} {body} {headerActions} />
</div>

<AboutDialog bind:open={aboutOpen} />

<style>
  .settabs { width: 100%; }
  .settabs :global(.seg) { display: flex; width: 100%; }

  .s-group { padding-top: 2px; }
  .s-group + .s-group { border-top: 1px solid #373737; margin-top: 10px; padding-top: 8px; }

  .s-head {
    margin: 0 0 6px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .s-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
  }
  .s-row-stack { flex-direction: column; align-items: stretch; gap: 6px; }
  .s-disabled { opacity: 0.4; pointer-events: none; }
  .s-indent { padding-left: 16px; }

  .s-label { font-size: 12px; color: #e0e0e0; }
  .s-err { font-size: 11px; color: #d40000; }
  /* Read-only value display (not an input) — matches the locked-field look without being editable. */
  .s-readout {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    height: 28px;
    padding: 0 8px;
    background: #2a2a2a;
    border: 1px solid #3a3a3a;
    border-radius: 4px;
    color: #cfcfcf;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .s-label-disabled { opacity: 0.45; }
  .s-loc { display: flex; align-items: center; gap: 8px; }
  .s-loc-coords { font-size: 11px; color: #949494; font-variant-numeric: tabular-nums; white-space: nowrap; }

  /* Form controls match the md button height (28px) — consistent with the rest of the framework. */
  .s-select,
  .s-input {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .s-select { min-width: 80px; }

  .path-picker-row { display: flex; gap: 6px; align-items: center; }
  .path-input { flex: 1; min-width: 0; }

  /* The one kept hint (Cesium token) — bumped up a touch so it's actually readable. */
  .cesium-hint {
    font-size: 11px;
    color: #9a9a9a;
    line-height: 1.45;
    margin: 4px 0 0 0;
  }
  .cesium-hint a { color: #37a8db; }

  .cache-bar-container { display: flex; align-items: center; gap: 6px; padding: 4px 0 2px 0; }
  .cache-bar-track { flex: 1; height: 6px; background: #333; border-radius: 3px; overflow: hidden; }
  .cache-bar-fill { height: 100%; background: #37a8db; border-radius: 3px; transition: width 0.3s ease; }
  .cache-bar-fill.cache-bar-warning { background: #e8a317; }
  .cache-bar-label { font-size: 9px; color: #888; white-space: nowrap; }
  .cache-inline { display: flex; align-items: center; gap: 8px; }

  .widget-toggle-group { display: flex; align-items: center; gap: 8px; }
  .widget-panel-indicator { font-size: 9px; color: #888; min-width: 38px; text-align: right; }
</style>
