<script lang="ts">
  import { t } from 'svelte-i18n';
  import {
    buildFlightTree,
    formatDurationSec,
    type BlackboxImportProgress,
    type Flight,
    type FlightSummary,
    type FlightTree,
    type LogbookSortMode,
  } from '$lib/stores/flightlog';

  let {
    flightLoggingEnabled,
    logbookMinimized,
    logbookLoading,
    blackboxImporting,
    blackboxImportProgress,
    flightSummaries,
    selectedFlight,
    selectedFlightId,
    selectedFlightTrackCount,
    selectedFlightNotes = $bindable(),
    weatherTempC = $bindable(),
    weatherWindMs = $bindable(),
    weatherWindDir = $bindable(),
    weatherDesc = $bindable(),
    weatherEditing = $bindable(),
    onLoadLogbook,
    onImportBlackbox,
    onSelectFlight,
    onSaveNotes,
    onSaveWeather,
    onDeleteFlight,
  }: {
    flightLoggingEnabled: boolean;
    logbookMinimized: boolean;
    logbookLoading: boolean;
    blackboxImporting: boolean;
    blackboxImportProgress: BlackboxImportProgress | null;
    flightSummaries: FlightSummary[];
    selectedFlight: Flight | null;
    selectedFlightId: number | null;
    selectedFlightTrackCount: number;
    selectedFlightNotes: string;
    weatherTempC: string;
    weatherWindMs: string;
    weatherWindDir: string;
    weatherDesc: string;
    weatherEditing: boolean;
    onLoadLogbook: () => void;
    onImportBlackbox: () => void;
    onSelectFlight: (id: number) => void;
    onSaveNotes: () => void;
    onSaveWeather: () => void;
    onDeleteFlight: () => void;
  } = $props();

  // Internal tree state
  let logbookSortMode = $state<LogbookSortMode>('aircraft-location-date');
  let logbookTreeOpenTop = $state<Set<string>>(new Set());
  let logbookTreeOpenSecond = $state<Set<string>>(new Set());
  let prevLogbookSortMode = $state<LogbookSortMode>('aircraft-location-date');

  const flightTree = $derived<FlightTree>(buildFlightTree(flightSummaries, logbookSortMode));

  // Reset tree open state when sort mode changes
  $effect(() => {
    if (prevLogbookSortMode === logbookSortMode) return;
    prevLogbookSortMode = logbookSortMode;
    logbookTreeOpenTop = new Set();
    logbookTreeOpenSecond = new Set();
  });

  // Tree helpers
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

  // UI helpers
  function autoResizeNotes(el: HTMLTextAreaElement) {
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 140) + 'px';
  }

  function notesAutoSize(el: HTMLTextAreaElement) {
    autoResizeNotes(el);
    return { update() { autoResizeNotes(el); } };
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

  function formatDateTime(value: string): string {
    const d = new Date(value);
    return d.toLocaleString();
  }
</script>

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
        <span class="setting-label">{$t('logbook.notes')}</span>
        <textarea
          class="setting-input notes-input notes-input-auto"
          rows="2"
          readonly
          bind:value={selectedFlightNotes}
          use:notesAutoSize
        ></textarea>
      </div>
      <div class="setting-row">
        <button class="cache-clear-btn" onclick={onSaveNotes}>{$t('logbook.saveNotes')}</button>
        <button class="cache-clear-btn logbook-danger" onclick={onDeleteFlight}>{$t('logbook.deleteFlight')}</button>
      </div>
    </div>
  {:else}
    <div class="setting-row">
      <span class="setting-label">{$t('logbook.sortMode')}</span>
      <select class="setting-select" bind:value={logbookSortMode}>
        <option value="aircraft-location-date">{$t('logbook.sortAircraftLocationDate')}</option>
        <option value="location-date-aircraft">{$t('logbook.sortLocationDateAircraft')}</option>
        <option value="date-location-aircraft">{$t('logbook.sortDateLocationAircraft')}</option>
        <option value="aircraft-date-location">{$t('logbook.sortAircraftDateLocation')}</option>
      </select>
    </div>

    <div class="setting-row">
      <button class="cache-clear-btn" onclick={onLoadLogbook} disabled={logbookLoading}>
        {#if logbookLoading}
          {$t('logbook.loading')}
        {:else}
          {$t('logbook.refresh')}
        {/if}
      </button>
      <button class="cache-clear-btn" onclick={onImportBlackbox} disabled={blackboxImporting}>
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
                              onclick={() => onSelectFlight(f.id)}
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
                <button class="cache-clear-btn weather-save-btn" onclick={onSaveWeather}>{$t('logbook.saveWeather')}</button>
              </div>
            {/if}

            <div class="setting-row setting-row-stack">
              <span class="setting-label">{$t('logbook.notes')}</span>
              <textarea
                class="setting-input notes-input notes-input-auto"
                rows="2"
                bind:value={selectedFlightNotes}
                oninput={(e: Event) => autoResizeNotes(e.target as HTMLTextAreaElement)}
                use:notesAutoSize
              ></textarea>
            </div>
            <div class="setting-row">
              <button class="cache-clear-btn" onclick={onSaveNotes}>{$t('logbook.saveNotes')}</button>
              <button class="cache-clear-btn logbook-danger" onclick={onDeleteFlight}>{$t('logbook.deleteFlight')}</button>
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

<style>
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

  .logbook-detail-minimized {
    border: none;
    background: none;
    padding: 0;
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
</style>
