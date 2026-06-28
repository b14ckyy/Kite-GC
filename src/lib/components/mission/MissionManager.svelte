<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- MissionManager.svelte
     Mission library on the panel framework (docs/active/PANEL_FRAMEWORK.md): its own PanelShell
     (compact ↔ advanced 1:2) — grouped mission list in the main field, mission detail (preview
     map · name/notes · stats · linked flights) in the detail field. Opened from the mission
     planner's "Mission Manager" toggle.
-->
<script lang="ts">
  import { t, locale } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { save, open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { settings } from '$lib/stores/settings';
  import {
    missionDbList, missionDbFlights, missionDbDelete, missionDbSetMeta,
    missionExportFileFromJson, missionDbGeocode, missionDbSave,
    formatDurationSec,
  } from '$lib/stores/flightlog';
  import type { LibraryMission, FlightSummary } from '$lib/stores/flightlogTypes';
  import {
    mission, missionModified, missionSetWaypoints, loadedMissionId, markMissionSynced,
    missionImportXml, missionLoadFile, applyMissionLaunchDefault, type Waypoint,
  } from '$lib/stores/mission';
  import {
    arduMission, arduSelectedWpIndex, arduLoadedMissionId, markArduMissionSynced, arduClearUndoHistory,
    parseWaypoints, type ArduWaypoint,
  } from '$lib/stores/missionArdupilot';
  import { frameMissionOnMap } from '$lib/stores/mapCamera';
  import {
    autopilotSystem, autopilotLocked, setAutopilotSystem, confirmSystemSwitch,
    pendingSystemSwitch, type AutopilotSystem,
  } from '$lib/stores/autopilotContext';
  import { missionManagerSelectedId, requestOpenFlightId } from '$lib/stores/missionManager';
  import { buildMissionInput, findLibraryMissionId } from '$lib/helpers/missionLibrary';
  import { buildArduMissionInput, findArduLibraryMissionId } from '$lib/helpers/missionLibraryArdu';
  import { convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import MissionPreviewMap from '$lib/components/mission/MissionPreviewMap.svelte';

  let { onBack }: { onBack: () => void } = $props();

  let ui = $derived($settings.interface);
  let confirmDialog: ReturnType<typeof ConfirmDialog>;

  let missions = $state<LibraryMission[]>([]);
  let linkedFlights = $state<FlightSummary[]>([]);
  let collapsed = $state<Set<string>>(new Set());
  let statusMessage = $state('');
  // Auto-clear the transient status line after 10s — persistent state is shown elsewhere.
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });
  let nameDraft = $state('');
  let notesDraft = $state('');
  let dragOver = $state(false);

  const UNKNOWN = ''; // sorts first; rendered as the localized "unknown" label

  let selected = $derived(missions.find((m) => m.id === $missionManagerSelectedId) ?? null);
  const variant = $derived<PanelVariant>(selected != null ? 'advanced' : 'compact');

  // ── Format filter (shown only when the library holds ≥2 mission formats) ──────────
  let formatFilter = $state<string>('all');

  /** Distinct formats present in the library, ordered INAV → ArduPilot → PX4 → other. */
  let presentFormats = $derived.by(() => {
    const set = new Set<string>();
    for (const m of missions) set.add(m.format || 'inav');
    const order = ['inav', 'ardupilot', 'px4'];
    const rank = (f: string) => { const i = order.indexOf(f); return i < 0 ? 99 : i; };
    return [...set].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
  });

  // Drop a stale filter (its last mission was deleted) back to "all".
  $effect(() => {
    if (formatFilter !== 'all' && !presentFormats.includes(formatFilter)) formatFilter = 'all';
  });

  let filteredMissions = $derived(
    formatFilter === 'all' ? missions : missions.filter((m) => (m.format || 'inav') === formatFilter),
  );

  /** Preview aspect ratio (width / height) = the mission's geographic bbox aspect. */
  let previewAspect = $derived.by(() => {
    const m = selected;
    if (!m || m.bndbox_min_lat == null || m.bndbox_min_lon == null || m.bndbox_max_lat == null || m.bndbox_max_lon == null) return 1.6;
    const meanLat = ((m.bndbox_min_lat + m.bndbox_max_lat) / 2) * Math.PI / 180;
    const lonSpan = Math.max(1e-9, (m.bndbox_max_lon - m.bndbox_min_lon) * Math.cos(meanLat));
    const latSpan = Math.max(1e-9, m.bndbox_max_lat - m.bndbox_min_lat);
    return Math.min(3, Math.max(1, lonSpan / latSpan));
  });

  function dbPath(): string { return get(settings).flightLogDbPath; }
  function lang(): string { return get(locale) ?? 'en'; }

  /** Group missions by location_name (missions without one go to the Unknown group). */
  let groups = $derived.by(() => {
    const map = new Map<string, LibraryMission[]>();
    for (const m of filteredMissions) {
      const key = m.location_name?.trim() ? m.location_name.trim() : UNKNOWN;
      const arr = map.get(key);
      if (arr) arr.push(m);
      else map.set(key, [m]);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  });

  async function reload() {
    try {
      missions = await missionDbList(dbPath());
    } catch (e) {
      statusMessage = $t('missionMgr.loadFailed', { values: { error: String(e) } });
    }
  }

  async function loadDetails(id: number) {
    const m = missions.find((x) => x.id === id);
    nameDraft = m?.name ?? '';
    notesDraft = m?.notes ?? '';
    try {
      linkedFlights = await missionDbFlights(id, dbPath());
    } catch {
      linkedFlights = [];
    }
    if (m && !m.location_name) {
      const name = await missionDbGeocode(id, lang(), dbPath()).catch(() => null);
      if (name) await reload();
    }
  }

  function select(id: number) {
    missionManagerSelectedId.set(id);
    void loadDetails(id);
  }

  function toggleGroup(key: string) {
    const n = new Set(collapsed);
    if (n.has(key)) n.delete(key); else n.add(key);
    collapsed = n;
  }

  async function saveMeta() {
    const id = get(missionManagerSelectedId);
    if (id == null) return;
    try {
      await missionDbSetMeta(id, nameDraft.trim(), notesDraft.trim() || null, dbPath());
      await reload();
      statusMessage = $t('missionMgr.saved');
    } catch (e) {
      statusMessage = $t('missionMgr.saveFailed', { values: { error: String(e) } });
    }
  }

  /** The autopilot system a library mission belongs to (its DB `format` → active-system enum). */
  function formatToSystem(fmt: string): AutopilotSystem {
    return fmt === 'ardupilot' || fmt === 'px4' ? (fmt as AutopilotSystem) : 'inav';
  }

  function sameFamily(a: AutopilotSystem, b: AutopilotSystem): boolean {
    return (a === 'ardupilot' || a === 'px4') && (b === 'ardupilot' || b === 'px4');
  }

  /** Short badge label for a non-INAV mission format. */
  function formatBadge(fmt: string): string {
    if (fmt === 'ardupilot') return 'ArduPilot';
    if (fmt === 'px4') return 'PX4';
    return fmt.toUpperCase();
  }

  /** Readable name for an autopilot system (switch-dialog message). */
  function systemLabel(sys: AutopilotSystem): string {
    if (sys === 'ardupilot') return 'ArduPilot';
    if (sys === 'px4') return 'PX4';
    return 'INAV';
  }

  /** Cross-format guard: switch the active editor to the system the mission belongs to. INAV and
   *  ArduPilot keep separate stores, so the switch never destroys data — the user chooses whether to
   *  keep the current editor's mission in memory or discard it. Returns false if blocked (FC locked
   *  to another system) or cancelled. */
  async function ensureSystemForFormat(targetSys: AutopilotSystem): Promise<boolean> {
    const cur = get(autopilotSystem);
    if (cur === targetSys) return true;
    if (get(autopilotLocked)) {
      statusMessage = $t('missionMgr.systemLocked');
      return false;
    }
    const curHasMission = cur === 'inav'
      ? get(mission).waypoints.length > 0
      : get(arduMission).length > 0;
    let action: 'keep' | 'clear' = 'keep';
    if (curHasMission && !sameFamily(cur, targetSys)) {
      const ans = await confirmDialog.show({
        title: $t('mission.systemSwitchTitle'),
        message: $t('mission.systemSwitchChooseBody', { values: { from: systemLabel(cur) } }),
        buttons: [
          { label: $t('mission.switchClear'), value: 'clear', danger: true },
          { label: $t('mission.switchKeep'), value: 'keep', primary: true },
        ],
      });
      if (ans == null) return false;
      action = ans as 'keep' | 'clear';
    }
    setAutopilotSystem(targetSys);
    // setAutopilotSystem defers a cross-family switch to a pending request when the source has a
    // mission — resolve it synchronously here with the user's choice (no global dialog flashes).
    if (get(pendingSystemSwitch)) confirmSystemSwitch(action);
    return get(autopilotSystem) === targetSys;
  }

  async function loadToMap(m: LibraryMission) {
    const targetSys = formatToSystem(m.format);
    const curSys = get(autopilotSystem);

    if (targetSys === curSys) {
      // Same system: warn before discarding unsaved changes in the active editor.
      const dirty = curSys === 'inav'
        ? (get(missionModified) && get(mission).waypoints.length > 0)
        : (get(arduMission).length > 0 && get(arduLoadedMissionId) !== m.id);
      if (dirty) {
        const ans = await confirmDialog.show({
          title: $t('missionMgr.replaceTitle'),
          message: $t('missionMgr.replaceMsg'),
          buttons: [{ label: $t('missionMgr.replaceYes'), value: 'replace', primary: true }],
        });
        if (ans !== 'replace') return;
      }
    } else if (!(await ensureSystemForFormat(targetSys))) {
      return;
    }

    try {
      if (targetSys === 'inav') {
        const loaded = await missionSetWaypoints(JSON.parse(m.waypoints_json));
        loadedMissionId.set(m.id);
        markMissionSynced('db');
        // Launch/home reference (REL altitude + 3D-preview): UAV HOME → the DB-saved home → WP1.
        applyMissionLaunchDefault(
          loaded,
          m.home_lat != null && m.home_lon != null ? { lat: m.home_lat, lng: m.home_lon } : undefined,
          true, // fresh load → reset to this mission's home/first-WP, don't keep the previous launch
        );
      } else {
        const wps = JSON.parse(m.waypoints_json) as ArduWaypoint[];
        arduMission.set(wps);
        arduSelectedWpIndex.set(-1);
        arduLoadedMissionId.set(m.id);
        arduClearUndoHistory(); // loaded mission = fresh undo baseline
        markArduMissionSynced('db', wps);
      }
      frameMissionOnMap(); // standalone library load → frame the mission (not a replay-linked attach)
      onBack();
    } catch (e) {
      statusMessage = $t('missionMgr.loadToMapFailed', { values: { error: String(e) } });
    }
  }

  async function exportMission(m: LibraryMission) {
    try {
      const path = await save({
        title: $t('missionMgr.exportTitle'),
        defaultPath: `${(m.name || 'mission').replace(/[^\w\-]+/g, '_')}.mission`,
        filters: [{ name: 'Mission', extensions: ['mission'] }],
      });
      if (!path) return;
      await missionExportFileFromJson(path, m.waypoints_json);
      statusMessage = $t('missionMgr.exported');
    } catch (e) {
      statusMessage = $t('missionMgr.exportFailed', { values: { error: String(e) } });
    }
  }

  async function deleteMission(m: LibraryMission) {
    const count = linkedFlights.length;
    const ans = await confirmDialog.show({
      title: $t('missionMgr.deleteTitle'),
      message: count > 0
        ? $t('missionMgr.deleteMsgLinked', { values: { count: String(count) } })
        : $t('missionMgr.deleteMsg'),
      buttons: [{ label: $t('missionMgr.deleteYes'), value: 'delete', danger: true }],
    });
    if (ans !== 'delete') return;
    try {
      await missionDbDelete(m.id, dbPath());
      if (get(missionManagerSelectedId) === m.id) { missionManagerSelectedId.set(null); linkedFlights = []; }
      await reload();
      statusMessage = $t('missionMgr.deleted');
    } catch (e) {
      statusMessage = $t('missionMgr.deleteFailed', { values: { error: String(e) } });
    }
  }

  /** After a file is loaded onto the map, ask whether to also save it to the library. */
  async function importMission(loadedWps: Waypoint[], suggestedName: string) {
    const existingId = await findLibraryMissionId(loadedWps, dbPath()).catch(() => null);
    const ans = await confirmDialog.show({
      title: $t('missionMgr.importTitle'),
      message: $t('missionMgr.importMsg'),
      buttons: [
        { label: $t('missionMgr.importMapOnly'), value: 'map' },
        { label: $t('missionMgr.importDb'), value: 'db', primary: true },
      ],
    });
    if (ans == null) return;
    if (ans === 'db') {
      if (existingId != null) {
        loadedMissionId.set(existingId);
        markMissionSynced('db');
      } else {
        const input = await buildMissionInput(loadedWps, { name: suggestedName });
        const id = await missionDbSave(input, dbPath());
        loadedMissionId.set(id);
        markMissionSynced('db');
        void missionDbGeocode(id, lang(), dbPath()).catch(() => {});
      }
      await reload();
      statusMessage = $t('missionMgr.imported');
    } else {
      if (existingId != null) { loadedMissionId.set(existingId); markMissionSynced('db'); }
      statusMessage = $t('missionMgr.loadedToMap');
    }
  }

  /** After an ArduPilot `.waypoints` file is parsed, ask whether to add it to the library and/or
   *  load it onto the map (mirrors {@link importMission}, over the ArduPilot store). */
  async function importArduMission(wps: ArduWaypoint[], suggestedName: string) {
    const existingId = await findArduLibraryMissionId(wps, dbPath()).catch(() => null);
    const ans = await confirmDialog.show({
      title: $t('missionMgr.importTitle'),
      message: $t('missionMgr.importMsg'),
      buttons: [
        { label: $t('missionMgr.importMapOnly'), value: 'map' },
        { label: $t('missionMgr.importDb'), value: 'db', primary: true },
      ],
    });
    if (ans == null) return;
    if (ans === 'db') {
      if (existingId == null) {
        const input = await buildArduMissionInput(wps, { name: suggestedName, format: 'ardupilot' });
        const id = await missionDbSave(input, dbPath());
        void missionDbGeocode(id, lang(), dbPath()).catch(() => {});
      }
      await reload();
      statusMessage = $t('missionMgr.imported');
    } else {
      if (!(await ensureSystemForFormat('ardupilot'))) return;
      arduMission.set(wps);
      arduSelectedWpIndex.set(-1);
      arduLoadedMissionId.set(existingId);
      arduClearUndoHistory(); // loaded mission = fresh undo baseline
      statusMessage = $t('missionMgr.loadedToMap');
    }
  }

  async function handleImportButton() {
    try {
      const path = await open({ title: $t('missionMgr.openTitle'), multiple: false, filters: [{ name: 'Mission', extensions: ['mission', 'waypoints', 'txt'] }] });
      if (!path || typeof path !== 'string') return;
      const stem = path.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '');
      const lower = path.toLowerCase();
      if (lower.endsWith('.waypoints') || lower.endsWith('.txt')) {
        const text = await invoke<string>('read_text_file', { path });
        await importArduMission(parseWaypoints(text), stem || autoName());
      } else {
        const m = await missionLoadFile(path);
        await importMission(m.waypoints, stem || autoName());
      }
    } catch (e) {
      statusMessage = $t('missionMgr.importFailed', { values: { error: String(e) } });
    }
  }

  function autoName(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, '0');
    return `New Mission - ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  function onDragOver(e: DragEvent) { e.preventDefault(); e.stopPropagation(); dragOver = true; }
  function onDragLeave() { dragOver = false; }
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    dragOver = false;
    const file = e.dataTransfer?.files?.[0];
    const name = file?.name.toLowerCase() ?? '';
    if (!file || !(name.endsWith('.mission') || name.endsWith('.waypoints') || name.endsWith('.txt'))) {
      statusMessage = $t('missionMgr.onlyMission'); return;
    }
    try {
      const text = await file.text();
      const stem = file.name.replace(/\.[^.]+$/, '');
      if (name.endsWith('.waypoints') || name.endsWith('.txt')) {
        await importArduMission(parseWaypoints(text), stem || autoName());
      } else {
        const m = await missionImportXml(text);
        await importMission(m.waypoints, stem || autoName());
      }
    } catch (err) {
      statusMessage = $t('missionMgr.importFailed', { values: { error: String(err) } });
    }
  }

  // ── Formatters ─────────────────────────────────────────────────────
  function fmtDist(m: number | null): string {
    if (m == null) return '—';
    const c = convertDistance(m, ui.distanceUnit);
    return formatConverted(c, c.unit === 'm' || c.unit === 'ft' ? 0 : 1);
  }
  function fmtAlt(m: number | null): string {
    if (m == null) return '—';
    return formatConverted(convertAltitude(m, ui.altitudeUnit), 1);
  }
  function fmtDateTime(value: string): string {
    return new Date(value).toLocaleString();
  }

  /** Jump to a flight in the Logbook (request handled by +page). */
  function openFlight(id: number) {
    requestOpenFlightId.set(id);
  }

  function autoResizeNotes(el: HTMLTextAreaElement, allowShrink = false) {
    const current = el.offsetHeight;
    el.style.height = 'auto';
    const extra = el.offsetHeight - el.clientHeight;
    const minH = allowShrink ? 44 : Math.max(44, current);
    el.style.height = Math.max(minH, Math.min(el.scrollHeight + extra, 140)) + 'px';
  }
  function notesAutoSize(el: HTMLTextAreaElement, _value: string) {
    autoResizeNotes(el, true);
    return { update() { autoResizeNotes(el, true); } };
  }

  // Init once: load the list and restore any persisted selection.
  let didInit = false;
  $effect(() => {
    if (didInit) return;
    didInit = true;
    void (async () => {
      await reload();
      const id = get(missionManagerSelectedId);
      if (id != null && missions.some((m) => m.id === id)) await loadDetails(id);
      else if (id != null) missionManagerSelectedId.set(null);
    })();
  });
</script>

{#snippet toolbar()}
  <div class="mmv2-toolbtns">
    <Button variant="standard" icon="library" onclick={onBack}>← {$t('missionMgr.back')}</Button>
    <Button variant="data" icon="import" onclick={handleImportButton}>{$t('missionMgr.import')}</Button>
    {#if presentFormats.length >= 2}
      <select class="fmt-filter" bind:value={formatFilter} title={$t('missionMgr.filterByFormat')}>
        <option value="all">{$t('missionMgr.filterAll')}</option>
        {#each presentFormats as f}
          <option value={f}>{formatBadge(f)}</option>
        {/each}
      </select>
    {/if}
  </div>
{/snippet}

{#snippet body()}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="mmv2-dropzone" class:drag-over={dragOver} ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
    {#if missions.length === 0}
      <div class="panel-empty">
        <span class="panel-empty-icon">🗂</span>
        <span>{$t('missionMgr.empty')}</span>
      </div>
    {:else}
      {#each groups as [key, items] (key)}
        <div class="tree-node">
          <button class="tree-toggle" onclick={() => toggleGroup(key)}>
            <span class="tree-caret">{collapsed.has(key) ? '▸' : '▾'}</span>
            <span class="tree-label">{key === UNKNOWN ? $t('missionMgr.unknownLocation') : key}</span>
            <span class="tree-count">{items.length}</span>
          </button>
          {#if !collapsed.has(key)}
            <div class="tree-items">
              {#each items as m (m.id)}
                <button class="lib-item" class:selected={m.id === $missionManagerSelectedId} onclick={() => select(m.id)}>
                  <div class="lib-item-title">{m.name || $t('missionMgr.unnamed')}</div>
                  <div class="lib-item-meta">
                    <span>{m.wp_count} WP</span>
                    <span>{fmtDist(m.total_distance_m)}</span>
                    <span class="fmt-badge">{formatBadge(m.format || 'inav')}</span>
                  </div>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
    {#if dragOver}<div class="drop-overlay">{$t('missionMgr.dropHint')}</div>{/if}
  </div>
{/snippet}

{#snippet detailToolbar()}
  <div class="mmv2-detail-actions">
    <Button variant="data" icon="map" onclick={() => selected && loadToMap(selected)}>{$t('missionMgr.loadToMap')}</Button>
    <Button variant="data" icon="export" onclick={() => selected && exportMission(selected)}>{$t('missionMgr.export')}</Button>
    <Button variant="danger" icon="delete" onclick={() => selected && deleteMission(selected)}>{$t('missionMgr.delete')}</Button>
  </div>
{/snippet}

{#snippet detail()}
  {#if selected}
    {@const m = selected}
    <div class="mmv2-detail">
      {#if m.bndbox_min_lat != null}
        <div class="preview-wrap" style="aspect-ratio: {previewAspect};">
          {#key m.id}
            <MissionPreviewMap waypointsJson={m.waypoints_json} format={m.format} />
          {/key}
        </div>
      {/if}

      <label class="fld">
        <span class="fld-label">{$t('missionMgr.name')}</span>
        <input class="fld-input" type="text" bind:value={nameDraft} />
      </label>
      <label class="fld">
        <span class="fld-label">{$t('missionMgr.notes')}</span>
        <textarea
          class="fld-input fld-area"
          rows="2"
          bind:value={notesDraft}
          oninput={(e: Event) => autoResizeNotes(e.target as HTMLTextAreaElement)}
          use:notesAutoSize={notesDraft}
        ></textarea>
      </label>
      <div class="mmv2-save-row">
        <Button variant="data" icon="save" onclick={saveMeta}>{$t('missionMgr.saveMeta')}</Button>
      </div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('missionMgr.waypoints')}</span><span class="fc-value">{m.wp_count}</span>
        <span class="fc-label">{$t('missionMgr.distance')}</span><span class="fc-value">{fmtDist(m.total_distance_m)}</span>
        <span class="fc-label">{$t('missionMgr.altDiff')}</span><span class="fc-value">{fmtAlt(m.alt_diff_m)}</span>
        <span class="fc-label">{$t('missionMgr.altRange')}</span><span class="fc-value">{fmtAlt(m.min_alt_m)} … {fmtAlt(m.max_alt_m)}</span>
        <span class="fc-label">{$t('missionMgr.location')}</span><span class="fc-value">{m.location_name || $t('missionMgr.unknownLocation')}</span>
        <span class="fc-label">{$t('missionMgr.created')}</span><span class="fc-value">{new Date(m.created_at + 'Z').toLocaleString()}</span>
      </div>

      <div class="det-flights">
        <div class="section-heading">{$t('missionMgr.linkedFlights')} ({linkedFlights.length})</div>
        {#each linkedFlights as f (f.id)}
          <button class="flight-row" onclick={() => openFlight(f.id)} title={$t('missionMgr.openFlight')}>
            <span class="flight-name">{f.craft_name || $t('missionMgr.unnamed')}</span>
            <span class="flight-meta">{fmtDateTime(f.start_time)} · {formatDurationSec(f.duration_sec)}</span>
          </button>
        {/each}
        {#if linkedFlights.length === 0}<div class="flight-none">{$t('missionMgr.noFlights')}</div>{/if}
      </div>
    </div>
  {/if}
{/snippet}

<div class="mmv2">
  <PanelShell {variant} title={$t('missionMgr.title')} {toolbar} {body} {detailToolbar} {detail} />
</div>

{#if statusMessage}<div class="mmv2-status">{statusMessage}</div>{/if}

<ConfirmDialog bind:this={confirmDialog} />

<style>
  .mmv2-toolbtns { display: flex; align-items: center; gap: 6px; width: 100%; }
  /* Matches the framework form-control height (28px md button), like AutopilotSelect's .ap-select. */
  .fmt-filter { margin-left: auto; height: 28px; padding: 0 8px; font-size: 12px; color: #e0e0e0; background: #434343; border: 1px solid #555; border-radius: 4px; font-family: 'Segoe UI', Tahoma, sans-serif; cursor: pointer; }
  .fmt-filter:focus { outline: none; border-color: #37a8db; }

  .mmv2-detail-actions { display: flex; flex: 1; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }

  .section-heading { margin: 8px 0 6px 0; font-size: 11px; font-weight: 600; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }

  .mmv2-dropzone { position: relative; min-height: 100%; }
  .mmv2-dropzone.drag-over { outline: 2px dashed #37a8db; outline-offset: -2px; border-radius: 4px; }

  .panel-empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding: 40px 0; color: #555; font-size: 12px; }
  .panel-empty-icon { font-size: 28px; opacity: 0.4; }

  .tree-node { margin-bottom: 4px; }
  .tree-toggle { width: 100%; text-align: left; border: 1px solid #555; border-radius: 4px; background: #353535; color: #ddd; cursor: pointer; display: grid; grid-template-columns: 14px minmax(0, 1fr) auto; align-items: center; gap: 6px; padding: 5px 7px; font-size: 12px; font-weight: 600; }
  .tree-toggle:hover { border-color: #37a8db; }
  .tree-caret { color: #9cc6d9; font-size: 11px; line-height: 1; }
  .tree-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-count { font-size: 10px; color: #8fb4c5; background: rgba(55, 168, 219, 0.12); border: 1px solid rgba(55, 168, 219, 0.32); border-radius: 999px; padding: 1px 6px; }
  .tree-items { margin-top: 4px; margin-left: 12px; }

  .lib-item { width: calc(100% - 12px); text-align: left; border: 1px solid #555; border-radius: 4px; background: #383838; color: #ddd; margin-bottom: 4px; padding: 6px; cursor: pointer; }
  .lib-item:hover { border-color: #37a8db; }
  .lib-item.selected { border-color: #37a8db; background: rgba(55, 168, 219, 0.18); }
  .lib-item-title { font-size: 12px; color: #fff; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .lib-item-meta { margin-top: 2px; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; font-size: 10px; color: #aaa; }
  .fmt-badge { font-size: 9px; font-weight: 600; color: #9cc6d9; background: rgba(55, 168, 219, 0.14); border: 1px solid rgba(55, 168, 219, 0.34); border-radius: 999px; padding: 0 6px; text-transform: uppercase; letter-spacing: 0.4px; }

  .mmv2-detail { display: flex; flex-direction: column; gap: 10px; overflow-anchor: none; }
  .preview-wrap { width: 100%; max-height: 300px; flex-shrink: 0; overflow: hidden; border-radius: 4px; }
  .mmv2-save-row { display: flex; }

  .fld { display: block; }
  .fld-label { display: block; font-size: 11px; font-weight: 600; color: #949494; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 3px; }
  .fld-input { box-sizing: border-box; width: 100%; padding: 5px 7px; font-size: 12px; color: #e0e0e0; background: #1f1f1f; border: 1px solid #444; border-radius: 4px; font-family: 'Segoe UI', Tahoma, sans-serif; }
  .fld-input:focus { outline: none; border-color: #37a8db; }
  .fld-area { resize: vertical; }

  .fc-info-grid { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 10px; font-size: 12px; }
  .fc-label { color: #949494; }
  .fc-value { color: #e0e0e0; font-weight: 600; overflow-wrap: anywhere; }

  .det-flights { border-top: 1px solid #333; padding-top: 8px; }
  .flight-row { width: 100%; box-sizing: border-box; display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: 12px; color: #e0e0e0; padding: 4px 6px; background: none; border: none; border-radius: 4px; cursor: pointer; text-align: left; }
  .flight-row:hover { background: rgba(55, 168, 219, 0.15); }
  .flight-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .flight-meta { color: #888; flex-shrink: 0; }
  .flight-none { color: #777; font-size: 12px; padding: 4px 0; }

  .drop-overlay { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(55, 168, 219, 0.18); border: 2px dashed #37a8db; border-radius: 6px; color: #fff; font-weight: 600; pointer-events: none; z-index: 10; }

  .mmv2-status { position: fixed; bottom: 14px; left: 50%; transform: translateX(-50%); z-index: 1001; padding: 6px 12px; font-size: 11px; color: #f39c12; background: rgba(0, 0, 0, 0.8); border-radius: 6px; }
</style>
