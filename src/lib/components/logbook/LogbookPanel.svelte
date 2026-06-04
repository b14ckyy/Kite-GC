<script lang="ts">
  // Flight Logbook on the new panel framework (docs/dev/PANEL_FRAMEWORK.md). Three live formats:
  //   • info     — minimized flight card (click to expand)
  //   • compact  — the (flight or battery) list only
  //   • advanced — flight list + FlightDetail (1:2 split); the Battery Manager subview uses
  //                advanced as a wide *single* column (it renders its own list/detail split).
  // Pure presentation port of LogbookPanel: identical tree/search/multi-select/battery logic,
  // reusing the same stores/controllers — only the chrome moves onto PanelShell.
  import { untrack } from 'svelte';
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
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import FlightDetail from './FlightDetail.svelte';
  import BatteryManager from './BatteryManager.svelte';
  import { batteryManagerOpen } from '$lib/stores/batteryManager';

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
    onExpand,
    onLoadLogbook,
    onImportBlackbox,
    onSelectFlight,
    onSaveNotes,
    onSaveWeather,
    onSaveCraftName,
    onSavePilot,
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
    onExpand?: () => void;
    onLoadLogbook: () => void;
    onImportBlackbox: () => void;
    onSelectFlight: (id: number) => void;
    onSaveNotes: () => void;
    onSaveWeather: () => void;
    onSaveCraftName: (name: string) => void;
    onSavePilot: (pilotName: string, pilotId: string) => void;
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

  // Shell variant for the flight view (the battery library is its own panel — BatteryManager).
  const fullView = $derived(selectedFlight != null);
  const minimizedInfo = $derived(flightLoggingEnabled && logbookMinimized && selectedFlight != null);
  const flightDetailColumn = $derived(selectedFlight != null);
  const variant = $derived<PanelVariant>(
    minimizedInfo ? 'info' : (fullView ? 'advanced' : 'compact')
  );

  // When a flight becomes selected (e.g. jumped to from a battery's linked-flights list), expand
  // the tree groups that contain it so the highlighted row is actually visible. Reads the open
  // sets untracked so it only fires on selection/tree changes, not when the user toggles a group.
  $effect(() => {
    const fid = selectedFlightId;
    const tree = flightTree;
    if (fid == null) return;
    let topId: string | null = null;
    let secondId: string | null = null;
    for (const top of tree.groups) {
      for (const second of top.children) {
        if (second.flights.some((f) => f.id === fid)) {
          topId = treeTopId(top.key);
          secondId = treeSecondId(top.key, second.key);
          break;
        }
      }
      if (topId) break;
    }
    if (!topId || !secondId) return;
    const tId = topId, sId = secondId;
    untrack(() => {
      if (!logbookTreeOpenTop.has(tId)) {
        const n = new Set(logbookTreeOpenTop); n.add(tId); logbookTreeOpenTop = n;
      }
      if (!logbookTreeOpenSecond.has(sId)) {
        const n = new Set(logbookTreeOpenSecond); n.add(sId); logbookTreeOpenSecond = n;
      }
    });
  });

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

  // Scroll the selected flight row into view (e.g. after jumping from a battery / its group was
  // just auto-expanded). `block: 'nearest'` is a no-op when it's already visible.
  function revealSelected(node: HTMLElement, isSel: boolean) {
    if (isSel) node.scrollIntoView({ block: 'nearest' });
    return { update(s: boolean) { if (s) node.scrollIntoView({ block: 'nearest' }); } };
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

<!-- Toolbar button groups, on the shared control library (docs/dev/PANEL_FRAMEWORK.md):
     import/export are data-transfer actions (`data` variant); the Batteries toggle is `standard`. -->
{#snippet leftGroup()}
  <div class="tb-left">
    <Button variant="standard" icon="battery" onclick={() => batteryManagerOpen.set(true)}>{$t('batteryMgr.toBatteries')}</Button>
  </div>
{/snippet}

{#snippet importGroup()}
  <div class="tb-right">
    <Button variant="data" icon="import" disabled={blackboxImporting} onclick={onImportBlackbox}>
      {blackboxImporting ? $t('logbook.importingBlackbox') : $t('logbook.importBlackbox')}
    </Button>
    <Button variant="data" icon="import" onclick={onImportKflight}>{$t('logbook.importKflight')}</Button>
  </div>
{/snippet}

{#snippet exportGroup()}
  <div class="tb-right">
    <Button variant="data" icon="export" disabled={!hasBlackboxFile} onclick={onExportBlackbox}>
      {$t('logbook.exportBlackbox')}
    </Button>
    <Button variant="data" icon="export" disabled={getExportIds().length === 0} onclick={() => onExportFlights(getExportIds())}>
      {$t('logbook.exportKflight')}{#if hasMultiSelection} ({multiSelectedIds.size}){/if}
    </Button>
  </div>
{/snippet}

<!-- Main toolbar: sort/group select, search, then the button row. In the flight-detail (advanced)
     view the export group moves to the detail column's toolbar; otherwise it stays here. -->
{#snippet mainToolbar()}
  <div class="lbv2-toolstack">
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

    <div class="logbook-toolbtns">
      {@render leftGroup()}
      {@render importGroup()}
    </div>
  </div>
{/snippet}

{#snippet detailExportToolbar()}
  <div class="lbv2-detail-toolbar">{@render exportGroup()}</div>
{/snippet}

<!-- Main field: progress + empty states + flight tree. -->
{#snippet mainBody()}
  {#if !flightLoggingEnabled}
    <div class="panel-empty">
      <span class="panel-empty-icon">⊘</span>
      <span>{$t('logbook.disabled')}</span>
    </div>
  {:else}
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
                            use:revealSelected={selectedFlightId === f.id && !hasMultiSelection}
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
    {/if}
  {/if}
{/snippet}

<!-- Info variant body: the minimized flight card (click anywhere to expand). -->
{#snippet infoBody()}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="lbv2-mini" onclick={() => onExpand?.()}>
    {#if selectedFlight}
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
        {onSavePilot}
        {onDeleteFlight}
        {onExportTrack}
      />
    {/if}
  </div>
{/snippet}

<!-- Detail (right) column: the full FlightDetail for the selected flight. -->
{#snippet flightDetail()}
  <div class="lbv2-detailwrap">
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
        {onSavePilot}
        {onDeleteFlight}
        {onExportTrack}
      />
    {/if}
  </div>
{/snippet}

{#if $batteryManagerOpen}
  <BatteryManager onBack={() => batteryManagerOpen.set(false)} />
{:else}
  <div class="lbv2">
    <PanelShell
      {variant}
      title={$t('logbook.title')}
      body={variant === 'info' ? infoBody : mainBody}
      toolbar={variant === 'info' ? undefined : mainToolbar}
      detail={flightDetailColumn ? flightDetail : undefined}
      detailToolbar={flightDetailColumn ? detailExportToolbar : undefined}
    />
  </div>
{/if}

<style>
  /* The shell + its embedded list/detail/subviews are all reachable from this wrapper, so the
     few cross-component frame overrides below stay scoped to this panel. */

  /* The tree list sits directly in the shell's framed field → drop its own frame/max-height. */
  .lbv2 :global(.logbook-list) {
    max-height: none;
    border: none;
    background: none;
    padding: 0;
    overflow: visible;
  }
  /* FlightDetail brings its own frame; the shell field already frames the detail column. */
  .lbv2 :global(.logbook-detail) {
    border: none;
    background: none;
    padding: 0;
    max-height: none;
    overflow: visible;
  }

  .lbv2-mini {
    cursor: pointer;
  }
  .lbv2-detailwrap {
    height: 100%;
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

  /* The toolbar slot is a flex row; stack our rows (sort · search · buttons) full-width so each
     fills the panel and aligns with the list field below. */
  .lbv2-toolstack {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }

  /* Toolbar button row — left group + import/export, wraps gracefully at narrow widths. */
  .logbook-toolbtns {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 6px;
  }
  .tb-left { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .tb-right { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }

  /* Detail (right) column toolbar: export buttons flush right. */
  .lbv2-detail-toolbar { display: flex; flex: 1; justify-content: flex-end; }

  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }

  .setting-label {
    font-size: 12px;
    color: #e0e0e0;
  }

  /* Form controls match the md button height (28px) so toolbars align cleanly (no vertical jog). */
  .setting-select {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
    min-width: 70px;
  }

  .setting-input {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
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
</style>
