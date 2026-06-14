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
  import { t } from 'svelte-i18n';
  import {
    arduMission, arduSelectedWpIndex, arduEditMode,
    arduMissionClear, arduRemoveWp, groupArduMission,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    serializeWaypoints, parseWaypoints,
    type ArduWaypoint,
  } from '$lib/stores/missionArdupilot';
  import { cmdName, cmdShort, cmdHasLocation, cmdDef, enumLabel } from '$lib/helpers/arduCommandCatalog';
  import { connection } from '$lib/stores/connection';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import AutopilotSelect from '$lib/components/mission/AutopilotSelect.svelte';

  let currentMission  = $state<ArduWaypoint[]>(get(arduMission));
  let currentSelIdx   = $state<number>(get(arduSelectedWpIndex));
  let currentEditing  = $state<boolean>(get(arduEditMode));
  let currentConn     = $state(get(connection));
  let statusMessage   = $state('');
  let dragOver        = $state(false);
  // Auto-clear the transient status line after 10s — persistent state is shown by the flags.
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });

  const unsubMission  = arduMission.subscribe(m => { currentMission = m; });
  const unsubSelIdx   = arduSelectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = arduEditMode.subscribe(e => { currentEditing = e; });
  const unsubConn     = connection.subscribe(c => { currentConn = c; });

  const isMavlinkConnected = $derived(
    currentConn.status === 'connected' && currentConn.protocolType === 'mavlink'
  );

  onDestroy(() => { unsubMission(); unsubSelIdx(); unsubEditMode(); unsubConn(); });

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
      statusMessage = $t('mission.loaded', { values: { count: wps.length } });
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
      statusMessage = $t('mission.loadedFromFile', { values: { count: wps.length, file: file.name } });
    } catch (e) {
      statusMessage = $t('mission.importFailed', { values: { error: String(e) } });
    }
  }

  function handleClear() { arduMissionClear(); statusMessage = $t('mission.missionCleared'); }
  function removeSelected() { if (currentSelIdx >= 0) arduRemoveWp(currentSelIdx); }

  async function handleFcDownload() {
    statusMessage = $t('arduMission.downloading');
    try {
      const wps = await invoke<ArduWaypoint[]>('ardu_mission_download');
      arduMission.set(wps);
      arduSelectedWpIndex.set(-1);
      statusMessage = $t('mission.downloaded', { values: { count: wps.length } });
    } catch (e) {
      statusMessage = $t('mission.downloadFailed', { values: { error: String(e) } });
    }
  }

  async function handleFcUpload() {
    const wps = get(arduMission);
    if (wps.length === 0) { statusMessage = $t('mission.noWpToUpload'); return; }
    statusMessage = $t('arduMission.uploading');
    try {
      await invoke<void>('ardu_mission_upload', { waypoints: wps });
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

  /** Per-param detail entries (label + display value) for the footer detail. */
  function paramEntries(wp: ArduWaypoint): { label: string; display: string }[] {
    const def = cmdDef(wp.command);
    if (!def?.params) return [];
    const vals = [wp.param1, wp.param2, wp.param3, wp.param4, wp.lat, wp.lon, wp.alt];
    const out: { label: string; display: string }[] = [];
    for (const pidx of [1, 2, 3, 4, 5, 6, 7] as const) {
      const spec = def.params[pidx];
      if (!spec) continue;
      const v = vals[pidx - 1];
      const display = spec.enumStrings && spec.enumValues ? enumLabel(spec, v) : `${v}${spec.units ? ' ' + spec.units : ''}`;
      out.push({ label: spec.label, display });
    }
    return out;
  }

  function formatCoord(valE7: number): string { return (valE7 / 1e7).toFixed(6); }
</script>

{#snippet toolbar()}
  <div class="miss-toolbar">
    <Button variant="mode" active={currentEditing} icon="edit" onclick={() => arduEditMode.update(v => !v)} title={$t('mission.toggleEdit')}>
      {currentEditing ? $t('mission.editing') : $t('mission.edit')}
    </Button>
    <div class="tb-spacer"></div>
    {#if currentEditing && currentSelIdx >= 0}
      <Button variant="danger" icon="close" onclick={removeSelected} title={$t('mission.removeWp')} />
    {/if}
    <Button variant="standard" icon="delete" onclick={handleClear} title={$t('mission.clearMission')} />
  </div>
{/snippet}

{#snippet body()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="miss-dropzone" class:drag-over={dragOver} ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
    {#if currentMission.length === 0}
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
                <td class="col-type">{cmdShort(g.anchor.command)}</td>
                <td class="col-alt">{formatAltShort(g.anchor)}</td>
                <td class="col-param">{paramSummary(g.anchor)}</td>
              </tr>
            {/if}
            {#each g.modifiers as m}
              <tr class="wp-row wp-mod-row" class:selected={m.idx === currentSelIdx} onclick={() => arduSelectedWpIndex.set(m.idx)}>
                <td class="col-num"><span class="wp-num-badge mod">{m.idx + 1}</span></td>
                <td class="col-type">{cmdShort(m.wp.command)}</td>
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
        {#if cmdHasLocation(wp.command)}
          <div class="detail-row"><span class="detail-label">{$t('mission.lat')}</span><span class="detail-value">{formatCoord(wp.lat)}</span></div>
          <div class="detail-row"><span class="detail-label">{$t('mission.lon')}</span><span class="detail-value">{formatCoord(wp.lon)}</span></div>
          <div class="detail-row"><span class="detail-label">{$t('mission.alt')}</span><span class="detail-value">{formatAltShort(wp)}</span></div>
        {/if}
        {#each paramEntries(wp) as p}
          <div class="detail-row"><span class="detail-label">{p.label}</span><span class="detail-value">{p.display}</span></div>
        {/each}
        {#if currentEditing}<div class="detail-hint">{$t('mission.clickMarkerHint')}</div>{/if}
      </div>
    {/if}

    <div class="ctrl-row">
      <Button variant="data" icon="download" full disabled={!isMavlinkConnected} onclick={handleFcDownload}>{$t('mission.fcDownload')}</Button>
      <Button variant="data" icon="upload" full disabled={!isMavlinkConnected} onclick={handleFcUpload}>{$t('mission.fcUpload')}</Button>
    </div>
    <div class="ctrl-row">
      <Button variant="standard" icon="folder" full onclick={handleOpenFile}>{$t('mission.open')}</Button>
      <Button variant="standard" icon="save" full onclick={handleSaveFile}>{$t('mission.save')}</Button>
    </div>

    {#if statusMessage}<div class="mission-status">{statusMessage}</div>{/if}
    {#if currentMission.length > 0}<div class="mission-summary">{currentMission.length} WPs</div>{/if}
  </div>
{/snippet}

<div class="amv2">
  <PanelShell variant="compact" title={$t('nav.mission')} {toolbar} {body} {footer}>
    {#snippet headerActions()}<AutopilotSelect />{/snippet}
  </PanelShell>
</div>

<style>
  .miss-toolbar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; width: 100%; }
  .tb-spacer { flex: 1; }

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
  .wp-detail { padding: 6px 8px; border: 1px solid #333; border-radius: 4px; background: #1e1e1e; max-height: 180px; overflow-y: auto; }
  .detail-header { font-weight: bold; font-size: 13px; color: #37a8db; margin-bottom: 4px; padding-bottom: 3px; border-bottom: 1px solid #333; }
  .detail-row { display: flex; justify-content: space-between; padding: 1px 0; font-size: 12px; }
  .detail-label { color: #888; font-size: 11px; }
  .detail-value { color: #ccc; font-size: 12px; }
  .detail-hint { color: #37a8db; font-size: 11px; text-align: center; margin-top: 4px; font-style: italic; }
  .ctrl-row { display: flex; gap: 4px; }
  .mission-status { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; }
  .mission-summary { display: flex; justify-content: center; padding: 3px; font-size: 12px; color: #888; }
  .drop-overlay { position: absolute; inset: 0; background: rgba(55,168,219,0.15); border: 2px dashed #37a8db; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: #37a8db; font-size: 13px; font-weight: bold; z-index: 10; pointer-events: none; }
</style>
