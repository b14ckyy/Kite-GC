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
    geozoneConfig, GEOZONE_TYPE_INCLUSIVE, GEOZONE_SHAPE_CIRCULAR,
    GEOZONE_ACTION_AVOID, GEOZONE_ACTION_POSHOLD, GEOZONE_ACTION_RTH, type GeoZone,
  } from '$lib/stores/geozone';
  import { settings, AERO_DISTANCE_OPTIONS, type DistanceUnit } from '$lib/stores/settings';
  import { haversineDistance } from '$lib/utils/geo';
  import { aeroPointInfo } from '$lib/helpers/airspaceStyle';
  import { geozoneColor, geozoneRadiusM } from '$lib/helpers/geozoneStyle';
  import { convertDistance, formatConverted } from '$lib/utils/units';

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

  // The Geozones toggle + list only exist when a geozone-capable INAV FC (≥8.0) is connected.
  const hasGeozones = $derived($geozoneConfig?.has_geozones ?? false);
  const visibleLayers = $derived(LAYERS.filter((l) => l.key !== 'geozones' || hasGeozones));

  // Accordion: at most one expanded geozone row.
  let expandedZone = $state<number | null>(null);
  function toggleZone(id: number) { expandedZone = expandedZone === id ? null : id; }

  /** Representative point of a zone (circle centre / polygon centroid) for the focus-on-click. */
  function geozoneCenter(z: GeoZone): { lat: number; lon: number } | null {
    if (z.vertices.length === 0) return null;
    if (z.shape === GEOZONE_SHAPE_CIRCULAR) return { lat: z.vertices[0].lat / 1e7, lon: z.vertices[0].lon / 1e7 };
    let sx = 0, sy = 0;
    for (const v of z.vertices) { sx += v.lat; sy += v.lon; }
    return { lat: sx / z.vertices.length / 1e7, lon: sy / z.vertices.length / 1e7 };
  }
  function fmtAltCm(cm: number): string { return `${Math.round(cm / 100)} m`; }
  function fmtRadius(z: GeoZone): string {
    const r = geozoneRadiusM(z);
    return r != null ? `${Math.round(r)} m` : '—';
  }
  function actionLabel(a: number): string {
    if (a === GEOZONE_ACTION_AVOID) return $t('geozone.actionAvoid');
    if (a === GEOZONE_ACTION_POSHOLD) return $t('geozone.actionPoshold');
    if (a === GEOZONE_ACTION_RTH) return $t('geozone.actionRth');
    return $t('geozone.actionNone');
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
          <span class="gz-count">{$geozoneConfig?.zones.length ?? 0}</span>
        </div>
        {#if !$geozoneConfig?.zones.length}
          <div class="gz-empty">{$t('geozone.none')}</div>
        {:else}
          {#each $geozoneConfig.zones as zone (zone.id)}
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
                  <div class="gz-kv"><span>{$t('geozone.type')}</span><span>{inclusive ? $t('geozone.inclusive') : $t('geozone.exclusive')}</span></div>
                  <div class="gz-kv"><span>{$t('geozone.lowerAlt')}</span><span>{fmtAltCm(zone.min_alt_cm)}</span></div>
                  <div class="gz-kv"><span>{$t('geozone.upperAlt')}</span><span>{zone.max_alt_cm > 0 ? fmtAltCm(zone.max_alt_cm) : '∞'}</span></div>
                  <div class="gz-kv"><span>{$t('geozone.reference')}</span><span>{zone.is_sealevel_ref ? 'AMSL' : 'AGL'}</span></div>
                  <div class="gz-kv"><span>{$t('geozone.action')}</span><span>{actionLabel(zone.fence_action)}</span></div>
                  {#if center}
                    <button class="gz-focus" onclick={() => focusAero(center.lat, center.lon)}>{$t('geozone.focus')}</button>
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
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
  .gz-kv { display: flex; justify-content: space-between; font-size: 11.5px; }
  .gz-kv span:first-child { color: #949494; }
  .gz-kv span:last-child { color: #e0e0e0; }
  .gz-focus {
    margin-top: 4px; align-self: flex-start; background: none; border: 1px solid #37a8db; color: #37a8db;
    border-radius: 4px; padding: 2px 8px; font-size: 11px; cursor: pointer;
  }
  .gz-focus:hover { background: rgba(55, 168, 219, 0.12); }
</style>
