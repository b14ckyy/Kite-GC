<script lang="ts">
  import { t, locale } from 'svelte-i18n';
  import { SUPPORTED_LOCALES } from '$lib/i18n';
  import { MAP_PROVIDERS } from '$lib/config/mapProviders';
  import { WIDGET_DEFS } from '$lib/config/widgetRegistry';
  import type { AppSettings } from '$lib/stores/settings';
  import type { TileCacheStats } from '$lib/cache/tileCache';

  let {
    localeValue = 'en',
    mapProvider = 'osm',
    mapCacheMaxMB = 200,
    cacheStats = { usedBytes: 0, maxBytes: 0, tileCount: 0 },
    cesiumIonToken = '',
    attitudeRateHz = 5,
    positionRateHz = 2,
    airspeedEnabled = false,
    flightLoggingEnabled = false,
    flightLogRawEnabled = false,
    flightLogDbPath = '',
    defaultFlightLogPath = '',
    defaultWpAltitudeM = 50,
    defaultPhTimeSec = 30,
    warnAltitudeM = 120,
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
    mapProvider?: string;
    mapCacheMaxMB?: number;
    cacheStats?: TileCacheStats;
    cesiumIonToken?: string;
    attitudeRateHz?: number;
    positionRateHz?: number;
    airspeedEnabled?: boolean;
    flightLoggingEnabled?: boolean;
    flightLogRawEnabled?: boolean;
    flightLogDbPath?: string;
    defaultFlightLogPath?: string;
    defaultWpAltitudeM?: number;
    defaultPhTimeSec?: number;
    warnAltitudeM?: number;
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

  function handleFlightRawToggle(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    onPatch({ flightLogRawEnabled: checked });
  }

  function decWpAltitude() {
    const value = Math.max(1, defaultWpAltitudeM - 1);
    onPatch({ defaultWpAltitudeM: value });
  }

  function incWpAltitude() {
    const value = Math.min(1000, defaultWpAltitudeM + 1);
    onPatch({ defaultWpAltitudeM: value });
  }

  function onWpAltitudeInput(event: Event) {
    const value = Number((event.target as HTMLInputElement).value);
    const clamped = Math.max(1, Math.min(1000, value));
    onPatch({ defaultWpAltitudeM: clamped });
  }

  function decPhTime() {
    const value = Math.max(1, defaultPhTimeSec - 1);
    onPatch({ defaultPhTimeSec: value });
  }

  function incPhTime() {
    const value = Math.min(600, defaultPhTimeSec + 1);
    onPatch({ defaultPhTimeSec: value });
  }

  function onPhTimeInput(event: Event) {
    const value = Number((event.target as HTMLInputElement).value);
    const clamped = Math.max(1, Math.min(600, value));
    onPatch({ defaultPhTimeSec: clamped });
  }

  function decWarnAlt() {
    const value = Math.max(0, warnAltitudeM - 10);
    onPatch({ warnAltitudeM: value });
  }

  function incWarnAlt() {
    const value = Math.min(5000, warnAltitudeM + 10);
    onPatch({ warnAltitudeM: value });
  }

  function onWarnAltInput(event: Event) {
    const value = Number((event.target as HTMLInputElement).value);
    const clamped = Math.max(0, Math.min(5000, value));
    onPatch({ warnAltitudeM: clamped });
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
    <label class="setting-label" for="flightlog-raw">{$t('settings.rawFlightLogs')}</label>
    <label class="toggle-switch">
      <input type="checkbox" id="flightlog-raw" checked={flightLogRawEnabled} onchange={handleFlightRawToggle} />
      <span class="toggle-slider"></span>
    </label>
  </div>
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
    <div class="setting-stepper">
      <button class="stepper-btn" onclick={decWpAltitude}>-</button>
      <input type="number" class="stepper-input" min="1" max="1000" value={defaultWpAltitudeM} onchange={onWpAltitudeInput} />
      <button class="stepper-btn" onclick={incWpAltitude}>+</button>
      <span class="setting-unit">m</span>
    </div>
  </div>
  <div class="setting-row">
    <span class="setting-label">{$t('settings.defaultPhTime')}</span>
    <div class="setting-stepper">
      <button class="stepper-btn" onclick={decPhTime}>-</button>
      <input type="number" class="stepper-input" min="1" max="600" value={defaultPhTimeSec} onchange={onPhTimeInput} />
      <button class="stepper-btn" onclick={incPhTime}>+</button>
      <span class="setting-unit">s</span>
    </div>
  </div>
</section>

<section class="panel-section">
  <h4 class="section-heading">{$t('settings.alerts')}</h4>
  <div class="setting-row">
    <span class="setting-label">{$t('settings.altitude')}</span>
    <div class="setting-stepper">
      <button class="stepper-btn" onclick={decWarnAlt}>-</button>
      <input type="number" class="stepper-input" min="0" max="5000" step="10" value={warnAltitudeM} onchange={onWarnAltInput} />
      <button class="stepper-btn" onclick={incWarnAlt}>+</button>
      <span class="setting-unit">m</span>
    </div>
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