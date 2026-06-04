<!-- ArduMissionPanelV2.svelte
     ArduPilot / PX4 mission planner on the panel framework (docs/dev/PANEL_FRAMEWORK.md): a
     `compact` PanelShell. Single mission (no multi-mission tabs). Parallel build alongside the
     legacy ArduMissionPanel; logic identical — only the chrome moves onto PanelShell + <Button>.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { t } from 'svelte-i18n';
  import {
    arduMission, arduSelectedWpIndex, arduEditMode,
    arduMissionClear, arduRemoveWp,
    arduHasLocation, MAV_CMD_LABELS, MAV_CMD_SHORT,
    MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TIME,
    MAV_CMD_NAV_LOITER_TURNS, MAV_CMD_DO_JUMP, MAV_CMD_DO_CHANGE_SPEED,
    MAV_CMD_NAV_WAYPOINT, MAV_CMD_CONDITION_DELAY,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    serializeWaypoints, parseWaypoints,
    type ArduWaypoint,
  } from '$lib/stores/missionArdupilot';
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
    } catch (e: any) {
      statusMessage = $t('mission.saveFailed', { values: { error: e } });
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
    if (!file.name.endsWith('.waypoints') && !file.name.endsWith('.txt')) {
      statusMessage = $t('arduMission.onlyWaypointsFiles'); return;
    }
    try {
      const wps = parseWaypoints(await file.text());
      arduMission.set(wps);
      arduSelectedWpIndex.set(-1);
      statusMessage = $t('mission.loadedFromFile', { values: { count: wps.length, file: file.name } });
    } catch (e: any) {
      statusMessage = $t('mission.importFailed', { values: { error: e } });
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
    } catch (e: any) {
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
    } catch (e: any) {
      statusMessage = $t('mission.uploadFailed', { values: { error: String(e) } });
    }
  }

  function frameLabel(frame: number): string {
    if (frame === MAV_FRAME_GLOBAL) return 'AMSL';
    if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return 'TRN';
    return 'REL';
  }

  function formatAltShort(wp: ArduWaypoint): string {
    return arduHasLocation(wp.command) ? `${wp.alt.toFixed(0)}m ${frameLabel(wp.frame)}` : '—';
  }

  function formatParam(wp: ArduWaypoint): string {
    switch (wp.command) {
      case MAV_CMD_NAV_LOITER_TIME:    return `${wp.param1.toFixed(0)}s`;
      case MAV_CMD_NAV_LOITER_TURNS:   return `×${wp.param1.toFixed(0)}`;
      case MAV_CMD_NAV_LOITER_UNLIM:
      case MAV_CMD_NAV_WAYPOINT:       return wp.param3 > 0 ? `R${wp.param3.toFixed(0)}m` : '';
      case MAV_CMD_DO_JUMP:            return `→${wp.param1.toFixed(0)} ×${wp.param2 < 0 ? '∞' : wp.param2.toFixed(0)}`;
      case MAV_CMD_DO_CHANGE_SPEED:    return `${wp.param2.toFixed(1)}m/s`;
      case MAV_CMD_CONDITION_DELAY:    return `${wp.param1.toFixed(0)}s`;
      default:                         return '';
    }
  }

  function formatCoord(valE7: number): string { return (valE7 / 1e7).toFixed(6); }
  function shortType(cmd: number): string { return MAV_CMD_SHORT[cmd] ?? `C${cmd}`; }
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
          {#each currentMission as wp, i}
            <tr class="wp-row" class:selected={i === currentSelIdx} onclick={() => arduSelectedWpIndex.set(i)}>
              <td class="col-num"><span class="wp-num-badge">{i + 1}</span></td>
              <td class="col-type">{shortType(wp.command)}</td>
              <td class="col-alt">{formatAltShort(wp)}</td>
              <td class="col-param">{formatParam(wp)}</td>
            </tr>
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
        <div class="detail-header">WP {currentSelIdx + 1} — {MAV_CMD_LABELS[wp.command] ?? `CMD${wp.command}`}</div>
        {#if arduHasLocation(wp.command)}
          <div class="detail-row"><span class="detail-label">{$t('mission.lat')}</span><span class="detail-value">{formatCoord(wp.lat)}</span></div>
          <div class="detail-row"><span class="detail-label">{$t('mission.lon')}</span><span class="detail-value">{formatCoord(wp.lon)}</span></div>
          <div class="detail-row"><span class="detail-label">{$t('mission.alt')}</span><span class="detail-value">{formatAltShort(wp)}</span></div>
        {/if}
        {#if wp.command === MAV_CMD_NAV_LOITER_UNLIM || wp.command === MAV_CMD_NAV_LOITER_TIME || wp.command === MAV_CMD_NAV_LOITER_TURNS}
          <div class="detail-row"><span class="detail-label">{$t('arduMission.radius')}</span><span class="detail-value">{wp.param3.toFixed(0)}m</span></div>
        {/if}
        {#if wp.command === MAV_CMD_NAV_LOITER_TIME}
          <div class="detail-row"><span class="detail-label">{$t('mission.hold')}</span><span class="detail-value">{wp.param1.toFixed(0)}s</span></div>
        {/if}
        {#if wp.command === MAV_CMD_NAV_LOITER_TURNS}
          <div class="detail-row"><span class="detail-label">{$t('arduMission.turns')}</span><span class="detail-value">{wp.param1.toFixed(0)}</span></div>
        {/if}
        {#if wp.command === MAV_CMD_DO_JUMP}
          <div class="detail-row"><span class="detail-label">{$t('mission.target')}</span><span class="detail-value">WP {wp.param1.toFixed(0)}</span></div>
          <div class="detail-row"><span class="detail-label">{$t('mission.repeat')}</span><span class="detail-value">{wp.param2 < 0 ? '∞' : wp.param2.toFixed(0)}</span></div>
        {/if}
        {#if wp.command === MAV_CMD_DO_CHANGE_SPEED}
          <div class="detail-row"><span class="detail-label">{$t('arduMission.speed')}</span><span class="detail-value">{wp.param2.toFixed(1)} m/s</span></div>
        {/if}
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
