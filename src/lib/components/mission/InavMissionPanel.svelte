<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- InavMissionPanel.svelte
     INAV MSP mission planner on the panel framework (docs/active/PANEL_FRAMEWORK.md): a `compact`
     PanelShell. Header = title + autopilot select; toolbar = edit/manager/undo/redo/pattern/clear;
     content field = multi-mission tabs + WP table; footer = selected-WP detail + FC/EEPROM/file
     controls. Renders MissionManager when the library view is open.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    mission, selectedWpIndex, selectedWpIndices, editMode,
    selectWpSingle, toggleWpSelection, selectWpRange, clearWpSelection, removeSelectedWps,
    missionDownload, missionUpload,
    missionImportXml,
    missionSaveFile, missionLoadFile,
    activeMissionIndex, missionCount,
    switchMission, addMission, removeMission, getTotalWpCount,
    canUndo, canRedo, undo, redo,
    MAX_MISSIONS, MAX_WAYPOINTS_TOTAL,
    type Waypoint, type Mission, WpAction, WP_ACTION_KEYS,
    hasLocation, isModifier, missionFlags, missionModified,
    loadedMissionId, markMissionSynced,
    onMissionDownloadProgress,
  } from '$lib/stores/mission';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import MissionSaveDialog from '$lib/components/mission/MissionSaveDialog.svelte';
  import MissionManager from '$lib/components/mission/MissionManager.svelte';
  import SafeHomeManager from '$lib/components/mission/SafeHomeManager.svelte';
  import { safeHomeManagerOpen } from '$lib/stores/safehome';
  import { geozoneMissionResult } from '$lib/stores/geozone';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import AutopilotSelect from '$lib/components/mission/AutopilotSelect.svelte';
  import { missionManagerOpen } from '$lib/stores/missionManager';
  import { buildMissionInput, computeMissionStats } from '$lib/helpers/missionLibrary';
  import { missionDbSave, missionDbUpdate, missionDbGet, missionDbFindByHash, missionDbGeocode } from '$lib/stores/flightlog';
  import { locale } from 'svelte-i18n';
  import { isFlyByHome } from '$lib/helpers/missionGeometry';
  import { contextMenu } from '$lib/actions/contextMenu';
  import { buildWaypointMenu } from '$lib/helpers/waypointMenu';
  import { connection } from '$lib/stores/connection';
  import { telemetry, type TelemetryData } from '$lib/stores/telemetry';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { convertAltitude, convertSpeed, convertDistance, formatConverted } from '$lib/utils/units';
  import { inavWpDetailLines } from '$lib/helpers/missionWpDetails';
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
  let missionSaveDialog: ReturnType<typeof MissionSaveDialog>;

  let downloadLoading = $state(false);
  let uploadLoading = $state(false);
  let eepromSaveLoading = $state(false);
  let eepromLoadLoading = $state(false);
  let statusMessage = $state('');
  // Auto-clear the transient status line after 10s — persistent state is shown by the flags.
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });
  let dragOver = $state(false);
  let currentTelem = $state<TelemetryData>(get(telemetry));
  const unsubTelem = telemetry.subscribe(t => { currentTelem = t; });

  const ARMING_FLAG_ARMED = 2;

  function isConnected(): boolean {
    return get(connection)?.status === 'connected';
  }

  // Safe Home Manager button: only with a live INAV link that supports the autoland/safehome config
  // (≥7.1). Safehome display works on older INAV, but editing (this panel) is gated to ≥7.1.
  const showSafehomeBtn = $derived(
    $connection.status === 'connected'
      && $connection.protocolType === 'msp'
      && !!$connection.fcInfo?.features?.autoland_config,
  );

  function isArmed(): boolean {
    return currentTelem.lastUpdate > 0 && (currentTelem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  }

  // Live "x of n" status while the FC streams waypoints. Returns the unlisten fn (call in `finally`).
  function listenDownloadProgress() {
    return onMissionDownloadProgress(({ current, total }) => {
      statusMessage = total > 0
        ? $t('mission.downloadingProgress', { values: { current, total } })
        : $t('mission.downloading');
    });
  }

  async function handleDownload() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    downloadLoading = true; statusMessage = $t('mission.downloading');
    const un = await listenDownloadProgress();
    try {
      const m = await missionDownload(false);
      statusMessage = $t('mission.downloaded', { values: { count: m.waypoints.length } });
    } catch (e) {
      statusMessage = $t('mission.downloadFailed', { values: { error: String(e) } });
    } finally { un(); downloadLoading = false; }
  }

  async function handleUpload() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    uploadLoading = true; statusMessage = '';
    try {
      const m = await missionUpload(false);
      statusMessage = $t('mission.uploaded', { values: { count: m.waypoints.length } });
    } catch (e) {
      statusMessage = $t('mission.uploadFailed', { values: { error: String(e) } });
    } finally { uploadLoading = false; }
  }

  async function handleEepromSave() {
    if (!isConnected() || isArmed()) { statusMessage = isArmed() ? $t('mission.eepromSaveArmedMsg') : $t('mission.notConnected'); return; }
    eepromSaveLoading = true; statusMessage = '';
    try {
      const m = await missionUpload(true);
      statusMessage = $t('mission.eepromSaved', { values: { count: m.waypoints.length } });
    } catch (e) {
      statusMessage = $t('mission.eepromSaveFailed', { values: { error: String(e) } });
    } finally { eepromSaveLoading = false; }
  }

  async function handleEepromLoad() {
    if (!isConnected()) { statusMessage = $t('mission.notConnected'); return; }
    eepromLoadLoading = true; statusMessage = $t('mission.downloading');
    const un = await listenDownloadProgress();
    try {
      const m = await missionDownload(true);
      statusMessage = $t('mission.eepromLoaded', { values: { count: m.waypoints.length } });
    } catch (e) {
      statusMessage = $t('mission.eepromLoadFailed', { values: { error: String(e) } });
    } finally { un(); eepromLoadLoading = false; }
  }

  async function handleSaveFile() {
    try {
      const path = await save({ title: $t('mission.saveMissionTitle'), defaultPath: 'mission.mission', filters: [{ name: 'Mission', extensions: ['mission'] }] });
      if (!path) return;
      await missionSaveFile(path);
      statusMessage = $t('mission.missionSaved');
    } catch (e) {
      statusMessage = $t('mission.saveFailed', { values: { error: String(e) } });
    }
  }

  /** Auto-name for fresh missions: "New Mission - YYYY-MM-DD HH:MM". */
  function autoMissionName(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, '0');
    return `New Mission - ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  /** Save the current map mission to the DB library (dedup by content hash). */
  async function handleSaveToLibrary() {
    const wps = currentMission.waypoints;
    if (wps.length === 0) return;
    const dbPath = get(settings).flightLogDbPath;
    const lang = get(locale) ?? 'en';
    const id = get(loadedMissionId);
    const flags = get(missionFlags);
    try {
      if (id != null && flags.db) { statusMessage = $t('mission.saveLibAlready'); return; }

      if (id != null && !flags.db) {
        const ans = await confirmDialog.show({
          title: $t('mission.saveLibUpdateTitle'),
          message: $t('mission.saveLibUpdateMsg'),
          buttons: [
            { label: $t('mission.saveLibNew'), value: 'new' },
            { label: $t('mission.saveLibOverwrite'), value: 'overwrite', primary: true },
          ],
        });
        if (ans == null) return;
        if (ans === 'overwrite') {
          const existing = await missionDbGet(id, dbPath);
          const input = await buildMissionInput(wps, { name: existing?.name ?? autoMissionName(), notes: existing?.notes ?? '' });
          const collide = await missionDbFindByHash(input.content_hash, dbPath);
          if (collide && collide.id !== id) {
            loadedMissionId.set(collide.id);
            markMissionSynced('db');
            statusMessage = $t('mission.saveLibDuplicate');
            return;
          }
          await missionDbUpdate(id, input, dbPath);
          loadedMissionId.set(id);
          markMissionSynced('db');
          void missionDbGeocode(id, lang, dbPath).catch(() => {});
          statusMessage = $t('mission.saveLibUpdated');
          return;
        }
      }

      const res = await missionSaveDialog.show({ defaultName: autoMissionName() });
      if (!res) return;
      const input = await buildMissionInput(wps, { name: res.name || autoMissionName(), notes: res.notes });
      const newId = await missionDbSave(input, dbPath);
      loadedMissionId.set(newId);
      markMissionSynced('db');
      void missionDbGeocode(newId, lang, dbPath).catch(() => {});
      statusMessage = $t('mission.saveLibSaved');
    } catch (e) {
      statusMessage = $t('mission.saveLibFailed', { values: { error: String(e) } });
    }
  }

  async function handleOpenFile() {
    try {
      const path = await open({ title: $t('mission.openMissionTitle'), multiple: false, filters: [{ name: 'Mission', extensions: ['mission'] }] });
      if (!path) return;
      const m = await missionLoadFile(typeof path === 'string' ? path : path);
      statusMessage = $t('mission.loaded', { values: { count: m.waypoints.length } });
    } catch (e) {
      statusMessage = $t('mission.openFailed', { values: { error: String(e) } });
    }
  }

  function onDragOver(e: DragEvent) { if (get(missionManagerOpen)) return; e.preventDefault(); dragOver = true; }
  function onDragLeave() { dragOver = false; }
  async function onDrop(e: DragEvent) {
    if (get(missionManagerOpen)) return;
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
    } catch (e) {
      statusMessage = $t('mission.importFailed', { values: { error: String(e) } });
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

  function togglePattern() {
    if (showPatternPanel) {
      import('$lib/stores/surveyPattern.svelte').then(m => { m.exitPatternMode(); showPatternPanel = false; });
    } else {
      import('$lib/stores/surveyPattern.svelte').then(m => { m.enterPatternMode('rectangle'); showPatternPanel = true; });
    }
  }

  // List selection. Plain click = single; Ctrl/⌘ = toggle; Shift = range; a tap
  // on the number badge toggles too. Multi-select gestures are edit-mode only.
  function onRowClick(e: MouseEvent, i: number) {
    if (currentEditing && e.shiftKey && selAnchor >= 0) {
      selectWpRange(selAnchor, i);
    } else if (currentEditing && (e.ctrlKey || e.metaKey)) {
      toggleWpSelection(i);
      selAnchor = i;
    } else if (!currentEditing && currentSelIdx === i) {
      clearWpSelection();
    } else {
      selectWpSingle(i);
      selAnchor = i;
    }
  }
  function onBadgeClick(e: MouseEvent, i: number) {
    e.stopPropagation();
    if (currentEditing) toggleWpSelection(i);
    else if (currentSelIdx === i) clearWpSelection();
    else selectWpSingle(i);
    selAnchor = i;
  }
  function wpMenuFor(i: number) {
    if (!currentSel.has(i)) selectWpSingle(i);
    return buildWaypointMenu();
  }
  async function removeSelected() { if (currentSel.size > 0) await removeSelectedWps(); }

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

  // Mission stats (distance / climb+descent / estimated flight time) for the footer summary.
  const stats = $derived(computeMissionStats(currentMission.waypoints));
  function fmtDist(m: number): string {
    return formatConverted(convertDistance(m, iface.distanceUnit), m >= 1000 ? 2 : 0);
  }
  function fmtAltDelta(m: number): string {
    const a = convertAltitude(m, iface.altitudeUnit);
    return `${Math.round(a.value)}${a.unit}`;
  }
  function fmtDuration(s: number): string {
    const total = Math.round(s);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const sec = total % 60;
    if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m`;
    if (m > 0) return `${m}m ${String(sec).padStart(2, '0')}s`;
    return `${sec}s`;
  }
  const estTimeText = $derived.by(() => {
    if (stats.estTimeS === null) return null;
    const base = fmtDuration(stats.estTimeS);
    const prefix = stats.hasUnlimitedHold ? '≥ ' : stats.estTimeApprox ? '~' : '';
    return `${prefix}${base}`;
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet toolbar()}
  <div class="miss-toolbar">
    <Button variant="mode" active={currentEditing} icon="edit" onclick={() => editMode.update(v => !v)} title={$t('mission.toggleEdit')}>
      {currentEditing ? $t('mission.editing') : $t('mission.edit')}
    </Button>
    {#if !currentEditing}
      <Button variant="standard" icon="library" onclick={() => missionManagerOpen.set(true)} title={$t('mission.missionManager')}>
        {$t('mission.missionManager')}
      </Button>
      {#if showSafehomeBtn}
        <Button variant="standard" icon="home" onclick={() => safeHomeManagerOpen.set(true)} title={$t('safehome.title')} />
      {/if}
    {/if}
    {#if currentEditing && !showPatternPanel}
      <Button variant="standard" icon="undo" disabled={!canUndoNow} onclick={() => undo()} title={$t('mission.undo')} />
      <Button variant="standard" icon="redo" disabled={!canRedoNow} onclick={() => redo()} title={$t('mission.redo')} />
    {/if}
    {#if currentEditing}
      <Button variant="mode" active={showPatternPanel} icon="map" onclick={togglePattern}>{$t('survey.pattern')}</Button>
    {/if}
    <div class="tb-spacer"></div>
    {#if currentEditing && currentSel.size > 0}
      <Button variant="danger" icon="close" onclick={removeSelected} title={$t('mission.removeWp')} />
    {/if}
    <Button variant="standard" icon="delete" onclick={handleClear} title={$t('mission.clearMission')} />
  </div>
{/snippet}

{#snippet body()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="miss-dropzone" class:drag-over={dragOver} ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
    {#if $geozoneMissionResult.active}
      {#if $geozoneMissionResult.nfzLaunchInside}
        <div class="gz-warn gz-warn-red">{$t('geozone.warnNfzLaunch')}</div>
      {/if}
      {#if $geozoneMissionResult.nfzPathViolated}
        <div class="gz-warn gz-warn-amber">{$t('geozone.warnNfzPath')}</div>
      {/if}
      {#if $geozoneMissionResult.inclusiveActive && $geozoneMissionResult.inclusiveViolated}
        <div class="gz-warn gz-warn-amber">{$t('geozone.warnInclusion')}</div>
      {/if}
    {/if}
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
    {:else if currentMission.waypoints.length === 0}
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
            {#if isFlyByHome(wp)}
              <tr class="wp-row modifier fbh" class:selected={currentSel.has(i)} class:greyed={missionEndIdx >= 0 && i > missionEndIdx} onclick={(e) => onRowClick(e, i)} use:contextMenu={() => wpMenuFor(i)}>
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <td class="col-num"><span class="wp-num-badge fbh-num" onclick={(e) => onBadgeClick(e, i)}>{displayNums.get(i) ?? ''}</span></td>
                <td class="col-type mod-indent">↳ {$t('mission.flagFbh')}</td>
                <td class="col-alt">{formatAltShort(wp)}</td>
                <td class="col-param">→ {$t('mission.home')}</td>
              </tr>
            {:else if isModifier(wp.action)}
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

    {#if dragOver}<div class="drop-overlay">{$t('mission.dropHint')}</div>{/if}
  </div>
{/snippet}

{#snippet footer()}
  <div class="miss-footer">
    {#if currentSelIdx >= 0 && currentSelIdx < currentMission.waypoints.length}
      {@const wp = currentMission.waypoints[currentSelIdx]}
      <div class="wp-detail">
        <div class="detail-header">{isModifier(wp.action) ? '' : `WP ${displayNums.get(currentSelIdx) ?? ''} — `}{isFlyByHome(wp) ? $t('mission.flagFbh') : $t(WP_ACTION_KEYS[wp.action])}</div>
        {#each inavWpDetailLines(wp, $t, iface) as p}
          <div class="detail-row"><span class="detail-label">{p.label}</span><span class="detail-value">{p.value}</span></div>
        {/each}
        {#if currentEditing}<div class="detail-hint">{$t('mission.clickMarkerHint')}</div>{/if}
      </div>
    {/if}

    {#if !showPatternPanel}
      <div class="ctrl-row">
        <Button variant="data" icon="download" full disabled={downloadLoading} onclick={handleDownload}>{$t('mission.fcDownload')}</Button>
        <Button variant="data" icon="upload" full disabled={uploadLoading} onclick={handleUpload}>{$t('mission.fcUpload')}</Button>
      </div>
      <div class="ctrl-row">
        <Button variant="warning" icon="download" full disabled={eepromLoadLoading} onclick={handleEepromLoad}>{$t('mission.eepromLoad')}</Button>
        <Button variant="warning" icon="save" full disabled={eepromSaveLoading || isArmed()} title={isArmed() ? $t('mission.eepromSaveArmed') : $t('mission.eepromSaveTooltip')} onclick={handleEepromSave}>{$t('mission.eepromSave')}</Button>
      </div>
      <div class="ctrl-row">
        <Button variant="data" icon="library" full disabled={currentMission.waypoints.length === 0} onclick={handleSaveToLibrary}>{$t('mission.saveToLibrary')}</Button>
      </div>
      <div class="ctrl-row">
        <Button variant="standard" icon="folder" full onclick={handleOpenFile}>{$t('mission.open')}</Button>
        <Button variant="standard" icon="save" full onclick={handleSaveFile}>{$t('mission.save')}</Button>
      </div>
    {/if}

    {#if statusMessage}<div class="mission-status">{statusMessage}</div>{/if}

    {#if !showPatternPanel && stats.geoCount >= 2}
      <div class="mission-stats">
        <span class="stat" title={$t('mission.statDistance')}>⤢ {fmtDist(stats.legDistanceM)}</span>
        <span class="stat" title={$t('mission.statClimbDescent')}>↑{fmtAltDelta(stats.climbM)} ↓{fmtAltDelta(stats.descentM)}</span>
        {#if estTimeText}
          <span class="stat" title={stats.estTimeApprox || stats.hasUnlimitedHold ? _t('mission.statTimeTip', { speed: formatConverted(convertSpeed(stats.assumedCruiseMs, iface.speedUnit), 0) }) : $t('mission.statTime')}>⏱ {estTimeText}</span>
        {/if}
      </div>
    {/if}

    {#if currentMission.waypoints.length > 0}
      <div class="mission-summary">
        {#if currentMissionCount > 1}
          M{currentMissionIdx}: {currentMission.waypoints.length} WPs | Total: {getTotalWpCount()}/{MAX_WAYPOINTS_TOTAL}
        {:else}
          {currentMission.waypoints.length}/{MAX_WAYPOINTS_TOTAL} WPs
        {/if}
        {#if $missionModified}<span class="dirty-badge">{$t('mission.modified')}</span>{/if}
        {#if $missionFlags.fc}<span class="prov-badge prov-fc" title={$t('mission.provFcTip')}>{$t('mission.provFc')}</span>{/if}
        {#if $missionFlags.file}<span class="prov-badge prov-file" title={$t('mission.provFileTip')}>{$t('mission.provFile')}</span>{/if}
        {#if $missionFlags.db}<span class="prov-badge prov-db" title={$t('mission.provDbTip')}>{$t('mission.provDb')}</span>{/if}
      </div>
    {/if}
  </div>
{/snippet}

{#if $safeHomeManagerOpen}
  <SafeHomeManager onBack={() => safeHomeManagerOpen.set(false)} />
{:else if $missionManagerOpen}
  <MissionManager onBack={() => missionManagerOpen.set(false)} />
{:else}
  <div class="imv2">
    <PanelShell variant="compact" title={$t('nav.mission')} {toolbar} {body} {footer}>
      {#snippet headerActions()}<AutopilotSelect />{/snippet}
    </PanelShell>
  </div>
{/if}

<ConfirmDialog bind:this={confirmDialog} />
<MissionSaveDialog bind:this={missionSaveDialog} />

<style>
  .miss-toolbar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; width: 100%; }
  .tb-spacer { flex: 1; }

  .miss-dropzone { position: relative; min-height: 100%; }
  .miss-dropzone.drag-over { outline: 2px dashed #37a8db; outline-offset: -2px; border-radius: 4px; }

  /* Geozone safety-check warnings (above the mission tabs/WP list). */
  .gz-warn { padding: 4px 8px; font-size: 11.5px; font-weight: 600; border-radius: 4px; margin-bottom: 4px; }
  .gz-warn-red { background: rgba(212, 0, 0, 0.18); color: #ff6b6b; border: 1px solid rgba(212, 0, 0, 0.5); }
  .gz-warn-amber { background: rgba(245, 166, 35, 0.16); color: #f5a623; border: 1px solid rgba(245, 166, 35, 0.45); }

  .mission-tabs { display: flex; border-bottom: 1px solid #444; background: #1a1a1a; border-radius: 4px 4px 0 0; margin-bottom: 4px; }
  .mission-tab { flex: 1; padding: 4px 0; border: none; background: transparent; color: #666; cursor: pointer; font-size: 11px; font-weight: 600; text-align: center; transition: all 0.15s; border-right: 1px solid #333; }
  .mission-tab:last-child { border-right: none; }
  .mission-tab:hover:not(.active) { color: #aaa; background: #252525; }
  .mission-tab.active { color: #37a8db; background: #1e2e3e; border-bottom: 2px solid #37a8db; }
  .mission-tab-add { min-width: 32px; flex: none; font-size: 14px; font-weight: bold; color: #555; }
  .mission-tab-add:hover { color: #37a8db !important; background: #1e2e3e; }

  .pattern-loading { padding: 12px; color: #888; font-size: 13px; }
  .pattern-error { padding: 12px; background: #3a1a1a; color: #ffaaaa; font-size: 13px; border: 1px solid #5a2a2a; }
  .pattern-error button { margin-top: 8px; padding: 2px 8px; background: #5a2a2a; color: #fff; border: none; cursor: pointer; }

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
  .wp-num-badge.fbh-num { background: #e67e22; }
  .mod-indent { padding-left: 8px; color: #e67e22; font-style: italic; }

  .miss-footer { width: 100%; display: flex; flex-direction: column; gap: 4px; }
  /* No internal scroll: the PanelShell footer is pinned and the column scrolls as a last resort, so a
     full WP's params show without a spurious inner scrollbar (the detail is bounded — a handful of rows). */
  .wp-detail { padding: 6px 8px; border: 1px solid #333; border-radius: 4px; background: #1e1e1e; }
  .detail-header { font-weight: bold; font-size: 13px; color: #37a8db; margin-bottom: 4px; padding-bottom: 3px; border-bottom: 1px solid #333; }
  .detail-row { display: flex; justify-content: space-between; padding: 1px 0; font-size: 12px; color: #ccc; }
  .detail-label { color: #888; font-size: 11px; }
  .detail-value { color: #ccc; font-size: 12px; }
  .detail-hint { color: #37a8db; font-size: 11px; text-align: center; margin-top: 4px; font-style: italic; }
  .ctrl-row { display: flex; gap: 4px; }
  .mission-status { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; }
  .mission-summary { display: flex; align-items: center; justify-content: center; gap: 8px; padding: 3px; font-size: 12px; color: #888; flex-wrap: wrap; }
  .mission-stats { display: flex; align-items: center; justify-content: center; gap: 12px; padding: 4px 3px 0; font-size: 12px; color: #9ad0e8; flex-wrap: wrap; font-variant-numeric: tabular-nums; }
  .mission-stats .stat { white-space: nowrap; cursor: default; }
  .dirty-badge { background: #f39c12; color: #1a1a1a; padding: 1px 6px; border-radius: 8px; font-size: 11px; font-weight: bold; }
  .prov-badge { color: #fff; padding: 1px 6px; border-radius: 8px; font-size: 11px; font-weight: bold; margin-left: 4px; }
  .prov-fc { background: #37a8db; }
  .prov-file { background: #6c7a89; }
  .prov-db { background: #59aa29; }
  .drop-overlay { position: absolute; inset: 0; background: rgba(55,168,219,0.15); border: 2px dashed #37a8db; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: #37a8db; font-size: 13px; font-weight: bold; z-index: 10; pointer-events: none; }
</style>
