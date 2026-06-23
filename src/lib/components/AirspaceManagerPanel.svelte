<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Airspace Manager panel (aeronautical data) on the panel framework — the static counterpart to the
  // Radar panel. `advanced` two-column: left = per-layer 2D/3D visibility + the render/list ranges;
  // right = the per-layer grouped nearby list (Obstacles · Airspaces · Airfields). A Compact button
  // collapses it to the `info` variant (list only). See docs/active/AIRSPACE_MANAGER.md.
  import { t } from 'svelte-i18n';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import { aeroData, focusAero, type AeroLayerKey, type Airspace } from '$lib/stores/airspace';
  import {
    geozoneWorking, geozoneDirty, geozoneEditing,
    GEOZONE_TYPE_INCLUSIVE, GEOZONE_TYPE_EXCLUSIVE, GEOZONE_SHAPE_CIRCULAR, GEOZONE_SHAPE_POLYGON,
    GEOZONE_ACTION_NONE, GEOZONE_ACTION_AVOID, GEOZONE_ACTION_POSHOLD, GEOZONE_ACTION_RTH,
    MAX_GEOZONES, addGeozone, deleteGeozone, setGeozoneType, setGeozoneAction, setGeozoneAlts,
    setGeozoneSealevel, setGeozoneRadius, saveGeozoneConfig, revertGeozoneWorking, type GeoZone,
  } from '$lib/stores/geozone';
  import { validateGeozones, ensureCCWConfig } from '$lib/helpers/geozoneSanity';
  import { settings, AERO_DISTANCE_OPTIONS, type DistanceUnit } from '$lib/stores/settings';
  import { haversineDistance } from '$lib/utils/geo';
  import { aeroPointInfo } from '$lib/helpers/airspaceStyle';
  import { geozoneColor, geozoneRadiusM } from '$lib/helpers/geozoneStyle';
  import { convertDistance, formatConverted } from '$lib/utils/units';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import { invoke } from '@tauri-apps/api/core';

  let { reference = null, distanceUnit = 'metric' }: {
    reference?: { lat: number; lon: number } | null;
    distanceUnit?: DistanceUnit;
  } = $props();

  const LIST_CAP = 10; // entries per group, nearest first

  const compact = $derived($settings.airspace.compact);
  const variant = $derived(compact ? ('info' as const) : ('advanced' as const));

  const LAYERS: { key: AeroLayerKey; label: () => string }[] = [
    { key: 'airspaces', label: () => $t('airspace.layerAirspaces') },
    { key: 'geozones', label: () => $t('airspace.layerGeozones') },
    { key: 'obstacles', label: () => $t('airspace.layerObstacles') },
    { key: 'airports', label: () => $t('airspace.layerAirports') },
    { key: 'rc', label: () => $t('airspace.layerRc') },
  ];

  // The Geozones toggle + list only exist when a geozone-capable INAV FC (≥8.0) is connected (the
  // working copy is non-null only then, and is cleared on disconnect → it doubles as the edit gate).
  const hasGeozones = $derived($geozoneWorking?.has_geozones ?? false);
  const visibleLayers = $derived(LAYERS.filter((l) => l.key !== 'geozones' || hasGeozones));

  // Accordion: at most one expanded geozone row.
  let expandedZone = $state<number | null>(null);
  function toggleZone(id: number) { expandedZone = expandedZone === id ? null : id; }

  // ── Geozone editing (P2) ──
  const zones = $derived($geozoneWorking?.zones ?? []);
  const canAddZone = $derived(zones.length < MAX_GEOZONES);
  const issues = $derived(validateGeozones(zones));
  const hasErrors = $derived(issues.some((i) => i.level === 'error'));

  const ACTIONS = [
    { v: GEOZONE_ACTION_NONE, label: () => $t('geozone.actionNone') },
    { v: GEOZONE_ACTION_AVOID, label: () => $t('geozone.actionAvoid') },
    { v: GEOZONE_ACTION_POSHOLD, label: () => $t('geozone.actionPoshold') },
    { v: GEOZONE_ACTION_RTH, label: () => $t('geozone.actionRth') },
  ];

  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  let busy = $state(false);

  // Local altitude mirror (m) for the expanded zone's steppers (NumberStepper needs bind:value; button
  // clicks fire a target-less event). Re-seeded whenever the expanded zone / its data changes.
  let editMinM = $state(0);
  let editMaxM = $state(0);
  let editRadiusM = $state(0);
  $effect(() => {
    const z = zones.find((x) => x.id === expandedZone);
    if (z) {
      editMinM = Math.round(z.min_alt_cm / 100);
      editMaxM = Math.round(z.max_alt_cm / 100);
      editRadiusM = z.radius_cm != null ? Math.round(z.radius_cm / 100) : 0;
    }
  });
  function commitAlts(id: number) {
    setGeozoneAlts(id, Math.round((editMinM || 0) * 100), Math.round((editMaxM || 0) * 100));
  }
  function commitRadius(id: number) { setGeozoneRadius(id, (editRadiusM || 0) * 100); }

  // Transient note shown when the AGL↔MSL conversion can't read a terrain elevation.
  let terrainNote = $state('');

  /** Toggle the altitude reference and convert the lower/upper values via the terrain elevation at the
   *  zone (AGL→MSL adds the ground elevation, MSL→AGL subtracts it) so the physical altitude is kept.
   *  An upper altitude of 0 means "no upper limit" and is left untouched. */
  async function onSealevelToggle(zone: GeoZone, newVal: boolean) {
    if (zone.is_sealevel_ref === newVal) return;
    const c = geozoneCenter(zone);
    let elev: number | null = null;
    if (c) {
      try { elev = await invoke<number | null>('terrain_elevation', { lat: c.lat, lon: c.lon }); }
      catch { elev = null; }
    }
    if (elev != null && isFinite(elev)) {
      const signCm = (newVal ? 1 : -1) * Math.round(elev * 100);
      const newMax = zone.max_alt_cm === 0 ? 0 : zone.max_alt_cm + signCm; // 0 = no upper limit → keep
      setGeozoneAlts(zone.id, zone.min_alt_cm + signCm, newMax);
      terrainNote = '';
    } else {
      terrainNote = $t('geozone.noTerrain');
    }
    setGeozoneSealevel(zone.id, newVal);
  }

  function onAddZone(shape: number) {
    const id = addGeozone(shape);
    if (id != null) { expandedZone = id; geozoneEditing.set(true); }
  }
  function onDeleteZone(id: number) {
    deleteGeozone(id);
    if (expandedZone === id) expandedZone = null;
  }
  function onRevert() { revertGeozoneWorking(); expandedZone = null; }
  async function onSave() {
    if (busy || hasErrors) return;
    const ans = await confirmDialog.show({
      title: $t('geozone.saveTitle'),
      message: $t('geozone.saveMsg'),
      buttons: [{ label: $t('geozone.saveConfirm'), value: 'ok', primary: true }],
    });
    if (ans !== 'ok') return;
    busy = true;
    try {
      geozoneWorking.update((c) => (c ? ensureCCWConfig(c) : c)); // CCW-normalise polygons before write
      await saveGeozoneConfig();
    } finally {
      busy = false;
    }
  }

  /** Representative point of a zone (circle centre / polygon centroid) for the focus-on-click. */
  function geozoneCenter(z: GeoZone): { lat: number; lon: number } | null {
    if (z.vertices.length === 0) return null;
    if (z.shape === GEOZONE_SHAPE_CIRCULAR) return { lat: z.vertices[0].lat / 1e7, lon: z.vertices[0].lon / 1e7 };
    let sx = 0, sy = 0;
    for (const v of z.vertices) { sx += v.lat; sy += v.lon; }
    return { lat: sx / z.vertices.length / 1e7, lon: sy / z.vertices.length / 1e7 };
  }
  function fmtRadius(z: GeoZone): string {
    const r = geozoneRadiusM(z);
    return r != null ? `${Math.round(r)} m` : '—';
  }

  function setVis(key: AeroLayerKey, dim: 'd2' | 'd3', on: boolean) {
    settings.update((s) => ({
      ...s,
      airspace: { ...s.airspace, layers: { ...s.airspace.layers, [key]: { ...s.airspace.layers[key], [dim]: on } } },
    }));
  }
  function setRange(field: 'obstacleDistanceKm' | 'airfieldDistanceKm', km: number) {
    settings.update((s) => ({ ...s, airspace: { ...s.airspace, [field]: km } }));
  }
  function setCompact(v: boolean) {
    settings.update((s) => ({ ...s, airspace: { ...s.airspace, compact: v } }));
  }

  function distM(lat: number, lon: number): number {
    return reference ? haversineDistance(reference.lat, reference.lon, lat, lon) : Number.POSITIVE_INFINITY;
  }
  function fmtDist(m: number): string {
    return m === Number.POSITIVE_INFINITY ? '—' : formatConverted(convertDistance(m, distanceUnit), m < 10000 ? 1 : 0);
  }

  // Nearest vertex distance to an airspace (cheap approximation for sorting/list).
  function airspaceDist(a: Airspace): number {
    let best = Number.POSITIVE_INFINITY;
    for (const ring of a.outlines) {
      for (const [lon, lat] of ring) {
        const d = distM(lat, lon);
        if (d < best) best = d;
      }
    }
    return best;
  }

  // Lists: nearest LIST_CAP per group. Point groups are range-limited (the same range used for 3D
  // rendering); airspaces are sparse → nearest-only. With no reference (no UAV + no GCS) the range
  // filter is skipped so the list isn't empty.
  const hasRef = $derived(reference != null);
  const obstacleMaxM = $derived($settings.airspace.obstacleDistanceKm * 1000);
  const airfieldMaxM = $derived($settings.airspace.airfieldDistanceKm * 1000);

  const nearObstacles = $derived(
    $aeroData.obstacles
      .map((p) => ({ p, d: distM(p.lat, p.lon) }))
      .filter((x) => !hasRef || x.d <= obstacleMaxM)
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );
  const nearAirspaces = $derived(
    $aeroData.airspaces
      .map((a) => ({ a, d: airspaceDist(a) }))
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );
  const nearAirfields = $derived(
    [...$aeroData.airports, ...$aeroData.rcAirfields]
      .map((p) => ({ p, d: distM(p.lat, p.lon) }))
      .filter((x) => !hasRef || x.d <= airfieldMaxM)
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );
</script>

<PanelShell
  {variant}
  title={$t('airspace.title')}
  detailTitle={$t('airspace.nearby')}
  detailActions={compact ? undefined : compactBtn}
  body={compact ? compactBody : controls}
  detail={compact ? undefined : nearbyList}
/>

<ConfirmDialog bind:this={confirmDialog} />

{#snippet compactBtn()}
  <Button variant="standard" size="sm" onclick={() => setCompact(true)}>← {$t('airspace.compact')}</Button>
{/snippet}

{#snippet compactBody()}
  <!-- info variant: clicking the panel re-expands it (like the logbook). -->
  <div
    class="am-compact"
    role="button"
    tabindex="0"
    title={$t('airspace.expand')}
    onclick={() => setCompact(false)}
    onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setCompact(false); } }}
  >
    {@render nearbyList()}
  </div>
{/snippet}

{#snippet controls()}
  <div class="am-controls">
    <div class="am-head-row">
      <span class="am-col-label"></span>
      <span class="am-dim">2D</span>
      <span class="am-dim">3D</span>
    </div>
    {#each visibleLayers as layer}
      <div class="am-row">
        <span class="am-layer">{layer.label()}</span>
        <Toggle checked={$settings.airspace.layers[layer.key].d2} onchange={(c) => setVis(layer.key, 'd2', c)} />
        <Toggle checked={$settings.airspace.layers[layer.key].d3} onchange={(c) => setVis(layer.key, 'd3', c)} />
      </div>
    {/each}

    <div class="am-ranges">
      <label class="am-range">
        <span class="am-range-label">{$t('airspace.rangeObstacles')}</span>
        <select
          class="am-select"
          value={$settings.airspace.obstacleDistanceKm}
          onchange={(e) => setRange('obstacleDistanceKm', Number(e.currentTarget.value))}
        >
          {#each AERO_DISTANCE_OPTIONS as km}<option value={km}>{km} km</option>{/each}
        </select>
      </label>
      <label class="am-range">
        <span class="am-range-label">{$t('airspace.rangeAirfields')}</span>
        <select
          class="am-select"
          value={$settings.airspace.airfieldDistanceKm}
          onchange={(e) => setRange('airfieldDistanceKm', Number(e.currentTarget.value))}
        >
          {#each AERO_DISTANCE_OPTIONS as km}<option value={km}>{km} km</option>{/each}
        </select>
      </label>
    </div>

    {#if hasGeozones}
      <div class="gz-section">
        <div class="gz-head">
          <span class="gz-title">{$t('geozone.title')}</span>
          <span class="gz-count">{zones.length}/{MAX_GEOZONES}</span>
        </div>

        <!-- Toolbar: add zone + the map edit-lock toggle. -->
        <div class="gz-toolbar">
          <button class="gz-add" disabled={!canAddZone} title={$t('geozone.addCircle')} onclick={() => onAddZone(GEOZONE_SHAPE_CIRCULAR)}>○ {$t('geozone.shapeCircle')}</button>
          <button class="gz-add" disabled={!canAddZone} title={$t('geozone.addPolygon')} onclick={() => onAddZone(GEOZONE_SHAPE_POLYGON)}>▱ {$t('geozone.shapePolygon')}</button>
          <span class="gz-spacer"></span>
          <label class="gz-editlock" title={$t('geozone.editLockHint')}>
            <span>{$t('geozone.editLock')}</span>
            <Toggle checked={$geozoneEditing} onchange={(c) => geozoneEditing.set(c)} />
          </label>
        </div>

        {#if !zones.length}
          <div class="gz-empty">{$t('geozone.none')}</div>
        {:else}
          {#each zones as zone (zone.id)}
            {@const inclusive = zone.zone_type === GEOZONE_TYPE_INCLUSIVE}
            {@const circle = zone.shape === GEOZONE_SHAPE_CIRCULAR}
            {@const center = geozoneCenter(zone)}
            <div class="gz-row" style="--gz-color:{geozoneColor(zone)}">
              <button class="gz-rowhead" onclick={() => toggleZone(zone.id)} title={$t('geozone.expand')}>
                <span class="gz-dot"></span>
                <span class="gz-name">{$t('geozone.abbrev')}{zone.id + 1}</span>
                <span class="gz-shape">{circle ? $t('geozone.shapeCircle') : $t('geozone.shapePolygon')}</span>
                <span class="gz-sub">
                  {circle ? fmtRadius(zone) : $t('geozone.vertexCount', { values: { n: zone.vertices.length } })}
                </span>
              </button>
              {#if expandedZone === zone.id}
                <div class="gz-detail">
                  <!-- Type (Inclusive/Exclusive) slide toggle. -->
                  <div class="gz-edit">
                    <span class="gz-elabel">{$t('geozone.type')}</span>
                    <div class="gz-typetoggle">
                      <span class="gz-tname">{inclusive ? $t('geozone.inclusive') : $t('geozone.exclusive')}</span>
                      <Toggle checked={inclusive} onchange={(c) => setGeozoneType(zone.id, c ? GEOZONE_TYPE_INCLUSIVE : GEOZONE_TYPE_EXCLUSIVE)} />
                    </div>
                  </div>
                  <!-- Fence action. -->
                  <div class="gz-edit">
                    <span class="gz-elabel">{$t('geozone.action')}</span>
                    <select class="am-select" value={zone.fence_action} onchange={(e) => setGeozoneAction(zone.id, Number(e.currentTarget.value))}>
                      {#each ACTIONS as a}<option value={a.v}>{a.label()}</option>{/each}
                    </select>
                  </div>
                  <!-- Altitudes (10 m steps; upper 0 = no limit). -->
                  <div class="gz-edit">
                    <span class="gz-elabel">{$t('geozone.lowerAlt')}</span>
                    <NumberStepper bind:value={editMinM} min={0} step={10} unit="m" onchange={() => commitAlts(zone.id)} />
                  </div>
                  <div class="gz-edit">
                    <span class="gz-elabel">{$t('geozone.upperAlt')}</span>
                    <NumberStepper bind:value={editMaxM} min={0} step={10} unit="m" onchange={() => commitAlts(zone.id)} />
                  </div>
                  {#if zone.max_alt_cm === 0}<div class="gz-hint">{$t('geozone.upperUnlimited')}</div>{/if}
                  {#if circle}
                    <div class="gz-edit">
                      <span class="gz-elabel">{$t('geozone.radius')}</span>
                      <NumberStepper bind:value={editRadiusM} min={1} step={10} unit="m" onchange={() => commitRadius(zone.id)} />
                    </div>
                  {/if}
                  <!-- Altitude reference. -->
                  <div class="gz-edit">
                    <span class="gz-elabel">{$t('geozone.reference')}</span>
                    <div class="gz-typetoggle">
                      <span class="gz-tname">{zone.is_sealevel_ref ? 'AMSL' : 'AGL'}</span>
                      <Toggle checked={zone.is_sealevel_ref} onchange={(c) => onSealevelToggle(zone, c)} />
                    </div>
                  </div>
                  {#if terrainNote}<div class="gz-hint">{terrainNote}</div>{/if}
                  <div class="gz-rowactions">
                    {#if center}
                      <button class="gz-focus" onclick={() => focusAero(center.lat, center.lon)}>{$t('geozone.focus')}</button>
                    {/if}
                    <button class="gz-delete" onclick={() => onDeleteZone(zone.id)}>{$t('geozone.delete')}</button>
                  </div>
                </div>
              {/if}
            </div>
          {/each}
        {/if}

        <!-- Sanity issues. -->
        {#if issues.length}
          <div class="gz-issues">
            {#each issues as iss}
              <div class="gz-issue" class:gz-err={iss.level === 'error'}>{$t('geozone.' + iss.key, { values: iss.values })}</div>
            {/each}
          </div>
        {/if}

        <!-- Save / Revert (only when there are pending edits). -->
        {#if $geozoneDirty}
          <div class="gz-save">
            <Button variant="data" icon="save" disabled={busy || hasErrors} onclick={onSave}>
              {busy ? $t('geozone.saving') : $t('geozone.saveToFc')}
            </Button>
            <Button variant="standard" disabled={busy} onclick={onRevert}>{$t('geozone.revert')}</Button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet nearbyList()}
  <div class="am-list">
    <!-- Obstacles -->
    <div class="am-group-head">{$t('airspace.layerObstacles')} <span class="am-group-count">{nearObstacles.length}</span></div>
    {#each nearObstacles as { p, d }}
      <button class="am-item" onclick={() => focusAero(p.lat, p.lon)}>
        <span class="am-item-name">{p.name || p.subtype}</span>
        <span class="am-item-sub">{p.subtype}{p.heightM != null ? ` · ${Math.round(p.heightM)} m` : ''}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}

    <!-- Airspaces -->
    <div class="am-group-head">{$t('airspace.layerAirspaces')} <span class="am-group-count">{nearAirspaces.length}</span></div>
    {#each nearAirspaces as { a, d }}
      <button class="am-item" onclick={() => { const o = a.outlines[0]?.[0]; if (o) focusAero(o[1], o[0]); }}>
        <span class="am-item-name">{a.name}</span>
        <span class="am-item-sub">{a.typeName} · {a.lower.label}–{a.upper.label}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}

    <!-- Airfields -->
    <div class="am-group-head">{$t('airspace.layerAirfields')} <span class="am-group-count">{nearAirfields.length}</span></div>
    {#each nearAirfields as { p, d }}
      <button class="am-item" onclick={() => focusAero(p.lat, p.lon)}>
        <span class="am-item-name">{p.name || p.subtype}</span>
        <span class="am-item-sub">{p.kind === 'rc' ? 'RC' : p.subtype}{aeroPointInfo(p) ? ` · ${aeroPointInfo(p)}` : ''}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}
  </div>
{/snippet}

<style>
  .am-controls { display: flex; flex-direction: column; gap: 4px; }
  .am-head-row, .am-row {
    display: grid;
    grid-template-columns: 1fr 36px 36px;
    align-items: center;
    gap: 8px;
  }
  .am-head-row { padding: 0 2px 2px; }
  .am-dim { font-size: 11px; color: #949494; text-align: center; }
  .am-layer { font-size: 13px; color: #e0e0e0; }
  .am-row { padding: 4px 2px; }

  .am-ranges {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px solid #272727;
  }
  .am-range { display: grid; grid-template-columns: 1fr auto; align-items: center; gap: 8px; }
  .am-range-label { font-size: 12px; color: #949494; }
  .am-select {
    background: #1f1f1f; color: #e0e0e0; border: 1px solid #272727; border-radius: 4px;
    padding: 2px 6px; font-size: 12px; cursor: pointer;
  }

  .am-compact { cursor: pointer; }

  .am-list { display: flex; flex-direction: column; }
  .am-group-head {
    font-size: 11px; font-weight: 700; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px;
    padding: 8px 4px 3px; position: sticky; top: 0; background: #2e2e2e; z-index: 1;
  }
  .am-group-count { color: #949494; font-weight: 600; }
  .am-item {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas: "name dist" "sub dist";
    gap: 0 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-bottom: 1px solid #272727;
    padding: 5px 4px;
    cursor: pointer;
    color: inherit;
  }
  .am-item:hover { background: rgba(55, 168, 219, 0.08); }
  .am-item-name { grid-area: name; font-size: 12px; color: #e0e0e0; }
  .am-item-sub { grid-area: sub; font-size: 10.5px; color: #949494; }
  .am-item-dist { grid-area: dist; align-self: center; font-size: 11px; color: #37a8db; white-space: nowrap; }

  /* Geozones section (FC config) — collapsible zone rows, colour-coded by type. */
  .gz-section { margin-top: 10px; padding-top: 8px; border-top: 1px solid #272727; display: flex; flex-direction: column; gap: 3px; }
  .gz-head { display: flex; align-items: center; justify-content: space-between; padding: 0 2px 4px; }
  .gz-title { font-size: 11px; font-weight: 700; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }
  .gz-count { font-size: 11px; color: #949494; font-weight: 600; }
  .gz-empty { font-size: 12px; color: #949494; padding: 2px 4px 4px; }
  .gz-row { border-left: 3px solid var(--gz-color); background: #272727; border-radius: 3px; overflow: hidden; }
  .gz-rowhead {
    display: grid; grid-template-columns: auto auto 1fr auto; align-items: center; gap: 8px;
    width: 100%; text-align: left; background: none; border: none; padding: 5px 8px; cursor: pointer; color: inherit;
  }
  .gz-rowhead:hover { background: rgba(55, 168, 219, 0.08); }
  .gz-dot { width: 9px; height: 9px; border-radius: 50%; background: var(--gz-color); }
  .gz-name { font-size: 12px; font-weight: 700; color: #e0e0e0; }
  .gz-shape { font-size: 11px; color: #b8b8b8; }
  .gz-sub { font-size: 10.5px; color: #949494; text-align: right; }
  .gz-detail { display: flex; flex-direction: column; gap: 2px; padding: 4px 10px 8px; border-top: 1px solid #1f1f1f; }
  .gz-focus {
    align-self: flex-start; background: none; border: 1px solid #37a8db; color: #37a8db;
    border-radius: 4px; padding: 2px 8px; font-size: 11px; cursor: pointer;
  }
  .gz-focus:hover { background: rgba(55, 168, 219, 0.12); }

  /* Editing toolbar + per-zone edit controls (P2). */
  .gz-toolbar { display: flex; align-items: center; gap: 6px; padding: 2px 2px 6px; }
  .gz-add {
    background: #1f1f1f; color: #e0e0e0; border: 1px solid #272727; border-radius: 4px;
    padding: 3px 8px; font-size: 11px; cursor: pointer;
  }
  .gz-add:hover:not(:disabled) { background: rgba(55, 168, 219, 0.12); border-color: #37a8db; }
  .gz-add:disabled { opacity: 0.4; cursor: not-allowed; }
  .gz-spacer { flex: 1; }
  .gz-editlock { display: inline-flex; align-items: center; gap: 6px; font-size: 11px; color: #949494; }

  .gz-edit { display: grid; grid-template-columns: 1fr auto; align-items: center; gap: 8px; padding: 3px 0; }
  .gz-elabel { font-size: 11.5px; color: #949494; }
  .gz-typetoggle { display: inline-flex; align-items: center; gap: 6px; }
  .gz-tname { font-size: 11.5px; color: #e0e0e0; min-width: 70px; text-align: right; }
  .gz-hint { font-size: 10.5px; color: #949494; font-style: italic; padding: 0 0 3px; }
  .gz-rowactions { display: flex; gap: 6px; margin-top: 6px; }
  .gz-delete {
    background: none; border: 1px solid #d40000; color: #ff5a5a; border-radius: 4px;
    padding: 2px 8px; font-size: 11px; cursor: pointer;
  }
  .gz-delete:hover { background: rgba(212, 0, 0, 0.12); }

  .gz-issues { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; }
  .gz-issue { font-size: 11px; color: #f5a623; }
  .gz-issue.gz-err { color: #ff5a5a; }
  .gz-save { display: flex; gap: 8px; margin-top: 8px; }
</style>
