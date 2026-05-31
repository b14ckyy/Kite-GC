<!-- InavMissionPanel.svelte
     Sidebar panel for INAV MSP mission planning.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    mission, selectedWpIndex, selectedWpIndices, editMode,
    selectWpSingle, toggleWpSelection, selectWpRange, clearWpSelection, removeSelectedWps,
    missionClear,
    missionDownload, missionUpload,
    missionExportXml, missionImportXml,
    missionSaveFile, missionLoadFile,
    activeMissionIndex, missionCount,
    switchMission, addMission, removeMission, getTotalWpCount,
    canUndo, canRedo, undo, redo,
    MAX_MISSIONS, MAX_WAYPOINTS_TOTAL,
    type Waypoint, type Mission, WpAction, WP_ACTION_LABELS, WP_ACTION_KEYS,
    hasLocation, isModifier,
  } from '$lib/stores/mission';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import { contextMenu } from '$lib/actions/contextMenu';
  import { buildWaypointMenu } from '$lib/helpers/waypointMenu';
  import { connection } from '$lib/stores/connection';
  import { telemetry, type TelemetryData } from '$lib/stores/telemetry';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { convertAltitude, convertSpeed } from '$lib/utils/units';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { t } from 'svelte-i18n';
  // Helper: untyped $t wrapper for dynamic params (svelte-i18n types are too strict)
  function _t(id: string, params?: Record<string, string>): string {
    return (get(t) as any)(id, { values: params });
  }

  let currentMission = $state<Mission>(get(mission));
  let currentSelIdx = $state<number>(get(selectedWpIndex));
  let currentSel = $state<Set<number>>(get(selectedWpIndices));
  let selAnchor = -1;
  let currentEditing = $state<boolean>(get(editMode));
  let currentMissionIdx = $state<number>(get(activeMissionIndex));
  let currentMissionCount = $state<number>(get(missionCount));
  let canUndoNow = $state<boolean>(get(canUndo));
  let canRedoNow = $state<boolean>(get(canRedo));

  // Lazy pattern panel state (to avoid loading heavy module on startup)
  let showPatternPanel = $state(false);

  const unsubMission = mission.subscribe(m => { currentMission = m; });
  const unsubSelIdx = selectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubSel = selectedWpIndices.subscribe(s => { currentSel = s; });
  const unsubEditMode = editMode.subscribe(e => {
    currentEditing = e;
    if (!e) {
      clearWpSelection(); // multi-select is edit-mode only
      if (showPatternPanel) {
        showPatternPanel = false;
        import('$lib/stores/surveyPattern.svelte').then(m => m.exitPatternMode());
      }
    }
  });
  const unsubMissionIdx = activeMissionIndex.subscribe(i => { currentMissionIdx = i; });
  const unsubMissionCount = missionCount.subscribe(c => { currentMissionCount = c; });
  const unsubCanUndo = canUndo.subscribe(v => { canUndoNow = v; });
  const unsubCanRedo = canRedo.subscribe(v => { canRedoNow = v; });

  onDestroy(() => { unsubMission(); unsubSelIdx(); unsubSel(); unsubEditMode(); unsubTelem(); unsubMissionIdx(); unsubMissionCount(); unsubCanUndo(); unsubCanRedo(); });

  // Keyboard: Ctrl+Z = undo, Ctrl+Y / Ctrl+Shift+Z = redo. Edit-mode only and
  // not while a text field is focused (so native input undo keeps working).
  function onKeydown(e: KeyboardEvent) {
    if (!currentEditing || showPatternPanel) return;
    if (!(e.ctrlKey || e.metaKey)) return;
    const tgt = e.target as HTMLElement | null;
    const tag = tgt?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tgt?.isContentEditable) return;
    const k = e.key.toLowerCase();
    if (k === 'z' && !e.shiftKey) { e.preventDefault(); undo(); }
    else if ((k === 'z' && e.shiftKey) || k === 'y') { e.preventDefault(); redo(); }
  }

  let confirmDialog: ReturnType<typeof ConfirmDialog>;

  let downloadLoading = $state(false);
  let uploadLoading = $state(false);
  let eepromSaveLoading = $state(false);
  let eepromLoadLoading = $state(false);
  let statusMessage = $state('');
  let dragOver = $state(false);
  let currentTelem = $state<TelemetryData>(get(telemetry));
  const unsubTelem = telemetry.subscribe(t => { currentTelem = t; });

  const ARMING_FLAG_ARMED = 2;

  function isConnected(): boolean {
    return get(connection)?.status === 'connected';
  }

  function isArmed(): boolean {
    return currentTelem.lastUpdate > 0 && (currentTelem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  }

  async function handleDownload() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    downloadLoading = true; statusMessage = '';
    try {
      const m = await missionDownload(false);
      statusMessage = $t('mission.downloaded', { values: { count: m.waypoints.length } });
    } catch (e: any) {
      statusMessage = $t('mission.downloadFailed', { values: { error: e } });
    } finally { downloadLoading = false; }
  }

  async function handleUpload() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    uploadLoading = true; statusMessage = '';
    try {
      const m = await missionUpload(false);
      statusMessage = $t('mission.uploaded', { values: { count: m.waypoints.length } });
    } catch (e: any) {
      statusMessage = $t('mission.uploadFailed', { values: { error: e } });
    } finally { uploadLoading = false; }
  }

  async function handleEepromSave() {
    if (!isConnected() || isArmed()) { statusMessage = isArmed() ? $t('mission.eepromSaveArmedMsg') : $t('mission.notConnected'); return; }
    eepromSaveLoading = true; statusMessage = '';
    try {
      const m = await missionUpload(true);
      statusMessage = $t('mission.eepromSaved', { values: { count: m.waypoints.length } });
    } catch (e: any) {
      statusMessage = $t('mission.eepromSaveFailed', { values: { error: e } });
    } finally { eepromSaveLoading = false; }
  }

  async function handleEepromLoad() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    eepromLoadLoading = true; statusMessage = '';
    try {
      const m = await missionDownload(true);
      statusMessage = $t('mission.eepromLoaded', { values: { count: m.waypoints.length } });
    } catch (e: any) {
      statusMessage = $t('mission.eepromLoadFailed', { values: { error: e } });
    } finally { eepromLoadLoading = false; }
  }

  async function handleSaveFile() {
    try {
      const path = await save({ title: $t('mission.saveMissionTitle'), defaultPath: 'mission.mission', filters: [{ name: 'Mission', extensions: ['mission'] }] });
      if (!path) return;
      await missionSaveFile(path);
      statusMessage = $t('mission.missionSaved');
    } catch (e: any) {
      statusMessage = $t('mission.saveFailed', { values: { error: e } });
    }
  }

  async function handleOpenFile() {
    try {
      const path = await open({ title: $t('mission.openMissionTitle'), multiple: false, filters: [{ name: 'Mission', extensions: ['mission'] }] });
      if (!path) return;
      const m = await missionLoadFile(typeof path === 'string' ? path : path);
      statusMessage = $t('mission.loaded', { values: { count: m.waypoints.length } });
    } catch (e: any) {
      statusMessage = $t('mission.openFailed', { values: { error: e } });
    }
  }

  function onDragOver(e: DragEvent) { e.preventDefault(); dragOver = true; }
  function onDragLeave() { dragOver = false; }
  async function onDrop(e: DragEvent) {
    e.preventDefault(); dragOver = false;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    const file = files[0];
    if (!file.name.endsWith('.mission')) { statusMessage = $t('mission.onlyMissionFiles'); return; }
    try {
      const xml = await file.text();
      if (!xml.includes('<mission')) { statusMessage = $t('mission.invalidMissionFile'); return; }
      const m = await missionImportXml(xml);
      statusMessage = $t('mission.loadedFromFile', { values: { count: m.waypoints.length, file: file.name } });
    } catch (e: any) {
      statusMessage = $t('mission.importFailed', { values: { error: e } });
    }
  }

  async function handleClear() {
    if (currentMission.waypoints.length > 0) {
      const ans = await confirmDialog.show({
        title: $t('mission.clearConfirmTitle'),
        message: $t('mission.clearConfirmMsg'),
        buttons: [{ label: $t('mission.clearConfirmYes'), value: 'clear', danger: true }],
      });
      if (ans !== 'clear') return;
    }
    await removeMission(currentMissionIdx);
    statusMessage = $t('mission.missionCleared');
  }
  // List selection. Plain click = single; Ctrl/⌘ = toggle; Shift = range; a tap
  // on the number badge toggles too (touch-friendly). Multi-select gestures are
  // edit-mode only.
  function onRowClick(e: MouseEvent, i: number) {
    if (currentEditing && e.shiftKey && selAnchor >= 0) {
      selectWpRange(selAnchor, i);
    } else if (currentEditing && (e.ctrlKey || e.metaKey)) {
      toggleWpSelection(i);
      selAnchor = i;
    } else {
      selectWpSingle(i);
      selAnchor = i;
    }
  }
  function onBadgeClick(e: MouseEvent, i: number) {
    e.stopPropagation();
    if (currentEditing) toggleWpSelection(i);
    else selectWpSingle(i);
    selAnchor = i;
  }
  // Right-click a row: if it isn't part of the selection, select it alone; then
  // build the menu over the current selection (so it acts on all selected).
  function wpMenuFor(i: number) {
    if (!currentSel.has(i)) selectWpSingle(i);
    return buildWaypointMenu();
  }
  async function removeSelected() { if (currentSel.size > 0) await removeSelectedWps(); }

  async function handleGenerateAppend() {
    // Button moved to SurveyPatternPanel; this is a no-op fallback
  }

  const IFACE_FALLBACK: InterfaceSettings = {
    speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c',
  };
  const iface = $derived($settings.interface ?? IFACE_FALLBACK);
  /** Waypoint speed (cm/s internal) in the user's speed unit. */
  function fmtSpeed(cms: number): string {
    const s = convertSpeed(cms / 100, iface.speedUnit);
    return `${s.value.toFixed(1)} ${s.unit}`;
  }

  function formatAltShort(wp: Waypoint): string {
    const m = wp.alt_mode ?? ((wp.p3 & 1) ? 1 : 0);
    const ref = m === 2 ? 'AGL' : m === 1 ? 'AMSL' : 'REL';
    const a = convertAltitude(wp.altitude / 100, iface.altitudeUnit);
    return `${Math.round(a.value)}${a.unit} ${ref}`;
  }

  function formatParam(wp: Waypoint): string {
    switch (wp.action) {
      case WpAction.PosholdTime: return `${wp.p1}s`;
      case WpAction.Jump:        return `→${wp.p1} ×${wp.p2 === -1 ? '∞' : wp.p2}`;
      case WpAction.SetHead:     return wp.p1 === -1 ? 'Free' : `${wp.p1}°`;
      case WpAction.Waypoint:
      case WpAction.Land:        return wp.p1 > 0 ? fmtSpeed(wp.p1) : '';
      default:                   return '';
    }
  }

  function formatCoord(val: number): string { return (val / 1e7).toFixed(6); }

  function shortType(action: WpAction): string {
    switch (action) {
      case WpAction.Waypoint:    return 'WPT';
      case WpAction.PosholdUnlim: return 'PH∞';
      case WpAction.PosholdTime:  return 'PHT';
      case WpAction.Rth:         return 'RTH';
      case WpAction.SetPoi:      return 'POI';
      case WpAction.Jump:        return 'JMP';
      case WpAction.SetHead:     return 'HDG';
      case WpAction.Land:        return 'LND';
      default:                   return '?';
    }
  }

  function buildDisplayNumbers(waypoints: Waypoint[]): Map<number, number> {
    const nums = new Map<number, number>();
    let dn = 1;
    for (let i = 0; i < waypoints.length; i++) {
      if (!isModifier(waypoints[i].action)) nums.set(i, dn++);
    }
    return nums;
  }

  function findMissionEndIndex(waypoints: Waypoint[]): number {
    for (let i = 0; i < waypoints.length; i++) {
      if (waypoints[i].action === WpAction.Land || waypoints[i].action === WpAction.Rth) return i;
    }
    return -1;
  }

  const displayNums = $derived(buildDisplayNumbers(currentMission.waypoints));
  const missionEndIdx = $derived(findMissionEndIndex(currentMission.waypoints));
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="mission-panel"
  class:pattern-mode={showPatternPanel}
  ondragover={onDragOver}
  ondragleave={onDragLeave}
  ondrop={onDrop}
  class:drag-over={dragOver}
>
  <div class="mission-toolbar">
    <button class="btn-edit" class:active={currentEditing} onclick={() => editMode.update(v => !v)} title={$t('mission.toggleEdit')}>
      ✏️ {currentEditing ? $t('mission.editing') : $t('mission.edit')}
    </button>

    {#if currentEditing && !showPatternPanel}
      <button class="btn-icon" onclick={() => undo()} disabled={!canUndoNow} title={$t('mission.undo')} aria-label={$t('mission.undo')}>↶</button>
      <button class="btn-icon" onclick={() => redo()} disabled={!canRedoNow} title={$t('mission.redo')} aria-label={$t('mission.redo')}>↷</button>
    {/if}

    {#if currentEditing}
      <button
        class="btn-pattern"
        class:active={showPatternPanel}
        onclick={() => {
          if (showPatternPanel) {
            // Toggle OFF: exit pattern mode
            import('$lib/stores/surveyPattern.svelte').then(m => {
              m.exitPatternMode();
              showPatternPanel = false;
            });
          } else {
            // Toggle ON: enter pattern mode
            import('$lib/stores/surveyPattern.svelte').then(m => {
              m.enterPatternMode('rectangle');
              showPatternPanel = true;
            });
          }
        }}
      >
        🗺️ {$t('survey.pattern')}
      </button>
    {/if}

    <div class="toolbar-spacer"></div>
    {#if currentEditing && currentSel.size > 0}
      <button class="btn-sm btn-danger" onclick={removeSelected} title={$t('mission.removeWp')}>✕</button>
    {/if}
    <button class="btn-sm" onclick={handleClear} title={$t('mission.clearMission')}>🗑️</button>
  </div>

  <div class="wp-frame">
    <div class="mission-tabs">
      {#each Array.from({length: currentMissionCount}, (_, i) => i + 1) as n}
        <button class="mission-tab" class:active={currentMissionIdx === n} onclick={() => switchMission(n)}>{n}</button>
      {/each}
      {#if currentMissionCount < MAX_MISSIONS}
        <button class="mission-tab mission-tab-add" onclick={() => { const idx = addMission(); if (idx > 0) switchMission(idx); }} title={$t('mission.addMission')}>+</button>
      {/if}
    </div>

    {#if showPatternPanel}
      {#await import('./SurveyPatternPanel.svelte')}
        <div class="pattern-loading">{$t('survey.loading')}</div>
      {:then { default: SurveyPatternPanel }}
        <SurveyPatternPanel ongenerate={() => { showPatternPanel = false; }} />
      {:catch error}
        <div class="pattern-error">
          {_t('survey.error', { error: String(error?.message || error) })}
          <button onclick={() => showPatternPanel = false}>Schließen</button>
        </div>
      {/await}
    {:else}
      <div class="wp-table-wrap">
        {#if currentMission.waypoints.length === 0}
          <div class="wp-empty">{currentEditing ? $t('mission.emptyEdit') : $t('mission.emptyView')}</div>
        {:else}
          <table class="wp-table">
            <thead>
              <tr>
                <th class="col-num">{$t('mission.colNumber')}</th>
                <th class="col-type">{$t('mission.colType')}</th>
                <th class="col-alt">{$t('mission.colAlt')}</th>
                <th class="col-param">{$t('mission.colParam')}</th>
              </tr>
            </thead>
            <tbody>
              {#each currentMission.waypoints as wp, i}
                {#if isModifier(wp.action)}
                  <tr class="wp-row modifier" class:selected={currentSel.has(i)} class:greyed={missionEndIdx >= 0 && i > missionEndIdx} onclick={(e) => onRowClick(e, i)} use:contextMenu={() => wpMenuFor(i)}>
                    <td class="col-num"></td>
                    <td class="col-type mod-indent">↳ {shortType(wp.action)}</td>
                    <td class="col-alt">—</td>
                    <td class="col-param">{formatParam(wp)}</td>
                  </tr>
                {:else}
                  <tr class="wp-row" class:selected={currentSel.has(i)} class:greyed={missionEndIdx >= 0 && i > missionEndIdx} onclick={(e) => onRowClick(e, i)} use:contextMenu={() => wpMenuFor(i)}>
                    <!-- svelte-ignore a11y_click_events_have_key_events -->
                    <!-- svelte-ignore a11y_no_static_element_interactions -->
                    <td class="col-num"><span class="wp-num-badge" onclick={(e) => onBadgeClick(e, i)}>{displayNums.get(i) ?? ''}</span></td>
                    <td class="col-type">{shortType(wp.action)}</td>
                    <td class="col-alt">{hasLocation(wp.action) ? formatAltShort(wp) : '—'}</td>
                    <td class="col-param">{formatParam(wp)}</td>
                  </tr>
                {/if}
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    {/if}
  </div>

  {#if currentSelIdx >= 0 && currentSelIdx < currentMission.waypoints.length}
    {@const wp = currentMission.waypoints[currentSelIdx]}
    <div class="wp-detail">
      <div class="detail-header">{isModifier(wp.action) ? '' : `WP ${displayNums.get(currentSelIdx) ?? ''} — `}{$t(WP_ACTION_KEYS[wp.action])}</div>
      {#if hasLocation(wp.action)}
        <div class="detail-row"><span class="detail-label">{$t('mission.lat')}</span><span class="detail-value">{formatCoord(wp.lat)}</span></div>
        <div class="detail-row"><span class="detail-label">{$t('mission.lon')}</span><span class="detail-value">{formatCoord(wp.lon)}</span></div>
      {/if}
      <div class="detail-row"><span class="detail-label">{$t('mission.alt')}</span><span class="detail-value">{formatAltShort(wp)}</span></div>
      {#if wp.action === WpAction.Waypoint || wp.action === WpAction.Land}
        <div class="detail-row"><span class="detail-label">{$t('mission.speed')}</span><span class="detail-value">{wp.p1 > 0 ? fmtSpeed(wp.p1) : $t('mission.speedDefault')}</span></div>
      {/if}
      {#if wp.action === WpAction.PosholdTime}
        <div class="detail-row"><span class="detail-label">{$t('mission.hold')}</span><span class="detail-value">{wp.p1}s</span></div>
      {/if}
      {#if wp.action === WpAction.PosholdUnlim}
        <div class="detail-row"><span class="detail-label">{$t('mission.hold')}</span><span class="detail-value">{$t('mission.holdUnlimited')}</span></div>
      {/if}
      {#if wp.action === WpAction.Rth}
        <div class="detail-row"><span class="detail-label">{$t('mission.land')}</span><span class="detail-value">{wp.p1 ? $t('mission.landYes') : $t('mission.landNoHover')}</span></div>
      {/if}
      {#if wp.action === WpAction.Jump}
        <div class="detail-row"><span class="detail-label">{$t('mission.target')}</span><span class="detail-value">WP {wp.p1}</span></div>
        <div class="detail-row"><span class="detail-label">{$t('mission.repeat')}</span><span class="detail-value">{wp.p2 === -1 ? '∞' : wp.p2}</span></div>
      {/if}
      {#if wp.action === WpAction.SetHead}
        <div class="detail-row"><span class="detail-label">{$t('mission.heading')}</span><span class="detail-value">{wp.p1 === -1 ? $t('mission.headingFree') : `${wp.p1}°`}</span></div>
      {/if}
      <div class="detail-row"><span class="detail-label">{$t('mission.flag')}</span><span class="detail-value">{wp.flag === 0xa5 ? $t('mission.flagLast') : wp.flag === 0x48 ? $t('mission.flagFbh') : $t('mission.flagNormal')}</span></div>
      {#if currentEditing}<div class="detail-hint">{$t('mission.clickMarkerHint')}</div>{/if}
    </div>
  {/if}

  <div class="mission-bottom">
    {#if !showPatternPanel}
      <div class="mission-controls">
        <div class="ctrl-row">
          <button class="btn-ctrl" onclick={handleDownload} disabled={downloadLoading}>{downloadLoading ? '⏳' : '⬇️'} {$t('mission.fcDownload')}</button>
          <button class="btn-ctrl" onclick={handleUpload} disabled={uploadLoading}>{uploadLoading ? '⏳' : '⬆️'} {$t('mission.fcUpload')}</button>
        </div>
        <div class="ctrl-row">
          <button class="btn-ctrl btn-eeprom" onclick={handleEepromLoad} disabled={eepromLoadLoading}>{eepromLoadLoading ? '⏳' : '💾'} {$t('mission.eepromLoad')}</button>
          <button class="btn-ctrl btn-eeprom" onclick={handleEepromSave} disabled={eepromSaveLoading || isArmed()} title={isArmed() ? $t('mission.eepromSaveArmed') : $t('mission.eepromSaveTooltip')}>{eepromSaveLoading ? '⏳' : '💾'} {$t('mission.eepromSave')}</button>
        </div>
        <div class="ctrl-row">
          <button class="btn-ctrl btn-file" onclick={handleOpenFile}>📂 {$t('mission.open')}</button>
          <button class="btn-ctrl btn-file" onclick={handleSaveFile}>💾 {$t('mission.save')}</button>
        </div>
      </div>
    {/if}
  </div>

  {#if statusMessage}<div class="mission-status">{statusMessage}</div>{/if}

  {#if currentMission.waypoints.length > 0}
    <div class="mission-summary">
      {#if currentMissionCount > 1}
        M{currentMissionIdx}: {currentMission.waypoints.length} WPs | Total: {getTotalWpCount()}/{MAX_WAYPOINTS_TOTAL}
      {:else}
        {currentMission.waypoints.length}/{MAX_WAYPOINTS_TOTAL} WPs
      {/if}
      {#if currentMission.dirty}<span class="dirty-badge">{$t('mission.modified')}</span>{/if}
    </div>
  {/if}

  {#if dragOver}<div class="drop-overlay">{$t('mission.dropHint')}</div>{/if}
</div>

<ConfirmDialog bind:this={confirmDialog} />

<style>
  .mission-panel { display: flex; flex-direction: column; gap: 0; flex: 1; min-height: 0; padding: 4px; position: relative; overflow: hidden; color-scheme: dark; font-size: 13px; }
  .mission-panel.pattern-mode { overflow-y: auto; }
  .mission-panel.drag-over { outline: 2px dashed #37a8db; outline-offset: -2px; }
  .mission-toolbar { display: flex; align-items: center; gap: 4px; padding: 2px 4px; flex-shrink: 0; margin-bottom: 4px; }
  .toolbar-spacer { flex: 1; }
  .btn-edit { padding: 4px 10px; border: 1px solid #555; border-radius: 4px; background: #2a2a2a; color: #ccc; cursor: pointer; font-size: 13px; }
  .btn-edit.active { background: #37a8db; color: #fff; border-color: #37a8db; }

  .btn-pattern {
    padding: 4px 10px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #3a3a3a;
    color: #ccc;
    cursor: pointer;
    font-size: 13px;
    margin-left: 6px;
  }
  .btn-pattern:hover {
    background: #4a4a4a;
  }
  .btn-pattern.active {
    background: #1a3a5c;
    color: #37a8db;
    border-color: #37a8db;
  }

  .pattern-loading {
    padding: 12px;
    color: #888;
    font-size: 13px;
  }

  .pattern-error {
    padding: 12px;
    background: #3a1a1a;
    color: #ffaaaa;
    font-size: 13px;
    border: 1px solid #5a2a2a;
  }
  .pattern-error button {
    margin-top: 8px;
    padding: 2px 8px;
    background: #5a2a2a;
    color: #fff;
    border: none;
    cursor: pointer;
  }
  .btn-icon { display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 26px; padding: 0; border: 1px solid #555; border-radius: 4px; background: #2a2a2a; color: #ccc; cursor: pointer; font-size: 15px; line-height: 1; margin-left: 4px; }
  .btn-icon:hover:not(:disabled) { background: #3a3a3a; color: #fff; }
  .btn-icon:disabled { opacity: 0.35; cursor: default; }
  .btn-sm { padding: 3px 8px; border: 1px solid #555; border-radius: 4px; background: #2a2a2a; color: #ccc; cursor: pointer; font-size: 13px; }
  .btn-sm:hover { background: #3a3a3a; }
  .btn-sm.btn-danger { border-color: #c0392b; color: #e74c3c; }
  .btn-sm.btn-danger:hover { background: #c0392b; color: #fff; }
  .wp-frame { flex: 1; min-height: 0; display: flex; flex-direction: column; border: 1px solid #333; border-radius: 4px; overflow: hidden; margin-bottom: 4px; }
  .mission-tabs { display: flex; flex-shrink: 0; border-bottom: 1px solid #444; background: #1a1a1a; }
  .mission-tab { flex: 1; padding: 4px 0; border: none; background: transparent; color: #666; cursor: pointer; font-size: 11px; font-weight: 600; text-align: center; transition: all 0.15s; border-right: 1px solid #333; }
  .mission-tab:last-child { border-right: none; }
  .mission-tab:hover:not(.active) { color: #aaa; background: #252525; }
  .mission-tab.active { color: #37a8db; background: #1e2e3e; border-bottom: 2px solid #37a8db; }
  .mission-tab-add { min-width: 32px; flex: none; font-size: 14px; font-weight: bold; color: #555; }
  .mission-tab-add:hover { color: #37a8db !important; background: #1e2e3e; }
  .wp-table-wrap { flex: 1; overflow-y: auto; min-height: 0; }
  .wp-table-wrap::-webkit-scrollbar { width: 6px; }
  .wp-table-wrap::-webkit-scrollbar-track { background: #1a1a1a; border-radius: 3px; }
  .wp-table-wrap::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }
  .wp-table-wrap::-webkit-scrollbar-thumb:hover { background: #777; }
  .wp-empty { padding: 16px; text-align: center; color: #888; font-size: 13px; }
  .wp-table { width: 100%; border-collapse: collapse; font-size: 12px; }
  .wp-table thead { position: sticky; top: 0; z-index: 1; }
  .wp-table th { background: #1e1e1e; color: #888; font-weight: 600; font-size: 11px; text-transform: uppercase; padding: 4px 5px; text-align: left; border-bottom: 1px solid #444; }
  .wp-row { cursor: pointer; border-bottom: 1px solid #2a2a2a; color: #ccc; }
  .wp-row:hover { background: #2a2a2a; }
  .wp-row.selected { background: #1a3a5c; color: #fff; }
  .wp-row.modifier { color: #999; font-style: italic; }
  .wp-row.greyed { opacity: 0.35; }
  .wp-row.greyed .col-alt, .wp-row.greyed .col-type, .wp-row.greyed .wp-num-badge { filter: grayscale(100%); }
  .wp-row td { padding: 4px 5px; white-space: nowrap; }
  .col-num { width: 30px; text-align: center; }
  .col-type { width: 40px; }
  .col-alt { width: 72px; color: #8bc34a; }
  .col-param { color: #aaa; }
  .wp-num-badge { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; border-radius: 50%; background: #37a8db; color: #fff; font-size: 10px; font-weight: bold; }
  .mod-indent { padding-left: 8px; color: #e67e22; font-style: italic; }
  .wp-detail { padding: 6px 8px; border: 1px solid #333; border-radius: 4px; background: #1e1e1e; flex-shrink: 0; margin-bottom: 4px; max-height: 180px; overflow-y: auto; }
  .detail-header { font-weight: bold; font-size: 13px; color: #37a8db; margin-bottom: 4px; padding-bottom: 3px; border-bottom: 1px solid #333; }
  .detail-row { display: flex; justify-content: space-between; padding: 1px 0; font-size: 12px; color: #ccc; }
  .detail-label { color: #888; font-size: 11px; }
  .detail-value { color: #ccc; font-size: 12px; }
  .detail-hint { color: #37a8db; font-size: 11px; text-align: center; margin-top: 4px; font-style: italic; }
  .mission-bottom { flex-shrink: 0; }
  .mission-controls { display: flex; flex-direction: column; gap: 4px; }
  .ctrl-row { display: flex; gap: 4px; }
  .btn-ctrl { flex: 1; padding: 5px 6px; border: 1px solid #37a8db; border-radius: 4px; background: #1a3a5c; color: #37a8db; cursor: pointer; font-size: 12px; white-space: nowrap; }
  .btn-ctrl:hover:not(:disabled) { background: #37a8db; color: #fff; }
  .btn-ctrl:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-ctrl.btn-file { border-color: #555; background: #2a2a2a; color: #ccc; }
  .btn-ctrl.btn-file:hover { background: #3a3a3a; }
  .btn-ctrl.btn-eeprom { border-color: #e67e22; background: #3a2a1a; color: #e67e22; }
  .btn-ctrl.btn-eeprom:hover:not(:disabled) { background: #e67e22; color: #fff; }
  .mission-status { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; flex-shrink: 0; }
  .mission-summary { display: flex; align-items: center; justify-content: center; gap: 8px; padding: 3px; font-size: 12px; color: #888; flex-shrink: 0; }
  .dirty-badge { background: #f39c12; color: #1a1a1a; padding: 1px 6px; border-radius: 8px; font-size: 11px; font-weight: bold; }
  .drop-overlay { position: absolute; inset: 0; background: rgba(55,168,219,0.15); border: 2px dashed #37a8db; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: #37a8db; font-size: 13px; font-weight: bold; z-index: 10; pointer-events: none; }
</style>
