<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Settings on the panel framework (docs/active/PANEL_FRAMEWORK.md): a `compact` PanelShell with
  // a SegmentedToggle tab switcher (Interface / Data) in the toolbar; each tab is grouped into
  // labelled subsections. On/off switches use the shared <Toggle>, actions use <Button>, selects
  // match the framework height. All tiny italic hints are dropped except the Cesium-token one.
  import { t, locale } from 'svelte-i18n';
  import { SUPPORTED_LOCALES } from '$lib/i18n';
  import { MAP_PROVIDERS } from '$lib/config/mapProviders';
  import { WIDGET_DEFS } from '$lib/config/widgetRegistry';
  import { DEFAULT_RADAR, DEFAULT_AIRSPACE } from '$lib/stores/settings';
  import { resetGcsManual, gcsManuallySet } from '$lib/stores/gcsLocation';
  import type { AppSettings, InterfaceSettings, RadarSettings, GcsMode, AirspaceSettings, AirspaceProvider } from '$lib/stores/settings';
  import type { TileCacheStats } from '$lib/cache/tileCache';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import UnitStepper from '$lib/components/UnitStepper.svelte';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';

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
    gcsMode = 'manual',
    userLocation = null,
    attitudeRateHz = 5,
    positionRateHz = 2,
    airspeedEnabled = false,
    flightLoggingEnabled = false,
    flightRecordingEnabled = false,
    flightLogRawEnabled = false,
    flightLogRawAlways = false,
    flightLogDbPath = '',
    defaultFlightLogPath = '',
    defaultWpAltitudeM = 50,
    defaultPhTimeSec = 30,
    warnAltitudeM = 120,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
    radar = DEFAULT_RADAR,
    airspace = DEFAULT_AIRSPACE,
    isWidgetActive = (_widgetId: string) => false,
    getWidgetPanelLabel = (_widgetId: string) => '',
    onPatch = (_patch: Partial<AppSettings>) => {},
    onSetCacheMaxMB = (_maxMB: number) => {},
    onClearCache = () => {},
    onChooseFlightLogPath = () => {},
    onResetFlightLogPath = () => {},
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
    gcsMode?: GcsMode;
    userLocation?: { lat: number; lon: number } | null;
    attitudeRateHz?: number;
    positionRateHz?: number;
    airspeedEnabled?: boolean;
    flightLoggingEnabled?: boolean;
    flightRecordingEnabled?: boolean;
    flightLogRawEnabled?: boolean;
    flightLogRawAlways?: boolean;
    flightLogDbPath?: string;
    defaultFlightLogPath?: string;
    defaultWpAltitudeM?: number;
    defaultPhTimeSec?: number;
    warnAltitudeM?: number;
    interfaceSettings?: InterfaceSettings;
    radar?: RadarSettings;
    airspace?: AirspaceSettings;
    isWidgetActive?: (widgetId: string) => boolean;
    getWidgetPanelLabel?: (widgetId: string) => string;
    onPatch?: (patch: Partial<AppSettings>) => void;
    onSetCacheMaxMB?: (maxMB: number) => void;
    onClearCache?: () => void;
    onChooseFlightLogPath?: () => void;
    onResetFlightLogPath?: () => void;
    onToggleWidget?: (widgetId: string) => void;
    onGeoCheck?: () => void;
  } = $props();

  let tab = $state<'interface' | 'data'>('interface');

  const DEV = import.meta.env.DEV;
  /** Patch the nested radar settings (onPatch merges shallowly, so pass the whole radar object). */
  function patchRadar(partial: Partial<RadarSettings>) {
    onPatch({ radar: { ...radar, ...partial } });
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
  function onWpAltChange() { onPatch({ defaultWpAltitudeM: Math.max(1, wpAlt) }); }
  function onPhTimeChange() { onPatch({ defaultPhTimeSec: Math.max(1, Math.round(phTime)) }); }
  function onWarnAltChange() { onPatch({ warnAltitudeM: Math.max(0, warnAlt) }); }

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
      onchange={(v) => (tab = v as 'interface' | 'data')}
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
          <Button variant="standard" size="sm" onclick={onClearCache} title={$t('settings.clear')}>{$t('settings.clear')}</Button>
        </div>
      {/if}
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
      <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
        <label class="s-label" for="radar-radio">{$t('settings.radarRadio')}</label>
        <Toggle checked={radar.radio.enabled} id="radar-radio" disabled={!radar.enabled} onchange={(c) => patchRadar({ radio: { enabled: c } })} />
      </div>
      {#if DEV}
        <div class="s-row s-indent" class:s-disabled={!radar.enabled}>
          <label class="s-label" for="radar-sim">{$t('settings.radarSimDev')}</label>
          <Toggle checked={radar.sim} id="radar-sim" disabled={!radar.enabled} onchange={(c) => patchRadar({ sim: c })} />
        </div>
      {/if}
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
    </div>
  {/if}
{/snippet}

<div class="spv2">
  <PanelShell variant="compact" title={$t('nav.settings')} {toolbar} {body} />
</div>

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

  .widget-toggle-group { display: flex; align-items: center; gap: 8px; }
  .widget-panel-indicator { font-size: 9px; color: #888; min-width: 38px; text-align: right; }
</style>
