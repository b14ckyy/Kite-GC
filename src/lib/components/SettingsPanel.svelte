<script lang="ts">
  import { t, locale } from 'svelte-i18n';
  import { SUPPORTED_LOCALES } from '$lib/i18n';
  import { MAP_PROVIDERS } from '$lib/config/mapProviders';
  import { WIDGET_DEFS } from '$lib/config/widgetRegistry';
  import type { AppSettings, InterfaceSettings } from '$lib/stores/settings';
  import type { TileCacheStats } from '$lib/cache/tileCache';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import UnitStepper from '$lib/components/UnitStepper.svelte';

  let {
    localeValue = 'en',
    uiScale = 1,
    mapProvider = 'osm',
    mapCacheMaxMB = 200,
    cacheStats = { usedBytes: 0, maxBytes: 0, tileCount: 0 },
    cesiumIonToken = '',
    altitudeCurtain3D = true,
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
    isWidgetActive = (_widgetId: string) => false,
    getWidgetPanelLabel = (_widgetId: string) => '',
    onPatch = (_patch: Partial<AppSettings>) => {},
    onSetCacheMaxMB = (_maxMB: number) => {},
    onClearCache = () => {},
    onChooseFlightLogPath = () => {},
    onResetFlightLogPath = () => {},
    onToggleWidget = (_widgetId: string) => {},
  }: {
    localeValue?: string;
    uiScale?: number;
    mapProvider?: string;
    mapCacheMaxMB?: number;
    cacheStats?: TileCacheStats;
    cesiumIonToken?: string;
    altitudeCurtain3D?: boolean;
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
    isWidgetActive?: (widgetId: string) => boolean;
    getWidgetPanelLabel?: (widgetId: string) => string;
    onPatch?: (patch: Partial<AppSettings>) => void;
    onSetCacheMaxMB?: (maxMB: number) => void;
    onClearCache?: () => void;
    onChooseFlightLogPath?: () => void;
    onResetFlightLogPath?: () => void;
    onToggleWidget?: (widgetId: string) => void;
  } = $props();

  function handleLocaleChange(event: Event) {
    const value = (event.target as HTMLSelectElement).value;
    locale.set(value);
    onPatch({ locale: value });
  }

  function handleUiScaleChange(event: Event) {
    const value = Number((event.target as HTMLSelectElement).value);
    onPatch({ uiScale: value });
  }

  function handleMapProviderChange(event: Event) {
    const value = (event.target as HTMLSelectElement).value;
    onPatch({ mapProvider: value });
  }

  function handleCacheSizeChange(event: Event) {
    const value = Number((event.target as HTMLSelectElement).value);
    onPatch({ mapCacheMaxMB: value });
    onSetCacheMaxMB(value);
  }

  function handleAttitudeRateChange(event: Event) {
    const value = Number((event.target as HTMLSelectElement).value);
    onPatch({ attitudeRateHz: value });
  }

  function handlePositionRateChange(event: Event) {
    const value = Number((event.target as HTMLSelectElement).value);
    onPatch({ positionRateHz: value });
  }

  function handleAirspeedToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ airspeedEnabled: checked });
  }

  function handleFlightLoggingToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ flightLoggingEnabled: checked });
  }

  function handleFlightRecordingToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ flightRecordingEnabled: checked });
    if (!checked) onPatch({ flightLogRawEnabled: false });
  }

  function handleFlightRawToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ flightLogRawEnabled: checked });
  }

  function handleFlightRawAlwaysToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ flightLogRawAlways: checked });
  }

  // Stepper-backed mission/alert settings (metric internal, unit-aware display).
  // Initialised from props (the panel remounts each time it's opened).
  // svelte-ignore state_referenced_locally
  let wpAlt = $state(defaultWpAltitudeM);
  // svelte-ignore state_referenced_locally
  let phTime = $state(defaultPhTimeSec);
  // svelte-ignore state_referenced_locally
  let warnAlt = $state(warnAltitudeM);
  function onWpAltChange() {
    onPatch({ defaultWpAltitudeM: Math.max(1, wpAlt) });
  }
  function onPhTimeChange() {
    onPatch({ defaultPhTimeSec: Math.max(1, Math.round(phTime)) });
  }
  function onWarnAltChange() {
    onPatch({ warnAltitudeM: Math.max(0, warnAlt) });
  }

  function handleSpeedUnitChange(event: Event) {
    const speedUnit = (event.target as HTMLSelectElement).value as InterfaceSettings['speedUnit'];
    onPatch({
      interface: {
        ...interfaceSettings,
        speedUnit,
      },
    });
  }

  function handleAltitudeUnitChange(event: Event) {
    const altitudeUnit = (event.target as HTMLSelectElement).value as InterfaceSettings['altitudeUnit'];
    onPatch({
      interface: {
        ...interfaceSettings,
        altitudeUnit,
      },
    });
  }

  function handleDistanceUnitChange(event: Event) {
    const distanceUnit = (event.target as HTMLSelectElement).value as InterfaceSettings['distanceUnit'];
    onPatch({
      interface: {
        ...interfaceSettings,
        distanceUnit,
      },
    });
  }

  function handleVerticalSpeedUnitChange(event: Event) {
    const verticalSpeedUnit = (event.target as HTMLSelectElement).value as InterfaceSettings['verticalSpeedUnit'];
    onPatch({
      interface: {
        ...interfaceSettings,
        verticalSpeedUnit,
      },
    });
  }

  function handleTemperatureUnitChange(event: Event) {
    const temperatureUnit = (event.target as HTMLSelectElement).value as InterfaceSettings['temperatureUnit'];
    onPatch({
      interface: {
        ...interfaceSettings,
        temperatureUnit,
      },
    });
  }
</script>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.language')}</h4>
  <div class="setting-row">
    <label class="setting-label" for="lang-select">{$t('settings.language')}</label>
    <select id="lang-select" class="setting-select" value={localeValue} onchange={handleLocaleChange}>
      {#each SUPPORTED_LOCALES as loc}
        <option value={loc.code}>{loc.label}</option>
      {/each}
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="ui-scale">{$t('settings.uiScale')}</label>
    <select id="ui-scale" class="setting-select" value={uiScale} onchange={handleUiScaleChange}>
      <option value={1}>100%</option>
      <option value={1.25}>125%</option>
      <option value={1.5}>150%</option>
    </select>
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.interface')}</h4>
  <div class="setting-row">
    <label class="setting-label" for="speed-unit">{$t('settings.speedUnit')}</label>
    <select id="speed-unit" class="setting-select" value={interfaceSettings.speedUnit} onchange={handleSpeedUnitChange}>
      <option value="kmh">km/h</option>
      <option value="mph">mi/h</option>
      <option value="ms">m/s</option>
      <option value="fts">ft/s</option>
      <option value="kt">kt</option>
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="altitude-unit">{$t('settings.altitudeUnit')}</label>
    <select id="altitude-unit" class="setting-select" value={interfaceSettings.altitudeUnit} onchange={handleAltitudeUnitChange}>
      <option value="m">m</option>
      <option value="ft">ft</option>
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="distance-unit">{$t('settings.distanceUnit')}</label>
    <select id="distance-unit" class="setting-select" value={interfaceSettings.distanceUnit} onchange={handleDistanceUnitChange}>
      <option value="metric">m / km</option>
      <option value="imperial">ft / mi</option>
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="vertical-speed-unit">{$t('settings.verticalSpeedUnit')}</label>
    <select id="vertical-speed-unit" class="setting-select" value={interfaceSettings.verticalSpeedUnit} onchange={handleVerticalSpeedUnitChange}>
      <option value="ms">m/s</option>
      <option value="fts">ft/s</option>
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="temperature-unit">{$t('settings.temperatureUnit')}</label>
    <select id="temperature-unit" class="setting-select" value={interfaceSettings.temperatureUnit} onchange={handleTemperatureUnitChange}>
      <option value="c">°C</option>
      <option value="f">°F</option>
    </select>
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.map')}</h4>
  <div class="setting-row">
    <label class="setting-label" for="map-provider">{$t('settings.tileProvider')}</label>
    <select id="map-provider" class="setting-select" value={mapProvider} onchange={handleMapProviderChange}>
      {#each MAP_PROVIDERS as p}
        <option value={p.id}>{p.label}</option>
      {/each}
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="map-cache">{$t('settings.tileCache')}</label>
    <select id="map-cache" class="setting-select" value={mapCacheMaxMB} onchange={handleCacheSizeChange}>
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
      <button class="cache-clear-btn" onclick={onClearCache} title={$t('settings.clear')}>{$t('settings.clear')}</button>
    </div>
  {/if}
  <div class="setting-row">
    <label class="setting-label" for="cesium-ion-token">Cesium Ion Token</label>
    <input
      id="cesium-ion-token"
      class="setting-input"
      type="password"
      placeholder="(optional – enables 3D terrain)"
      value={cesiumIonToken}
      onchange={(e) => onPatch({ cesiumIonToken: (e.target as HTMLInputElement).value.trim() })}
    />
  </div>
  <p class="setting-hint">
    Free token from <a href="https://ion.cesium.com/signup" target="_blank" rel="noopener">ion.cesium.com</a> — enables 3D terrain in the map view. Restart 3D view after changing.
  </p>
  <div class="setting-row">
    <label class="setting-label" for="altitude-curtain">{$t('settings.altitudeCurtain')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="altitude-curtain" checked={altitudeCurtain3D} onchange={(e) => onPatch({ altitudeCurtain3D: (e.target as HTMLInputElement).checked })} />
      <span class="toggle-slider"></span>
    </label>
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.telemetryRates')}</h4>
  <div class="setting-row">
    <label class="setting-label" for="attitude-rate">{$t('settings.attitude')}</label>
    <select id="attitude-rate" class="setting-select" value={attitudeRateHz} onchange={handleAttitudeRateChange}>
      <option value={1}>1 Hz</option>
      <option value={2}>2 Hz</option>
      <option value={3}>3 Hz</option>
      <option value={5}>5 Hz</option>
    </select>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="position-rate">{$t('settings.gpsPosition')}</label>
    <select id="position-rate" class="setting-select" value={positionRateHz} onchange={handlePositionRateChange}>
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
      <input type="checkbox" id="airspeed-toggle" checked={airspeedEnabled} onchange={handleAirspeedToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.flightLogging')}</h4>
  <div class="setting-row">
    <label class="setting-label" for="flightlog-enabled">{$t('settings.enableFlightLogging')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="flightlog-enabled" checked={flightLoggingEnabled} onchange={handleFlightLoggingToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
  <div class="setting-row">
    <label class="setting-label" for="flightrecord-enabled">{$t('settings.enableFlightRecording')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="flightrecord-enabled" checked={flightRecordingEnabled} onchange={handleFlightRecordingToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
  <div class="setting-row" class:setting-disabled={!flightRecordingEnabled || (!flightLoggingEnabled && flightRecordingEnabled)}>
    <label class="setting-label" for="flightlog-raw">{$t('settings.rawFlightLogs')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="flightlog-raw" checked={flightLogRawEnabled || (!flightLoggingEnabled && flightRecordingEnabled)} disabled={!flightRecordingEnabled || (!flightLoggingEnabled && flightRecordingEnabled)} onchange={handleFlightRawToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
  <div class="setting-row" class:setting-disabled={!flightRecordingEnabled}>
    <label class="setting-label" for="flightlog-raw-always">{$t('settings.continuousRawLogging')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="flightlog-raw-always" checked={flightLogRawAlways} disabled={!flightRecordingEnabled} onchange={handleFlightRawAlwaysToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
  <p class="setting-hint" class:setting-disabled={!flightRecordingEnabled}>{$t('settings.continuousRawHint')}</p>
  <div class="setting-row setting-row-stack">
    <span class="setting-label">{$t('settings.flightLogDbPath')}</span>
    <div class="path-picker-row">
      <input class="setting-input path-input" type="text" readonly value={flightLogDbPath || defaultFlightLogPath || $t('settings.defaultPathUnknown')} />
      <button class="cache-clear-btn" onclick={onChooseFlightLogPath}>{$t('settings.choose')}</button>
      <button class="cache-clear-btn" onclick={onResetFlightLogPath}>{$t('settings.useDefault')}</button>
    </div>
  </div>
  <p class="setting-hint">{$t('settings.flightLogHint')}</p>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.missionControl')}</h4>
  <div class="setting-row">
    <span class="setting-label">{$t('settings.defaultWpAlt')}</span>
    <UnitStepper bind:value={wpAlt} kind="altitude" settings={interfaceSettings} min={1} max={1000} step={5} decimals={0} onchange={onWpAltChange} />
  </div>
  <div class="setting-row">
    <span class="setting-label">{$t('settings.defaultPhTime')}</span>
    <NumberStepper bind:value={phTime} min={1} max={600} step={1} decimals={0} unit="s" onchange={onPhTimeChange} />
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.alerts')}</h4>
  <div class="setting-row">
    <span class="setting-label">{$t('settings.altitude')}</span>
    <UnitStepper bind:value={warnAlt} kind="altitude" settings={interfaceSettings} min={0} max={5000} step={10} decimals={0} onchange={onWarnAltChange} />
  </div>
  <p class="setting-hint">{$t('settings.alertThresholdsHint')}</p>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.hudWidgets')}</h4>
  {#each WIDGET_DEFS as wdef}
    <div class="setting-row">
      <span class="setting-label">{$t(wdef.labelKey)}</span>
      <div class="widget-toggle-group">
        <span class="widget-panel-indicator">{getWidgetPanelLabel(wdef.id)}</span>
        <label class="toggle-switch">
          <input type="checkbox" checked={isWidgetActive(wdef.id)} onchange={() => onToggleWidget(wdef.id)} />
          <span class="toggle-slider"></span>
        </label>
      </div>
    </div>
  {/each}
</section>

<section class="panel-section">
  <p class="setting-hint">{$t('settings.rateHint')}</p>
</section>

<style>
  .panel-section {
    padding-top: 2px;
    border-top: 1px solid #373737;
    margin-top: 10px;
  }

  .section-heading {
    margin: 0 0 8px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
  }

  .setting-disabled {
    opacity: 0.4;
    pointer-events: none;
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

  .setting-hint {
    font-size: 10px;
    color: #666;
    margin: 4px 0 0 0;
    font-style: italic;
  }

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
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: #434343;
    border: 1px solid #555;
    border-radius: 20px;
    transition: background-color 0.2s;
  }

  .toggle-slider::before {
    content: '';
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
</style>