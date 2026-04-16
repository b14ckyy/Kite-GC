<!-- MissionPanel.svelte
     Sidebar panel for mission planning.
     - Clean WP list with columns: #, Type, Alt, Params
     - Read-only detail view for selected WP (non-edit mode)
     - Edit mode toggle (editing happens on the map via floating popups)
     - FC upload/download
     - File-based .mission import/export with native file picker + drag&drop
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    mission, selectedWpIndex, editMode,
    missionClear, missionRemoveWp,
    missionDownload, missionUpload,
    missionExportXml, missionImportXml,
    missionSaveFile, missionLoadFile,
    activeMissionIndex, missionCount,
    switchMission, addMission, removeMission, getTotalWpCount,
    MAX_MISSIONS, MAX_WAYPOINTS_TOTAL,
    type Waypoint, type Mission, WpAction, WP_ACTION_LABELS,
    hasLocation, isModifier,
  } from '$lib/stores/mission';
  import { connection } from '$lib/stores/connection';
  import { telemetry, type TelemetryData } from '$lib/stores/telemetry';
  import { get } from 'svelte/store';
  import { save, open } from '@tauri-apps/plugin-dialog';

  // Local reactive state mirroring stores
  let currentMission = $state<Mission>(get(mission));
  let currentSelIdx = $state<number>(get(selectedWpIndex));
  let currentEditing = $state<boolean>(get(editMode));
  let currentMissionIdx = $state<number>(get(activeMissionIndex));
  let currentMissionCount = $state<number>(get(missionCount));

  const unsubMission = mission.subscribe(m => { currentMission = m; });
  const unsubSelIdx = selectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = editMode.subscribe(e => { currentEditing = e; });
  const unsubMissionIdx = activeMissionIndex.subscribe(i => { currentMissionIdx = i; });
  const unsubMissionCount = missionCount.subscribe(c => { currentMissionCount = c; });

  onDestroy(() => { unsubMission(); unsubSelIdx(); unsubEditMode(); unsubTelem(); unsubMissionIdx(); unsubMissionCount(); });

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
    const info = get(connection);
    return info?.status === 'connected';
  }

  function isArmed(): boolean {
    return currentTelem.lastUpdate > 0 && (currentTelem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  }

  // ── FC transfer ──────────────────────────────────────────────────

  async function handleDownload() {
    if (!isConnected()) { statusMessage = 'Not connected'; return; }
    downloadLoading = true;
    statusMessage = '';
    try {
      const m = await missionDownload(false);
      statusMessage = `Downloaded ${m.waypoints.length} WPs`;
    } catch (e: any) {
      statusMessage = `Download failed: ${e}`;
    } finally {
      downloadLoading = false;
    }
  }

  async function handleUpload() {
    if (!isConnected()) { statusMessage = 'Not connected'; return; }
    uploadLoading = true;
    statusMessage = '';
    try {
      const m = await missionUpload(false);
      statusMessage = `Uploaded ${m.waypoints.length} WPs`;
    } catch (e: any) {
      statusMessage = `Upload failed: ${e}`;
    } finally {
      uploadLoading = false;
    }
  }
  // ── EEPROM Save/Load ────────────────────────────────────────────────

  async function handleEepromSave() {
    if (!isConnected() || isArmed()) { statusMessage = isArmed() ? 'Cannot save EEPROM while armed' : 'Not connected'; return; }
    eepromSaveLoading = true;
    statusMessage = '';
    try {
      const m = await missionUpload(true);
      statusMessage = `Saved ${m.waypoints.length} WPs to EEPROM`;
    } catch (e: any) {
      statusMessage = `EEPROM save failed: ${e}`;
    } finally {
      eepromSaveLoading = false;
    }
  }

  async function handleEepromLoad() {
    if (!isConnected()) { statusMessage = 'Not connected'; return; }
    eepromLoadLoading = true;
    statusMessage = '';
    try {
      const m = await missionDownload(true);
      statusMessage = `Loaded ${m.waypoints.length} WPs from EEPROM`;
    } catch (e: any) {
      statusMessage = `EEPROM load failed: ${e}`;
    } finally {
      eepromLoadLoading = false;
    }
  }
  // ── File-based import/export ─────────────────────────────────────

  async function handleSaveFile() {
    try {
      const path = await save({
        title: 'Save Mission',
        defaultPath: 'mission.mission',
        filters: [{ name: 'Mission', extensions: ['mission'] }],
      });
      if (!path) return;
      await missionSaveFile(path);
      statusMessage = 'Mission saved';
    } catch (e: any) {
      statusMessage = `Save failed: ${e}`;
    }
  }

  async function handleOpenFile() {
    try {
      const path = await open({
        title: 'Open Mission',
        multiple: false,
        filters: [{ name: 'Mission', extensions: ['mission'] }],
      });
      if (!path) return;
      const filePath = typeof path === 'string' ? path : path;
      const m = await missionLoadFile(filePath);
      statusMessage = `Loaded ${m.waypoints.length} WPs`;
    } catch (e: any) {
      statusMessage = `Open failed: ${e}`;
    }
  }

  // Drag & drop .mission files
  function onDragOver(e: DragEvent) {
    e.preventDefault();
    dragOver = true;
  }
  function onDragLeave() {
    dragOver = false;
  }
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    const file = files[0];
    if (!file.name.endsWith('.mission')) {
      statusMessage = 'Only .mission files supported';
      return;
    }
    try {
      const xml = await file.text();
      if (!xml.includes('<mission')) {
        statusMessage = 'Invalid mission file';
        return;
      }
      const m = await missionImportXml(xml);
      statusMessage = `Loaded ${m.waypoints.length} WPs from ${file.name}`;
    } catch (e: any) {
      statusMessage = `Import failed: ${e}`;
    }
  }

  // ── Helpers ──────────────────────────────────────────────────────

  function handleClear() {
    removeMission(currentMissionIdx);
    statusMessage = 'Mission cleared';
  }

  function selectWp(index: number) {
    selectedWpIndex.set(index);
  }

  async function removeSelected() {
    if (currentSelIdx >= 0) {
      await missionRemoveWp(currentSelIdx);
      selectedWpIndex.set(-1);
    }
  }

  function formatAltShort(wp: Waypoint): string {
    const m = (wp.altitude / 100).toFixed(0);
    const type = (wp.p3 & 1) ? 'AMSL' : 'REL';
    return `${m}m ${type}`;
  }

  function formatParam(wp: Waypoint): string {
    switch (wp.action) {
      case WpAction.PosholdTime:
        return `${wp.p1}s`;
      case WpAction.Jump:
        return `→${wp.p1} ×${wp.p2 === -1 ? '∞' : wp.p2}`;
      case WpAction.SetHead:
        return wp.p1 === -1 ? 'Free' : `${wp.p1}°`;
      case WpAction.Waypoint:
      case WpAction.Land:
        return wp.p1 > 0 ? `${wp.p1}cm/s` : '';
      default:
        return '';
    }
  }

  function formatCoord(val: number): string {
    return (val / 1e7).toFixed(6);
  }

  /** Short type label for table */
  function shortType(action: WpAction): string {
    switch (action) {
      case WpAction.Waypoint: return 'WPT';
      case WpAction.PosholdUnlim: return 'PH∞';
      case WpAction.PosholdTime: return 'PHT';
      case WpAction.Rth: return 'RTH';
      case WpAction.SetPoi: return 'POI';
      case WpAction.Jump: return 'JMP';
      case WpAction.SetHead: return 'HDG';
      case WpAction.Land: return 'LND';
      default: return '?';
    }
  }

  /** Build map: array-index → display number (modifiers get no number) */
  function buildDisplayNumbers(waypoints: Waypoint[]): Map<number, number> {
    const nums = new Map<number, number>();
    let dn = 1;
    for (let i = 0; i < waypoints.length; i++) {
      if (!isModifier(waypoints[i].action)) {
        nums.set(i, dn++);
      }
    }
    return nums;
  }

  const displayNums = $derived(buildDisplayNumbers(currentMission.waypoints));

  /** Index of first mission-terminating WP (LAND or RTH), or -1 */
  function findMissionEndIndex(waypoints: Waypoint[]): number {
    for (let i = 0; i < waypoints.length; i++) {
      if (waypoints[i].action === WpAction.Land || waypoints[i].action === WpAction.Rth) {
        return i;
      }
    }
    return -1;
  }
  const missionEndIdx = $derived(findMissionEndIndex(currentMission.waypoints));
</script>

<div
  class="mission-panel"
  ondragover={onDragOver}
  ondragleave={onDragLeave}
  ondrop={onDrop}
  class:drag-over={dragOver}
>
  <!-- Toolbar -->
  <div class="mission-toolbar">
    <button
      class="btn-edit"
      class:active={currentEditing}
      onclick={() => editMode.update(v => !v)}
      title="Toggle edit mode"
    >
      ✏️ {currentEditing ? 'Editing' : 'Edit'}
    </button>
    <div class="toolbar-spacer"></div>
    {#if currentEditing && currentSelIdx >= 0}
      <button class="btn-sm btn-danger" onclick={removeSelected} title="Remove selected WP">✕</button>
    {/if}
    <button class="btn-sm" onclick={handleClear} title="Clear mission">🗑️</button>
  </div>

  <!-- Multi-Mission Tabs + Waypoint Table Frame -->
  <div class="wp-frame">
    <div class="mission-tabs">
      {#each Array.from({length: currentMissionCount}, (_, i) => i + 1) as n}
        <button
          class="mission-tab"
          class:active={currentMissionIdx === n}
          onclick={() => switchMission(n)}
        >{n}</button>
      {/each}
      {#if currentMissionCount < MAX_MISSIONS}
        <button
          class="mission-tab mission-tab-add"
          onclick={() => { const idx = addMission(); if (idx > 0) switchMission(idx); }}
          title="Add mission"
        >+</button>
      {/if}
    </div>

    <!-- Waypoint Table (scrollable) -->
    <div class="wp-table-wrap">
    {#if currentMission.waypoints.length === 0}
      <div class="wp-empty">
        {#if currentEditing}
          Click on the map to add waypoints
        {:else}
          No waypoints — enable Edit mode or load mission
        {/if}
      </div>
    {:else}
      <table class="wp-table">
        <thead>
          <tr>
            <th class="col-num">#</th>
            <th class="col-type">Type</th>
            <th class="col-alt">Alt</th>
            <th class="col-param">Param</th>
          </tr>
        </thead>
        <tbody>
          {#each currentMission.waypoints as wp, i}
            {#if isModifier(wp.action)}
              <tr
                class="wp-row modifier"
                class:selected={i === currentSelIdx}
                class:greyed={missionEndIdx >= 0 && i > missionEndIdx}
                onclick={() => selectWp(i)}
              >
                <td class="col-num"></td>
                <td class="col-type mod-indent">↳ {shortType(wp.action)}</td>
                <td class="col-alt">—</td>
                <td class="col-param">{formatParam(wp)}</td>
              </tr>
            {:else}
              <tr
                class="wp-row"
                class:selected={i === currentSelIdx}
                class:greyed={missionEndIdx >= 0 && i > missionEndIdx}
                onclick={() => selectWp(i)}
              >
                <td class="col-num">
                  <span class="wp-num-badge">{displayNums.get(i) ?? ''}</span>
                </td>
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
  </div> <!-- /wp-frame -->

  <!-- Selected WP Detail (read-only info, for non-edit or reference) -->
  {#if currentSelIdx >= 0 && currentSelIdx < currentMission.waypoints.length}
    {@const wp = currentMission.waypoints[currentSelIdx]}
    <div class="wp-detail">
      <div class="detail-header">{isModifier(wp.action) ? '' : `WP ${displayNums.get(currentSelIdx) ?? ''} — `}{WP_ACTION_LABELS[wp.action]}</div>
      {#if hasLocation(wp.action)}
        <div class="detail-row">
          <span class="detail-label">Lat</span>
          <span class="detail-value">{formatCoord(wp.lat)}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Lon</span>
          <span class="detail-value">{formatCoord(wp.lon)}</span>
        </div>
      {/if}
      <div class="detail-row">
        <span class="detail-label">Alt</span>
        <span class="detail-value">{formatAltShort(wp)}</span>
      </div>
      {#if wp.action === WpAction.Waypoint || wp.action === WpAction.Land}
        <div class="detail-row">
          <span class="detail-label">Speed</span>
          <span class="detail-value">{wp.p1 > 0 ? `${wp.p1} cm/s` : 'Default'}</span>
        </div>
      {/if}
      {#if wp.action === WpAction.PosholdTime}
        <div class="detail-row">
          <span class="detail-label">Hold</span>
          <span class="detail-value">{wp.p1}s</span>
        </div>
      {/if}
      {#if wp.action === WpAction.PosholdUnlim}
        <div class="detail-row">
          <span class="detail-label">Hold</span>
          <span class="detail-value">Unlimited</span>
        </div>
      {/if}
      {#if wp.action === WpAction.Rth}
        <div class="detail-row">
          <span class="detail-label">Land</span>
          <span class="detail-value">{wp.p1 ? 'Yes' : 'No (hover)'}</span>
        </div>
      {/if}
      {#if wp.action === WpAction.Jump}
        <div class="detail-row">
          <span class="detail-label">Target</span>
          <span class="detail-value">WP {wp.p1}</span>
        </div>
        <div class="detail-row">
          <span class="detail-label">Repeat</span>
          <span class="detail-value">{wp.p2 === -1 ? '∞' : wp.p2}</span>
        </div>
      {/if}
      {#if wp.action === WpAction.SetHead}
        <div class="detail-row">
          <span class="detail-label">Heading</span>
          <span class="detail-value">{wp.p1 === -1 ? 'Free' : `${wp.p1}°`}</span>
        </div>
      {/if}
      <div class="detail-row">
        <span class="detail-label">Flag</span>
        <span class="detail-value">{wp.flag === 0xa5 ? 'LAST' : wp.flag === 0x48 ? 'FBH' : 'Normal'}</span>
      </div>
      {#if currentEditing}
        <div class="detail-hint">Click the marker on the map to edit parameters</div>
      {/if}
    </div>
  {/if}

  <!-- FC + File Controls -->
  <div class="mission-bottom">
    <div class="mission-controls">
    <div class="ctrl-row">
      <button class="btn-ctrl" onclick={handleDownload} disabled={downloadLoading}>
        {downloadLoading ? '⏳' : '⬇️'} FC Download
      </button>
      <button class="btn-ctrl" onclick={handleUpload} disabled={uploadLoading}>
        {uploadLoading ? '⏳' : '⬆️'} FC Upload
      </button>
    </div>
    <div class="ctrl-row">
      <button class="btn-ctrl btn-eeprom" onclick={handleEepromLoad} disabled={eepromLoadLoading}>
        {eepromLoadLoading ? '⏳' : '💾'} EEPROM Load
      </button>
      <button class="btn-ctrl btn-eeprom" onclick={handleEepromSave}
        disabled={eepromSaveLoading || isArmed()}
        title={isArmed() ? 'Cannot write EEPROM while armed' : 'Save mission to EEPROM'}>
        {eepromSaveLoading ? '⏳' : '💾'} EEPROM Save
      </button>
    </div>
    <div class="ctrl-row">
      <button class="btn-ctrl btn-file" onclick={handleOpenFile}>📂 Open</button>
      <button class="btn-ctrl btn-file" onclick={handleSaveFile}>💾 Save</button>
    </div>
    </div>
  </div>

  <!-- Status -->
  {#if statusMessage}
    <div class="mission-status">{statusMessage}</div>
  {/if}

  <!-- Summary -->
  {#if currentMission.waypoints.length > 0}
    <div class="mission-summary">
      {#if currentMissionCount > 1}
        M{currentMissionIdx}: {currentMission.waypoints.length} WPs | Total: {getTotalWpCount()}/{MAX_WAYPOINTS_TOTAL}
      {:else}
        {currentMission.waypoints.length}/{MAX_WAYPOINTS_TOTAL} WPs
      {/if}
      {#if currentMission.dirty}
        <span class="dirty-badge">Modified</span>
      {/if}
    </div>
  {/if}

  <!-- Drop zone overlay -->
  {#if dragOver}
    <div class="drop-overlay">Drop .mission file here</div>
  {/if}
</div>

<style>
  .mission-panel {
    display: flex;
    flex-direction: column;
    gap: 0;
    flex: 1;
    min-height: 0;
    padding: 4px;
    position: relative;
    overflow: hidden;
    color-scheme: dark;
    font-size: 13px;
  }
  .mission-panel.drag-over {
    outline: 2px dashed #37a8db;
    outline-offset: -2px;
  }

  .mission-toolbar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px;
    flex-shrink: 0;
    margin-bottom: 4px;
  }
  .toolbar-spacer { flex: 1; }

  .btn-edit {
    padding: 4px 10px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #2a2a2a;
    color: #ccc;
    cursor: pointer;
    font-size: 13px;
  }
  .btn-edit.active {
    background: #37a8db;
    color: #fff;
    border-color: #37a8db;
  }

  .btn-sm {
    padding: 3px 8px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #2a2a2a;
    color: #ccc;
    cursor: pointer;
    font-size: 13px;
  }
  .btn-sm:hover { background: #3a3a3a; }
  .btn-sm.btn-danger {
    border-color: #c0392b;
    color: #e74c3c;
  }
  .btn-sm.btn-danger:hover {
    background: #c0392b;
    color: #fff;
  }

  /* ── Waypoint Frame (tabs + table) ── */
  .wp-frame {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid #333;
    border-radius: 4px;
    overflow: hidden;
    margin-bottom: 4px;
  }

  .mission-tabs {
    display: flex;
    flex-shrink: 0;
    border-bottom: 1px solid #444;
    background: #1a1a1a;
    padding: 0;
  }
  .mission-tab {
    flex: 1;
    padding: 4px 0;
    border: none;
    background: transparent;
    color: #666;
    cursor: pointer;
    font-size: 11px;
    font-weight: 600;
    text-align: center;
    transition: all 0.15s;
    border-right: 1px solid #333;
  }
  .mission-tab:last-child {
    border-right: none;
  }
  .mission-tab:hover:not(.active) {
    color: #aaa;
    background: #252525;
  }
  .mission-tab.active {
    color: #37a8db;
    background: #1e2e3e;
    border-bottom: 2px solid #37a8db;
  }
  .mission-tab-add {
    min-width: 32px;
    flex: none;
    font-size: 14px;
    font-weight: bold;
    color: #555;
  }
  .mission-tab-add:hover {
    color: #37a8db !important;
    background: #1e2e3e;
  }

  /* ── Waypoint Table ── */
  .wp-table-wrap {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .wp-table-wrap::-webkit-scrollbar {
    width: 6px;
  }
  .wp-table-wrap::-webkit-scrollbar-track {
    background: #1a1a1a;
    border-radius: 3px;
  }
  .wp-table-wrap::-webkit-scrollbar-thumb {
    background: #555;
    border-radius: 3px;
  }
  .wp-table-wrap::-webkit-scrollbar-thumb:hover {
    background: #777;
  }

  .wp-empty {
    padding: 16px;
    text-align: center;
    color: #888;
    font-size: 13px;
  }

  .wp-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .wp-table thead {
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .wp-table th {
    background: #1e1e1e;
    color: #888;
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    padding: 4px 5px;
    text-align: left;
    border-bottom: 1px solid #444;
  }

  .wp-row {
    cursor: pointer;
    border-bottom: 1px solid #2a2a2a;
    color: #ccc;
  }
  .wp-row:hover {
    background: #2a2a2a;
  }
  .wp-row.selected {
    background: #1a3a5c;
    color: #fff;
  }
  .wp-row.modifier {
    color: #999;
    font-style: italic;
  }
  .wp-row.greyed {
    opacity: 0.35;
  }
  .wp-row.greyed .col-alt,
  .wp-row.greyed .col-type,
  .wp-row.greyed .wp-num-badge {
    filter: grayscale(100%);
  }
  .wp-row td {
    padding: 4px 5px;
    white-space: nowrap;
  }

  .col-num { width: 30px; text-align: center; }
  .col-type { width: 40px; }
  .col-alt { width: 72px; color: #8bc34a; }
  .col-param { color: #aaa; }

  .wp-num-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: #37a8db;
    color: #fff;
    font-size: 10px;
    font-weight: bold;
  }
  .mod-indent {
    padding-left: 8px;
    color: #e67e22;
    font-style: italic;
  }

  /* ── WP Detail (read-only) ── */
  .wp-detail {
    padding: 6px 8px;
    border: 1px solid #333;
    border-radius: 4px;
    background: #1e1e1e;
    flex-shrink: 0;
    margin-bottom: 4px;
    max-height: 180px;
    overflow-y: auto;
  }
  .detail-header {
    font-weight: bold;
    font-size: 13px;
    color: #37a8db;
    margin-bottom: 4px;
    padding-bottom: 3px;
    border-bottom: 1px solid #333;
  }
  .detail-row {
    display: flex;
    justify-content: space-between;
    padding: 1px 0;
    font-size: 12px;
    color: #ccc;
  }
  .detail-label {
    color: #888;
    font-size: 11px;
  }
  .detail-value {
    color: #ccc;
    font-size: 12px;
  }
  .detail-hint {
    color: #37a8db;
    font-size: 11px;
    text-align: center;
    margin-top: 4px;
    font-style: italic;
  }

  /* ── Bottom fixed section ── */
  .mission-bottom {
    flex-shrink: 0;
  }

  /* ── Controls ── */
  .mission-controls {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ctrl-row {
    display: flex;
    gap: 4px;
  }
  .btn-ctrl {
    flex: 1;
    padding: 5px 6px;
    border: 1px solid #37a8db;
    border-radius: 4px;
    background: #1a3a5c;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .btn-ctrl:hover:not(:disabled) {
    background: #37a8db;
    color: #fff;
  }
  .btn-ctrl:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-ctrl.btn-file {
    border-color: #555;
    background: #2a2a2a;
    color: #ccc;
  }
  .btn-ctrl.btn-file:hover {
    background: #3a3a3a;
  }
  .btn-ctrl.btn-eeprom {
    border-color: #e67e22;
    background: #3a2a1a;
    color: #e67e22;
  }
  .btn-ctrl.btn-eeprom:hover:not(:disabled) {
    background: #e67e22;
    color: #fff;
  }

  .mission-status {
    padding: 3px 6px;
    font-size: 11px;
    color: #f39c12;
    text-align: center;
    flex-shrink: 0;
  }

  .mission-summary {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 3px;
    font-size: 12px;
    color: #888;
    flex-shrink: 0;
  }
  .dirty-badge {
    background: #f39c12;
    color: #1a1a1a;
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: bold;
  }

  /* Drag & drop overlay */
  .drop-overlay {
    position: absolute;
    inset: 0;
    background: rgba(55, 168, 219, 0.15);
    border: 2px dashed #37a8db;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #37a8db;
    font-size: 13px;
    font-weight: bold;
    z-index: 10;
    pointer-events: none;
  }
</style>
