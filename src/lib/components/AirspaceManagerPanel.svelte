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
  import { settings, AERO_DISTANCE_OPTIONS, type DistanceUnit } from '$lib/stores/settings';
  import { haversineDistance } from '$lib/utils/geo';
  import { aeroPointInfo } from '$lib/helpers/airspaceStyle';
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
    { key: 'obstacles', label: () => $t('airspace.layerObstacles') },
    { key: 'airports', label: () => $t('airspace.layerAirports') },
    { key: 'rc', label: () => $t('airspace.layerRc') },
  ];

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
    {#each LAYERS as layer}
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
</style>
