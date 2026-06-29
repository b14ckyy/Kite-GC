<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ArduMissionPanel.svelte
     ArduPilot / PX4 mission planner on the panel framework (docs/active/PANEL_FRAMEWORK.md): a
     `compact` PanelShell. Single mission (no multi-mission tabs).
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { t, locale } from 'svelte-i18n';
  import {
    arduMission, arduSelectedWpIndex, arduEditMode, arduLoadedMissionId,
    arduMissionClear, arduRemoveWp, groupArduMission, markArduMissionSynced,
    arduMissionFlags, arduMissionModified, downloadArduMissionFromFc,
    arduUndo, arduRedo, arduCanUndo, arduCanRedo, arduClearUndoHistory,
    arduVehicleClass, setArduVehicleClass,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    serializeWaypoints, parseWaypoints,
    type ArduWaypoint,
  } from '$lib/stores/missionArdupilot';
  import { onMissionDownloadProgress } from '$lib/stores/mission';
  import { cmdName, cmdShort, cmdHasLocation, cmdDef, cmdValidForVehicle, cmdValidForPx4, enumLabel, type VehicleClass } from '$lib/helpers/arduCommandCatalog';
  import { arduWpDetailLines } from '$lib/helpers/missionWpDetails';
  import { frameMissionOnMap } from '$lib/stores/mapCamera';
  import { connection } from '$lib/stores/connection';
  import { settings } from '$lib/stores/settings';
  import { autopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';
  import { missionManagerOpen } from '$lib/stores/missionManager';
  import { buildArduMissionInput, computeArduMissionStats } from '$lib/helpers/missionLibraryArdu';
  import { convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';
  import { missionDbSave, missionDbUpdate, missionDbGet, missionDbFindByHash, missionDbGeocode } from '$lib/stores/flightlog';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import AutopilotSelect from '$lib/components/mission/AutopilotSelect.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import MissionSaveDialog from '$lib/components/mission/MissionSaveDialog.svelte';
  import MissionManager from '$lib/components/mission/MissionManager.svelte';

  let currentMission  = $state<ArduWaypoint[]>(get(arduMission));
  let currentSelIdx   = $state<number>(get(arduSelectedWpIndex));
  let currentEditing  = $state<boolean>(get(arduEditMode));
  let currentConn     = $state(get(connection));
  let statusMessage   = $state('');
  let dragOver        = $state(false);
  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  let missionSaveDialog: ReturnType<typeof MissionSaveDialog>;
  // Auto-clear the transient status line after 10s — persistent state is shown by the flags.
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });

  let currentVehicle  = $state<VehicleClass>(get(arduVehicleClass));
  let currentSystem   = $state<AutopilotSystem>(get(autopilotSystem));

  // Survey pattern generator (shared with INAV; the panel only renders it + the entry button).
  let showPatternPanel = $state(false);
  function togglePattern() {
    if (showPatternPanel) {
      import('$lib/stores/surveyPattern.svelte').then(m => { m.exitPatternMode(); showPatternPanel = false; });
    } else {
      import('$lib/stores/surveyPattern.svelte').then(m => { m.enterPatternMode('rectangle'); showPatternPanel = true; });
    }
  }
  // Leaving edit mode closes the pattern panel (mirror the INAV panel).
  $effect(() => {
    if (!currentEditing && showPatternPanel) {
      showPatternPanel = false;
      import('$lib/stores/surveyPattern.svelte').then(m => m.exitPatternMode());
    }
  });

  // Keyboard: Ctrl+Z = undo, Ctrl+Y / Ctrl+Shift+Z = redo. Edit-mode only, not while in the pattern
  // panel, and not while a text field is focused (so native input undo keeps working). Mirrors INAV.
  function onKeydown(e: KeyboardEvent) {
    if (!currentEditing || showPatternPanel) return;
    if (!(e.ctrlKey || e.metaKey)) return;
    const tgt = e.target as HTMLElement | null;
    const tag = tgt?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tgt?.isContentEditable) return;
    const k = e.key.toLowerCase();
    if (k === 'z' && !e.shiftKey) { e.preventDefault(); arduUndo(); }
    else if ((k === 'z' && e.shiftKey) || k === 'y') { e.preventDefault(); arduRedo(); }
  }

  const unsubMission  = arduMission.subscribe(m => { currentMission = m; });
  const unsubSelIdx   = arduSelectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = arduEditMode.subscribe(e => { currentEditing = e; });
  const unsubConn     = connection.subscribe(c => { currentConn = c; });
  const unsubVehicle  = arduVehicleClass.subscribe(v => { currentVehicle = v; });
  const unsubSystem   = autopilotSystem.subscribe(s => { currentSystem = s; });

  // Vehicle-class selector (ArduPilot only; PX4 uses the same panel but has no class palette yet).
  // Offline it is selectable; while connected it is locked to the detected FC.
  const VEHICLE_OPTIONS: { value: VehicleClass; key: string }[] = [
    { value: 'plane', key: 'arduMission.vehiclePlane' },
    { value: 'copter', key: 'arduMission.vehicleCopter' },
    { value: 'quadplane', key: 'arduMission.vehicleQuadplane' },
    { value: 'rover', key: 'arduMission.vehicleRover' },
    { value: 'boat', key: 'arduMission.vehicleBoat' },
    { value: 'sub', key: 'arduMission.vehicleSub' },
  ];
  const showVehicleSelect = $derived(currentSystem === 'ardupilot');
  const vehicleLocked = $derived(currentConn.status === 'connected');
  // Soft-warning is vehicle-class aware. ArduPilot's class is operator-chosen (meaningful offline too);
  // PX4 has no class selector, so its class is only known once connected → warn only then. PX4 also
  // flags commands the firmware doesn't support at all (would be rejected at upload).
  const vehicleWarnActive = $derived(currentSystem === 'ardupilot' || currentConn.status === 'connected');
  function cmdInvalid(cmd: number): boolean {
    if (!vehicleWarnActive) return false;
    return currentSystem === 'px4'
      ? !cmdValidForPx4(cmd, currentVehicle)
      : !cmdValidForVehicle(cmd, currentVehicle);
  }
  const invalidCount = $derived(currentMission.filter((w) => cmdInvalid(w.command)).length);

  const isMavlinkConnected = $derived(
    currentConn.status === 'connected' && currentConn.protocolType === 'mavlink'
  );

  onDestroy(() => { unsubMission(); unsubSelIdx(); unsubEditMode(); unsubConn(); unsubVehicle(); unsubSystem(); });

  async function handleSaveFile() {
    try {
      const path = await save({
        title: $t('mission.saveMissionTitle'),
        defaultPath: 'mission.waypoints',
        filters: [{ name: 'Waypoints', extensions: ['waypoints'] }],
      });
      if (!path) return;
      await invoke<void>('write_text_file', { path, content: serializeWaypoints(get(arduMission)) });
      statusMessage = $t('mission.missionSaved');
    } catch (e) {
      statusMessage = $t('mission.saveFailed', { values: { error: String(e) } });
    }
  }

  async function handleOpenFile() {
    try {
      const path = await open({
        title: $t('mission.openMissionTitle'),
        multiple: false,
        filters: [{ name: 'Waypoints', extensions: ['waypoints', 'txt'] }],
      });
      if (!path) return;
      const content = await invoke<string>('read_text_file', { path: typeof path === 'string' ? path : path });
      const wps = parseWaypoints(content);
      arduMission.set(wps);
      arduSelectedWpIndex.set(-1);
      arduLoadedMissionId.set(null); // fresh file → not yet a library mission
      arduClearUndoHistory(); // loaded mission = fresh undo baseline
      markArduMissionSynced('file', wps);
      statusMessage = $t('mission.loaded', { values: { count: wps.length } });
      frameMissionOnMap();
    } catch (e) {
      statusMessage = $t('mission.openFailed', { values: { error: String(e) } });
    }
  }

  function onDragOver(e: DragEvent) { e.preventDefault(); dragOver = true; }
  function onDragLeave() { dragOver = false; }
  async function onDrop(e: DragEvent) {
    e.preventDefault(); dragOver = false;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    const file = files[0];
    if (!file.name.endsWith('.waypoints') && !file.name.endsWith('.txt')) {
      statusMessage = $t('arduMission.onlyWaypointsFiles'); return;
    }
    try {
      const wps = parseWaypoints(await file.text());
      arduMission.set(wps);
      arduSelectedWpIndex.set(-1);
      arduLoadedMissionId.set(null); // fresh file → not yet a library mission
      arduClearUndoHistory(); // loaded mission = fresh undo baseline
      markArduMissionSynced('file', wps);
      statusMessage = $t('mission.loadedFromFile', { values: { count: wps.length, file: file.name } });
      frameMissionOnMap();
    } catch (e) {
      statusMessage = $t('mission.importFailed', { values: { error: String(e) } });
    }
  }

  /** Auto-name for fresh missions: "New Mission - YYYY-MM-DD HH:MM". */
  function autoMissionName(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, '0');
    return `New Mission - ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  /** Save the current ArduPilot/PX4 mission to the DB library (dedup by content hash). Mirrors the
   *  INAV panel's save-to-library: NEW vs OVERWRITE driven by `arduLoadedMissionId`. */
  async function handleSaveToLibrary() {
    const wps = get(arduMission);
    if (wps.length === 0) return;
    const dbPath = get(settings).flightLogDbPath;
    const lang = get(locale) ?? 'en';
    const fmt = get(autopilotSystem) === 'px4' ? 'px4' : 'ardupilot';
    const id = get(arduLoadedMissionId);
    try {
      if (id != null) {
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
          const input = await buildArduMissionInput(wps, { name: existing?.name ?? autoMissionName(), notes: existing?.notes ?? '', format: fmt });
          const collide = await missionDbFindByHash(input.content_hash, dbPath);
          if (collide && collide.id !== id) {
            arduLoadedMissionId.set(collide.id);
            markArduMissionSynced('db', wps);
            statusMessage = $t('mission.saveLibDuplicate');
            return;
          }
          await missionDbUpdate(id, input, dbPath);
          arduLoadedMissionId.set(id);
          markArduMissionSynced('db', wps);
          void missionDbGeocode(id, lang, dbPath).catch(() => {});
          statusMessage = $t('mission.saveLibUpdated');
          return;
        }
      }

      const input = await buildArduMissionInput(wps, { name: autoMissionName(), format: fmt });
      const collide = await missionDbFindByHash(input.content_hash, dbPath);
      if (collide) {
        arduLoadedMissionId.set(collide.id);
        markArduMissionSynced('db', wps);
        statusMessage = $t('mission.saveLibDuplicate');
        return;
      }
      const res = await missionSaveDialog.show({ defaultName: autoMissionName() });
      if (!res) return;
      const named = await buildArduMissionInput(wps, { name: res.name || autoMissionName(), notes: res.notes, format: fmt });
      const newId = await missionDbSave(named, dbPath);
      arduLoadedMissionId.set(newId);
      markArduMissionSynced('db', wps);
      void missionDbGeocode(newId, lang, dbPath).catch(() => {});
      statusMessage = $t('mission.saveLibSaved');
    } catch (e) {
      statusMessage = $t('mission.saveLibFailed', { values: { error: String(e) } });
    }
  }

  function handleClear() { arduMissionClear(); statusMessage = $t('mission.missionCleared'); }
  function removeSelected() { if (currentSelIdx >= 0) arduRemoveWp(currentSelIdx); }

  async function handleFcDownload() {
    statusMessage = $t('arduMission.downloading');
    // Live "x of n" status while the FC streams items (the count arrives before the items).
    const un = await onMissionDownloadProgress(({ current, total }) => {
      statusMessage = total > 0
        ? $t('mission.downloadingProgress', { values: { current, total } })
        : $t('mission.downloading');
    });
    try {
      const count = await downloadArduMissionFromFc();
      statusMessage = $t('mission.downloaded', { values: { count } });
      frameMissionOnMap();
    } catch (e) {
      statusMessage = $t('mission.downloadFailed', { values: { error: String(e) } });
    } finally { un(); }
  }

  async function handleFcUpload() {
    const wps = get(arduMission);
    if (wps.length === 0) { statusMessage = $t('mission.noWpToUpload'); return; }
    statusMessage = $t('arduMission.uploading');
    try {
      await invoke<void>('ardu_mission_upload', { waypoints: wps });
      markArduMissionSynced('fc', wps); // FC now holds exactly this mission
      statusMessage = $t('mission.uploaded', { values: { count: wps.length } });
    } catch (e) {
      statusMessage = $t('mission.uploadFailed', { values: { error: String(e) } });
    }
  }

  // INAV-style grouped list: each location (NAV) command is a primary row; its trailing non-location
  // commands are its modifiers (indented sub-rows, numbered — ArduPilot numbers every item).
  let groups = $derived(groupArduMission(currentMission));

  function frameLabel(frame: number): string {
    if (frame === MAV_FRAME_GLOBAL) return 'AMSL';
    if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return 'TRN';
    return 'REL';
  }

  // Mission stats (distance / climb+descent / flight time) for the footer — same shape + rendering as
  // the INAV panel. Units follow the global settings. Time only when WP speeds are set (see helper).
  const stats = $derived(computeArduMissionStats(currentMission));
  function fmtDist(m: number): string {
    return formatConverted(convertDistance(m, $settings.interface.distanceUnit), m >= 1000 ? 2 : 0);
  }
  function fmtAltDelta(m: number): string {
    const a = convertAltitude(m, $settings.interface.altitudeUnit);
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
    return `${stats.hasUnlimitedHold ? '≥ ' : ''}${fmtDuration(stats.estTimeS)}`;
  });

  function formatAltShort(wp: ArduWaypoint): string {
    return cmdHasLocation(wp.command) ? `${wp.alt.toFixed(0)}m ${frameLabel(wp.frame)}` : '—';
  }

  /** Catalog-driven one-line param summary (enum labels / value+unit; non-zero numbers only). */
  function paramSummary(wp: ArduWaypoint): string {
    const def = cmdDef(wp.command);
    if (!def?.params) return '';
    const vals = [wp.param1, wp.param2, wp.param3, wp.param4, wp.lat, wp.lon, wp.alt];
    const parts: string[] = [];
    for (const pidx of [1, 2, 3, 4, 5, 6, 7] as const) {
      const spec = def.params[pidx];
      if (!spec) continue;
      if (spec.advanced) continue; // keep the list summary concise — advanced params are detail-only
      const v = vals[pidx - 1];
      if (spec.enumStrings && spec.enumValues) parts.push(enumLabel(spec, v));
      else if (v !== 0) parts.push(`${v}${spec.units ?? ''}`);
    }
    return parts.slice(0, 3).join(' · ');
  }

</script>

{#snippet toolbar()}
  <div class="miss-toolbar">
    <Button variant="mode" active={currentEditing} icon="edit" onclick={() => arduEditMode.update(v => !v)} title={$t('mission.toggleEdit')}>
      {currentEditing ? $t('mission.editing') : $t('mission.edit')}
    </Button>
    {#if currentEditing}
      <Button variant="mode" active={showPatternPanel} icon="map" onclick={togglePattern}>{$t('survey.pattern')}</Button>
    {/if}
    {#if currentEditing && !showPatternPanel}
      <Button variant="standard" icon="undo" disabled={!$arduCanUndo} onclick={() => arduUndo()} title={$t('mission.undo')} />
      <Button variant="standard" icon="redo" disabled={!$arduCanRedo} onclick={() => arduRedo()} title={$t('mission.redo')} />
    {/if}
    {#if !currentEditing}
      <Button variant="standard" icon="library" onclick={() => missionManagerOpen.set(true)} title={$t('mission.missionManager')}>
        {$t('mission.missionManager')}
      </Button>
    {/if}
    <div class="tb-spacer"></div>
    {#if showVehicleSelect}
      <select
        class="ap-vehicle-select"
        value={currentVehicle}
        disabled={vehicleLocked}
        title={vehicleLocked ? $t('arduMission.vehicleLocked') : $t('arduMission.vehicleClass')}
        onchange={(e) => setArduVehicleClass((e.target as HTMLSelectElement).value as VehicleClass)}
      >
        {#each VEHICLE_OPTIONS as o}
          <option value={o.value}>{$t(o.key)}</option>
        {/each}
      </select>
    {/if}
    {#if currentEditing && currentSelIdx >= 0}
      <Button variant="danger" icon="close" onclick={removeSelected} title={$t('mission.removeWp')} />
    {/if}
    <Button variant="standard" icon="delete" onclick={handleClear} title={$t('mission.clearMission')} />
  </div>
{/snippet}

{#snippet body()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="miss-dropzone" class:drag-over={dragOver} ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
    {#if showPatternPanel}
      {#await import('./SurveyPatternPanel.svelte')}
        <div class="wp-empty">{$t('survey.loading')}</div>
      {:then { default: SurveyPatternPanel }}
        <SurveyPatternPanel ongenerate={() => { showPatternPanel = false; }} />
      {:catch error}
        <div class="wp-empty">⚠ {String(error)}</div>
      {/await}
    {:else if currentMission.length === 0}
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
          {#each groups as g}
            {#if g.anchor}
              <tr class="wp-row" class:selected={g.anchorIdx === currentSelIdx} onclick={() => arduSelectedWpIndex.set(g.anchorIdx)}>
                <td class="col-num"><span class="wp-num-badge">{g.anchorIdx + 1}</span></td>
                <td class="col-type">
                  {cmdShort(g.anchor.command)}
                  {#if cmdInvalid(g.anchor.command)}<span class="wp-warn" title={$t('arduMission.cmdInvalidForVehicle')}>⚠</span>{/if}
                </td>
                <td class="col-alt">{formatAltShort(g.anchor)}</td>
                <td class="col-param">{paramSummary(g.anchor)}</td>
              </tr>
            {/if}
            {#each g.modifiers as m}
              <tr class="wp-row wp-mod-row" class:selected={m.idx === currentSelIdx} onclick={() => arduSelectedWpIndex.set(m.idx)}>
                <td class="col-num"><span class="wp-num-badge mod">{m.idx + 1}</span></td>
                <td class="col-type">
                  {cmdShort(m.wp.command)}
                  {#if cmdInvalid(m.wp.command)}<span class="wp-warn" title={$t('arduMission.cmdInvalidForVehicle')}>⚠</span>{/if}
                </td>
                <td class="col-alt">—</td>
                <td class="col-param">{paramSummary(m.wp)}</td>
              </tr>
            {/each}
          {/each}
        </tbody>
      </table>
    {/if}
    {#if dragOver}<div class="drop-overlay">{$t('arduMission.dropHint')}</div>{/if}
  </div>
{/snippet}

{#snippet footer()}
  <div class="miss-footer">
    {#if currentSelIdx >= 0 && currentSelIdx < currentMission.length}
      {@const wp = currentMission[currentSelIdx]}
      <div class="wp-detail">
        <div class="detail-header">WP {currentSelIdx + 1} — {cmdName(wp.command)}</div>
        {#each arduWpDetailLines(wp, $t) as p}
          <div class="detail-row"><span class="detail-label">{p.label}</span><span class="detail-value">{p.value}</span></div>
        {/each}
        {#if currentEditing}<div class="detail-hint">{$t('mission.clickMarkerHint')}</div>{/if}
      </div>
    {/if}

    <div class="ctrl-row">
      <Button variant="data" icon="download" full disabled={!isMavlinkConnected} onclick={handleFcDownload}>{$t('mission.fcDownload')}</Button>
      <Button variant="data" icon="upload" full disabled={!isMavlinkConnected} onclick={handleFcUpload}>{$t('mission.fcUpload')}</Button>
    </div>
    <div class="ctrl-row">
      <Button variant="data" icon="library" full disabled={currentMission.length === 0} onclick={handleSaveToLibrary}>{$t('mission.saveToLibrary')}</Button>
    </div>
    <div class="ctrl-row">
      <Button variant="standard" icon="folder" full onclick={handleOpenFile}>{$t('mission.open')}</Button>
      <Button variant="standard" icon="save" full onclick={handleSaveFile}>{$t('mission.save')}</Button>
    </div>

    {#if statusMessage}<div class="mission-status">{statusMessage}</div>{/if}
    {#if invalidCount > 0}<div class="mission-warn">⚠ {$t('arduMission.cmdInvalidCount', { values: { count: invalidCount } })}</div>{/if}
    {#if stats.geoCount >= 2}
      <div class="mission-stats">
        <span class="stat" title={$t('mission.statDistance')}>⤢ {fmtDist(stats.legDistanceM)}</span>
        <span class="stat" title={$t('mission.statClimbDescent')}>↑{fmtAltDelta(stats.climbM)} ↓{fmtAltDelta(stats.descentM)}</span>
        {#if estTimeText}
          <span class="stat" title={$t('mission.statTime')}>⏱ {estTimeText}</span>
        {/if}
      </div>
    {/if}
    {#if currentMission.length > 0}
      <div class="mission-summary">
        {currentMission.length} WPs
        {#if $arduMissionModified}<span class="dirty-badge">{$t('mission.modified')}</span>{/if}
        {#if $arduMissionFlags.fc}<span class="prov-badge prov-fc" title={$t('mission.provFcTip')}>{$t('mission.provFc')}</span>{/if}
        {#if $arduMissionFlags.file}<span class="prov-badge prov-file" title={$t('mission.provFileTip')}>{$t('mission.provFile')}</span>{/if}
        {#if $arduMissionFlags.db}<span class="prov-badge prov-db" title={$t('mission.provDbTip')}>{$t('mission.provDb')}</span>{/if}
      </div>
    {/if}
  </div>
{/snippet}

{#if $missionManagerOpen}
  <MissionManager onBack={() => missionManagerOpen.set(false)} />
{:else}
  <div class="amv2">
    <PanelShell variant="compact" title={$t('nav.mission')} {toolbar} {body} {footer}>
      {#snippet headerActions()}<AutopilotSelect />{/snippet}
    </PanelShell>
  </div>
{/if}

<svelte:window onkeydown={onKeydown} />

<ConfirmDialog bind:this={confirmDialog} />
<MissionSaveDialog bind:this={missionSaveDialog} />

<style>
  .miss-toolbar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; width: 100%; }
  .tb-spacer { flex: 1; }
  /* Vehicle-class dropdown — matches the framework form-control height (28px). */
  .ap-vehicle-select {
    height: 28px;
    padding: 0 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
    cursor: pointer;
  }
  .ap-vehicle-select:disabled { opacity: 0.55; cursor: not-allowed; color: #f39c12; }

  .miss-dropzone { position: relative; min-height: 100%; }
  .miss-dropzone.drag-over { outline: 2px dashed #37a8db; outline-offset: -2px; border-radius: 4px; }

  .wp-empty { padding: 16px; text-align: center; color: #888; font-size: 13px; }
  .wp-table { width: 100%; border-collapse: collapse; font-size: 12px; }
  .wp-table thead { position: sticky; top: 0; z-index: 1; }
  .wp-table th { background: #1e1e1e; color: #888; font-weight: 600; font-size: 11px; text-transform: uppercase; padding: 4px 5px; text-align: left; border-bottom: 1px solid #444; }
  .wp-row { cursor: pointer; border-bottom: 1px solid #2a2a2a; color: #ccc; }
  .wp-row:hover { background: #2a2a2a; }
  .wp-row.selected { background: #1a3a5c; color: #fff; }
  .wp-row td { padding: 4px 5px; white-space: nowrap; }
  .col-num { width: 30px; text-align: center; }
  .col-type { width: 40px; }
  .col-alt { width: 72px; color: #8bc34a; }
  .col-param { color: #aaa; }
  .wp-num-badge { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; border-radius: 50%; background: #37a8db; color: #fff; font-size: 10px; font-weight: bold; }
  /* Modifier sub-rows: indented + smaller, like INAV modifier waypoints, but numbered. */
  .wp-mod-row td { font-size: 11px; color: #aaa; }
  .wp-mod-row .col-num { padding-left: 14px; }
  .wp-num-badge.mod { width: 17px; height: 17px; font-size: 9px; background: #5a6b75; }

  .miss-footer { width: 100%; display: flex; flex-direction: column; gap: 4px; }
  /* No internal scroll: the PanelShell footer is pinned and the column scrolls as a last resort, so a
     full WP's params show without a spurious inner scrollbar (the detail is bounded — a handful of rows). */
  .wp-detail { padding: 6px 8px; border: 1px solid #333; border-radius: 4px; background: #1e1e1e; }
  .detail-header { font-weight: bold; font-size: 13px; color: #37a8db; margin-bottom: 4px; padding-bottom: 3px; border-bottom: 1px solid #333; }
  .detail-row { display: flex; justify-content: space-between; padding: 1px 0; font-size: 12px; }
  .detail-label { color: #888; font-size: 11px; }
  .detail-value { color: #ccc; font-size: 12px; }
  .detail-hint { color: #37a8db; font-size: 11px; text-align: center; margin-top: 4px; font-style: italic; }
  .ctrl-row { display: flex; gap: 4px; }
  .mission-status { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; }
  .mission-warn { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; }
  .mission-stats { display: flex; align-items: center; justify-content: center; gap: 12px; padding: 4px 3px 0; font-size: 12px; color: #9ad0e8; flex-wrap: wrap; font-variant-numeric: tabular-nums; }
  .mission-stats .stat { white-space: nowrap; cursor: default; }
  .mission-summary { display: flex; align-items: center; justify-content: center; gap: 4px; padding: 3px; font-size: 12px; color: #888; flex-wrap: wrap; }
  .dirty-badge { background: #f39c12; color: #1a1a1a; padding: 1px 6px; border-radius: 8px; font-size: 11px; font-weight: bold; }
  .prov-badge { color: #fff; padding: 1px 6px; border-radius: 8px; font-size: 11px; font-weight: bold; }
  .prov-fc { background: #37a8db; }
  .prov-file { background: #6c7a89; }
  .prov-db { background: #59aa29; }
  .wp-warn { color: #f39c12; margin-left: 3px; cursor: help; }
  .drop-overlay { position: absolute; inset: 0; background: rgba(55,168,219,0.15); border: 2px dashed #37a8db; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: #37a8db; font-size: 13px; font-weight: bold; z-index: 10; pointer-events: none; }
</style>
