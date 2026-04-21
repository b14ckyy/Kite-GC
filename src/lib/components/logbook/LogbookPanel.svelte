<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import {
    buildFlightTree,
    formatDurationSec,
    type BlackboxImportProgress,
    type Flight,
    type FlightSummary,
    type FlightTree,
    type LogbookSortMode,
  } from '$lib/stores/flightlog';
  import FlightDetail from './FlightDetail.svelte';

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
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
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
    onSaveCraftName,
    onDeleteFlight,
    onExportFlights,
    onExportBlackbox,
    onExportTrack,
    onImportKflight,
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
    interfaceSettings?: InterfaceSettings;
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
    onSaveCraftName: (name: string) => void;
    onDeleteFlight: () => void;
    onExportFlights: (ids: number[]) => void;
    onExportBlackbox: () => void;
    onExportTrack: () => void;
    onImportKflight: () => void;
  } = $props();

  let logbookSortMode = $state<LogbookSortMode>('aircraft-location-date');
  let logbookTreeOpenTop = $state<Set<string>>(new Set());
  let logbookTreeOpenSecond = $state<Set<string>>(new Set());
  let prevLogbookSortMode = $state<LogbookSortMode>('aircraft-location-date');
  let searchQuery = $state('');
  let multiSelectedIds = $state<Set<number>>(new Set());

  const hasMultiSelection = $derived(multiSelectedIds.size > 0);

  function handleFlightClick(id: number, event: MouseEvent) {
    if (event.ctrlKey || event.metaKey) {
      const next = new Set(multiSelectedIds);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      multiSelectedIds = next;
    } else {
      multiSelectedIds = new Set();
      onSelectFlight(id);
    }
  }

  function isMultiSelected(id: number): boolean {
    return multiSelectedIds.has(id);
  }

  function getExportIds(): number[] {
    if (multiSelectedIds.size > 0) return [...multiSelectedIds];
    if (selectedFlightId != null) return [selectedFlightId];
    return [];
  }

  const hasBlackboxFile = $derived(
    selectedFlight != null && !hasMultiSelection &&
    (selectedFlight.source === 'blackbox' || selectedFlight.source === 'both')
  );

  const filteredSummaries = $derived.by<FlightSummary[]>(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return flightSummaries;
    return flightSummaries.filter((f) => {
      const craft = (f.craft_name || '').toLowerCase();
      const location = (f.location_name || '').toLowerCase();
      const date = (f.start_time || '').toLowerCase();
      const notes = (f.notes || '').toLowerCase();
      return craft.includes(q) || location.includes(q) || date.includes(q) || notes.includes(q);
    });
  });

  const flightTree = $derived<FlightTree>(buildFlightTree(filteredSummaries, logbookSortMode));

  $effect(() => {
    if (prevLogbookSortMode === logbookSortMode) return;
    prevLogbookSortMode = logbookSortMode;
    logbookTreeOpenTop = new Set();
    logbookTreeOpenSecond = new Set();
  });

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

  function formatDateTime(value: string): string {
    const d = new Date(value);
    return d.toLocaleString();
  }

  function flightListMarker(f: FlightSummary): string {
    let marker = '';
    if (f.source === 'blackbox') marker = '◈ ';
    else if (f.source === 'both') marker = '◉ ';
    if (f.linked_flight_id) marker += '🔗 ';
    return marker;
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
    <FlightDetail
      flight={selectedFlight}
      trackCount={selectedFlightTrackCount}
      minimized={true}
      {interfaceSettings}
      bind:notes={selectedFlightNotes}
      bind:weatherEditing
      bind:weatherTempC
      bind:weatherWindMs
      bind:weatherWindDir
      bind:weatherDesc
      {onSaveNotes}
      {onSaveWeather}
      {onSaveCraftName}
      {onDeleteFlight}
      {onExportTrack}
    />
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
      <input
        type="text"
        class="setting-input logbook-search-input"
        placeholder={$t('logbook.searchPlaceholder')}
        bind:value={searchQuery}
      />
      {#if searchQuery}
        <button class="logbook-search-clear" onclick={() => searchQuery = ''} title={$t('logbook.searchClear')}>✕</button>
      {/if}
    </div>

    <div class="setting-row">
      <button class="cache-clear-btn" onclick={onLoadLogbook} disabled={logbookLoading}>
        {#if logbookLoading}
          {$t('logbook.loading')}
        {:else}
          {$t('logbook.refresh')}
        {/if}
      </button>
      <div class="logbook-btn-group-right">
        <div class="logbook-btn-group">
          <button class="cache-clear-btn" onclick={onImportBlackbox} disabled={blackboxImporting}>
            {#if blackboxImporting}
              {$t('logbook.importingBlackbox')}
            {:else}
              {$t('logbook.importBlackbox')}
            {/if}
          </button>
          <button class="cache-clear-btn" onclick={onExportBlackbox} disabled={!hasBlackboxFile}>
            {$t('logbook.exportBlackbox')}
          </button>
        </div>
        <div class="logbook-btn-group">
          <button class="cache-clear-btn" onclick={onImportKflight}>
            {$t('logbook.importKflight')}
          </button>
          <button class="cache-clear-btn" onclick={() => onExportFlights(getExportIds())} disabled={getExportIds().length === 0}>
            {$t('logbook.exportKflight')}
            {#if hasMultiSelection}
              ({multiSelectedIds.size})
            {/if}
          </button>
        </div>
      </div>
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
    {:else if filteredSummaries.length === 0}
      <div class="panel-empty">
        <span class="panel-empty-icon">🔍</span>
        <span>{$t('logbook.noResults')}</span>
      </div>
    {:else}
      <div class="logbook-layout" class:logbook-layout-detail={selectedFlight != null}>
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
                              class:selected={selectedFlightId === f.id && !hasMultiSelection}
                              class:multi-selected={isMultiSelected(f.id)}
                              onclick={(e) => handleFlightClick(f.id, e)}
                            >
                              <div class="logbook-item-title">{flightListMarker(f)}{formatDateTime(f.start_time)} <span class="logbook-item-id">#{f.id}</span></div>
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

        {#if selectedFlight}
          <FlightDetail
            flight={selectedFlight}
            trackCount={selectedFlightTrackCount}
            {interfaceSettings}
            bind:notes={selectedFlightNotes}
            bind:weatherEditing
            bind:weatherTempC
            bind:weatherWindMs
            bind:weatherWindDir
            bind:weatherDesc
            {onSaveNotes}
            {onSaveWeather}
            {onSaveCraftName}
            {onDeleteFlight}
            {onExportTrack}
          />
        {/if}
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

  .logbook-search-input {
    flex: 1;
    min-width: 0;
  }

  .logbook-search-clear {
    background: none;
    border: none;
    color: #777;
    cursor: pointer;
    font-size: 13px;
    padding: 2px 4px;
    line-height: 1;
    flex-shrink: 0;
  }

  .logbook-search-clear:hover {
    color: #e0e0e0;
  }

  .logbook-btn-group-right {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-left: auto;
  }

  .logbook-btn-group {
    display: flex;
    align-items: center;
    gap: 4px;
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
    grid-template-columns: 1fr;
    gap: 12px;
    min-height: 420px;
  }

  .logbook-layout.logbook-layout-detail {
    grid-template-columns: 380px minmax(0, 1fr);
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

  .logbook-item.multi-selected {
    border-color: #37a8db;
    background: rgba(55, 168, 219, 0.12);
    outline: 1px solid rgba(55, 168, 219, 0.4);
    outline-offset: -1px;
  }

  .logbook-item-title {
    font-size: 12px;
    color: #fff;
    font-weight: 600;
  }

  .logbook-item-id {
    font-size: 10px;
    font-weight: 400;
    color: #777;
    margin-left: 4px;
  }

  .logbook-item-meta {
    margin-top: 2px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: 10px;
    color: #aaa;
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

  @media (max-width: 980px) {
    .logbook-layout {
      grid-template-columns: 1fr;
    }
  }
</style>
